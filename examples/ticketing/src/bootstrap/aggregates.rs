//! Aggregate consumer registration.
//!
//! This module provides factory functions for creating aggregate event consumers.
//! Each aggregate (Inventory, Payment) gets its own consumer that listens to a
//! specific event bus topic and dispatches commands to the aggregate's store.
//!
//! # Design Philosophy
//!
//! Aggregates use the **per-message store pattern**: Each event creates a fresh
//! `Store` instance, processes the action, and then discards the store. This ensures:
//! - **Privacy**: No state shared across different users/messages
//! - **Memory efficiency**: State cleared after each message
//! - **Event sourcing**: Each store loads only the data it needs from event store
//!
//! # Example
//!
//! ```rust,ignore
//! let resources = ResourceManager::from_config(&config).await?;
//! let consumers = register_aggregate_consumers(&resources, shutdown_rx);
//!
//! // Spawn all consumers
//! for consumer in consumers {
//!     consumer.spawn();
//! }
//! ```

use crate::bootstrap::ResourceManager;
use crate::projections::query_adapters::{PostgresInventoryQuery, PostgresPaymentQuery};
use crate::projections::{PostgresAvailableSeatsProjection, PostgresPaymentsProjection};
use crate::runtime::consumer::EventConsumer;
use crate::runtime::handlers::{InventoryHandler, PaymentHandler};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Register all aggregate event consumers.
///
/// Creates consumers for:
/// - Inventory aggregate (seat reservation/release)
/// - Payment aggregate (payment processing)
///
/// # Arguments
///
/// * `resources` - Infrastructure resources (event bus, event store, etc.)
/// * `shutdown` - Shutdown signal receiver for graceful termination
///
/// # Returns
///
/// A vector of `EventConsumer` instances ready to be spawned.
///
/// # Example
///
/// ```rust,ignore
/// let consumers = register_aggregate_consumers(&resources, shutdown_rx);
/// let handles: Vec<_> = consumers
///     .into_iter()
///     .map(|c| c.spawn())
///     .collect();
/// ```
pub fn register_aggregate_consumers(
    resources: &ResourceManager,
    shutdown: broadcast::Receiver<()>,
) -> Vec<EventConsumer> {
    vec![
        create_inventory_consumer(resources, shutdown.resubscribe()),
        create_payment_consumer(resources, shutdown.resubscribe()),
    ]
}

/// Create inventory aggregate consumer.
///
/// The inventory consumer listens to the inventory topic and dispatches
/// commands to the inventory aggregate (seat reservation, release, etc.).
fn create_inventory_consumer(
    resources: &ResourceManager,
    shutdown: broadcast::Receiver<()>,
) -> EventConsumer {
    // Create projection query for on-demand state loading
    let available_seats_projection = Arc::new(PostgresAvailableSeatsProjection::new(
        resources.projections_pool.clone(),
    ));
    let inventory_query = Arc::new(PostgresInventoryQuery::new(available_seats_projection));

    // Create handler
    let handler = Arc::new(InventoryHandler {
        clock: resources.clock.clone(),
        event_store: resources.event_store.clone(),
        event_bus: resources.event_bus.clone(),
        query: inventory_query,
    });

    // Create consumer with EventConsumer builder
    EventConsumer::builder()
        .name("inventory")
        .topics(vec![resources.config.redpanda.inventory_topic.clone()])
        .event_bus(resources.event_bus.clone())
        .handler(handler)
        .shutdown(shutdown)
        .build()
}

/// Create payment aggregate consumer.
///
/// The payment consumer listens to the payment topic and dispatches
/// commands to the payment aggregate (process payment, refund, etc.).
fn create_payment_consumer(
    resources: &ResourceManager,
    shutdown: broadcast::Receiver<()>,
) -> EventConsumer {
    // Create projection query for on-demand state loading
    let payments_projection = Arc::new(PostgresPaymentsProjection::new(
        resources.projections_pool.clone(),
    ));
    let payment_query = Arc::new(PostgresPaymentQuery::new(payments_projection));

    // Create handler
    let handler = Arc::new(PaymentHandler {
        clock: resources.clock.clone(),
        event_store: resources.event_store.clone(),
        event_bus: resources.event_bus.clone(),
        query: payment_query,
    });

    // Create consumer with EventConsumer builder
    EventConsumer::builder()
        .name("payment")
        .topics(vec![resources.config.redpanda.payment_topic.clone()])
        .event_bus(resources.event_bus.clone())
        .handler(handler)
        .shutdown(shutdown)
        .build()
}
