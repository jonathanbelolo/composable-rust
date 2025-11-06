//! Event bus abstraction for cross-aggregate communication.
//!
//! This module provides the [`EventBus`] trait for publishing and subscribing to events
//! across aggregate boundaries. Events flow from the event store (source of truth) through
//! the event bus to enable saga coordination, projections, and other cross-aggregate patterns.
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
//! │  1. Save Event  │
//! │   to Postgres   │◄─── Source of truth
//! │  (event store)  │
//! └────────┬────────┘
//!          │
//!          ▼
//! ┌─────────────────┐
//! │ 2. Publish to   │
//! │    Event Bus    │◄─── At-least-once delivery
//! └────────┬────────┘
//!          │
//!     ┌────┴────┐
//!     │         │
//!     ▼         ▼
//! ┌───────┐ ┌───────┐
//! │ Saga  │ │ Other │
//! │       │ │ Aggr. │
//! └───────┘ └───────┘
//! ```
//!
//! # Key Principles
//!
//! - **Postgres First**: Events are persisted to the event store before publishing
//! - **At-least-once delivery**: Events may be delivered multiple times
//! - **Idempotency**: Subscribers must handle duplicate events
//! - **Ordered within partition**: Events from the same aggregate maintain order
//!
//! # Topic Naming Convention
//!
//! Topics follow the pattern `{aggregate-type}-events`:
//! - `order-events` - All events from Order aggregates
//! - `payment-events` - All events from Payment aggregates
//! - `inventory-events` - All events from Inventory aggregates
//!
//! # Implementations
//!
//! - [`InMemoryEventBus`](../../composable_rust_testing/event_bus/struct.InMemoryEventBus.html) - For testing (fast, synchronous)
//! - [`RedpandaEventBus`](../../composable_rust_redpanda/struct.RedpandaEventBus.html) - For production (Kafka-compatible)
//!
//! # Example
//!
//! ```rust,ignore
//! use composable_rust_core::event_bus::{EventBus, EventStream};
//! use composable_rust_core::event::SerializedEvent;
//!
//! async fn example(event_bus: impl EventBus) {
//!     // Publish an event
//!     let event = SerializedEvent::new("OrderPlaced".to_string(), vec![1, 2, 3], None);
//!     event_bus.publish("order-events", &event).await?;
//!
//!     // Subscribe to events
//!     let mut stream = event_bus.subscribe(&["order-events", "payment-events"]).await?;
//!     while let Some(result) = stream.next().await {
//!         match result {
//!             Ok(event) => println!("Received: {:?}", event.event_type),
//!             Err(e) => eprintln!("Error: {}", e),
//!         }
//!     }
//! }
//! ```

use crate::event::SerializedEvent;
use futures::Stream;
use std::future::Future;
use std::pin::Pin;
use thiserror::Error;

/// Errors that can occur during event bus operations.
#[derive(Error, Debug, Clone)]
pub enum EventBusError {
    /// Failed to connect to the event bus
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Failed to publish an event to a topic
    #[error("Publish failed for topic '{topic}': {reason}")]
    PublishFailed {
        /// The topic that failed
        topic: String,
        /// The reason for failure
        reason: String,
    },

    /// Failed to subscribe to topics
    #[error("Subscription failed for topics {topics:?}: {reason}")]
    SubscriptionFailed {
        /// The topics that failed to subscribe
        topics: Vec<String>,
        /// The reason for failure
        reason: String,
    },

    /// Failed to deserialize an event
    #[error("Deserialization failed: {0}")]
    DeserializationFailed(String),

    /// Topic not found or invalid
    #[error("Invalid topic: {0}")]
    InvalidTopic(String),

    /// Consumer group error
    #[error("Consumer group error: {0}")]
    ConsumerGroupError(String),

    /// Network or transport error
    #[error("Transport error: {0}")]
    TransportError(String),

    /// Generic error for other failures
    #[error("Event bus error: {0}")]
    Other(String),
}

/// Stream of events from subscriptions.
///
/// This type represents an asynchronous stream of [`SerializedEvent`] values,
/// where each item is a `Result` that may contain an event or an error.
///
/// # Examples
///
/// ```rust,ignore
/// use futures::StreamExt;
///
/// let mut stream = event_bus.subscribe(&["order-events"]).await?;
/// while let Some(result) = stream.next().await {
///     match result {
///         Ok(event) => process_event(event),
///         Err(e) => log::error!("Event stream error: {}", e),
///     }
/// }
/// ```
pub type EventStream = Pin<Box<dyn Stream<Item = Result<SerializedEvent, EventBusError>> + Send>>;

/// Trait for event bus implementations.
///
/// The [`EventBus`] trait provides publish/subscribe capabilities for cross-aggregate
/// communication. Events are published to topics and delivered to all subscribers
/// of those topics with at-least-once delivery semantics.
///
/// # Design Principles
///
/// - **Async-first**: All operations are async for non-blocking I/O
/// - **Ordered delivery**: Events maintain order within the same partition
/// - **At-least-once**: Subscribers may receive duplicate events
/// - **Idempotency**: Subscribers must handle duplicates via correlation IDs
///
/// # Thread Safety
///
/// All implementations must be `Send + Sync` to support concurrent access
/// from multiple reducers and effect executors.
///
/// # Examples
///
/// ## Publishing Events
///
/// ```rust,ignore
/// use composable_rust_core::event::SerializedEvent;
///
/// // After persisting to event store, publish to event bus
/// let event = SerializedEvent::new(
///     "OrderPlaced".to_string(),
///     bincode::serialize(&order_placed_event)?,
///     Some(serde_json::json!({ "correlation_id": "saga-123" })),
/// );
///
/// event_bus.publish("order-events", &event).await?;
/// ```
///
/// ## Subscribing to Events
///
/// ```rust,ignore
/// use futures::StreamExt;
///
/// // Subscribe to multiple topics
/// let mut stream = event_bus.subscribe(&[
///     "order-events",
///     "payment-events",
/// ]).await?;
///
/// while let Some(result) = stream.next().await {
///     match result {
///         Ok(event) => {
///             // Process event (idempotent!)
///             process_event(&event)?;
///         }
///         Err(e) => {
///             tracing::error!("Event stream error: {}", e);
///         }
///     }
/// }
/// ```
///
/// ## Idempotency Pattern
///
/// ```rust,ignore
/// fn process_event(event: &SerializedEvent) -> Result<(), Error> {
///     // Check correlation ID to detect duplicates
///     if let Some(metadata) = &event.metadata {
///         if let Some(correlation_id) = metadata.get("correlation_id") {
///             if already_processed(correlation_id)? {
///                 tracing::debug!("Skipping duplicate event: {}", correlation_id);
///                 return Ok(());
///             }
///         }
///     }
///
///     // Process event and mark as processed
///     handle_event(event)?;
///     mark_processed(correlation_id)?;
///     Ok(())
/// }
/// ```
///
/// # Dyn Compatibility
///
/// This trait uses explicit `Pin<Box<dyn Future>>` returns instead of `async fn`
/// to enable trait object usage (`Arc<dyn EventBus>`). This is required for
/// the effect system where reducers create effects that capture the event bus.
pub trait EventBus: Send + Sync {
    /// Publish an event to a topic.
    ///
    /// Events are published with at-least-once semantics. The event may be
    /// delivered to subscribers multiple times, so subscribers must be idempotent.
    ///
    /// # Arguments
    ///
    /// - `topic`: The topic to publish to (e.g., "order-events")
    /// - `event`: The serialized event to publish
    ///
    /// # Errors
    ///
    /// Returns [`EventBusError::PublishFailed`] if the publish operation fails.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let event = SerializedEvent::new(
    ///     "OrderPlaced".to_string(),
    ///     bincode::serialize(&event_data)?,
    ///     Some(serde_json::json!({ "correlation_id": "order-123" })),
    /// );
    ///
    /// event_bus.publish("order-events", &event).await?;
    /// ```
    fn publish(
        &self,
        topic: &str,
        event: &SerializedEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), EventBusError>> + Send + '_>>;

    /// Subscribe to one or more topics and receive a stream of events.
    ///
    /// Returns an [`EventStream`] that yields events from all subscribed topics.
    /// The stream will deliver events with at-least-once semantics.
    ///
    /// # Arguments
    ///
    /// - `topics`: Array of topic names to subscribe to
    ///
    /// # Errors
    ///
    /// Returns [`EventBusError::SubscriptionFailed`] if subscription fails.
    ///
    /// # Consumer Groups
    ///
    /// Implementations typically use consumer groups to enable multiple instances
    /// of the same subscriber to share the workload. Each consumer group receives
    /// its own copy of every event.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use futures::StreamExt;
    ///
    /// // Subscribe to order and payment events
    /// let mut stream = event_bus.subscribe(&["order-events", "payment-events"]).await?;
    ///
    /// // Process events as they arrive
    /// while let Some(result) = stream.next().await {
    ///     match result {
    ///         Ok(event) => {
    ///             match event.event_type.as_str() {
    ///                 "OrderPlaced" => handle_order_placed(&event)?,
    ///                 "PaymentCompleted" => handle_payment_completed(&event)?,
    ///                 _ => {}
    ///             }
    ///         }
    ///         Err(e) => tracing::error!("Stream error: {}", e),
    ///     }
    /// }
    /// ```
    fn subscribe(
        &self,
        topics: &[&str],
    ) -> Pin<Box<dyn Future<Output = Result<EventStream, EventBusError>> + Send + '_>>;
}
