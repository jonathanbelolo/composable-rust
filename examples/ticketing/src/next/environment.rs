//! Environment implementation for the ticketing system.
//!
//! This module provides the [`TicketingEnvironment`] which implements
//! [`HandlerEnvironment`] to provide infrastructure dependencies to the Handler.

use composable_rust_next::{
    Clock, EventBus, EventBusError, HandlerEnvironment, ProjectionError, Projector,
    SerializedEvent, SystemClock,
};
use composable_rust_postgres_next::PostgresEventStore;

use super::EventProjector;

// ═══════════════════════════════════════════════════════════════════════════
// No-Op Event Bus (placeholder until we need Redpanda)
// ═══════════════════════════════════════════════════════════════════════════

/// A no-op event bus that does nothing.
///
/// Used when event broadcasting is not needed or not yet configured.
#[derive(Debug, Clone)]
pub struct NoOpEventBus;

impl EventBus for NoOpEventBus {
    async fn publish(&self, _topic: &str, _event: SerializedEvent) -> Result<(), EventBusError> {
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// No-Op Projector (placeholder for when projections aren't needed)
// ═══════════════════════════════════════════════════════════════════════════

/// A no-op projector that does nothing.
///
/// Used when projections are not needed for a particular aggregate.
#[derive(Debug, Clone)]
pub struct NoOpProjector;

impl Projector for NoOpProjector {
    async fn project(&self, _events: &[SerializedEvent]) -> Result<(), ProjectionError> {
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Ticketing Environment
// ═══════════════════════════════════════════════════════════════════════════

/// Environment providing infrastructure dependencies for ticketing aggregates.
///
/// This implements [`HandlerEnvironment`] with concrete types for:
/// - Event store: `PostgreSQL` via [`PostgresEventStore`]
/// - Projector: [`EventProjector`] for updating read models
/// - Event bus: [`NoOpEventBus`] (placeholder, can be replaced with Redpanda)
///
/// # Example
///
/// ```ignore
/// let env = TicketingEnvironment::new(
///     event_store,
///     Some(projector),
///     None, // No event bus for now
///     "event-events",
/// );
///
/// let handler = Handler::new(EventBusinessLogic, NoOpCallExecutor, env);
/// let result = handler.handle(command).await?;
/// ```
pub struct TicketingEnvironment<P = NoOpProjector, EB = NoOpEventBus>
where
    P: Projector,
    EB: EventBus,
{
    clock: SystemClock,
    event_store: PostgresEventStore,
    projector: Option<P>,
    event_bus: Option<EB>,
    broadcast_topic: String,
}

impl<P, EB> TicketingEnvironment<P, EB>
where
    P: Projector,
    EB: EventBus,
{
    /// Create a new ticketing environment.
    ///
    /// # Arguments
    ///
    /// * `event_store` - `PostgreSQL` event store for persistence
    /// * `projector` - Optional projector for updating read models
    /// * `event_bus` - Optional event bus for broadcasting events
    /// * `broadcast_topic` - Topic name for broadcasting (used if `event_bus` is `Some`)
    #[must_use]
    pub fn new(
        event_store: PostgresEventStore,
        projector: Option<P>,
        event_bus: Option<EB>,
        broadcast_topic: impl Into<String>,
    ) -> Self {
        Self {
            clock: SystemClock,
            event_store,
            projector,
            event_bus,
            broadcast_topic: broadcast_topic.into(),
        }
    }

    /// Get a reference to the underlying event store.
    #[must_use]
    pub const fn event_store_ref(&self) -> &PostgresEventStore {
        &self.event_store
    }
}

impl TicketingEnvironment<NoOpProjector, NoOpEventBus> {
    /// Create a minimal environment without projector or event bus.
    ///
    /// Useful for testing or simple use cases.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // String::new() is not const
    pub fn minimal(event_store: PostgresEventStore) -> Self {
        Self {
            clock: SystemClock,
            event_store,
            projector: None,
            event_bus: None,
            broadcast_topic: String::new(),
        }
    }
}

impl<P, EB> HandlerEnvironment for TicketingEnvironment<P, EB>
where
    P: Projector + Send + Sync,
    EB: EventBus + Send + Sync,
{
    type EventStore = PostgresEventStore;
    type Projector = P;
    type EventBus = EB;

    fn clock(&self) -> &dyn Clock {
        &self.clock
    }

    fn event_store(&self) -> &Self::EventStore {
        &self.event_store
    }

    fn projector(&self) -> Option<&Self::Projector> {
        self.projector.as_ref()
    }

    fn event_bus(&self) -> Option<&Self::EventBus> {
        self.event_bus.as_ref()
    }

    fn broadcast_topic(&self) -> &str {
        &self.broadcast_topic
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Environment with Real Projector
// ═══════════════════════════════════════════════════════════════════════════

impl TicketingEnvironment<EventProjector, NoOpEventBus> {
    /// Create an environment with a projector but no event bus.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // String::new() is not const
    pub fn with_projector(event_store: PostgresEventStore, projector: EventProjector) -> Self {
        Self {
            clock: SystemClock,
            event_store,
            projector: Some(projector),
            event_bus: None,
            broadcast_topic: String::new(),
        }
    }
}
