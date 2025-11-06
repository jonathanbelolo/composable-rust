//! Redpanda event bus implementation for Composable Rust.
//!
//! This crate provides a production-ready Redpanda-based event bus that implements
//! the [`EventBus`] trait from `composable-rust-core`. It uses rdkafka for Kafka-compatible
//! event streaming.
//!
//! # Why Redpanda?
//!
//! - **Kafka-compatible**: Uses standard Kafka protocol, works with any Kafka-compatible system
//! - **Vendor swappable**: Can use Redpanda, Apache Kafka, AWS MSK, Azure Event Hubs, etc.
//! - **Simpler operations**: Redpanda is easier to deploy and operate than Kafka
//! - **Self-hostable**: Docker, Kubernetes, bare metal - full control
//! - **BSL 1.1 license**: Permits internal use, becomes Apache 2.0 after 4 years
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐
//! │   Command   │
//! └──────┬──────┘
//!        │
//!        ▼
//! ┌─────────────────┐
//! │    Reducer      │
//! └────────┬────────┘
//!          │
//!          ▼
//! ┌─────────────────┐
//! │  1. Postgres    │
//! │   (persist)     │◄─── Source of truth
//! └────────┬────────┘
//!          │
//!          ▼
//! ┌─────────────────┐
//! │  2. Redpanda    │
//! │   (publish)     │◄─── Distribution
//! └────────┬────────┘
//!          │
//!     ┌────┴────┐
//!     │         │
//!     ▼         ▼
//! ┌───────┐ ┌───────┐
//! │ Saga  │ │ Other │
//! └───────┘ └───────┘
//! ```
//!
//! # Delivery Semantics
//!
//! **At-least-once delivery** with manual offset commits:
//! - Events are committed AFTER successful delivery to the subscriber's channel
//! - If the process crashes before commit, events will be redelivered
//! - Subscribers MUST be idempotent (use correlation IDs to detect duplicates)
//! - Ordering is guaranteed within the same partition (same event type)
//!
//! **Configuration options**:
//! - `consumer_group`: Explicit consumer group for load balancing
//! - `buffer_size`: Event buffer (default: 1000) for handling bursts
//! - `auto_offset_reset`: Where to start reading (default: "latest")
//!
//! # Example
//!
//! ```no_run
//! use composable_rust_redpanda::RedpandaEventBus;
//! use composable_rust_core::event_bus::EventBus;
//! use composable_rust_core::event::SerializedEvent;
//! use futures::StreamExt;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create event bus
//! let event_bus = RedpandaEventBus::new("localhost:9092")?;
//!
//! // Publish an event
//! let event = SerializedEvent::new(
//!     "OrderPlaced".to_string(),
//!     vec![1, 2, 3],
//!     None,
//! );
//! event_bus.publish("order-events", &event).await?;
//!
//! // Subscribe to events
//! let mut stream = event_bus.subscribe(&["order-events"]).await?;
//! while let Some(result) = stream.next().await {
//!     match result {
//!         Ok(event) => println!("Received: {:?}", event.event_type),
//!         Err(e) => eprintln!("Error: {}", e),
//!     }
//! }
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use composable_rust_core::event::SerializedEvent;
use composable_rust_core::event_bus::{EventBus, EventBusError, EventStream};
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::message::Message;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::util::Timeout;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// Redpanda event bus implementation.
///
/// This implementation uses rdkafka (Kafka-compatible client) to provide
/// production-ready event streaming with:
///
/// - **At-least-once delivery**: Events may be delivered multiple times
/// - **Ordering within partition**: Events from the same aggregate maintain order
/// - **Consumer groups**: Multiple instances of a subscriber share the workload
/// - **Fault tolerance**: Automatic reconnection and retry
///
/// # Configuration
///
/// The event bus can be configured with:
/// - **Broker addresses**: Bootstrap servers (required)
/// - **Producer settings**: Acks, compression, timeout
/// - **Consumer group**: Explicit ID or auto-generated from topics
/// - **Buffer size**: Event buffer capacity (default: 1000)
/// - **Offset reset**: Where new groups start reading (default: "latest")
///
/// # Performance
///
/// - **Producer**: Async sends with batching for high throughput
/// - **Consumer**: Streaming API for efficient message processing
/// - **Partitioning**: Events partitioned by topic for parallel processing
///
/// # Example
///
/// ```no_run
/// use composable_rust_redpanda::RedpandaEventBus;
/// use composable_rust_core::event_bus::EventBus;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Basic configuration
/// let event_bus = RedpandaEventBus::new("localhost:9092")?;
///
/// // Custom configuration
/// let event_bus = RedpandaEventBus::builder()
///     .brokers("localhost:9092,localhost:9093")
///     .producer_acks("all")  // Wait for all replicas
///     .compression("lz4")
///     .build()?;
/// # Ok(())
/// # }
/// ```
pub struct RedpandaEventBus {
    /// Kafka producer for publishing events
    producer: FutureProducer,
    /// Broker addresses (for creating consumers)
    brokers: String,
    /// Producer timeout
    timeout: Duration,
    /// Consumer group ID (if explicitly set)
    consumer_group: Option<String>,
    /// Event buffer size for subscribers
    buffer_size: usize,
    /// Auto offset reset policy
    auto_offset_reset: String,
}

impl RedpandaEventBus {
    /// Create a new Redpanda event bus with default configuration.
    ///
    /// # Parameters
    ///
    /// - `brokers`: Comma-separated list of broker addresses (e.g., "localhost:9092")
    ///
    /// # Errors
    ///
    /// Returns [`EventBusError::ConnectionFailed`] if:
    /// - Cannot connect to any broker
    /// - Broker addresses are invalid
    /// - Authentication fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// use composable_rust_redpanda::RedpandaEventBus;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let event_bus = RedpandaEventBus::new("localhost:9092")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(brokers: &str) -> Result<Self, EventBusError> {
        Self::builder().brokers(brokers).build()
    }

    /// Create a new builder for configuring the event bus.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use composable_rust_redpanda::RedpandaEventBus;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let event_bus = RedpandaEventBus::builder()
    ///     .brokers("localhost:9092")
    ///     .producer_acks("all")
    ///     .compression("lz4")
    ///     .timeout(std::time::Duration::from_secs(5))
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn builder() -> RedpandaEventBusBuilder {
        RedpandaEventBusBuilder::default()
    }

    /// Get a reference to the brokers string.
    #[must_use]
    pub fn brokers(&self) -> &str {
        &self.brokers
    }
}

/// Builder for configuring a [`RedpandaEventBus`].
///
/// Provides a fluent API for setting producer and consumer configuration.
///
/// # Example
///
/// ```no_run
/// use composable_rust_redpanda::RedpandaEventBus;
/// use std::time::Duration;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let event_bus = RedpandaEventBus::builder()
///     .brokers("localhost:9092,localhost:9093")
///     .producer_acks("all")
///     .compression("lz4")
///     .timeout(Duration::from_secs(10))
///     .build()?;
/// # Ok(())
/// # }
/// ```
#[derive(Default)]
pub struct RedpandaEventBusBuilder {
    brokers: Option<String>,
    producer_acks: Option<String>,
    compression: Option<String>,
    timeout: Option<Duration>,
    consumer_group: Option<String>,
    buffer_size: Option<usize>,
    auto_offset_reset: Option<String>,
}

impl RedpandaEventBusBuilder {
    /// Set the broker addresses.
    ///
    /// # Parameters
    ///
    /// - `brokers`: Comma-separated list of broker addresses (e.g., "localhost:9092")
    #[must_use]
    pub fn brokers(mut self, brokers: impl Into<String>) -> Self {
        self.brokers = Some(brokers.into());
        self
    }

    /// Set the producer acknowledgment mode.
    ///
    /// # Parameters
    ///
    /// - `acks`: "0" (no acks), "1" (leader ack), "all" (all replicas ack)
    ///
    /// Default: "1"
    #[must_use]
    pub fn producer_acks(mut self, acks: impl Into<String>) -> Self {
        self.producer_acks = Some(acks.into());
        self
    }

    /// Set the compression codec.
    ///
    /// # Parameters
    ///
    /// - `compression`: "none", "gzip", "snappy", "lz4", "zstd"
    ///
    /// Default: "none"
    #[must_use]
    pub fn compression(mut self, compression: impl Into<String>) -> Self {
        self.compression = Some(compression.into());
        self
    }

    /// Set the producer send timeout.
    ///
    /// Default: 5 seconds
    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set the consumer group ID for subscriptions.
    ///
    /// If not set, the consumer group will be auto-generated based on subscribed topics.
    /// Setting an explicit consumer group ID allows multiple instances of your service
    /// to share the workload (consumer group semantics).
    ///
    /// # Parameters
    ///
    /// - `consumer_group`: The consumer group ID (e.g., "my-service-orders")
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use composable_rust_redpanda::RedpandaEventBus;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let event_bus = RedpandaEventBus::builder()
    ///     .brokers("localhost:9092")
    ///     .consumer_group("payment-saga-coordinator")
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn consumer_group(mut self, consumer_group: impl Into<String>) -> Self {
        self.consumer_group = Some(consumer_group.into());
        self
    }

    /// Set the event buffer size for subscriptions.
    ///
    /// This controls how many events can be buffered in memory between the Kafka
    /// consumer and the subscriber. Larger buffers provide more tolerance for
    /// temporary processing slowdowns, but use more memory.
    ///
    /// # Parameters
    ///
    /// - `buffer_size`: Number of events to buffer (default: 1000)
    ///
    /// # Panics
    ///
    /// Panics if `buffer_size` is 0.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use composable_rust_redpanda::RedpandaEventBus;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let event_bus = RedpandaEventBus::builder()
    ///     .brokers("localhost:9092")
    ///     .buffer_size(5000)  // Large buffer for high-throughput
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn buffer_size(mut self, buffer_size: usize) -> Self {
        assert!(buffer_size > 0, "buffer_size must be greater than 0");
        self.buffer_size = Some(buffer_size);
        self
    }

    /// Set the auto offset reset policy for new consumer groups.
    ///
    /// Controls where new consumer groups start reading when no committed offset exists:
    /// - `"earliest"`: Start from the beginning of the topic
    /// - `"latest"`: Start from the end (only new events)
    /// - `"error"`: Throw error if no offset exists
    ///
    /// # Parameters
    ///
    /// - `policy`: The offset reset policy (default: "latest")
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use composable_rust_redpanda::RedpandaEventBus;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let event_bus = RedpandaEventBus::builder()
    ///     .brokers("localhost:9092")
    ///     .auto_offset_reset("earliest")  // Process historical events
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn auto_offset_reset(mut self, policy: impl Into<String>) -> Self {
        self.auto_offset_reset = Some(policy.into());
        self
    }

    /// Build the [`RedpandaEventBus`].
    ///
    /// # Errors
    ///
    /// Returns [`EventBusError::ConnectionFailed`] if:
    /// - Brokers not set
    /// - Cannot create producer
    /// - Invalid configuration
    pub fn build(self) -> Result<RedpandaEventBus, EventBusError> {
        let brokers = self.brokers.ok_or_else(|| EventBusError::ConnectionFailed(
            "Brokers not configured".to_string(),
        ))?;

        // Create producer configuration
        let mut producer_config = ClientConfig::new();
        producer_config
            .set("bootstrap.servers", &brokers)
            .set("message.timeout.ms", "5000")
            .set("acks", self.producer_acks.as_deref().unwrap_or("1"))
            .set("compression.type", self.compression.as_deref().unwrap_or("none"));

        // Create producer
        let producer: FutureProducer = producer_config.create().map_err(|e| {
            EventBusError::ConnectionFailed(format!("Failed to create producer: {e}"))
        })?;

        tracing::info!(
            brokers = %brokers,
            acks = self.producer_acks.as_deref().unwrap_or("1"),
            compression = self.compression.as_deref().unwrap_or("none"),
            buffer_size = self.buffer_size.unwrap_or(1000),
            auto_offset_reset = self.auto_offset_reset.as_deref().unwrap_or("latest"),
            "RedpandaEventBus created successfully"
        );

        Ok(RedpandaEventBus {
            producer,
            brokers,
            timeout: self.timeout.unwrap_or(Duration::from_secs(5)),
            consumer_group: self.consumer_group,
            buffer_size: self.buffer_size.unwrap_or(1000),
            auto_offset_reset: self.auto_offset_reset.unwrap_or_else(|| "latest".to_string()),
        })
    }
}

impl EventBus for RedpandaEventBus {
    fn publish(
        &self,
        topic: &str,
        event: &SerializedEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), EventBusError>> + Send + '_>> {
        // Clone data before moving into async block
        let topic = topic.to_string();
        let event = event.clone();
        let timeout = self.timeout;

        Box::pin(async move {
            // Serialize event using bincode
            let payload = bincode::serialize(&event).map_err(|e| {
                EventBusError::PublishFailed {
                    topic: topic.clone(),
                    reason: format!("Failed to serialize event: {e}"),
                }
            })?;

            // Use event_type as the message key for partitioning
            // Events of the same type go to the same partition (ordering guarantee)
            let key = event.event_type.as_bytes();

            // Create Kafka record
            let record = FutureRecord::to(&topic)
                .payload(&payload)
                .key(key);

            // Send the message
            let send_result = self
                .producer
                .send(record, Timeout::After(timeout))
                .await;

            match send_result {
                Ok((partition, offset)) => {
                    tracing::debug!(
                        topic = %topic,
                        partition = partition,
                        offset = offset,
                        event_type = %event.event_type,
                        "Event published successfully"
                    );
                    Ok(())
                },
                Err((kafka_error, _)) => {
                    tracing::error!(
                        topic = %topic,
                        error = %kafka_error,
                        "Failed to publish event"
                    );
                    Err(EventBusError::PublishFailed {
                        topic,
                        reason: kafka_error.to_string(),
                    })
                },
            }
        })
    }

    #[allow(clippy::too_many_lines)] // Complexity required for at-least-once delivery with manual commits
    fn subscribe(
        &self,
        topics: &[&str],
    ) -> Pin<Box<dyn Future<Output = Result<EventStream, EventBusError>> + Send + '_>> {
        // Clone configuration before moving into async block
        let topics: Vec<String> = topics.iter().map(|s| (*s).to_string()).collect();
        let brokers = self.brokers.clone();
        let consumer_group = self.consumer_group.clone();
        let buffer_size = self.buffer_size;
        let auto_offset_reset = self.auto_offset_reset.clone();

        Box::pin(async move {
            // Determine consumer group ID
            // If explicitly set, use it; otherwise generate from sorted topics
            let consumer_group_id = if let Some(group) = consumer_group {
                group
            } else {
                // Sort topics for deterministic consumer group naming
                let mut sorted_topics = topics.clone();
                sorted_topics.sort();
                format!("composable-rust-{}", sorted_topics.join("-"))
            };

            // Create consumer configuration with manual commit for at-least-once delivery
            let consumer: StreamConsumer = ClientConfig::new()
                .set("bootstrap.servers", &brokers)
                .set("group.id", &consumer_group_id)
                .set("enable.auto.commit", "false")  // Manual commit for at-least-once
                .set("auto.offset.reset", &auto_offset_reset)
                .set("session.timeout.ms", "6000")
                .set("enable.partition.eof", "false")
                .create()
                .map_err(|e| EventBusError::SubscriptionFailed {
                    topics: topics.clone(),
                    reason: format!("Failed to create consumer: {e}"),
                })?;

            // Subscribe to topics
            let topic_refs: Vec<&str> = topics.iter().map(String::as_str).collect();
            consumer.subscribe(&topic_refs).map_err(|e| {
                EventBusError::SubscriptionFailed {
                    topics: topics.clone(),
                    reason: format!("Failed to subscribe to topics: {e}"),
                }
            })?;

            tracing::info!(
                topics = ?topics,
                consumer_group = %consumer_group_id,
                buffer_size = buffer_size,
                auto_offset_reset = %auto_offset_reset,
                manual_commit = true,
                "Subscribed to topics"
            );

            // Create a channel for forwarding messages
            // Buffer size is configurable to handle bursts and slow subscribers
            let (tx, rx) = tokio::sync::mpsc::channel(buffer_size);

            // Spawn a task that owns the consumer and forwards messages
            tokio::spawn(async move {
                use futures::StreamExt;
                use rdkafka::consumer::CommitMode;

                let mut stream = consumer.stream();

                while let Some(msg_result) = stream.next().await {
                    match msg_result {
                        Ok(message) => {
                            // Process the message
                            let event_result = {
                                // Get payload
                                let Some(payload) = message.payload() else {
                                    // No payload - send error but continue
                                    let err = EventBusError::DeserializationFailed(
                                        "Message has no payload".to_string(),
                                    );
                                    if tx.send(Err(err)).await.is_err() {
                                        break; // Receiver dropped
                                    }
                                    // Commit even failed messages to avoid reprocessing
                                    if let Err(e) = consumer.commit_message(&message, CommitMode::Async) {
                                        tracing::warn!(
                                            error = %e,
                                            "Failed to commit message with no payload"
                                        );
                                    }
                                    continue;
                                };

                                // Deserialize event
                                match bincode::deserialize::<SerializedEvent>(payload) {
                                    Ok(event) => {
                                        tracing::trace!(
                                            topic = message.topic(),
                                            partition = message.partition(),
                                            offset = message.offset(),
                                            event_type = %event.event_type,
                                            "Received event"
                                        );
                                        Ok(event)
                                    },
                                    Err(e) => {
                                        Err(EventBusError::DeserializationFailed(format!(
                                            "Failed to deserialize event: {e}"
                                        )))
                                    },
                                }
                            };

                            // Forward to channel
                            // CRITICAL: Only commit AFTER successful send to channel
                            if tx.send(event_result).await.is_err() {
                                tracing::debug!("Channel receiver dropped, exiting consumer task");
                                break; // Receiver dropped, exit WITHOUT committing
                            }

                            // Commit offset AFTER successful delivery to channel
                            // This provides at-least-once semantics: if we crash before commit,
                            // the message will be redelivered
                            if let Err(e) = consumer.commit_message(&message, CommitMode::Async) {
                                tracing::warn!(
                                    topic = message.topic(),
                                    partition = message.partition(),
                                    offset = message.offset(),
                                    error = %e,
                                    "Failed to commit offset (message may be redelivered)"
                                );
                                // Don't break - continue processing even if commit fails
                                // This is safer than stopping, though may cause duplicates
                            }
                        },
                        Err(e) => {
                            // Kafka error - send to stream and continue
                            let err = EventBusError::TransportError(format!(
                                "Failed to receive message: {e}"
                            ));
                            if tx.send(Err(err)).await.is_err() {
                                break; // Receiver dropped
                            }
                        },
                    }
                }

                tracing::debug!("Consumer task exiting");
            });

            // Create stream from channel receiver
            let stream = async_stream::stream! {
                let mut rx = rx;
                while let Some(result) = rx.recv().await {
                    yield result;
                }
            };

            Ok(Box::pin(stream) as EventStream)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redpanda_event_bus_is_send_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}

        assert_send::<RedpandaEventBus>();
        assert_sync::<RedpandaEventBus>();
    }

    #[test]
    fn builder_default_works() {
        let _builder = RedpandaEventBus::builder();
    }
}
