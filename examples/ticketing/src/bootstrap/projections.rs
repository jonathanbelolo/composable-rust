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
use crate::projections::{CustomerHistoryProjection, ProjectionManagers, SalesAnalyticsProjection};
use crate::runtime::consumer::EventConsumer;
use crate::runtime::handlers::{CustomerHistoryHandler, SalesAnalyticsHandler};
use crate::types::{CustomerId, PaymentId, ReservationId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

/// Complete projection system with all projections and consumers.
///
/// This struct bundles together all projection-related components:
/// - PostgreSQL projection managers (with checkpointing)
/// - In-memory analytics projections (sales, customer history)
/// - Ownership indices for security (WebSocket notification filtering)
/// - Event consumers that update the projections
pub struct ProjectionSystem {
    /// PostgreSQL projection managers (available seats, etc.)
    pub managers: ProjectionManagers,

    /// In-memory sales analytics projection
    pub sales_analytics: Arc<RwLock<SalesAnalyticsProjection>>,

    /// In-memory customer history projection
    pub customer_history: Arc<RwLock<CustomerHistoryProjection>>,

    /// Ownership index: ReservationId → CustomerId (for WebSocket filtering)
    pub reservation_ownership: Arc<RwLock<HashMap<ReservationId, CustomerId>>>,

    /// Ownership index: PaymentId → ReservationId (for WebSocket filtering)
    pub payment_ownership: Arc<RwLock<HashMap<PaymentId, ReservationId>>>,

    /// Event consumers that update projections
    pub consumers: Vec<EventConsumer>,
}

/// Register all projection consumers and create the complete projection system.
///
/// This function:
/// 1. Sets up PostgreSQL projection managers (available seats)
/// 2. Creates in-memory projections (sales analytics, customer history)
/// 3. Creates ownership indices for security
/// 4. Registers event consumers to update projections
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
/// // Spawn event consumers
/// let consumer_handles: Vec<_> = projection_system.consumers
///     .into_iter()
///     .map(|c| c.spawn())
///     .collect();
/// ```
pub async fn register_projections(
    resources: &ResourceManager,
    shutdown: broadcast::Receiver<()>,
) -> Result<ProjectionSystem, Box<dyn std::error::Error>> {
    // Setup PostgreSQL projection managers (available seats, etc.)
    let managers = setup_projection_managers(
        resources.config.as_ref(),
        resources.event_bus.clone(),
        resources.event_bus.clone(),
    )
    .await?;

    // Create in-memory analytics projections
    let sales_analytics = Arc::new(RwLock::new(SalesAnalyticsProjection::new()));
    let customer_history = Arc::new(RwLock::new(CustomerHistoryProjection::new()));

    // Create security ownership indices
    let reservation_ownership = Arc::new(RwLock::new(HashMap::new()));
    let payment_ownership = Arc::new(RwLock::new(HashMap::new()));

    // Create event consumers
    let consumers = vec![
        create_sales_analytics_consumer(
            resources,
            sales_analytics.clone(),
            reservation_ownership.clone(),
            payment_ownership.clone(),
            shutdown.resubscribe(),
        ),
        create_customer_history_consumer(
            resources,
            customer_history.clone(),
            reservation_ownership.clone(),
            shutdown.resubscribe(),
        ),
    ];

    Ok(ProjectionSystem {
        managers,
        sales_analytics,
        customer_history,
        reservation_ownership,
        payment_ownership,
        consumers,
    })
}

/// Create sales analytics projection consumer.
///
/// The sales analytics consumer listens to reservation and payment topics,
/// updating revenue and sales volume statistics. It also maintains ownership
/// indices for security filtering in WebSocket notifications.
fn create_sales_analytics_consumer(
    resources: &ResourceManager,
    sales_projection: Arc<RwLock<SalesAnalyticsProjection>>,
    reservation_ownership: Arc<RwLock<HashMap<ReservationId, CustomerId>>>,
    payment_ownership: Arc<RwLock<HashMap<PaymentId, ReservationId>>>,
    shutdown: broadcast::Receiver<()>,
) -> EventConsumer {
    // Create handler
    let handler = Arc::new(SalesAnalyticsHandler {
        projection: sales_projection,
        reservation_ownership,
        payment_ownership,
    });

    // Create consumer with EventConsumer builder
    // Subscribe to both reservation and payment topics
    EventConsumer::builder()
        .name("sales-analytics")
        .topics(vec![
            resources.config.redpanda.reservation_topic.clone(),
            resources.config.redpanda.payment_topic.clone(),
        ])
        .event_bus(resources.event_bus.clone())
        .handler(handler)
        .shutdown(shutdown)
        .build()
}

/// Create customer history projection consumer.
///
/// The customer history consumer listens to reservation topic, building
/// per-customer reservation history for analytics. It also maintains a
/// backup copy of the reservation ownership index.
fn create_customer_history_consumer(
    resources: &ResourceManager,
    customer_projection: Arc<RwLock<CustomerHistoryProjection>>,
    reservation_ownership: Arc<RwLock<HashMap<ReservationId, CustomerId>>>,
    shutdown: broadcast::Receiver<()>,
) -> EventConsumer {
    // Create handler
    let handler = Arc::new(CustomerHistoryHandler {
        projection: customer_projection,
        reservation_ownership,
    });

    // Create consumer with EventConsumer builder
    EventConsumer::builder()
        .name("customer-history")
        .topics(vec![resources.config.redpanda.reservation_topic.clone()])
        .event_bus(resources.event_bus.clone())
        .handler(handler)
        .shutdown(shutdown)
        .build()
}
