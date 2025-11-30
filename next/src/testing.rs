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
    Clock, EventBus, EventBusError, EventStore, EventStoreError, HandlerEnvironment,
    MetadataContext, NoOpProjectionQueries, ProjectionError, ProjectionQueries, Projector,
    SerializedEvent, StreamId, Version,
};
use std::collections::HashMap;
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

        #[allow(clippy::cast_possible_truncation)] // Event streams won't exceed usize on 32-bit
        let start_idx = from_version.map_or(0, |v| v.as_u64() as usize);

        Ok(events.iter().skip(start_idx).cloned().collect())
    }

    #[allow(clippy::expect_used)] // Test infrastructure
    async fn append(
        &self,
        stream_id: &StreamId,
        expected_version: Option<Version>,
        events: Vec<SerializedEvent>,
    ) -> Result<Version, EventStoreError> {
        let mut streams = self
            .streams
            .write()
            .expect("InMemoryEventStore lock poisoned");

        let stream = streams
            .entry(stream_id.as_str().to_string())
            .or_default();
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

        // Append events with version numbers
        let start_version = current_version.as_u64() + 1;
        for (i, mut event) in events.into_iter().enumerate() {
            event.version = Some(Version::new(start_version + i as u64));
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
#[derive(Debug, Clone)]
pub struct TestEnvironment<C: Clock, P: ProjectionQueries = NoOpProjectionQueries> {
    clock: C,
    event_store: InMemoryEventStore,
    projector: InMemoryProjector,
    event_bus: InMemoryEventBus,
    broadcast_topic: String,
    projections: P,
    metadata: MetadataContext,
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
        }
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
}

// ═══════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::FixedClock;
    use chrono::Utc;

    fn make_event(event_type: &str) -> SerializedEvent {
        SerializedEvent {
            event_type: event_type.to_string(),
            payload: vec![1, 2, 3],
            metadata: None,
            version: None,
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

        // Load from version
        let events = store
            .load(&stream_id, Some(Version::new(1)))
            .await
            .unwrap();
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

        assert!(
            matches!(result, Err(ProjectionError::Custom(msg)) if msg == "Database down")
        );
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

        assert_eq!(
            env.metadata().correlation_id,
            Some("test-123".to_string())
        );
        assert_eq!(env.metadata().user_id, Some("user-abc".to_string()));
    }
}
