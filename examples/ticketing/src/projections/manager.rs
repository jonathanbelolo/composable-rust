//! Projection manager setup for the ticketing system.
//!
//! This module starts projection consumers that consume events from the event
//! bus and update `PostgreSQL` projections using `ProjectionStream`.
//!
//! # Architecture
//!
//! ```text
//! Event Store (PostgreSQL) → Redpanda/Kafka → ProjectionStream → Projections (PostgreSQL)
//! ```
//!
//! Each projection runs with:
//! - Dedicated consumer group (for independent progress tracking)
//! - Checkpoint persistence (resume from last processed event)
//! - Manual event deserialization (client knows concrete types)
//! - Error handling (continue on failure, log errors)

use crate::config::Config;
use crate::projections::{
    CorrelationId, PostgresAvailableSeatsProjection, ProjectionCompleted,
    ProjectionCompletionEvent, ProjectionFailed, TicketingEvent,
};
use composable_rust_core::event::SerializedEvent;
use composable_rust_core::event_bus::EventBus;
use composable_rust_core::projection::Projection;
use composable_rust_projections::{
    PostgresProjectionCheckpoint, PostgresProjectionStore, ProjectionStream,
};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// Setup function to create and configure all projection consumers.
///
/// Creates projection streams for:
/// - Available Seats Projection (seat availability queries)
///
/// Returns task handles and shutdown senders for graceful termination.
///
/// # Arguments
///
/// - `config`: Application configuration
/// - `_event_bus`: Domain event bus (currently unused, may be used for direct subscription)
/// - `completion_bus`: Event bus for publishing projection completion events
///
/// # Errors
///
/// Returns error if projection database connection fails.
pub async fn setup_projection_managers(
    config: &Config,
    _event_bus: Arc<dyn EventBus>,
    completion_bus: Arc<dyn EventBus>,
) -> Result<ProjectionManagers, Box<dyn std::error::Error>> {
    // Connect to projection database (separate from event store)
    let projection_pool = PgPool::connect(&config.projections.url).await?;

    // Run migrations to create projection tables
    let projection_store =
        PostgresProjectionStore::new(projection_pool.clone(), "projection_data".to_string());
    projection_store.migrate().await?;

    // Create checkpoint tracker
    let checkpoint = Arc::new(PostgresProjectionCheckpoint::new(projection_pool.clone()));

    // Setup Available Seats Projection
    let available_seats_projection =
        PostgresAvailableSeatsProjection::new(Arc::new(projection_pool.clone()));

    // Create event bus with consumer group for this projection
    let available_seats_event_bus = Arc::new(
        composable_rust_redpanda::RedpandaEventBus::builder()
            .brokers(&config.redpanda.brokers)
            .consumer_group("ticketing-available-seats-projection")
            .build()?,
    );

    // Create projection stream
    let available_seats_stream = ProjectionStream::new(
        available_seats_event_bus,
        checkpoint.clone(),
        &config.redpanda.inventory_topic,
        "ticketing-available-seats-projection",
        "ticketing-available-seats-projection",
    )
    .await?;

    // Create shutdown channel
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    Ok(ProjectionManagers {
        available_seats: AvailableSeatsProjectionRunner {
            projection: available_seats_projection,
            stream: available_seats_stream,
            shutdown: shutdown_rx,
            completion_bus,
        },
        shutdown: shutdown_tx,
    })
}

/// Runner for available seats projection with manual event loop.
struct AvailableSeatsProjectionRunner {
    projection: PostgresAvailableSeatsProjection,
    stream: ProjectionStream,
    shutdown: watch::Receiver<bool>,
    completion_bus: Arc<dyn EventBus>,
}

impl AvailableSeatsProjectionRunner {
    /// Start the projection consumer loop.
    ///
    /// Continuously consumes events, deserializes them, applies to projection,
    /// and commits checkpoints.
    #[allow(clippy::cognitive_complexity)] // Event loop is naturally complex
    async fn run(mut self) {
        let projection_name = self.projection.name();
        tracing::info!(
            projection = projection_name,
            "Starting projection consumer"
        );

        loop {
            tokio::select! {
                // Process next event
                Some(result) = self.stream.next() => {
                    match result {
                        Ok(serialized) => {
                            // Extract correlation_id from metadata (if present)
                            let correlation_id = Self::extract_correlation_id(&serialized);

                            // Deserialize to concrete type (client knows TicketingEvent!)
                            match bincode::deserialize::<TicketingEvent>(&serialized.data) {
                                Ok(event) => {
                                    // Apply to projection
                                    if let Err(e) = self.projection.apply_event(&event).await {
                                        tracing::error!(
                                            projection = projection_name,
                                            error = ?e,
                                            event_type = %serialized.event_type,
                                            "Failed to apply event to projection"
                                        );

                                        // Publish ProjectionFailed event
                                        if let Some(cid) = correlation_id {
                                            Self::publish_failure(
                                                &self.completion_bus,
                                                projection_name,
                                                &serialized.event_type,
                                                cid,
                                                &e.to_string(),
                                            ).await;
                                        }

                                        // Continue processing (don't crash on single event failure)
                                    } else {
                                        // Commit checkpoint after successful processing
                                        if let Err(e) = self.stream.commit().await {
                                            tracing::error!(
                                                projection = projection_name,
                                                error = ?e,
                                                "Failed to commit checkpoint"
                                            );
                                        } else {
                                            // Publish ProjectionCompleted event
                                            if let Some(cid) = correlation_id {
                                                Self::publish_completion(
                                                    &self.completion_bus,
                                                    projection_name,
                                                    &serialized.event_type,
                                                    cid,
                                                ).await;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(
                                        projection = projection_name,
                                        error = ?e,
                                        event_type = %serialized.event_type,
                                        "Failed to deserialize event"
                                    );
                                    // Continue processing (bincode errors are non-recoverable for this event)
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                projection = projection_name,
                                error = ?e,
                                "Error receiving event from bus"
                            );
                            // Backoff on transport errors
                            tokio::time::sleep(Duration::from_secs(5)).await;
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

        tracing::info!(projection = projection_name, "Projection consumer stopped");
    }

    /// Extract correlation_id from event metadata.
    ///
    /// Returns `None` if metadata is missing or doesn't contain correlation_id.
    fn extract_correlation_id(event: &SerializedEvent) -> Option<CorrelationId> {
        event.metadata.as_ref().and_then(|metadata| {
            metadata
                .get("correlation_id")
                .and_then(|v| v.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(CorrelationId::from_uuid)
        })
    }

    /// Publish a ProjectionCompleted event to the completion bus.
    async fn publish_completion(
        completion_bus: &Arc<dyn EventBus>,
        projection_name: &str,
        event_type: &str,
        correlation_id: CorrelationId,
    ) {
        let completion = ProjectionCompleted {
            correlation_id,
            projection_name: projection_name.to_string(),
            event_type: event_type.to_string(),
        };

        let event = ProjectionCompletionEvent::Completed(completion);

        match bincode::serialize(&event) {
            Ok(data) => {
                let serialized = SerializedEvent::new(
                    "ProjectionCompleted".to_string(),
                    data,
                    Some(serde_json::json!({
                        "correlation_id": correlation_id.to_string(),
                        "projection_name": projection_name,
                    })),
                );

                if let Err(e) = completion_bus.publish("projection.completed", &serialized).await {
                    tracing::error!(
                        projection = projection_name,
                        error = ?e,
                        "Failed to publish ProjectionCompleted event"
                    );
                }
            }
            Err(e) => {
                tracing::error!(
                    projection = projection_name,
                    error = ?e,
                    "Failed to serialize ProjectionCompleted event"
                );
            }
        }
    }

    /// Publish a ProjectionFailed event to the completion bus.
    async fn publish_failure(
        completion_bus: &Arc<dyn EventBus>,
        projection_name: &str,
        event_type: &str,
        correlation_id: CorrelationId,
        error: &str,
    ) {
        let failure = ProjectionFailed {
            correlation_id,
            projection_name: projection_name.to_string(),
            event_type: event_type.to_string(),
            error: error.to_string(),
        };

        let event = ProjectionCompletionEvent::Failed(failure);

        match bincode::serialize(&event) {
            Ok(data) => {
                let serialized = SerializedEvent::new(
                    "ProjectionFailed".to_string(),
                    data,
                    Some(serde_json::json!({
                        "correlation_id": correlation_id.to_string(),
                        "projection_name": projection_name,
                    })),
                );

                if let Err(e) = completion_bus.publish("projection.completed", &serialized).await {
                    tracing::error!(
                        projection = projection_name,
                        error = ?e,
                        "Failed to publish ProjectionFailed event"
                    );
                }
            }
            Err(e) => {
                tracing::error!(
                    projection = projection_name,
                    error = ?e,
                    "Failed to serialize ProjectionFailed event"
                );
            }
        }
    }
}

/// Container for all projection runners and shutdown sender.
pub struct ProjectionManagers {
    /// Available seats projection runner
    available_seats: AvailableSeatsProjectionRunner,
    /// Shared shutdown signal (dropped when struct is dropped, signaling shutdown)
    #[allow(dead_code)]
    shutdown: watch::Sender<bool>,
}

impl ProjectionManagers {
    /// Start all projection consumers.
    ///
    /// Spawns each consumer in its own tokio task.
    /// Consumers run until shutdown signal is received.
    #[must_use]
    pub fn start_all(self) -> Vec<tokio::task::JoinHandle<()>> {
        let mut handles = Vec::new();

        // Start available seats projection
        handles.push(tokio::spawn(async move {
            self.available_seats.run().await;
        }));

        handles
    }
}
