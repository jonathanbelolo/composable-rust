//! Testing utilities for the next-generation architecture.
//!
//! This module provides in-memory implementations of infrastructure traits
//! for fast, deterministic testing without external dependencies.
//!
//! # Components
//!
//! - [`InMemoryEventStore`]: In-memory event store with version tracking
//! - [`InMemoryEventBus`]: In-memory event bus that captures published events
//! - [`InMemoryProjector`]: In-memory projector that tracks projections
//! - [`TestEnvironment`]: Pre-configured environment for testing
//!
//! # Example
//!
//! ```rust,ignore
//! use composable_rust_next::testing::{InMemoryEventStore, InMemoryEventBus, InMemoryProjector, TestEnvironment};
//! use composable_rust_next::{Handler, FixedClock, NoOpCallExecutor, NoOpQueryFetcher};
//!
//! // Create test environment
//! let env = TestEnvironment::new(FixedClock::new(Utc::now()));
//!
//! // Create handler
//! let handler = Handler::new(
//!     MyBusinessLogic,
//!     NoOpCallExecutor,
//!     NoOpQueryFetcher,
//!     env.clone(),
//! );
//!
//! // Test command processing
//! let result = handler.handle(MyCommand::DoSomething { ... }).await?;
//!
//! // Inspect stored events
//! let stored = env.event_store().events_for_stream("my-stream");
//! assert_eq!(stored.len(), 1);
//!
//! // Inspect published events
//! let published = env.event_bus().published_events();
//! assert_eq!(published.len(), 1);
//! ```

use crate::{
    AtomicError, Clock, DynAtomicPersist, EventBus, EventBusError, EventStore, EventStoreError,
    HandlerEnvironment, MetadataContext, NoOpProjectionQueries, ProjectionError, ProjectionQueries,
    Projector, SerializedEvent, StreamId, Subject, SubscriptionHandle, Version,
};
use futures::future::BoxFuture;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, RwLock};

// ═══════════════════════════════════════════════════════════════════
// In-Memory Event Store
// ═══════════════════════════════════════════════════════════════════

/// In-memory event store for testing.
///
/// Stores events in memory with full version tracking and optimistic concurrency.
/// Events are persisted across calls but not across process restarts.
///
/// # Thread Safety
///
/// Uses `RwLock` for concurrent access. All operations are thread-safe.
///
/// # Example
///
/// ```rust,ignore
/// let store = InMemoryEventStore::new();
///
/// // Append events
/// let version = store.append(
///     &StreamId::new("order-123"),
///     None, // First append
///     vec![event1, event2],
/// ).await?;
///
/// // Load events
/// let events = store.load(&StreamId::new("order-123"), None).await?;
/// assert_eq!(events.len(), 2);
///
/// // Version conflict detection
/// let result = store.append(
///     &StreamId::new("order-123"),
///     Some(Version::initial()), // Wrong version!
///     vec![event3],
/// ).await;
/// assert!(matches!(result, Err(EventStoreError::VersionConflict { .. })));
/// ```
#[derive(Debug, Clone, Default)]
pub struct InMemoryEventStore {
    /// Events indexed by `stream_id`
    streams: Arc<RwLock<HashMap<String, Vec<SerializedEvent>>>>,
}

impl InMemoryEventStore {
    /// Create a new empty in-memory event store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the store to empty state.
    ///
    /// Useful for test isolation when reusing a store instance.
    ///
    /// # Panics
    ///
    /// Panics if the `RwLock` is poisoned.
    #[allow(clippy::expect_used)] // Test infrastructure
    pub fn reset(&self) {
        self.streams
            .write()
            .expect("InMemoryEventStore lock poisoned")
            .clear();
    }

    /// Get all events for a stream (for test assertions).
    ///
    /// # Panics
    ///
    /// Panics if the `RwLock` is poisoned.
    #[must_use]
    #[allow(clippy::expect_used)] // Test infrastructure
    pub fn events_for_stream(&self, stream_id: &str) -> Vec<SerializedEvent> {
        self.streams
            .read()
            .expect("InMemoryEventStore lock poisoned")
            .get(stream_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Get the current version for a stream.
    ///
    /// Returns `None` if the stream doesn't exist.
    ///
    /// # Panics
    ///
    /// Panics if the `RwLock` is poisoned.
    #[must_use]
    #[allow(clippy::expect_used)] // Test infrastructure
    pub fn current_version(&self, stream_id: &str) -> Option<Version> {
        let streams = self
            .streams
            .read()
            .expect("InMemoryEventStore lock poisoned");
        streams.get(stream_id).map(|events| {
            if events.is_empty() {
                Version::initial()
            } else {
                Version::new(events.len() as u64)
            }
        })
    }

    /// Count total events across all streams (for test assertions).
    ///
    /// # Panics
    ///
    /// Panics if the `RwLock` is poisoned.
    #[must_use]
    #[allow(clippy::expect_used)] // Test infrastructure
    pub fn total_event_count(&self) -> usize {
        self.streams
            .read()
            .expect("InMemoryEventStore lock poisoned")
            .values()
            .map(Vec::len)
            .sum()
    }

    /// Append events and run an in-"transaction" projection atomically.
    ///
    /// Models a transactional store for tests: the events are appended, then
    /// `project` is awaited with the final version and the version-stamped events;
    /// if `project` returns `Err`, the appended events are rolled back (truncated)
    /// and [`AtomicError::Projection`] is returned, so callers observe all-or-nothing
    /// semantics without a real database.
    ///
    /// The internal lock is never held across the `.await`. On projection failure the
    /// rollback removes exactly the versions this call appended (by version range),
    /// rather than truncating to a prior length — so it never discards a concurrent
    /// writer's events even if one slipped in during the projection window.
    ///
    /// # Errors
    ///
    /// Returns [`AtomicError::Append`] on a version conflict, or
    /// [`AtomicError::Projection`] (after rolling the append back) if `project` fails.
    ///
    /// # Panics
    ///
    /// Panics if the internal `RwLock` is poisoned.
    #[allow(clippy::expect_used)] // Test infrastructure
    pub async fn append_with_projection<F, Fut>(
        &self,
        stream_id: &StreamId,
        expected_version: Option<Version>,
        events: Vec<SerializedEvent>,
        project: F,
    ) -> Result<Version, AtomicError>
    where
        F: FnOnce(Version, Vec<SerializedEvent>) -> Fut,
        Fut: std::future::Future<Output = Result<(), ProjectionError>>,
    {
        let key = stream_id.as_str().to_string();

        // Phase 1: append under the lock (no `.await` held), recording the version
        // range this call assigns so rollback can target exactly those events.
        let (appended_lo, final_version, versioned) = {
            let mut streams = self
                .streams
                .write()
                .expect("InMemoryEventStore lock poisoned");
            let stream = streams.entry(key.clone()).or_default();
            let current_version = if stream.is_empty() {
                Version::initial()
            } else {
                Version::new(stream.len() as u64)
            };
            if let Some(expected) = expected_version {
                if expected != current_version {
                    return Err(AtomicError::Append(EventStoreError::VersionConflict {
                        expected: Some(expected),
                        actual: current_version,
                    }));
                }
            }
            let start_version = current_version.as_u64() + 1;
            let mut versioned = Vec::with_capacity(events.len());
            for (i, mut event) in events.into_iter().enumerate() {
                event.version = Some(Version::new(start_version + i as u64));
                event.stream_id = Some(key.clone());
                versioned.push(event.clone());
                stream.push(event);
            }
            let final_version = Version::new(stream.len() as u64);
            (start_version, final_version, versioned)
        };

        // Phase 2: run the projection with the lock released.
        if let Err(e) = project(final_version, versioned).await {
            // Phase 3: roll back only the versions this call appended, so a concurrent
            // writer's events (outside [appended_lo, final_version]) are preserved.
            let hi = final_version.as_u64();
            let mut streams = self
                .streams
                .write()
                .expect("InMemoryEventStore lock poisoned");
            if let Some(stream) = streams.get_mut(&key) {
                stream.retain(|e| {
                    e.version.is_none_or(|v| {
                        let n = v.as_u64();
                        n < appended_lo || n > hi
                    })
                });
            }
            return Err(AtomicError::Projection(e));
        }

        Ok(final_version)
    }
}

impl EventStore for InMemoryEventStore {
    #[allow(clippy::expect_used)] // Test infrastructure
    async fn load(
        &self,
        stream_id: &StreamId,
        from_version: Option<Version>,
    ) -> Result<Vec<SerializedEvent>, EventStoreError> {
        let streams = self
            .streams
            .read()
            .expect("InMemoryEventStore lock poisoned");

        let Some(events) = streams.get(stream_id.as_str()) else {
            return Ok(Vec::new()); // Empty stream
        };

        // `from_version` is INCLUSIVE, matching `PostgresEventStore`'s
        // `WHERE version >= $2` (events sit at versions 1.., so the event
        // with version v is at index v-1).
        #[allow(clippy::cast_possible_truncation)] // Event streams won't exceed usize on 32-bit
        let start_idx = from_version.map_or(0, |v| (v.as_u64() as usize).saturating_sub(1));

        Ok(events.iter().skip(start_idx).cloned().collect())
    }

    #[allow(clippy::expect_used)] // Test infrastructure
    async fn append(
        &self,
        stream_id: &StreamId,
        expected_version: Option<Version>,
        events: Vec<SerializedEvent>,
    ) -> Result<Version, EventStoreError> {
        // Reject empty batches, matching PostgresEventStore (pinned by the
        // conformance suite).
        if events.is_empty() {
            return Err(EventStoreError::Serialization(
                "Cannot append empty event list".to_string(),
            ));
        }

        let mut streams = self
            .streams
            .write()
            .expect("InMemoryEventStore lock poisoned");

        let stream = streams.entry(stream_id.as_str().to_string()).or_default();
        let current_version = if stream.is_empty() {
            Version::initial()
        } else {
            Version::new(stream.len() as u64)
        };

        // Check expected version
        if let Some(expected) = expected_version {
            if expected != current_version {
                return Err(EventStoreError::VersionConflict {
                    expected: Some(expected),
                    actual: current_version,
                });
            }
        }

        // Append events with version numbers + source stream identity (the same
        // stamping contract as PostgresEventStore, pinned by the conformance suite)
        let start_version = current_version.as_u64() + 1;
        for (i, mut event) in events.into_iter().enumerate() {
            event.version = Some(Version::new(start_version + i as u64));
            event.stream_id = Some(stream_id.as_str().to_string());
            stream.push(event);
        }

        Ok(Version::new(stream.len() as u64))
    }
}

// ═══════════════════════════════════════════════════════════════════
// In-Memory Event Bus
// ═══════════════════════════════════════════════════════════════════

/// In-memory event bus for testing.
///
/// Captures all published events for later inspection in tests.
/// Unlike a real event bus, this doesn't deliver events to subscribers.
///
/// # Example
///
/// ```rust,ignore
/// let bus = InMemoryEventBus::new();
///
/// bus.publish("orders", event).await?;
///
/// let published = bus.published_events();
/// assert_eq!(published.len(), 1);
/// assert_eq!(published[0].0, "orders"); // topic
/// ```
#[derive(Debug, Clone, Default)]
pub struct InMemoryEventBus {
    /// Published events: (topic, event)
    events: Arc<RwLock<Vec<(String, SerializedEvent)>>>,
}

impl InMemoryEventBus {
    /// Create a new empty in-memory event bus.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the bus to empty state.
    ///
    /// # Panics
    ///
    /// Panics if the `RwLock` is poisoned.
    #[allow(clippy::expect_used)] // Test infrastructure
    pub fn reset(&self) {
        self.events
            .write()
            .expect("InMemoryEventBus lock poisoned")
            .clear();
    }

    /// Get all published events (for test assertions).
    ///
    /// Returns tuples of (topic, event).
    ///
    /// # Panics
    ///
    /// Panics if the `RwLock` is poisoned.
    #[must_use]
    #[allow(clippy::expect_used)] // Test infrastructure
    pub fn published_events(&self) -> Vec<(String, SerializedEvent)> {
        self.events
            .read()
            .expect("InMemoryEventBus lock poisoned")
            .clone()
    }

    /// Get published events for a specific topic.
    ///
    /// # Panics
    ///
    /// Panics if the `RwLock` is poisoned.
    #[must_use]
    #[allow(clippy::expect_used)] // Test infrastructure
    pub fn events_for_topic(&self, topic: &str) -> Vec<SerializedEvent> {
        self.events
            .read()
            .expect("InMemoryEventBus lock poisoned")
            .iter()
            .filter(|(t, _)| t == topic)
            .map(|(_, e)| e.clone())
            .collect()
    }
}

impl EventBus for InMemoryEventBus {
    #[allow(clippy::expect_used)] // Test infrastructure
    async fn publish(&self, topic: &str, event: SerializedEvent) -> Result<(), EventBusError> {
        self.events
            .write()
            .expect("InMemoryEventBus lock poisoned")
            .push((topic.to_string(), event));
        Ok(())
    }

    // Use default implementation for publish_batch (calls publish for each)

    /// Subscribe to events (no-op for testing).
    ///
    /// This test implementation does not deliver events to subscribers.
    /// It only captures published events for inspection via [`published_events()`](Self::published_events).
    ///
    /// Returns a handle that can be cancelled, but no events will be delivered.
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
        // Create a oneshot channel for cancellation, but don't actually subscribe
        let (cancel_tx, _cancel_rx) = tokio::sync::oneshot::channel();
        Ok(SubscriptionHandle::new(cancel_tx))
    }
}

// ═══════════════════════════════════════════════════════════════════
// In-Memory Projector
// ═══════════════════════════════════════════════════════════════════

/// In-memory projector for testing.
///
/// Records all projected events for inspection. By default, projections
/// always succeed. Use [`with_failure`](Self::with_failure) to simulate failures.
///
/// # Example
///
/// ```rust,ignore
/// let projector = InMemoryProjector::new();
///
/// projector.project(&[event1, event2]).await?;
///
/// let projected = projector.projected_events();
/// assert_eq!(projected.len(), 2);
///
/// // Simulate failure
/// let failing_projector = InMemoryProjector::new().with_failure("Database error");
/// let result = failing_projector.project(&[event]).await;
/// assert!(result.is_err());
/// ```
#[derive(Debug, Clone, Default)]
pub struct InMemoryProjector {
    /// Projected events
    events: Arc<RwLock<Vec<SerializedEvent>>>,
    /// Optional failure message to return
    failure: Option<String>,
}

impl InMemoryProjector {
    /// Create a new in-memory projector that always succeeds.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a projector that always fails with the given message.
    #[must_use]
    pub fn with_failure(mut self, message: impl Into<String>) -> Self {
        self.failure = Some(message.into());
        self
    }

    /// Reset the projector to empty state.
    ///
    /// # Panics
    ///
    /// Panics if the `RwLock` is poisoned.
    #[allow(clippy::expect_used)] // Test infrastructure
    pub fn reset(&self) {
        self.events
            .write()
            .expect("InMemoryProjector lock poisoned")
            .clear();
    }

    /// Get all projected events (for test assertions).
    ///
    /// # Panics
    ///
    /// Panics if the `RwLock` is poisoned.
    #[must_use]
    #[allow(clippy::expect_used)] // Test infrastructure
    pub fn projected_events(&self) -> Vec<SerializedEvent> {
        self.events
            .read()
            .expect("InMemoryProjector lock poisoned")
            .clone()
    }

    /// Count projected events.
    ///
    /// # Panics
    ///
    /// Panics if the `RwLock` is poisoned.
    #[must_use]
    #[allow(clippy::expect_used)] // Test infrastructure
    pub fn projection_count(&self) -> usize {
        self.events
            .read()
            .expect("InMemoryProjector lock poisoned")
            .len()
    }
}

impl Projector for InMemoryProjector {
    #[allow(clippy::expect_used)] // Test infrastructure
    async fn project(&self, events: &[SerializedEvent]) -> Result<(), ProjectionError> {
        if let Some(ref message) = self.failure {
            return Err(ProjectionError::Custom(message.clone()));
        }

        self.events
            .write()
            .expect("InMemoryProjector lock poisoned")
            .extend(events.iter().cloned());
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════
// Test Environment
// ═══════════════════════════════════════════════════════════════════

/// Pre-configured environment for testing.
///
/// Bundles in-memory implementations of all infrastructure traits.
/// Provides a convenient way to create a complete test environment.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_next::testing::TestEnvironment;
/// use composable_rust_next::{Handler, FixedClock, NoOpCallExecutor, NoOpQueryFetcher};
///
/// let env = TestEnvironment::new(FixedClock::new(Utc::now()));
///
/// let handler = Handler::new(MyBusinessLogic, NoOpCallExecutor, NoOpQueryFetcher, env.clone());
///
/// // Process commands
/// handler.handle(MyCommand::DoSomething).await?;
///
/// // Inspect results
/// let stored = env.event_store().events_for_stream("my-stream");
/// let projected = env.projector().projected_events();
/// let published = env.event_bus().published_events();
/// ```
#[derive(Clone)]
pub struct TestEnvironment<C: Clock, P: ProjectionQueries = NoOpProjectionQueries> {
    clock: C,
    event_store: InMemoryEventStore,
    projector: InMemoryProjector,
    event_bus: InMemoryEventBus,
    broadcast_topic: String,
    projections: P,
    metadata: MetadataContext,
    subject: Option<Subject>,
    atomic_persist: Option<Arc<dyn DynAtomicPersist>>,
}

impl<C: Clock + std::fmt::Debug, P: ProjectionQueries + std::fmt::Debug> std::fmt::Debug
    for TestEnvironment<C, P>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestEnvironment")
            .field("clock", &self.clock)
            .field("event_store", &self.event_store)
            .field("projector", &self.projector)
            .field("event_bus", &self.event_bus)
            .field("broadcast_topic", &self.broadcast_topic)
            .field("projections", &self.projections)
            .field("metadata", &self.metadata)
            .field("subject", &self.subject)
            .field("atomic_persist", &self.atomic_persist.is_some())
            .finish()
    }
}

impl<C: Clock> TestEnvironment<C, NoOpProjectionQueries> {
    /// Create a new test environment with the given clock.
    ///
    /// Uses `NoOpProjectionQueries` by default. For testing queries,
    /// use [`TestEnvironment::with_projections`] to provide a custom implementation.
    #[must_use]
    pub fn new(clock: C) -> Self {
        Self {
            clock,
            event_store: InMemoryEventStore::new(),
            projector: InMemoryProjector::new(),
            event_bus: InMemoryEventBus::new(),
            broadcast_topic: "test-events".to_string(),
            projections: NoOpProjectionQueries,
            metadata: MetadataContext::new(),
            subject: None,
            atomic_persist: None,
        }
    }
}

impl<C: Clock, P: ProjectionQueries> TestEnvironment<C, P> {
    /// Create a test environment with custom projection queries.
    ///
    /// Use this when testing queries that need access to projection data.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let projections = InMemoryProjectionQueries::new();
    /// projections.insert_event(EventDto { id: event_id, name: "Test".into(), ... });
    ///
    /// let env = TestEnvironment::with_projections(FixedClock::new(now), projections);
    /// let handler = Handler::new(EventBusinessLogic, NoOpCallExecutor, NoOpQueryFetcher, env);
    ///
    /// let result = handler.handle(EventCommand::GetEvent { event_id }).await?;
    /// ```
    #[must_use]
    pub fn with_projections(clock: C, projections: P) -> Self {
        Self {
            clock,
            event_store: InMemoryEventStore::new(),
            projector: InMemoryProjector::new(),
            event_bus: InMemoryEventBus::new(),
            broadcast_topic: "test-events".to_string(),
            projections,
            metadata: MetadataContext::new(),
            subject: None,
            atomic_persist: None,
        }
    }

    /// Attach an atomic append+projection capability (exercises the Handler's
    /// transactional path instead of the separate append-then-project path).
    #[must_use]
    pub fn with_atomic_persist(mut self, atomic_persist: Arc<dyn DynAtomicPersist>) -> Self {
        self.atomic_persist = Some(atomic_persist);
        self
    }

    /// Create with a custom broadcast topic.
    #[must_use]
    pub fn with_broadcast_topic(mut self, topic: impl Into<String>) -> Self {
        self.broadcast_topic = topic.into();
        self
    }

    /// Create with a custom projector (e.g., one that fails).
    #[must_use]
    pub fn with_projector(mut self, projector: InMemoryProjector) -> Self {
        self.projector = projector;
        self
    }

    /// Create with custom metadata context.
    #[must_use]
    pub fn with_metadata(mut self, metadata: MetadataContext) -> Self {
        self.metadata = metadata;
        self
    }

    /// Create with a fixture subject (returned from `current_subject`).
    ///
    /// Commands handled with this environment stamp the subject's ID as
    /// `author_id` on every persisted event. Without a fixture,
    /// `current_subject` returns `None` and the Handler falls back to
    /// [`Subject::System`] (which stamps `author_id: None`).
    #[must_use]
    pub fn with_subject(mut self, subject: Subject) -> Self {
        self.subject = Some(subject);
        self
    }

    /// Get the event store (for test assertions).
    #[must_use]
    pub const fn event_store(&self) -> &InMemoryEventStore {
        &self.event_store
    }

    /// Get the projector (for test assertions).
    #[must_use]
    pub const fn projector(&self) -> &InMemoryProjector {
        &self.projector
    }

    /// Get the event bus (for test assertions).
    #[must_use]
    pub const fn event_bus(&self) -> &InMemoryEventBus {
        &self.event_bus
    }

    /// Reset all in-memory stores to empty state.
    pub fn reset(&self) {
        self.event_store.reset();
        self.projector.reset();
        self.event_bus.reset();
    }
}

impl<C: Clock + Send + Sync, P: ProjectionQueries> HandlerEnvironment for TestEnvironment<C, P> {
    type Clock = C;
    type EventStore = InMemoryEventStore;
    type Projector = InMemoryProjector;
    type EventBus = InMemoryEventBus;
    type Projections = P;

    fn clock(&self) -> &Self::Clock {
        &self.clock
    }

    fn event_store(&self) -> &Self::EventStore {
        &self.event_store
    }

    fn projector(&self) -> Option<&Self::Projector> {
        Some(&self.projector)
    }

    fn event_bus(&self) -> Option<&Self::EventBus> {
        Some(&self.event_bus)
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

    fn atomic_persist(&self) -> Option<&dyn DynAtomicPersist> {
        self.atomic_persist.as_deref()
    }

    fn current_subject(&self) -> Option<Subject> {
        self.subject.clone()
    }
}

/// In-memory [`DynAtomicPersist`] for exercising the Handler's transactional path.
///
/// Appends to an [`InMemoryEventStore`] and runs the projection into an
/// [`InMemoryProjector`] within the same (simulated) transaction — the append
/// rolls back if the projection fails. Both stores are exposed for assertions.
#[derive(Clone, Default)]
pub struct InMemoryAtomicPersist {
    event_store: InMemoryEventStore,
    projector: InMemoryProjector,
}

impl InMemoryAtomicPersist {
    /// Create an adapter over the given event store and projector.
    #[must_use]
    pub const fn new(event_store: InMemoryEventStore, projector: InMemoryProjector) -> Self {
        Self {
            event_store,
            projector,
        }
    }

    /// The event store events were appended to (for assertions).
    #[must_use]
    pub const fn event_store(&self) -> &InMemoryEventStore {
        &self.event_store
    }

    /// The projector that received the in-transaction projection (for assertions).
    #[must_use]
    pub const fn projector(&self) -> &InMemoryProjector {
        &self.projector
    }
}

impl DynAtomicPersist for InMemoryAtomicPersist {
    fn append_and_project<'a>(
        &'a self,
        stream_id: &'a StreamId,
        expected_version: Option<Version>,
        events: Vec<SerializedEvent>,
    ) -> BoxFuture<'a, Result<Version, AtomicError>> {
        Box::pin(async move {
            self.event_store
                .append_with_projection(
                    stream_id,
                    expected_version,
                    events,
                    |_version, versioned| async move { self.projector.project(&versioned).await },
                )
                .await
        })
    }
}

// ═══════════════════════════════════════════════════════════════════
// RecordingUnitExecutor
// ═══════════════════════════════════════════════════════════════════

/// Closure-backed single-call executor that records every dispatched call.
///
/// The test double for durable saga runs: `execute_one` logs the call (at
/// dispatch time, synchronously) and delegates to the provided closure.
/// Tests control completion timing by wiring `tokio::sync::oneshot`
/// receivers (or any future) into the closure — a call whose gate never
/// fires simply stays in flight.
///
/// The batch [`CallExecutor`](crate::CallExecutor) impl (required by the
/// `Handler` struct bounds) runs `execute_one` for each call concurrently.
///
/// # Example
///
/// ```rust,ignore
/// // Echo every call immediately:
/// let executor = RecordingUnitExecutor::new(|call: u64| async move { call });
///
/// // Gate call 2 on a oneshot:
/// let (tx, rx) = tokio::sync::oneshot::channel();
/// let gates = Arc::new(Mutex::new(HashMap::from([(2u64, rx)])));
/// let executor = RecordingUnitExecutor::new(move |call: u64| {
///     let gate = gates.lock().unwrap().remove(&call);
///     async move {
///         match gate {
///             Some(rx) => rx.await.unwrap_or(call),
///             None => call,
///         }
///     }
/// });
/// ```
pub struct RecordingUnitExecutor<C, R> {
    handler: Arc<dyn Fn(C) -> BoxFuture<'static, R> + Send + Sync>,
    dispatched: Arc<std::sync::Mutex<Vec<C>>>,
}

impl<C, R> Clone for RecordingUnitExecutor<C, R> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
            dispatched: Arc::clone(&self.dispatched),
        }
    }
}

impl<C, R> std::fmt::Debug for RecordingUnitExecutor<C, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.dispatched.lock().map(|log| log.len()).unwrap_or(0);
        f.debug_struct("RecordingUnitExecutor")
            .field("dispatch_count", &count)
            .finish_non_exhaustive()
    }
}

impl<C, R> RecordingUnitExecutor<C, R>
where
    C: Clone + Send + 'static,
    R: Send + 'static,
{
    /// Create an executor backed by the given per-call closure.
    pub fn new<F, Fut>(execute: F) -> Self
    where
        F: Fn(C) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
    {
        Self {
            handler: Arc::new(move |call| Box::pin(execute(call))),
            dispatched: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// All calls dispatched so far, in dispatch order (for test assertions).
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned.
    #[must_use]
    #[allow(clippy::expect_used)] // Test infrastructure
    pub fn dispatched(&self) -> Vec<C> {
        self.dispatched
            .lock()
            .expect("RecordingUnitExecutor lock poisoned")
            .clone()
    }

    /// Number of calls dispatched so far.
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned.
    #[must_use]
    #[allow(clippy::expect_used)] // Test infrastructure
    pub fn dispatch_count(&self) -> usize {
        self.dispatched
            .lock()
            .expect("RecordingUnitExecutor lock poisoned")
            .len()
    }
}

impl<C, R> crate::UnitCallExecutor<C, R> for RecordingUnitExecutor<C, R>
where
    C: Clone + Send + 'static,
    R: Send + 'static,
{
    #[allow(clippy::expect_used)] // Test infrastructure
    fn execute_one(&self, call: C) -> impl Future<Output = R> + Send {
        self.dispatched
            .lock()
            .expect("RecordingUnitExecutor lock poisoned")
            .push(call.clone());
        (self.handler)(call)
    }
}

impl<C, R> crate::CallExecutor<C, R> for RecordingUnitExecutor<C, R>
where
    C: Clone + Send + 'static,
    R: Send + 'static,
{
    async fn execute(&self, calls: Vec<C>) -> Vec<R> {
        futures::future::join_all(
            calls
                .into_iter()
                .map(|call| crate::UnitCallExecutor::execute_one(self, call)),
        )
        .await
    }
}

// ═══════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{
        BusinessLogic, BusinessResult, CallExecutor, FixedClock, Handler, NoOpQueryFetcher,
    };
    use chrono::Utc;

    fn make_event(event_type: &str) -> SerializedEvent {
        SerializedEvent {
            event_type: event_type.to_string(),
            payload: vec![1, 2, 3],
            metadata: None,
            version: None,
            stream_id: None,
        }
    }

    #[tokio::test]
    async fn event_store_append_and_load() {
        let store = InMemoryEventStore::new();
        let stream_id = StreamId::new("test-stream");

        // Append first event
        let version = store
            .append(&stream_id, None, vec![make_event("Event1")])
            .await
            .unwrap();
        assert_eq!(version, Version::new(1));

        // Append second event with correct version
        let version = store
            .append(
                &stream_id,
                Some(Version::new(1)),
                vec![make_event("Event2")],
            )
            .await
            .unwrap();
        assert_eq!(version, Version::new(2));

        // Load all events
        let events = store.load(&stream_id, None).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "Event1");
        assert_eq!(events[1].event_type, "Event2");

        // Load from version — INCLUSIVE, matching PostgresEventStore
        let events = store.load(&stream_id, Some(Version::new(1))).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "Event1");

        let events = store.load(&stream_id, Some(Version::new(2))).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "Event2");
    }

    #[tokio::test]
    async fn event_store_version_conflict() {
        let store = InMemoryEventStore::new();
        let stream_id = StreamId::new("test-stream");

        // Append first event
        store
            .append(&stream_id, None, vec![make_event("Event1")])
            .await
            .unwrap();

        // Try to append with wrong version
        let result = store
            .append(
                &stream_id,
                Some(Version::initial()),
                vec![make_event("Event2")],
            )
            .await;

        assert!(matches!(
            result,
            Err(EventStoreError::VersionConflict { expected, actual })
                if expected == Some(Version::initial()) && actual == Version::new(1)
        ));
    }

    #[tokio::test]
    async fn event_bus_publish_and_retrieve() {
        let bus = InMemoryEventBus::new();

        bus.publish("topic1", make_event("Event1")).await.unwrap();
        bus.publish("topic2", make_event("Event2")).await.unwrap();
        bus.publish("topic1", make_event("Event3")).await.unwrap();

        let all = bus.published_events();
        assert_eq!(all.len(), 3);

        let topic1 = bus.events_for_topic("topic1");
        assert_eq!(topic1.len(), 2);
        assert_eq!(topic1[0].event_type, "Event1");
        assert_eq!(topic1[1].event_type, "Event3");
    }

    #[tokio::test]
    async fn projector_success() {
        let projector = InMemoryProjector::new();

        projector
            .project(&[make_event("Event1"), make_event("Event2")])
            .await
            .unwrap();

        let projected = projector.projected_events();
        assert_eq!(projected.len(), 2);
    }

    #[tokio::test]
    async fn projector_failure() {
        let projector = InMemoryProjector::new().with_failure("Database down");

        let result = projector.project(&[make_event("Event1")]).await;

        assert!(matches!(result, Err(ProjectionError::Custom(msg)) if msg == "Database down"));
    }

    #[test]
    fn test_environment_creation() {
        let clock = FixedClock::new(Utc::now());
        let env = TestEnvironment::new(clock);

        assert_eq!(env.event_store().total_event_count(), 0);
        assert_eq!(env.projector().projection_count(), 0);
        assert!(env.event_bus().published_events().is_empty());
    }

    #[test]
    fn test_environment_with_metadata() {
        let clock = FixedClock::new(Utc::now());
        let metadata = MetadataContext::new()
            .with_correlation_id("test-123")
            .with_user_id("user-abc");

        let env = TestEnvironment::new(clock).with_metadata(metadata);

        assert_eq!(env.metadata().correlation_id, Some("test-123".to_string()));
        assert_eq!(env.metadata().user_id, Some("user-abc".to_string()));
    }

    #[tokio::test]
    async fn append_with_projection_commits_on_success() {
        let store = InMemoryEventStore::new();
        let stream_id = StreamId::new("saga-1");

        let version = store
            .append_with_projection(
                &stream_id,
                None,
                vec![make_event("E1"), make_event("E2")],
                |final_version, events| async move {
                    assert_eq!(final_version, Version::new(2));
                    assert_eq!(events.len(), 2);
                    Ok(())
                },
            )
            .await
            .unwrap();

        assert_eq!(version, Version::new(2));
        assert_eq!(store.events_for_stream("saga-1").len(), 2);
    }

    #[tokio::test]
    async fn append_with_projection_rolls_back_on_failure() {
        let store = InMemoryEventStore::new();
        let stream_id = StreamId::new("saga-1");

        // Seed one committed event.
        store
            .append(&stream_id, None, vec![make_event("Seed")])
            .await
            .unwrap();

        let result = store
            .append_with_projection(
                &stream_id,
                Some(Version::new(1)),
                vec![make_event("E2")],
                |_version, _events| async { Err(ProjectionError::Custom("boom".to_string())) },
            )
            .await;

        // The projection failed → the append was rolled back atomically.
        assert!(matches!(
            result,
            Err(AtomicError::Projection(ProjectionError::Custom(msg))) if msg == "boom"
        ));
        assert_eq!(store.events_for_stream("saga-1").len(), 1);
    }

    #[tokio::test]
    async fn append_with_projection_rollback_preserves_concurrent_events() {
        // Regression guard: rollback must remove only the versions THIS call appended,
        // never a concurrent writer's events that landed during the projection window.
        let store = InMemoryEventStore::new();
        let stream_id = StreamId::new("saga-1");

        // Seed one committed event (version 1).
        store
            .append(&stream_id, None, vec![make_event("Seed")])
            .await
            .unwrap();

        let concurrent = store.clone();
        let concurrent_stream = stream_id.clone();
        let result = store
            .append_with_projection(
                &stream_id,
                Some(Version::new(1)),
                vec![make_event("E2")], // appended as version 2
                |_version, _events| async move {
                    // A concurrent writer commits version 3 while the projection runs.
                    concurrent
                        .append(
                            &concurrent_stream,
                            Some(Version::new(2)),
                            vec![make_event("E3")],
                        )
                        .await
                        .unwrap();
                    Err(ProjectionError::Custom("boom".to_string()))
                },
            )
            .await;

        assert!(matches!(result, Err(AtomicError::Projection(_))));

        let events = store.events_for_stream("saga-1");
        // Seed (v1) + the concurrent E3 (v3) survive; only our E2 (v2) is rolled back.
        assert_eq!(
            events.len(),
            2,
            "concurrent writer's event must survive rollback"
        );
        assert!(
            events.iter().any(|e| e.event_type == "E3"),
            "the concurrent writer's E3 must be preserved"
        );
        assert!(
            events.iter().all(|e| e.event_type != "E2"),
            "our failed append (E2) must be rolled back"
        );
    }

    #[tokio::test]
    async fn append_with_projection_version_conflict_skips_projection() {
        let store = InMemoryEventStore::new();
        let stream_id = StreamId::new("saga-1");
        store
            .append(&stream_id, None, vec![make_event("Seed")])
            .await
            .unwrap();

        // Wrong expected version → conflict before the projection runs.
        let result = store
            .append_with_projection(
                &stream_id,
                Some(Version::initial()),
                vec![make_event("E2")],
                |_version, _events| async { Ok(()) },
            )
            .await;

        assert!(matches!(
            result,
            Err(AtomicError::Append(EventStoreError::VersionConflict { .. }))
        ));
        assert_eq!(store.events_for_stream("saga-1").len(), 1);
    }

    // ── Minimal saga to exercise the Handler's atomic-persist + feedback path ──

    #[derive(Clone)]
    #[allow(dead_code)] // `results` is carried for realism; this minimal saga ignores it
    enum SagaIn {
        Start { id: u64 },
        Feedback { id: u64, results: Vec<()> },
    }

    #[derive(Clone, serde::Serialize, serde::Deserialize)]
    enum SagaEv {
        Started(u64),
        Finished(u64),
    }

    #[derive(Debug, thiserror::Error)]
    #[error("saga error")]
    struct SagaErr;

    #[derive(Default)]
    struct SagaState {
        started: bool,
    }

    struct SagaLogic;

    impl BusinessLogic for SagaLogic {
        type State = SagaState;
        type Input = SagaIn;
        type Event = SagaEv;
        type Error = SagaErr;
        type Call = ();
        type CallResult = ();
        type Response = ();

        fn stream_id(input: &SagaIn) -> StreamId {
            let id = match input {
                SagaIn::Start { id } | SagaIn::Feedback { id, .. } => *id,
            };
            StreamId::new(format!("saga-{id}"))
        }

        fn process(
            &self,
            input: SagaIn,
            _clock: &dyn Clock,
        ) -> Result<BusinessResult<SagaEv, (), ()>, SagaErr> {
            match input {
                SagaIn::Start { id } => Ok(BusinessResult::Continue {
                    events: vec![SagaEv::Started(id)],
                    calls: vec![()],
                }),
                SagaIn::Feedback { id, .. } => Ok(BusinessResult::Done(vec![SagaEv::Finished(id)])),
            }
        }

        fn apply(&self, state: &mut SagaState, event: &SagaEv) {
            if matches!(event, SagaEv::Started(_)) {
                state.started = true;
            }
        }

        fn feedback_input_from(prior: &SagaIn, results: Vec<()>) -> SagaIn {
            let id = match prior {
                SagaIn::Start { id } | SagaIn::Feedback { id, .. } => *id,
            };
            SagaIn::Feedback { id, results }
        }

        fn event_type_name(event: &SagaEv) -> &'static str {
            match event {
                SagaEv::Started(_) => "Started",
                SagaEv::Finished(_) => "Finished",
            }
        }
    }

    struct OneCallExecutor;

    impl CallExecutor<(), ()> for OneCallExecutor {
        async fn execute(&self, calls: Vec<()>) -> Vec<()> {
            calls
        }
    }

    #[tokio::test]
    async fn handler_uses_atomic_persist_and_threads_feedback_input() {
        let store = InMemoryEventStore::new();
        let projector = InMemoryProjector::new();
        let ap = InMemoryAtomicPersist::new(store.clone(), projector.clone());

        let env =
            TestEnvironment::new(FixedClock::new(Utc::now())).with_atomic_persist(Arc::new(ap));
        let env_handle = env.clone();

        let handler = Handler::new(SagaLogic, OneCallExecutor, NoOpQueryFetcher, env);
        let result = handler.handle(SagaIn::Start { id: 42 }).await;

        // Start -> Continue -> feedback -> Done.
        assert!(
            matches!(
                result,
                Ok(crate::HandleResult::Command { event_count: 1, .. })
            ),
            "expected the saga to finish via Done, got {result:?}"
        );

        // Both events were persisted via the ATOMIC path onto the SAME id-derived
        // stream — which only holds if feedback_input_from threaded id 42 from the
        // prior input (not from the empty call results).
        assert_eq!(
            store.events_for_stream("saga-42").len(),
            2,
            "Started + Finished persisted on the id-derived stream via atomic_persist"
        );
        // The in-transaction projection ran for both events.
        assert_eq!(projector.projection_count(), 2);
        // The environment's own (non-atomic) projector was bypassed entirely.
        assert_eq!(env_handle.projector().projection_count(), 0);
    }

    #[tokio::test]
    async fn handler_stamps_stream_id_on_projected_and_broadcast_events() {
        let env = TestEnvironment::new(FixedClock::new(Utc::now()));
        let env_handle = env.clone();

        let handler = Handler::new(SagaLogic, OneCallExecutor, NoOpQueryFetcher, env);
        handler
            .handle(SagaIn::Start { id: 7 })
            .await
            .expect("saga run succeeds");

        // Both the projector and the bus receive infrastructure-stamped events:
        // version AND stream_id (per-stream read-model guards depend on both).
        let projected = env_handle.projector().projected_events();
        assert_eq!(projected.len(), 2, "Started + Finished projected");
        for event in &projected {
            assert_eq!(
                event.stream_id.as_deref(),
                Some("saga-7"),
                "projected events must carry the source stream_id"
            );
            assert!(event.version.is_some(), "projected events must be versioned");
        }
        let published = env_handle.event_bus().published_events();
        assert_eq!(published.len(), 2, "Started + Finished broadcast");
        for (_, event) in &published {
            assert_eq!(
                event.stream_id.as_deref(),
                Some("saga-7"),
                "broadcast events must carry the source stream_id"
            );
        }
    }

    // ── Backward-compat: a saga overriding ONLY the legacy `feedback_input` ──
    //
    // Downstream sagas implement `feedback_input` (not the newer `feedback_input_from`).
    // The Handler always calls `feedback_input_from`, whose default delegates to
    // `feedback_input` — so old-style sagas must keep working unchanged. This is the
    // regression guard for keeping `feedback_input` on the trait.

    #[derive(Clone)]
    #[allow(dead_code)] // `results` is carried for realism; this minimal saga ignores it
    enum OldSagaIn {
        Start { id: u64 },
        Feedback { id: u64, results: Vec<u64> },
    }

    struct OldStyleSagaLogic;

    impl BusinessLogic for OldStyleSagaLogic {
        type State = SagaState;
        type Input = OldSagaIn;
        type Event = SagaEv;
        type Error = SagaErr;
        type Call = u64;
        type CallResult = u64;
        type Response = ();

        fn stream_id(input: &OldSagaIn) -> StreamId {
            let id = match input {
                OldSagaIn::Start { id } | OldSagaIn::Feedback { id, .. } => *id,
            };
            StreamId::new(format!("old-saga-{id}"))
        }

        fn process(
            &self,
            input: OldSagaIn,
            _clock: &dyn Clock,
        ) -> Result<BusinessResult<SagaEv, u64, ()>, SagaErr> {
            match input {
                OldSagaIn::Start { id } => Ok(BusinessResult::Continue {
                    events: vec![SagaEv::Started(id)],
                    calls: vec![id],
                }),
                OldSagaIn::Feedback { id, .. } => {
                    Ok(BusinessResult::Done(vec![SagaEv::Finished(id)]))
                },
            }
        }

        fn apply(&self, state: &mut SagaState, event: &SagaEv) {
            if matches!(event, SagaEv::Started(_)) {
                state.started = true;
            }
        }

        // LEGACY method only — no `feedback_input_from` override. The id is recovered
        // from the call results, exactly as downstream sagas do.
        fn feedback_input(results: Vec<u64>) -> OldSagaIn {
            let id = results.first().copied().unwrap_or_default();
            OldSagaIn::Feedback { id, results }
        }

        fn event_type_name(event: &SagaEv) -> &'static str {
            match event {
                SagaEv::Started(_) => "Started",
                SagaEv::Finished(_) => "Finished",
            }
        }
    }

    struct IdCallExecutor;

    impl CallExecutor<u64, u64> for IdCallExecutor {
        async fn execute(&self, calls: Vec<u64>) -> Vec<u64> {
            calls // echo the id back as the call result
        }
    }

    #[tokio::test]
    async fn handler_routes_legacy_feedback_input_through_default_feedback_input_from() {
        let store = InMemoryEventStore::new();
        let projector = InMemoryProjector::new();
        let ap = InMemoryAtomicPersist::new(store.clone(), projector.clone());
        let env =
            TestEnvironment::new(FixedClock::new(Utc::now())).with_atomic_persist(Arc::new(ap));

        let handler = Handler::new(OldStyleSagaLogic, IdCallExecutor, NoOpQueryFetcher, env);
        let result = handler.handle(OldSagaIn::Start { id: 7 }).await;

        // Start -> Continue -> call -> feedback (via legacy feedback_input) -> Done.
        assert!(
            matches!(
                result,
                Ok(crate::HandleResult::Command { event_count: 1, .. })
            ),
            "old-style saga must finish through the Handler, got {result:?}"
        );
        // Both events landed on the id-derived stream — only true if the default
        // feedback_input_from delegated to the legacy feedback_input and threaded id 7.
        assert_eq!(
            store.events_for_stream("old-saga-7").len(),
            2,
            "Started + Finished persisted via the legacy feedback_input path"
        );
    }

    // ── Subject stamping: author_id on persisted event metadata ──
    //
    // A minimal aggregate (Infallible calls) to exercise the Handler's
    // context path: current_subject → InvocationContext → serialize_events →
    // to_event_metadata_with_subject.

    use crate::{NoOpCallExecutor, SubjectId};
    use std::convert::Infallible;

    #[derive(Clone)]
    struct CreateCmd {
        id: u64,
    }

    struct AggregateLogic;

    impl BusinessLogic for AggregateLogic {
        type State = SagaState;
        type Input = CreateCmd;
        type Event = SagaEv;
        type Error = SagaErr;
        type Call = Infallible;
        type CallResult = Infallible;
        type Response = ();

        fn stream_id(input: &CreateCmd) -> StreamId {
            StreamId::new(format!("agg-{}", input.id))
        }

        fn process(
            &self,
            input: CreateCmd,
            _clock: &dyn Clock,
        ) -> Result<BusinessResult<SagaEv, Infallible, ()>, SagaErr> {
            Ok(BusinessResult::Done(vec![SagaEv::Started(input.id)]))
        }

        fn apply(&self, state: &mut SagaState, event: &SagaEv) {
            if matches!(event, SagaEv::Started(_)) {
                state.started = true;
            }
        }

        fn event_type_name(event: &SagaEv) -> &'static str {
            match event {
                SagaEv::Started(_) => "Started",
                SagaEv::Finished(_) => "Finished",
            }
        }
    }

    #[tokio::test]
    async fn handler_stamps_author_id_from_fixture_subject() {
        let subject = Subject::Authenticated {
            id: SubjectId::new("user-9"),
            attributes: HashMap::new(),
        };
        let env = TestEnvironment::new(FixedClock::new(Utc::now())).with_subject(subject);
        let env_handle = env.clone();

        let handler = Handler::new(AggregateLogic, NoOpCallExecutor, NoOpQueryFetcher, env);
        handler.handle(CreateCmd { id: 1 }).await.unwrap();

        let stored = env_handle.event_store().events_for_stream("agg-1");
        assert_eq!(stored.len(), 1);
        let metadata = stored[0]
            .metadata
            .as_ref()
            .expect("handler must stamp metadata on persisted events");
        assert_eq!(
            metadata.author_id,
            Some(SubjectId::new("user-9")),
            "the fixture subject's id must land as author_id"
        );
        assert_eq!(metadata.origin_subject_id, None);
    }

    #[tokio::test]
    async fn handler_stamps_metadata_timestamp_from_injected_clock() {
        // µs-precision fixture time: the stamped value must equal it EXACTLY
        // (clock-threaded, not wall-clock; truncation is a no-op at µs).
        let fixed = chrono::DateTime::from_timestamp_micros(1_700_000_000_000_123)
            .expect("valid timestamp");
        let env = TestEnvironment::new(FixedClock::new(fixed));
        let env_handle = env.clone();

        let handler = Handler::new(AggregateLogic, NoOpCallExecutor, NoOpQueryFetcher, env);
        handler.handle(CreateCmd { id: 3 }).await.unwrap();

        let stored = env_handle.event_store().events_for_stream("agg-3");
        let metadata = stored[0]
            .metadata
            .as_ref()
            .expect("handler must stamp metadata");
        assert_eq!(
            metadata.timestamp, fixed,
            "event timestamps must come from the injected clock"
        );
    }

    #[tokio::test]
    async fn handler_defaults_to_system_subject_stamping_no_author_id() {
        // No fixture → current_subject() = None → the Handler resolves
        // Subject::System, which stamps author_id: None (a SubjectId only
        // exists for Authenticated).
        let env = TestEnvironment::new(FixedClock::new(Utc::now()));
        let env_handle = env.clone();

        let handler = Handler::new(AggregateLogic, NoOpCallExecutor, NoOpQueryFetcher, env);
        handler.handle(CreateCmd { id: 2 }).await.unwrap();

        let stored = env_handle.event_store().events_for_stream("agg-2");
        assert_eq!(stored.len(), 1);
        let metadata = stored[0]
            .metadata
            .as_ref()
            .expect("handler must stamp metadata even for the System subject");
        assert_eq!(
            metadata.author_id, None,
            "Subject::System must stamp author_id: None"
        );
        assert_eq!(metadata.origin_subject_id, None);
    }
}
