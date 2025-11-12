//! Projection manager setup for the ticketing system.
//!
//! This module wires up `ProjectionManager` instances to consume events
//! from the event bus and update PostgreSQL projections.
//!
//! # Architecture
//!
//! ```text
//! Event Store (PostgreSQL) → Redpanda/Kafka → ProjectionManager → Projections (PostgreSQL)
//! ```
//!
//! Each projection runs in its own manager with:
//! - Dedicated consumer group (for independent progress tracking)
//! - Checkpoint persistence (resume from last processed event)
//! - Error handling (continue on failure, log errors)

use crate::config::Config;
use crate::projections::PostgresAvailableSeatsProjection;
use composable_rust_core::event_bus::EventBus;
use composable_rust_projections::{
    PostgresProjectionCheckpoint, PostgresProjectionStore, ProjectionManager,
};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::watch;

/// Setup function to create and configure all projection managers.
///
/// Creates projection managers for:
/// - Available Seats Projection (seat availability queries)
/// - Sales Analytics Projection (revenue metrics)
/// - Customer History Projection (purchase history)
///
/// Returns managers and shutdown senders for graceful termination.
///
/// # Errors
///
/// Returns error if projection database connection fails.
pub async fn setup_projection_managers(
    config: &Config,
    _event_bus: Arc<dyn EventBus>,
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

    let (available_seats_manager, available_seats_shutdown) = ProjectionManager::new(
        available_seats_projection,
        available_seats_event_bus,
        checkpoint.clone(),
        &config.redpanda.inventory_topic,
        "ticketing-available-seats-projection",
    );

    Ok(ProjectionManagers {
        available_seats: (available_seats_manager, available_seats_shutdown),
    })
}

/// Container for all projection managers and their shutdown senders.
pub struct ProjectionManagers {
    /// Available seats projection manager and shutdown signal
    pub available_seats: (
        ProjectionManager<PostgresAvailableSeatsProjection>,
        watch::Sender<bool>,
    ),
}

impl ProjectionManagers {
    /// Start all projection managers.
    ///
    /// Spawns each manager in its own tokio task.
    /// Managers run until shutdown signal is received.
    pub fn start_all(self) -> Vec<tokio::task::JoinHandle<()>> {
        let mut handles = Vec::new();

        // Start available seats projection
        let (mut manager, _shutdown) = self.available_seats;
        handles.push(tokio::spawn(async move {
            if let Err(e) = manager.start().await {
                tracing::error!(error = ?e, "Available seats projection manager failed");
            }
        }));

        handles
    }

    /// Shutdown all projection managers gracefully.
    ///
    /// Sends shutdown signal to all managers and waits for them to finish.
    pub async fn shutdown_all(
        &self,
        handles: Vec<tokio::task::JoinHandle<()>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Send shutdown signals
        self.available_seats.1.send(true)?;

        // Wait for all managers to finish
        for handle in handles {
            handle.await?;
        }

        Ok(())
    }
}
