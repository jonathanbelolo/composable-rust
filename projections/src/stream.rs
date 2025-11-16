//! Projection event stream utilities.
//!
//! Provides `ProjectionStream` - a type-agnostic helper for consuming events
//! from an event bus with checkpoint tracking. The client handles deserialization
//! since only they know the concrete event types.
//!
//! # Design Philosophy
//!
//! This module takes a **layered approach**:
//! - **Transport layer** (this module): Handles event bus subscription, checkpointing, retries
//! - **Business logic** (client code): Handles deserialization and projection updates
//!
//! By separating concerns, we avoid fighting Rust's type system. The transport
//! layer works with `SerializedEvent` (raw bytes), while the client deserializes
//! to their specific event types.
//!
//! # Example
//!
//! ```ignore
//! use composable_rust_projections::ProjectionStream;
//!
//! let mut stream = ProjectionStream::new(
//!     event_bus,
//!     checkpoint,
//!     "order-events",
//!     "order-projection-group",
//!     "order-projection",
//! ).await?;
//!
//! // Client loop with type-specific deserialization
//! while let Some(result) = stream.next().await {
//!     let serialized = result?;
//!
//!     // Client knows the concrete type!
//!     let event: OrderEvent = bincode::deserialize(&serialized.data)?;
//!
//!     // Update projection
//!     projection.handle_event(&event)?;
//!
//!     // Commit checkpoint
//!     stream.commit().await?;
//! }
//! ```

use composable_rust_core::event::SerializedEvent;
use composable_rust_core::event_bus::{EventBus, EventBusError};
use composable_rust_core::projection::{EventPosition, ProjectionCheckpoint, ProjectionError};
use futures::stream::StreamExt;
use std::sync::Arc;

/// Result type for projection operations.
pub type Result<T> = std::result::Result<T, ProjectionError>;

/// Manages event stream consumption with checkpoint tracking.
///
/// `ProjectionStream` handles the transport layer for projections:
/// - Subscribes to event bus topics
/// - Manages Kafka consumer groups
/// - Tracks checkpoints for resumption
/// - Provides graceful error handling
///
/// The client is responsible for:
/// - Deserializing events (they know the concrete types)
/// - Applying events to projections
/// - Calling `commit()` after successful processing
///
/// # Architecture
///
/// ```text
/// ┌─────────────┐
/// │  Event Bus  │ (Redpanda/Kafka)
/// └──────┬──────┘
///        │ SerializedEvent
///        ▼
/// ┌─────────────────┐
/// │ProjectionStream │ (this module)
/// └────┬────────────┘
///      │ SerializedEvent (raw bytes)
///      ▼
/// ┌─────────────────┐
/// │  Client Code    │ (knows concrete types)
/// └─────────────────┘
///   - Deserializes to OrderEvent, InventoryEvent, etc.
///   - Updates projection
///   - Calls stream.commit()
/// ```
///
/// # Checkpoints vs Kafka Offsets
///
/// - **Kafka offset**: Tracks which messages the consumer received (managed by `EventBus`)
/// - **Projection checkpoint**: Tracks which events were successfully processed (managed here)
/// - Gap between them provides at-least-once delivery guarantee
///
/// # Example
///
/// ```ignore
/// let mut stream = ProjectionStream::new(
///     event_bus,
///     checkpoint,
///     "inventory-events",
///     "available-seats-projection",
///     "available-seats",
/// ).await?;
///
/// while let Some(result) = stream.next().await {
///     match result {
///         Ok(serialized) => {
///             // Deserialize to concrete type
///             let event: InventoryEvent = bincode::deserialize(&serialized.data)?;
///
///             // Apply to projection
///             projection.handle_event(&event)?;
///
///             // Commit checkpoint
///             stream.commit().await?;
///         }
///         Err(e) => {
///             tracing::error!("Stream error: {e}");
///             tokio::time::sleep(Duration::from_secs(5)).await;
///         }
///     }
/// }
/// ```
pub struct ProjectionStream {
    /// Event bus subscription stream
    event_stream: futures::stream::BoxStream<'static, std::result::Result<SerializedEvent, EventBusError>>,
    /// Checkpoint tracker
    checkpoint: Arc<dyn ProjectionCheckpoint>,
    /// Projection name (for checkpoint key)
    projection_name: String,
    /// Current position (event count)
    position: u64,
    /// Checkpoint save interval (save every N events)
    checkpoint_interval: u64,
    /// Events processed since last checkpoint
    events_since_checkpoint: u64,
}

impl ProjectionStream {
    /// Create a new projection stream.
    ///
    /// # Arguments
    ///
    /// - `event_bus`: Event bus to subscribe to (must be configured with consumer group)
    /// - `checkpoint`: Checkpoint tracker for resumption
    /// - `topic`: Topic name to subscribe to
    /// - `consumer_group`: Consumer group ID for Kafka offset tracking
    /// - `projection_name`: Name for checkpoint storage (usually same as consumer group)
    ///
    /// # Consumer Group Setup
    ///
    /// The `event_bus` should be created with a projection-specific consumer group:
    ///
    /// ```ignore
    /// let event_bus = Arc::new(RedpandaEventBus::builder()
    ///     .brokers("localhost:9092")
    ///     .consumer_group("order-summary-projection")
    ///     .build()?);
    /// ```
    ///
    /// Each projection should use a unique consumer group to track progress independently.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Cannot load checkpoint from storage
    /// - Cannot subscribe to event bus
    ///
    /// # Example
    ///
    /// ```ignore
    /// let stream = ProjectionStream::new(
    ///     event_bus,
    ///     checkpoint,
    ///     "order-events",
    ///     "order-projection-group",
    ///     "order-projection",
    /// ).await?;
    /// ```
    pub async fn new(
        event_bus: Arc<dyn EventBus>,
        checkpoint: Arc<dyn ProjectionCheckpoint>,
        topic: impl Into<String>,
        consumer_group: impl Into<String>,
        projection_name: impl Into<String>,
    ) -> Result<Self> {
        let topic = topic.into();
        let consumer_group = consumer_group.into();
        let projection_name = projection_name.into();

        // Load checkpoint to get starting position
        let last_position = checkpoint.load_position(&projection_name).await?;

        let position = if let Some(ref pos) = last_position {
            tracing::info!(
                projection = %projection_name,
                offset = pos.offset,
                timestamp = %pos.timestamp,
                "Resuming from checkpoint"
            );
            pos.offset
        } else {
            tracing::info!(projection = %projection_name, "Starting from beginning");
            0
        };

        // Subscribe to topic
        let event_stream = event_bus
            .subscribe(&[topic.as_str()])
            .await
            .map_err(|e| match e {
                EventBusError::SubscriptionFailed { topics, reason } => {
                    ProjectionError::EventProcessing(format!(
                        "Failed to subscribe to {topics:?}: {reason}"
                    ))
                }
                _ => ProjectionError::EventProcessing(format!("Subscription error: {e}")),
            })?;

        tracing::info!(
            projection = %projection_name,
            topic = %topic,
            consumer_group = %consumer_group,
            "Projection stream initialized"
        );

        Ok(Self {
            event_stream: event_stream.boxed(),
            checkpoint,
            projection_name,
            position,
            checkpoint_interval: 100, // Save every 100 events by default
            events_since_checkpoint: 0,
        })
    }

    /// Set the checkpoint save interval.
    ///
    /// Determines how often checkpoints are saved (every N events).
    /// Lower values = more frequent saves = better resumption granularity but more I/O.
    ///
    /// # Arguments
    ///
    /// - `interval`: Number of events between checkpoint saves
    ///
    /// # Example
    ///
    /// ```ignore
    /// let stream = stream.with_checkpoint_interval(50); // Save every 50 events
    /// ```
    #[must_use]
    pub const fn with_checkpoint_interval(mut self, interval: u64) -> Self {
        self.checkpoint_interval = interval;
        self
    }

    /// Get the next event from the stream.
    ///
    /// Returns `None` if the stream ends (should not happen in normal operation).
    /// Returns `Err` if there's a transport-level error (event bus disconnection, etc.).
    ///
    /// The client is responsible for deserializing the returned `SerializedEvent`
    /// to their concrete event type.
    ///
    /// # Example
    ///
    /// ```ignore
    /// while let Some(result) = stream.next().await {
    ///     let serialized = result?;
    ///     let event: MyEvent = bincode::deserialize(&serialized.data)?;
    ///     // ... process event ...
    ///     stream.commit().await?;
    /// }
    /// ```
    pub async fn next(&mut self) -> Option<std::result::Result<SerializedEvent, EventBusError>> {
        self.event_stream.next().await
    }

    /// Commit the current position to the checkpoint.
    ///
    /// Call this after successfully processing an event. Checkpoints are saved
    /// periodically based on `checkpoint_interval` (not on every commit call).
    ///
    /// # Errors
    ///
    /// Returns error if checkpoint save fails.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let serialized = stream.next().await?.ok_or("stream ended")?;
    /// let event: MyEvent = bincode::deserialize(&serialized.data)?;
    /// projection.handle_event(&event)?;
    /// stream.commit().await?; // Save checkpoint if interval reached
    /// ```
    pub async fn commit(&mut self) -> Result<()> {
        self.position += 1;
        self.events_since_checkpoint += 1;

        // Save checkpoint periodically
        if self.events_since_checkpoint >= self.checkpoint_interval {
            let position = EventPosition::new(self.position, chrono::Utc::now());

            self.checkpoint
                .save_position(&self.projection_name, position)
                .await?;

            tracing::debug!(
                projection = %self.projection_name,
                offset = self.position,
                "Checkpoint saved"
            );

            self.events_since_checkpoint = 0;
        }

        Ok(())
    }

    /// Get the current position (event count).
    ///
    /// Useful for logging and monitoring.
    #[must_use]
    pub const fn position(&self) -> u64 {
        self.position
    }

    /// Get the projection name.
    #[must_use]
    pub fn projection_name(&self) -> &str {
        &self.projection_name
    }
}
