//! Generic event bus consumer with automatic reconnection.
//!
//! This module provides `EventConsumer`, a generic consumer that handles all the
//! boilerplate of subscribing to an event bus, processing events, handling errors,
//! reconnecting on failures, and coordinating graceful shutdown.
//!
//! # Design Philosophy
//!
//! The `EventConsumer` is a **framework-level abstraction** that eliminates
//! hundreds of lines of duplicated code across different event consumers in
//! an application. Instead of hand-writing subscribe-process-reconnect loops
//! for each consumer, you implement a simple `EventHandler` trait and let
//! `EventConsumer` handle all the infrastructure concerns.
//!
//! # Pattern: Subscribe-Process-Reconnect Loop
//!
//! The consumer implements a resilient pattern for consuming events:
//!
//! ```text
//! loop {
//!     try_subscribe:
//!         loop {
//!             process_events:
//!                 - Handle event
//!                 - Log errors (don't crash)
//!                 - Check shutdown signal
//!         }
//!         if connection_lost:
//!             wait_and_retry
//! }
//! ```
//!
//! # Features
//!
//! - **Automatic reconnection**: If the event bus connection drops, automatically
//!   retry after a configurable delay
//! - **Graceful shutdown**: Respects shutdown signals and exits cleanly
//! - **Error resilience**: Logs errors but continues processing subsequent events
//! - **Structured logging**: All operations traced with consumer name and topic
//!
//! # Example
//!
//! ```rust,ignore
//! use ticketing::runtime::{EventConsumer, EventHandler};
//!
//! // Create a handler (implements EventHandler trait)
//! let handler = Arc::new(MyHandler::new());
//!
//! // Create consumer
//! let consumer = EventConsumer::builder()
//!     .name("my-consumer")
//!     .topics(vec!["my-topic".to_string()])
//!     .event_bus(event_bus)
//!     .handler(handler)
//!     .shutdown(shutdown_rx)
//!     .build();
//!
//! // Spawn as background task
//! let handle = consumer.spawn();
//! ```

use super::EventHandler;
use composable_rust_core::{event::SerializedEvent, event_bus::EventBus};
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

/// Generic event bus consumer.
///
/// This struct encapsulates all the boilerplate of consuming events from an
/// event bus: subscribing with retry, processing events with error handling,
/// reconnecting on stream end, and coordinating graceful shutdown.
///
/// # Thread Safety
///
/// `EventConsumer` is `Send` so it can be moved into a tokio task for background
/// execution via `spawn()`.
///
/// # Lifecycle
///
/// 1. Created via `builder()` or `new()`
/// 2. Spawned as background task via `spawn()`
/// 3. Runs until:
///    - Shutdown signal received
///    - Fatal error (rare - most errors are retried)
///
/// # Configuration
///
/// - `name`: Human-readable consumer name (for logging)
/// - `topics`: List of event bus topics to subscribe to
/// - `event_bus`: Event bus instance to consume from
/// - `handler`: Handler that processes each event
/// - `shutdown`: Broadcast receiver for graceful shutdown coordination
/// - `retry_delay`: How long to wait before retrying on failure (default: 5s)
pub struct EventConsumer {
    /// Consumer name (for logging and monitoring)
    name: String,

    /// Topics to subscribe to
    topics: Vec<String>,

    /// Event bus to consume from
    event_bus: Arc<dyn EventBus>,

    /// Handler for processing events
    handler: Arc<dyn EventHandler>,

    /// Shutdown signal receiver
    shutdown: broadcast::Receiver<()>,

    /// Retry delay on connection failure (default: 5 seconds)
    retry_delay: Duration,
}

impl EventConsumer {
    /// Create a new event consumer.
    ///
    /// # Arguments
    ///
    /// * `name` - Consumer name for logging (e.g., "inventory", "sales-analytics")
    /// * `topics` - List of event bus topics to subscribe to
    /// * `event_bus` - Event bus instance
    /// * `handler` - Handler that processes events
    /// * `shutdown` - Broadcast receiver for graceful shutdown
    ///
    /// # Returns
    ///
    /// A new `EventConsumer` with default retry delay (5 seconds).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let consumer = EventConsumer::new(
    ///     "inventory",
    ///     vec!["inventory".to_string()],
    ///     event_bus,
    ///     handler,
    ///     shutdown_rx,
    /// );
    /// ```
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        topics: Vec<String>,
        event_bus: Arc<dyn EventBus>,
        handler: Arc<dyn EventHandler>,
        shutdown: broadcast::Receiver<()>,
    ) -> Self {
        Self {
            name: name.into(),
            topics,
            event_bus,
            handler,
            shutdown,
            retry_delay: Duration::from_secs(5),
        }
    }

    /// Create a builder for configuring a consumer.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let consumer = EventConsumer::builder()
    ///     .name("my-consumer")
    ///     .topics(vec!["topic1".to_string()])
    ///     .event_bus(event_bus)
    ///     .handler(handler)
    ///     .shutdown(shutdown_rx)
    ///     .retry_delay(Duration::from_secs(10))
    ///     .build();
    /// ```
    #[must_use]
    pub fn builder() -> EventConsumerBuilder {
        EventConsumerBuilder::default()
    }

    /// Set custom retry delay.
    ///
    /// # Arguments
    ///
    /// * `delay` - Duration to wait before retrying after failure
    ///
    /// # Returns
    ///
    /// Self for method chaining.
    #[must_use]
    pub fn with_retry_delay(mut self, delay: Duration) -> Self {
        self.retry_delay = delay;
        self
    }

    /// Spawn the consumer as a background task.
    ///
    /// This starts the subscribe-process-reconnect loop in a separate tokio task.
    /// The task will run until a shutdown signal is received or a fatal error occurs.
    ///
    /// # Returns
    ///
    /// A `JoinHandle` that can be awaited to wait for consumer shutdown.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let handle = consumer.spawn();
    ///
    /// // Later, wait for consumer to finish
    /// handle.await?;
    /// ```
    #[must_use]
    pub fn spawn(mut self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.run().await;
        })
    }

    /// Run the consumer (internal implementation).
    ///
    /// This is the main event loop that:
    /// 1. Subscribes to event bus topics
    /// 2. Processes events from the stream
    /// 3. Handles errors and reconnects
    /// 4. Responds to shutdown signals
    async fn run(&mut self) {
        info!(consumer = %self.name, "Event consumer started");

        // Main loop: try to subscribe and process events
        loop {
            // Convert topics to string slice for subscription
            let topics: Vec<&str> = self.topics.iter().map(String::as_str).collect();

            // Check for shutdown signal before attempting subscription
            tokio::select! {
                _ = self.shutdown.recv() => {
                    info!(consumer = %self.name, "Event consumer received shutdown signal");
                    break;
                }
                subscribe_result = self.event_bus.subscribe(&topics) => {
                    match subscribe_result {
                        Ok(mut stream) => {
                            info!(consumer = %self.name, topics = ?self.topics, "Subscribed to event bus");

                            // Process events from stream until it ends or shutdown
                            if let Err(e) = self.process_stream(&mut stream).await {
                                error!(consumer = %self.name, error = %e, "Error processing stream");
                            }

                            // Stream ended - reconnect after delay
                            warn!(consumer = %self.name, "Event stream ended, reconnecting in {:?}", self.retry_delay);
                            tokio::time::sleep(self.retry_delay).await;
                        }
                        Err(e) => {
                            error!(
                                consumer = %self.name,
                                error = %e,
                                "Failed to subscribe to event bus, retrying in {:?}",
                                self.retry_delay
                            );
                            tokio::time::sleep(self.retry_delay).await;
                        }
                    }
                }
            }
        }

        info!(consumer = %self.name, "Event consumer stopped");
    }

    /// Process events from the stream until it ends or shutdown signal received.
    ///
    /// # Arguments
    ///
    /// * `stream` - Event stream from event bus
    ///
    /// # Returns
    ///
    /// - `Ok(())` if stream ended naturally or shutdown signal received
    /// - `Err(...)` if fatal error occurred (rare)
    async fn process_stream<S, E>(
        &mut self,
        stream: &mut S,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        S: futures::Stream<Item = Result<SerializedEvent, E>> + Unpin + Send,
        E: std::error::Error + 'static,
    {
        loop {
            tokio::select! {
                // Check for shutdown signal
                _ = self.shutdown.recv() => {
                    info!(consumer = %self.name, "Event consumer received shutdown signal during processing");
                    return Ok(());
                }
                // Process next event
                event_result = stream.next() => {
                    match event_result {
                        Some(result) => {
                            match result {
                                Ok(serialized_event) => {
                                    // Process event through handler
                                    if let Err(e) = self.handler.handle(&serialized_event.data).await {
                                        error!(
                                            consumer = %self.name,
                                            error = %e,
                                            "Failed to handle event"
                                        );
                                        // Continue processing subsequent events despite error
                                    }
                                }
                                Err(e) => {
                                    error!(
                                        consumer = %self.name,
                                        error = %e,
                                        "Error receiving event from stream"
                                    );
                                    // Continue processing
                                }
                            }
                        }
                        None => {
                            // Stream ended naturally
                            warn!(consumer = %self.name, "Event stream ended");
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
}

/// Builder for configuring an `EventConsumer`.
///
/// Provides a fluent API for constructing consumers with custom configuration.
#[derive(Default)]
pub struct EventConsumerBuilder {
    name: Option<String>,
    topics: Option<Vec<String>>,
    event_bus: Option<Arc<dyn EventBus>>,
    handler: Option<Arc<dyn EventHandler>>,
    shutdown: Option<broadcast::Receiver<()>>,
    retry_delay: Option<Duration>,
}

impl EventConsumerBuilder {
    /// Set consumer name.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set topics to subscribe to.
    #[must_use]
    pub fn topics(mut self, topics: Vec<String>) -> Self {
        self.topics = Some(topics);
        self
    }

    /// Set event bus instance.
    #[must_use]
    pub fn event_bus(mut self, event_bus: Arc<dyn EventBus>) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    /// Set event handler.
    #[must_use]
    pub fn handler(mut self, handler: Arc<dyn EventHandler>) -> Self {
        self.handler = Some(handler);
        self
    }

    /// Set shutdown signal receiver.
    #[must_use]
    pub fn shutdown(mut self, shutdown: broadcast::Receiver<()>) -> Self {
        self.shutdown = Some(shutdown);
        self
    }

    /// Set custom retry delay (default: 5 seconds).
    #[must_use]
    pub fn retry_delay(mut self, delay: Duration) -> Self {
        self.retry_delay = Some(delay);
        self
    }

    /// Build the `EventConsumer`.
    ///
    /// # Panics
    ///
    /// Panics if required fields are not set (name, topics, event_bus, handler, shutdown).
    #[must_use]
    pub fn build(self) -> EventConsumer {
        EventConsumer {
            name: self.name.expect("name is required"),
            topics: self.topics.expect("topics are required"),
            event_bus: self.event_bus.expect("event_bus is required"),
            handler: self.handler.expect("handler is required"),
            shutdown: self.shutdown.expect("shutdown is required"),
            retry_delay: self.retry_delay.unwrap_or_else(|| Duration::from_secs(5)),
        }
    }
}
