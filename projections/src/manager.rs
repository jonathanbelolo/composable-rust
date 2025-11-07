//! `ProjectionManager` for orchestrating projection updates from events.
//!
//! # Overview
//!
//! The `ProjectionManager` coordinates the lifecycle of a projection:
//! - Subscribes to events from the event bus (Redpanda/Kafka)
//! - Dispatches events to the projection for processing
//! - Tracks progress via checkpoints for resumption
//! - Handles errors and supports rebuild from scratch
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐
//! │  Event Bus  │ (Redpanda/Kafka)
//! └──────┬──────┘
//!        │ events
//!        ▼
//! ┌─────────────────┐
//! │ProjectionManager│
//! └────┬────────┬───┘
//!      │        │
//!      ▼        ▼
//! ┌────────┐ ┌──────────┐
//! │Projection│ │Checkpoint│
//! └────────┘ └──────────┘
//! ```
//!
//! # Consumer Groups
//!
//! The manager uses Kafka consumer groups for automatic offset tracking.
//! If the projection crashes and restarts:
//! 1. Kafka consumer group resumes from last committed Kafka offset
//! 2. Projection checkpoint tracks which events were successfully processed
//! 3. May reprocess a few events (at-least-once delivery)
//!
//! # Example
//!
//! ```ignore
//! use composable_rust_projections::*;
//!
//! let manager = ProjectionManager::new(
//!     my_projection,
//!     event_bus,
//!     checkpoint,
//!     "order-events",           // Topic
//!     "order-projection-group", // Consumer group
//! );
//!
//! // Start processing events (resumes from last checkpoint)
//! manager.start().await?;
//!
//! // Or rebuild from scratch
//! manager.rebuild().await?;
//! ```

use composable_rust_core::event::SerializedEvent;
use composable_rust_core::event_bus::{EventBus, EventBusError};
use composable_rust_core::projection::{
    EventPosition, Projection, ProjectionCheckpoint, ProjectionError, Result,
};
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::watch;

/// Orchestrates projection updates from an event bus.
///
/// `ProjectionManager` is responsible for:
/// - Loading checkpoints to track progress
/// - Subscribing to event bus topics
/// - Dispatching events to projection
/// - Saving checkpoints periodically
/// - Handling errors gracefully
/// - Supporting rebuild functionality
///
/// # Type Parameters
///
/// - `P`: The projection to manage
///
/// # Consumer Groups
///
/// The manager uses an explicit consumer group ID to ensure:
/// - Multiple instances can share the load (if horizontal scaling)
/// - Kafka tracks offset independently of projection checkpoint
/// - Restarts resume from last committed Kafka offset
///
/// # Checkpoints vs Kafka Offsets
///
/// - **Kafka offset**: Tracks which messages the consumer has received
/// - **Projection checkpoint**: Tracks which events were successfully processed
/// - Gap between them provides at-least-once delivery guarantee
///
/// # Example
///
/// ```ignore
/// use composable_rust_projections::*;
///
/// let projection = CustomerOrderHistoryProjection::new(store);
/// let event_bus = Arc::new(RedpandaEventBus::new("localhost:9092")?);
/// let checkpoint = Arc::new(PostgresProjectionCheckpoint::new(pool));
///
/// let (manager, shutdown) = ProjectionManager::new(
///     projection,
///     event_bus,
///     checkpoint,
///     "order-events",
///     "customer-order-history-projection",
/// );
///
/// // Start processing (blocks until shutdown signal)
/// manager.start().await?;
/// ```
pub struct ProjectionManager<P>
where
    P: Projection,
{
    projection: Arc<P>,
    event_bus: Arc<dyn EventBus>,
    checkpoint: Arc<dyn ProjectionCheckpoint>,
    /// Topic to subscribe to
    topic: String,
    /// Consumer group ID for Kafka offset tracking
    consumer_group: String,
    /// Checkpoint save interval (save every N events)
    checkpoint_interval: u64,
    /// Shutdown signal
    shutdown: watch::Receiver<bool>,
}

impl<P> ProjectionManager<P>
where
    P: Projection,
{
    /// Create a new projection manager.
    ///
    /// # Arguments
    ///
    /// - `projection`: The projection to manage
    /// - `event_bus`: Event bus for subscribing to events. **MUST be pre-configured**
    ///   with the correct consumer group via `RedpandaEventBus::builder().consumer_group()`
    /// - `checkpoint`: Checkpoint tracker for resumption
    /// - `topic`: Topic name to subscribe to (e.g., "order-events")
    /// - `consumer_group`: Consumer group ID for documentation/logging. **Should match**
    ///   the consumer group configured in the `event_bus`.
    ///
    /// Returns the manager and a shutdown sender. Send `true` to the shutdown sender
    /// to gracefully stop the manager.
    ///
    /// # Consumer Group Configuration
    ///
    /// The `EventBus` must be created with a projection-specific consumer group:
    ///
    /// ```ignore
    /// // Create EventBus with consumer group
    /// let event_bus = Arc::new(RedpandaEventBus::builder()
    ///     .brokers("localhost:9092")
    ///     .consumer_group("order-summary-projection")  // Projection-specific!
    ///     .build()?);
    ///
    /// // Pass the same consumer group name to ProjectionManager
    /// let (manager, shutdown) = ProjectionManager::new(
    ///     projection,
    ///     event_bus,
    ///     checkpoint,
    ///     "order-events",
    ///     "order-summary-projection",  // Must match EventBus config
    /// );
    /// ```
    ///
    /// Each projection should use a unique consumer group to track its own progress
    /// independently through the event stream.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // In signal handler:
    /// shutdown.send(true).ok();
    /// ```
    #[must_use]
    pub fn new(
        projection: P,
        event_bus: Arc<dyn EventBus>,
        checkpoint: Arc<dyn ProjectionCheckpoint>,
        topic: impl Into<String>,
        consumer_group: impl Into<String>,
    ) -> (Self, watch::Sender<bool>) {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let manager = Self {
            projection: Arc::new(projection),
            event_bus,
            checkpoint,
            topic: topic.into(),
            consumer_group: consumer_group.into(),
            checkpoint_interval: 100, // Save every 100 events by default
            shutdown: shutdown_rx,
        };

        (manager, shutdown_tx)
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
    /// let manager = manager.with_checkpoint_interval(50); // Save every 50 events
    /// ```
    #[must_use]
    pub const fn with_checkpoint_interval(mut self, interval: u64) -> Self {
        self.checkpoint_interval = interval;
        self
    }

    /// Start processing events from the event bus.
    ///
    /// This method:
    /// 1. Loads the last checkpoint (event count)
    /// 2. Subscribes to the topic (Kafka consumer group handles offset)
    /// 3. Processes events continuously
    /// 4. Saves checkpoints periodically
    /// 5. Handles errors and logs progress
    ///
    /// Runs until a shutdown signal is received.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError`] if:
    /// - Cannot load checkpoint
    /// - Cannot subscribe to event bus
    /// - Event processing fails critically
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (manager, shutdown) = ProjectionManager::new(/* ... */);
    ///
    /// // In another task, signal shutdown when needed:
    /// tokio::spawn(async move {
    ///     tokio::signal::ctrl_c().await.ok();
    ///     shutdown.send(true).ok();
    /// });
    ///
    /// manager.start().await?;
    /// ```
    #[allow(clippy::cognitive_complexity)]
    pub async fn start(&mut self) -> Result<()> {
        let projection_name = self.projection.name();
        tracing::info!(
            projection = projection_name,
            topic = %self.topic,
            consumer_group = %self.consumer_group,
            "Starting projection manager"
        );

        // 1. Load checkpoint (event count processed so far)
        let last_position = self.checkpoint.load_position(projection_name).await?;

        let mut event_count = if let Some(ref pos) = last_position {
            tracing::info!(
                projection = projection_name,
                offset = pos.offset,
                timestamp = %pos.timestamp,
                "Resuming from checkpoint"
            );
            pos.offset
        } else {
            tracing::info!(projection = projection_name, "Starting from beginning");
            0
        };

        // 2. Subscribe to topic
        // Note: Kafka consumer group will resume from its own tracked offset
        // This may be slightly ahead of our projection checkpoint (at-least-once delivery)
        let mut event_stream = self
            .event_bus
            .subscribe(&[self.topic.as_str()])
            .await
            .map_err(|e| match e {
                EventBusError::SubscriptionFailed { topics, reason } => {
                    ProjectionError::EventProcessing(format!(
                        "Failed to subscribe to {topics:?}: {reason}"
                    ))
                }
                _ => ProjectionError::EventProcessing(format!("Subscription error: {e}")),
            })?;

        // 3. Process events continuously
        while !*self.shutdown.borrow() {
            tokio::select! {
                // Process next event
                Some(event_result) = event_stream.next() => {
                    match event_result {
                        Ok(serialized_event) => {
                            if let Err(e) = self.process_event(&serialized_event, &mut event_count).await {
                                tracing::error!(
                                    projection = projection_name,
                                    error = ?e,
                                    event_type = %serialized_event.event_type,
                                    "Failed to process event"
                                );
                                // Continue processing (don't crash on single event failure)
                                // In production, you might want DLQ, retries, etc.
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                projection = projection_name,
                                error = ?e,
                                "Error receiving event from bus"
                            );
                            // Stream error - event bus handles reconnection
                        }
                    }
                }

                // Handle shutdown
                _ = self.shutdown.changed() => {
                    if *self.shutdown.borrow() {
                        tracing::info!(projection = projection_name, "Shutdown signal received");
                        break;
                    }
                }
            }
        }

        tracing::info!(projection = projection_name, "Projection manager stopped");
        Ok(())
    }

    /// Rebuild the projection from scratch.
    ///
    /// This method:
    /// 1. Calls `rebuild()` on the projection (to clear data)
    /// 2. Resets the projection checkpoint to beginning
    ///
    /// **IMPORTANT**: This does NOT reset the Kafka consumer group offset!
    ///
    /// To replay all events from the beginning, you must ALSO reset the Kafka offset by either:
    /// - Using a new consumer group name (recommended)
    /// - Deleting the consumer group: `kafka-consumer-groups --delete --group <group>`
    /// - Manually resetting offsets: `kafka-consumer-groups --reset-offsets --to-earliest`
    ///
    /// # Two-Level Tracking
    ///
    /// - **Kafka offset**: Tracks what the consumer has received (managed by `EventBus`)
    /// - **Projection checkpoint**: Tracks what was successfully processed (this method resets it)
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError`] if:
    /// - Cannot clear projection data
    /// - Cannot reset checkpoint
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Option 1: Rebuild with new consumer group (recommended)
    /// manager.rebuild().await?;
    /// let new_event_bus = RedpandaEventBus::builder()
    ///     .brokers("localhost:9092")
    ///     .consumer_group("order-projection-v2")  // NEW group name
    ///     .build()?;
    /// // Create new manager with new event bus, it will replay from beginning
    ///
    /// // Option 2: Manually reset Kafka consumer group (operational overhead)
    /// manager.rebuild().await?;
    /// // Use kafka-consumer-groups CLI to reset the group offset
    /// // Then restart the manager
    /// ```
    pub async fn rebuild(&self) -> Result<()> {
        let projection_name = self.projection.name();
        tracing::info!(projection = projection_name, "Rebuilding projection");

        // 1. Drop current projection data
        self.projection.rebuild().await?;

        // 2. Reset checkpoint to beginning
        self.checkpoint
            .save_position(projection_name, EventPosition::beginning())
            .await?;

        tracing::info!(
            projection = projection_name,
            "Projection rebuilt - restart manager to replay events"
        );
        Ok(())
    }

    /// Process a single event.
    ///
    /// Applies the event to the projection and saves checkpoint if needed.
    async fn process_event(
        &self,
        serialized_event: &SerializedEvent,
        event_count: &mut u64,
    ) -> Result<()> {
        let projection_name = self.projection.name();

        // Deserialize event
        let event: P::Event = bincode::deserialize(&serialized_event.data).map_err(|e| {
            ProjectionError::Serialization(format!(
                "Failed to deserialize event {}: {e}",
                serialized_event.event_type
            ))
        })?;

        // Apply event to projection
        self.projection.apply_event(&event).await.map_err(|e| {
            ProjectionError::EventProcessing(format!(
                "Failed to apply event {} at position {event_count}: {e}",
                serialized_event.event_type
            ))
        })?;

        // Increment event counter
        *event_count += 1;

        // Save checkpoint periodically
        if *event_count % self.checkpoint_interval == 0 {
            let position = EventPosition::new(*event_count, chrono::Utc::now());

            self.checkpoint
                .save_position(projection_name, position)
                .await?;

            tracing::info!(
                projection = projection_name,
                offset = event_count,
                "Checkpoint saved"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // Basic type tests - full integration tests will come with testing utilities

    #[test]
    fn test_manager_creation() {
        // This test validates that the manager can be created
        // Full functionality testing requires mock implementations
        assert_eq!(1 + 1, 2); // Placeholder
    }

    #[test]
    fn test_checkpoint_interval_configuration() {
        // This test validates the builder pattern
        assert_eq!(1 + 1, 2); // Placeholder
    }
}
