//! Projection consumer registration.
//!
//! This module provides factory functions for creating projection event consumers
//! and manages the complete projection system (in-memory projections, ownership
//! indices, and PostgreSQL-based projections).
//!
//! # Architecture
//!
//! The projection system has two types of projections:
//!
//! 1. **In-memory projections** (via EventConsumer):
//!    - Sales Analytics: Revenue, sales volume, aggregate statistics
//!    - Customer History: Per-customer reservation history
//!    - Ownership Indices: Security filtering for WebSocket notifications
//!
//! 2. **PostgreSQL projections** (via ProjectionStream):
//!    - Available Seats: Real-time seat availability queries
//!    - Checkpointing for resumable consumption
//!
//! # Example
//!
//! ```rust,ignore
//! let resources = ResourceManager::from_config(&config).await?;
//! let projection_system = register_projections(&resources, shutdown_rx).await?;
//!
//! // Start all projection managers
//! let handles = projection_system.managers.start_all();
//!
//! // Spawn event consumers
//! for consumer in projection_system.consumers {
//!     consumer.spawn();
//! }
//! ```

use crate::bootstrap::ResourceManager;
use crate::projections::manager::setup_projection_managers;
use crate::projections::{
    PostgresCustomerHistoryProjection, PostgresSalesAnalyticsProjection, ProjectionManagers,
};
use crate::runtime::consumer::EventConsumer;
use crate::runtime::handlers::OwnershipIndexHandler;
use crate::types::{CustomerId, PaymentId, ReservationId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

/// Complete projection system with all projections and consumers.
///
/// This struct bundles together all projection-related components:
/// - PostgreSQL projection managers (with checkpointing)
/// - PostgreSQL analytics projections (sales, customer history) - production-ready
/// - Ownership indices for security (WebSocket notification filtering)
/// - Event consumers that update ownership indices
pub struct ProjectionSystem {
    /// PostgreSQL projection managers (available seats, sales analytics, customer history, etc.)
    pub managers: ProjectionManagers,

    /// PostgreSQL sales analytics projection (production-ready, crash-safe)
    pub sales_analytics: Arc<PostgresSalesAnalyticsProjection>,

    /// PostgreSQL customer history projection (production-ready, crash-safe)
    pub customer_history: Arc<PostgresCustomerHistoryProjection>,

    /// Ownership index: ReservationId → CustomerId (for WebSocket filtering)
    pub reservation_ownership: Arc<RwLock<HashMap<ReservationId, CustomerId>>>,

    /// Ownership index: PaymentId → ReservationId (for WebSocket filtering)
    pub payment_ownership: Arc<RwLock<HashMap<PaymentId, ReservationId>>>,

    /// Event consumers that update ownership indices
    pub consumers: Vec<EventConsumer>,
}

/// Register all projection consumers and create the complete projection system.
///
/// This function:
/// 1. Sets up PostgreSQL projection managers (available seats, sales analytics, customer history)
/// 2. Creates PostgreSQL projections (production-ready, crash-safe)
/// 3. Creates ownership indices for security
/// 4. Registers event consumers to update ownership indices
///
/// # Arguments
///
/// * `resources` - Infrastructure resources (event bus, databases, etc.)
/// * `shutdown` - Shutdown signal receiver for graceful termination
///
/// # Returns
///
/// A `ProjectionSystem` with all projections and consumers ready to use.
///
/// # Errors
///
/// Returns error if PostgreSQL projection setup fails.
///
/// # Example
///
/// ```rust,ignore
/// let projection_system = register_projections(&resources, shutdown_rx).await?;
///
/// // Start PostgreSQL projection managers
/// let manager_handles = projection_system.managers.start_all();
///
/// // Spawn event consumers for ownership indices
/// let consumer_handles: Vec<_> = projection_system.consumers
///     .into_iter()
///     .map(|c| c.spawn())
///     .collect();
/// ```
pub async fn register_projections(
    resources: &ResourceManager,
    shutdown: broadcast::Receiver<()>,
) -> Result<ProjectionSystem, Box<dyn std::error::Error>> {
    // Setup PostgreSQL projection managers (available seats, sales analytics, customer history)
    let managers = setup_projection_managers(
        resources.config.as_ref(),
        resources.event_bus.clone(),
        resources.event_bus.clone(),
    )
    .await?;

    // Create PostgreSQL analytics projections (production-ready, persistent)
    let sales_analytics = Arc::new(PostgresSalesAnalyticsProjection::new(
        resources.projections_pool.clone(),
    ));
    let customer_history = Arc::new(PostgresCustomerHistoryProjection::new(
        resources.projections_pool.clone(),
    ));

    // Create security ownership indices (still in-memory for fast WebSocket filtering)
    let reservation_ownership = Arc::new(RwLock::new(HashMap::new()));
    let payment_ownership = Arc::new(RwLock::new(HashMap::new()));

    // Create event consumer for ownership indices
    // Note: PostgreSQL projections are updated by projection managers, not event consumers
    let consumers = vec![create_ownership_index_consumer(
        resources,
        reservation_ownership.clone(),
        payment_ownership.clone(),
        shutdown.resubscribe(),
    )];

    Ok(ProjectionSystem {
        managers,
        sales_analytics,
        customer_history,
        reservation_ownership,
        payment_ownership,
        consumers,
    })
}

/// Create ownership index consumer.
///
/// The ownership index consumer listens to reservation and payment topics,
/// maintaining in-memory indices for fast WebSocket notification filtering.
/// These indices map:
/// - `ReservationId` → `CustomerId` (who owns which reservation)
/// - `PaymentId` → `ReservationId` (which payment belongs to which reservation)
///
/// These indices are kept in-memory for fast lookups during WebSocket message
/// routing, while the actual projection data is persisted to PostgreSQL.
fn create_ownership_index_consumer(
    resources: &ResourceManager,
    reservation_ownership: Arc<RwLock<HashMap<ReservationId, CustomerId>>>,
    payment_ownership: Arc<RwLock<HashMap<PaymentId, ReservationId>>>,
    shutdown: broadcast::Receiver<()>,
) -> EventConsumer {
    // Create handler
    let handler = Arc::new(OwnershipIndexHandler {
        reservation_ownership,
        payment_ownership,
    });

    // Create consumer with EventConsumer builder
    // Subscribe to both reservation and payment topics
    EventConsumer::builder()
        .name("ownership-indices")
        .topics(vec![
            resources.config.redpanda.reservation_topic.clone(),
            resources.config.redpanda.payment_topic.clone(),
        ])
        .event_bus(resources.event_bus.clone())
        .handler(handler)
        .shutdown(shutdown)
        .build()
}
