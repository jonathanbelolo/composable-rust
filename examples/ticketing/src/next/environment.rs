//! Environment implementation for the ticketing system.
//!
//! This module provides the [`TicketingEnvironment`] which implements
//! [`HandlerEnvironment`] to provide infrastructure dependencies to the Handler.
//!
//! # Dependency Injection
//!
//! The environment is fully generic over all infrastructure dependencies:
//! - `C`: Clock (for timestamps)
//! - `ES`: Event store (for persistence)
//! - `P`: Projector (for read models)
//! - `EB`: Event bus (for broadcasting)
//!
//! This enables easy testing with in-memory implementations:
//!
//! ```ignore
//! use composable_rust_next::testing::{InMemoryEventStore, InMemoryProjector, InMemoryEventBus};
//! use composable_rust_next::FixedClock;
//!
//! // Test environment with in-memory dependencies
//! let env = TicketingEnvironment::new(
//!     FixedClock::new(Utc::now()),
//!     InMemoryEventStore::new(),
//!     Some(InMemoryProjector::new()),
//!     Some(InMemoryEventBus::new()),
//!     "test-events",
//! );
//!
//! let handler = Handler::new(EventBusinessLogic, NoOpCallExecutor, env);
//! let result = handler.handle(command).await?;
//! ```

use composable_rust_next::{
    Clock, EventBus, EventBusError, EventStore, HandlerEnvironment, MetadataContext,
    NoOpProjectionQueries, ProjectionError, ProjectionQueries, Projector, SerializedEvent,
    SubscriptionHandle, SystemClock,
};
use futures::future::BoxFuture;
use composable_rust_postgres_next::PostgresEventStore;

use super::EventProjector;

// ═══════════════════════════════════════════════════════════════════════════
// No-Op Implementations
// ═══════════════════════════════════════════════════════════════════════════

/// A no-op event bus that does nothing.
///
/// Used when event broadcasting is not needed or not yet configured.
#[derive(Debug, Clone, Default)]
pub struct NoOpEventBus;

impl EventBus for NoOpEventBus {
    async fn publish(&self, _topic: &str, _event: SerializedEvent) -> Result<(), EventBusError> {
        Ok(())
    }

    async fn subscribe<F>(
        &self,
        _topic: &str,
        _handler: F,
    ) -> Result<SubscriptionHandle, EventBusError>
    where
        F: Fn(SerializedEvent) -> BoxFuture<'static, Result<(), EventBusError>>
            + Send
            + Sync
            + 'static,
    {
        // No-op: create a handle that does nothing
        let (cancel_tx, _cancel_rx) = tokio::sync::oneshot::channel();
        Ok(SubscriptionHandle::new(cancel_tx))
    }
}

/// A no-op projector that does nothing.
///
/// Used when projections are not needed for a particular aggregate.
#[derive(Debug, Clone, Default)]
pub struct NoOpProjector;

impl Projector for NoOpProjector {
    async fn project(&self, _events: &[SerializedEvent]) -> Result<(), ProjectionError> {
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Ticketing Environment (Fully Generic)
// ═══════════════════════════════════════════════════════════════════════════

/// Environment providing infrastructure dependencies for ticketing aggregates.
///
/// This struct is fully generic over all infrastructure dependencies, enabling:
/// - **Production**: Use `PostgresEventStore`, real projectors, Redpanda event bus
/// - **Testing**: Use `InMemoryEventStore`, `InMemoryProjector`, `InMemoryEventBus`
///
/// # Type Parameters
///
/// - `C`: Clock implementation (`SystemClock` or `FixedClock`)
/// - `ES`: Event store implementation (`PostgresEventStore` or `InMemoryEventStore`)
/// - `P`: Projector implementation (`EventProjector`, `InMemoryProjector`, or `NoOpProjector`)
/// - `EB`: Event bus implementation (`RedpandaEventBus`, `InMemoryEventBus`, or `NoOpEventBus`)
///
/// # Example (Production)
///
/// ```ignore
/// let env = TicketingEnvironment::new(
///     SystemClock,
///     postgres_event_store,
///     Some(event_projector),
///     Some(redpanda_event_bus),
///     "ticketing-events",
/// );
/// ```
///
/// # Example (Testing)
///
/// ```ignore
/// use composable_rust_next::testing::{InMemoryEventStore, InMemoryProjector, InMemoryEventBus};
/// use composable_rust_next::FixedClock;
///
/// let env = TicketingEnvironment::new(
///     FixedClock::new(Utc::now()),
///     InMemoryEventStore::new(),
///     Some(InMemoryProjector::new()),
///     Some(InMemoryEventBus::new()),
///     "test-events",
/// );
/// ```
#[derive(Clone)]
pub struct TicketingEnvironment<
    C,
    ES,
    P = NoOpProjector,
    EB = NoOpEventBus,
    PQ = NoOpProjectionQueries,
> where
    C: Clock,
    ES: EventStore,
    P: Projector,
    EB: EventBus,
    PQ: ProjectionQueries,
{
    clock: C,
    event_store: ES,
    projector: Option<P>,
    event_bus: Option<EB>,
    broadcast_topic: String,
    projections: PQ,
    metadata: MetadataContext,
}

impl<C, ES, P, EB> TicketingEnvironment<C, ES, P, EB, NoOpProjectionQueries>
where
    C: Clock,
    ES: EventStore,
    P: Projector,
    EB: EventBus,
{
    /// Create a new ticketing environment without projection queries.
    ///
    /// Uses `NoOpProjectionQueries` which is suitable for aggregates that only
    /// write data and don't support query operations.
    ///
    /// For environments that support queries, use [`TicketingEnvironment::with_projections`].
    ///
    /// # Arguments
    ///
    /// * `clock` - Clock for timestamps
    /// * `event_store` - Event store for persistence
    /// * `projector` - Optional projector for updating read models
    /// * `event_bus` - Optional event bus for broadcasting events
    /// * `broadcast_topic` - Topic name for broadcasting (used if `event_bus` is `Some`)
    #[must_use]
    pub fn new(
        clock: C,
        event_store: ES,
        projector: Option<P>,
        event_bus: Option<EB>,
        broadcast_topic: impl Into<String>,
    ) -> Self {
        Self {
            clock,
            event_store,
            projector,
            event_bus,
            broadcast_topic: broadcast_topic.into(),
            projections: NoOpProjectionQueries,
            metadata: MetadataContext::new(),
        }
    }
}

impl<C, ES, P, EB, PQ> TicketingEnvironment<C, ES, P, EB, PQ>
where
    C: Clock,
    ES: EventStore,
    P: Projector,
    EB: EventBus,
    PQ: ProjectionQueries,
{
    /// Create a new ticketing environment with projection queries.
    ///
    /// Use this constructor when you need to support query operations
    /// that read from projection data.
    ///
    /// # Arguments
    ///
    /// * `clock` - Clock for timestamps
    /// * `event_store` - Event store for persistence
    /// * `projector` - Optional projector for updating read models
    /// * `event_bus` - Optional event bus for broadcasting events
    /// * `broadcast_topic` - Topic name for broadcasting
    /// * `projections` - Projection queries implementation for read operations
    #[must_use]
    pub fn with_projections(
        clock: C,
        event_store: ES,
        projector: Option<P>,
        event_bus: Option<EB>,
        broadcast_topic: impl Into<String>,
        projections: PQ,
    ) -> Self {
        Self {
            clock,
            event_store,
            projector,
            event_bus,
            broadcast_topic: broadcast_topic.into(),
            projections,
            metadata: MetadataContext::new(),
        }
    }

    /// Get a reference to the clock.
    #[must_use]
    pub const fn clock_ref(&self) -> &C {
        &self.clock
    }

    /// Get a reference to the event store.
    #[must_use]
    pub const fn event_store_ref(&self) -> &ES {
        &self.event_store
    }

    /// Get a reference to the projector (if configured).
    #[must_use]
    pub const fn projector_ref(&self) -> Option<&P> {
        self.projector.as_ref()
    }

    /// Get a reference to the event bus (if configured).
    #[must_use]
    pub const fn event_bus_ref(&self) -> Option<&EB> {
        self.event_bus.as_ref()
    }
}

impl<C, ES, P, EB, PQ> HandlerEnvironment for TicketingEnvironment<C, ES, P, EB, PQ>
where
    C: Clock + Send + Sync,
    ES: EventStore + Send + Sync,
    P: Projector + Send + Sync,
    EB: EventBus + Send + Sync,
    PQ: ProjectionQueries,
{
    type Clock = C;
    type EventStore = ES;
    type Projector = P;
    type EventBus = EB;
    type Projections = PQ;

    fn clock(&self) -> &Self::Clock {
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

    fn projections(&self) -> &Self::Projections {
        &self.projections
    }

    fn metadata(&self) -> &MetadataContext {
        &self.metadata
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Convenience Type Aliases
// ═══════════════════════════════════════════════════════════════════════════

/// Production environment with PostgreSQL and system clock.
pub type ProductionEnvironment<P = NoOpProjector, EB = NoOpEventBus> =
    TicketingEnvironment<SystemClock, PostgresEventStore, P, EB>;

/// Production environment with PostgreSQL, EventProjector, and no event bus.
pub type ProductionEnvironmentWithProjector =
    TicketingEnvironment<SystemClock, PostgresEventStore, EventProjector, NoOpEventBus>;

// ═══════════════════════════════════════════════════════════════════════════
// Production Convenience Constructors
// ═══════════════════════════════════════════════════════════════════════════

impl<P, EB> TicketingEnvironment<SystemClock, PostgresEventStore, P, EB>
where
    P: Projector,
    EB: EventBus,
{
    /// Create a production environment with PostgreSQL and system clock.
    #[must_use]
    pub fn production(
        event_store: PostgresEventStore,
        projector: Option<P>,
        event_bus: Option<EB>,
        broadcast_topic: impl Into<String>,
    ) -> Self {
        Self::new(SystemClock, event_store, projector, event_bus, broadcast_topic)
    }
}

impl
    TicketingEnvironment<
        SystemClock,
        PostgresEventStore,
        NoOpProjector,
        NoOpEventBus,
        NoOpProjectionQueries,
    >
{
    /// Create a minimal production environment without projector or event bus.
    ///
    /// Useful for simple use cases or when projections aren't needed.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // String::new() is not const
    pub fn minimal(event_store: PostgresEventStore) -> Self {
        Self {
            clock: SystemClock,
            event_store,
            projector: None,
            event_bus: None,
            broadcast_topic: String::new(),
            projections: NoOpProjectionQueries,
            metadata: MetadataContext::new(),
        }
    }
}

impl
    TicketingEnvironment<
        SystemClock,
        PostgresEventStore,
        EventProjector,
        NoOpEventBus,
        NoOpProjectionQueries,
    >
{
    /// Create a production environment with a projector but no event bus.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // String::new() is not const
    pub fn with_projector(event_store: PostgresEventStore, projector: EventProjector) -> Self {
        Self {
            clock: SystemClock,
            event_store,
            projector: Some(projector),
            event_bus: None,
            broadcast_topic: String::new(),
            projections: NoOpProjectionQueries,
            metadata: MetadataContext::new(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Test Type Aliases
// ═══════════════════════════════════════════════════════════════════════════

/// Test environment type alias with all in-memory dependencies.
///
/// Use this for handler integration tests:
///
/// ```ignore
/// use composable_rust_next::testing::{InMemoryEventStore, InMemoryProjector, InMemoryEventBus};
/// use composable_rust_next::FixedClock;
///
/// let clock = FixedClock::new(Utc::now());
/// let event_store = InMemoryEventStore::new();
/// let projector = InMemoryProjector::new();
/// let event_bus = InMemoryEventBus::new();
///
/// let env: TestEnvironment = TicketingEnvironment::new(
///     clock,
///     event_store.clone(),  // Clone to keep reference for assertions
///     Some(projector.clone()),
///     Some(event_bus.clone()),
///     "test-events",
/// );
///
/// let handler = Handler::new(MyLogic, NoOpCallExecutor, env);
/// handler.handle(command).await?;
///
/// // Assert on stored events
/// assert_eq!(event_store.events_for_stream("my-stream").len(), 1);
/// ```
#[cfg(test)]
pub type TestEnvironment = TicketingEnvironment<
    composable_rust_next::FixedClock,
    composable_rust_next::testing::InMemoryEventStore,
    composable_rust_next::testing::InMemoryProjector,
    composable_rust_next::testing::InMemoryEventBus,
>;
