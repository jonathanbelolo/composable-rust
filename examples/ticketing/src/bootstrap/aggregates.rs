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
use crate::runtime::consumer::EventConsumer;
use tokio::sync::broadcast;

/// Register all aggregate event consumers.
///
/// **Note**: With direct orchestration via broadcast channels, EventConsumers are no longer needed.
/// Aggregates now communicate directly via typed channels (inventory_actions, payment_actions, etc.)
/// instead of going through Redpanda topics. This function returns an empty vector for backwards
/// compatibility with the builder pattern.
///
/// # Arguments
///
/// * `resources` - Infrastructure resources (unused, kept for API compatibility)
/// * `shutdown` - Shutdown signal receiver (unused, kept for API compatibility)
///
/// # Returns
///
/// An empty vector (aggregates use direct channel communication now).
pub fn register_aggregate_consumers(
    _resources: &ResourceManager,
    _shutdown: broadcast::Receiver<()>,
) -> Vec<EventConsumer> {
    // No consumers needed - aggregates communicate via direct channels
    vec![]
}
