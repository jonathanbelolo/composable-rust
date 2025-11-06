//! # Composable Rust Testing
//!
//! Testing utilities and helpers for the Composable Rust architecture.
//!
//! This crate provides:
//! - Mock implementations of Environment traits
//! - Test helpers and builders
//! - Property-based testing utilities
//! - Assertion helpers for reducers and stores
//!
//! ## Example
//!
//! ```ignore
//! use composable_rust_testing::test_clock;
//! use composable_rust_runtime::Store;
//!
//! #[tokio::test]
//! async fn test_order_flow() {
//!     let env = test_environment();
//!     let store = OrderStore::new(OrderState::default(), OrderReducer, env);
//!
//!     store.send(OrderAction::PlaceOrder {
//!         customer_id: CustomerId::new(1),
//!         items: vec![],
//!     }).await;
//!
//!     let state = store.state(|s| s.clone()).await;
//!     assert_eq!(state.orders.len(), 1);
//! }
//! ```

use chrono::{DateTime, Utc};
use composable_rust_core::environment::Clock;

/// Mock implementations of Environment traits
///
/// # Phase 1 Implementation
///
/// This module will contain:
/// - `MockDatabase`: In-memory event store
/// - `FixedClock`: Deterministic time
/// - `MockEventPublisher`: Captures published events
/// - `MockHttpClient`: Stubbed HTTP responses
/// - `SequentialIdGenerator`: Predictable IDs
///
/// Mock implementations for testing.
pub mod mocks {
    use super::{Clock, DateTime, Utc};
    use chrono::Duration;
    use std::sync::{Arc, RwLock};

    /// Fixed clock for deterministic tests
    ///
    /// Provides controllable time for testing time-based behavior.
    /// Unlike a real clock, this clock only advances when you call `advance()`.
    ///
    /// # Example
    ///
    /// ```
    /// use composable_rust_testing::mocks::FixedClock;
    /// use composable_rust_core::environment::Clock;
    /// use chrono::{Duration, Utc};
    ///
    /// let clock = FixedClock::new(Utc::now());
    /// let time1 = clock.now();
    /// let time2 = clock.now();
    /// assert_eq!(time1, time2); // Always the same until advanced
    ///
    /// // Simulate time passage
    /// clock.advance(Duration::hours(1));
    /// let time3 = clock.now();
    /// assert_eq!(time3, time1 + Duration::hours(1));
    /// ```
    #[derive(Debug, Clone)]
    pub struct FixedClock {
        time: Arc<RwLock<DateTime<Utc>>>,
    }

    impl FixedClock {
        /// Create a new fixed clock with the given time
        #[must_use]
        pub fn new(time: DateTime<Utc>) -> Self {
            Self {
                time: Arc::new(RwLock::new(time)),
            }
        }

        /// Advance the clock by the given duration
        ///
        /// This simulates the passage of time in tests, allowing you to
        /// trigger time-based effects (delays, timeouts, etc.) deterministically.
        ///
        /// # Arguments
        ///
        /// - `duration`: How much to advance the clock
        ///
        /// # Example
        ///
        /// ```
        /// use composable_rust_testing::{test_clock, mocks::FixedClock};
        /// use composable_rust_core::environment::Clock;
        /// use chrono::Duration;
        ///
        /// let clock = test_clock();
        /// let start = clock.now();
        ///
        /// clock.advance(Duration::seconds(30));
        /// assert_eq!(clock.now(), start + Duration::seconds(30));
        ///
        /// clock.advance(Duration::minutes(5));
        /// assert_eq!(clock.now(), start + Duration::seconds(330)); // 30s + 5min
        /// ```
        ///
        /// # Panics
        ///
        /// Panics if the `RwLock` is poisoned (which should never happen in normal use).
        #[allow(clippy::expect_used)] // Test infrastructure, lock poison is unrecoverable
        pub fn advance(&self, duration: Duration) {
            let mut time = self
                .time
                .write()
                .expect("FixedClock lock poisoned - test infrastructure error");
            *time += duration;
        }

        /// Set the clock to a specific time
        ///
        /// Unlike `advance()`, this sets an absolute time rather than
        /// advancing by a duration.
        ///
        /// # Arguments
        ///
        /// - `time`: The new time to set
        ///
        /// # Example
        ///
        /// ```
        /// use composable_rust_testing::{test_clock, mocks::FixedClock};
        /// use composable_rust_core::environment::Clock;
        /// use chrono::{DateTime, Utc};
        ///
        /// let clock = test_clock();
        ///
        /// let new_time = DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
        ///     .unwrap()
        ///     .with_timezone(&Utc);
        ///
        /// clock.set(new_time);
        /// assert_eq!(clock.now(), new_time);
        /// ```
        ///
        /// # Panics
        ///
        /// Panics if the `RwLock` is poisoned (which should never happen in normal use).
        #[allow(clippy::expect_used)] // Test infrastructure, lock poison is unrecoverable
        pub fn set(&self, time: DateTime<Utc>) {
            let mut current_time = self
                .time
                .write()
                .expect("FixedClock lock poisoned - test infrastructure error");
            *current_time = time;
        }
    }

    impl Clock for FixedClock {
        /// Get the current time from this fixed clock
        ///
        /// # Panics
        ///
        /// Panics if the `RwLock` is poisoned (which should never happen in normal use).
        #[allow(clippy::expect_used)] // Test infrastructure, lock poison is unrecoverable
        fn now(&self) -> DateTime<Utc> {
            *self
                .time
                .read()
                .expect("FixedClock lock poisoned - test infrastructure error")
        }
    }

    /// Create a default fixed clock for tests (2025-01-01 00:00:00 UTC)
    ///
    /// # Panics
    ///
    /// This function will panic if the hardcoded timestamp fails to parse,
    /// which should never happen in practice.
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn test_clock() -> FixedClock {
        FixedClock::new(
            DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .expect("hardcoded timestamp should always parse")
                .with_timezone(&Utc),
        )
    }

    /// Type alias for snapshot storage: maps `stream_id` to `(version, state_bytes)`
    type SnapshotMap =
        std::collections::HashMap<String, (composable_rust_core::stream::Version, Vec<u8>)>;

    /// In-memory event store for fast, deterministic unit tests.
    ///
    /// This implementation uses `HashMap` for storage and provides the same
    /// optimistic concurrency semantics as `PostgresEventStore`, making it perfect
    /// for testing event-sourced aggregates without requiring a database.
    ///
    /// # Example
    ///
    /// ```
    /// use composable_rust_testing::mocks::InMemoryEventStore;
    /// use composable_rust_core::event_store::EventStore;
    /// use composable_rust_core::stream::{StreamId, Version};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let store = InMemoryEventStore::new();
    ///
    /// let stream_id = StreamId::new("order-123");
    /// let events = vec![/* SerializedEvent instances */];
    ///
    /// // Append events with optimistic concurrency
    /// let version = store.append_events(
    ///     stream_id.clone(),
    ///     Some(Version::new(0)),
    ///     events
    /// ).await?;
    ///
    /// // Load events back
    /// let loaded = store.load_events(stream_id, None).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[derive(Debug, Clone, Default)]
    pub struct InMemoryEventStore {
        /// Events indexed by `stream_id`, stored in version order
        events: Arc<
            RwLock<
                std::collections::HashMap<
                    String,
                    Vec<composable_rust_core::event::SerializedEvent>,
                >,
            >,
        >,
        /// Snapshots indexed by `stream_id`
        snapshots: Arc<RwLock<SnapshotMap>>,
    }

    impl InMemoryEventStore {
        /// Create a new empty in-memory event store.
        #[must_use]
        pub fn new() -> Self {
            Self {
                events: Arc::new(RwLock::new(std::collections::HashMap::new())),
                snapshots: Arc::new(RwLock::new(std::collections::HashMap::new())),
            }
        }

        /// Reset the event store to empty state.
        ///
        /// Useful for test isolation when reusing a store instance.
        ///
        /// # Panics
        ///
        /// Panics if the `RwLock` is poisoned.
        #[allow(clippy::expect_used)] // Test infrastructure, lock poison is unrecoverable
        pub fn reset(&self) {
            self.events
                .write()
                .expect("InMemoryEventStore lock poisoned")
                .clear();
            self.snapshots
                .write()
                .expect("InMemoryEventStore lock poisoned")
                .clear();
        }

        /// Get the current version for a stream.
        ///
        /// Returns `Version(0)` if the stream doesn't exist.
        ///
        /// # Panics
        ///
        /// Panics if the `RwLock` is poisoned.
        #[must_use]
        #[allow(clippy::expect_used)] // Test infrastructure, lock poison is unrecoverable
        pub fn current_version(
            &self,
            stream_id: &composable_rust_core::stream::StreamId,
        ) -> composable_rust_core::stream::Version {
            let events = self
                .events
                .read()
                .expect("InMemoryEventStore lock poisoned");

            events
                .get(stream_id.as_str())
                .map_or(composable_rust_core::stream::Version::new(0), |v| {
                    composable_rust_core::stream::Version::new(v.len() as u64)
                })
        }

        /// Get the total number of events in a stream.
        ///
        /// Returns 0 if the stream doesn't exist.
        ///
        /// # Panics
        ///
        /// Panics if the `RwLock` is poisoned.
        #[must_use]
        #[allow(clippy::expect_used)] // Test infrastructure, lock poison is unrecoverable
        pub fn event_count(&self, stream_id: &composable_rust_core::stream::StreamId) -> usize {
            let events = self
                .events
                .read()
                .expect("InMemoryEventStore lock poisoned");

            events.get(stream_id.as_str()).map_or(0, Vec::len)
        }

        /// Check if a stream exists.
        ///
        /// # Panics
        ///
        /// Panics if the `RwLock` is poisoned.
        #[must_use]
        #[allow(clippy::expect_used)] // Test infrastructure, lock poison is unrecoverable
        pub fn stream_exists(&self, stream_id: &composable_rust_core::stream::StreamId) -> bool {
            let events = self
                .events
                .read()
                .expect("InMemoryEventStore lock poisoned");

            events.contains_key(stream_id.as_str())
        }
    }

    impl composable_rust_core::event_store::EventStore for InMemoryEventStore {
        fn append_events(
            &self,
            stream_id: composable_rust_core::stream::StreamId,
            expected_version: Option<composable_rust_core::stream::Version>,
            mut events: Vec<composable_rust_core::event::SerializedEvent>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            composable_rust_core::stream::Version,
                            composable_rust_core::event_store::EventStoreError,
                        >,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async move {
                if events.is_empty() {
                    return Err(
                        composable_rust_core::event_store::EventStoreError::DatabaseError(
                            "Cannot append empty event list".to_string(),
                        ),
                    );
                }

                let mut store = self.events.write().map_err(|e| {
                    composable_rust_core::event_store::EventStoreError::DatabaseError(format!(
                        "Lock poisoned: {e}"
                    ))
                })?;

                let stream_events = store.entry(stream_id.as_str().to_string()).or_default();
                let current_version =
                    composable_rust_core::stream::Version::new(stream_events.len() as u64);

                // Check optimistic concurrency
                if let Some(expected) = expected_version {
                    if current_version != expected {
                        return Err(composable_rust_core::event_store::EventStoreError::ConcurrencyConflict {
                            stream_id,
                            expected,
                            actual: current_version,
                        });
                    }
                }

                // Append events
                stream_events.append(&mut events);
                let new_version =
                    composable_rust_core::stream::Version::new(stream_events.len() as u64);

                Ok(new_version - 1)
            })
        }

        fn load_events(
            &self,
            stream_id: composable_rust_core::stream::StreamId,
            from_version: Option<composable_rust_core::stream::Version>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            Vec<composable_rust_core::event::SerializedEvent>,
                            composable_rust_core::event_store::EventStoreError,
                        >,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async move {
                let store = self.events.read().map_err(|e| {
                    composable_rust_core::event_store::EventStoreError::DatabaseError(format!(
                        "Lock poisoned: {e}"
                    ))
                })?;

                let stream_events = store.get(stream_id.as_str());

                match (stream_events, from_version) {
                    (Some(events), Some(from_ver)) => {
                        let start_idx = usize::try_from(from_ver.value()).map_err(|e| {
                            composable_rust_core::event_store::EventStoreError::DatabaseError(
                                format!("Version too large for usize: {e}"),
                            )
                        })?;
                        Ok(events.get(start_idx..).unwrap_or(&[]).to_vec())
                    },
                    (Some(events), None) => Ok(events.clone()),
                    (None, _) => Ok(vec![]),
                }
            })
        }

        fn save_snapshot(
            &self,
            stream_id: composable_rust_core::stream::StreamId,
            version: composable_rust_core::stream::Version,
            state: Vec<u8>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<(), composable_rust_core::event_store::EventStoreError>,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async move {
                let mut store = self.snapshots.write().map_err(|e| {
                    composable_rust_core::event_store::EventStoreError::DatabaseError(format!(
                        "Lock poisoned: {e}"
                    ))
                })?;

                store.insert(stream_id.as_str().to_string(), (version, state));
                Ok(())
            })
        }

        fn load_snapshot(
            &self,
            stream_id: composable_rust_core::stream::StreamId,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            Option<(composable_rust_core::stream::Version, Vec<u8>)>,
                            composable_rust_core::event_store::EventStoreError,
                        >,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async move {
                let store = self.snapshots.read().map_err(|e| {
                    composable_rust_core::event_store::EventStoreError::DatabaseError(format!(
                        "Lock poisoned: {e}"
                    ))
                })?;

                Ok(store.get(stream_id.as_str()).cloned())
            })
        }
    }

    /// In-memory event bus for fast, deterministic unit tests.
    ///
    /// This implementation uses `HashMap` and tokio channels for storage and delivery,
    /// providing the same publish/subscribe semantics as `RedpandaEventBus` but with
    /// synchronous delivery and no network overhead.
    ///
    /// # Features
    ///
    /// - **Synchronous delivery**: Events delivered immediately when published
    /// - **Multiple subscribers**: Each topic can have multiple concurrent subscribers
    /// - **Test inspection**: Methods to inspect topic and subscriber counts
    /// - **Thread-safe**: Safe to use across async tasks
    ///
    /// # Example
    ///
    /// ```
    /// use composable_rust_testing::mocks::InMemoryEventBus;
    /// use composable_rust_core::event_bus::EventBus;
    /// use composable_rust_core::event::SerializedEvent;
    /// use futures::StreamExt;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let event_bus = InMemoryEventBus::new();
    ///
    /// // Publish an event
    /// let event = SerializedEvent::new(
    ///     "OrderPlaced".to_string(),
    ///     vec![1, 2, 3],
    ///     None,
    /// );
    /// event_bus.publish("order-events", &event).await?;
    ///
    /// // Subscribe to events
    /// let mut stream = event_bus.subscribe(&["order-events"]).await?;
    /// if let Some(result) = stream.next().await {
    ///     let received_event = result?;
    ///     assert_eq!(received_event.event_type, "OrderPlaced");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[derive(Debug, Clone, Default)]
    pub struct InMemoryEventBus {
        /// Subscribers indexed by topic
        /// Each topic has a list of channels to deliver events to
        subscribers: Arc<
            RwLock<
                std::collections::HashMap<
                    String,
                    Vec<
                        tokio::sync::mpsc::UnboundedSender<
                            composable_rust_core::event::SerializedEvent,
                        >,
                    >,
                >,
            >,
        >,
    }

    impl InMemoryEventBus {
        /// Create a new empty in-memory event bus.
        #[must_use]
        pub fn new() -> Self {
            Self {
                subscribers: Arc::new(RwLock::new(std::collections::HashMap::new())),
            }
        }

        /// Get the number of topics that have active subscribers.
        ///
        /// # Panics
        ///
        /// Panics if the `RwLock` is poisoned.
        #[must_use]
        #[allow(clippy::expect_used)] // Test infrastructure, lock poison is unrecoverable
        pub fn topic_count(&self) -> usize {
            self.subscribers
                .read()
                .expect("InMemoryEventBus lock poisoned")
                .len()
        }

        /// Get the number of active subscribers for a topic.
        ///
        /// Returns 0 if the topic doesn't exist or has no subscribers.
        ///
        /// # Panics
        ///
        /// Panics if the `RwLock` is poisoned.
        #[must_use]
        #[allow(clippy::expect_used)] // Test infrastructure, lock poison is unrecoverable
        pub fn subscriber_count(&self, topic: &str) -> usize {
            let subscribers = self
                .subscribers
                .read()
                .expect("InMemoryEventBus lock poisoned");

            subscribers.get(topic).map_or(0, Vec::len)
        }

        /// Reset the event bus by closing all subscriptions.
        ///
        /// Useful for test isolation when reusing an event bus instance.
        ///
        /// # Panics
        ///
        /// Panics if the `RwLock` is poisoned.
        #[allow(clippy::expect_used)] // Test infrastructure, lock poison is unrecoverable
        pub fn reset(&self) {
            self.subscribers
                .write()
                .expect("InMemoryEventBus lock poisoned")
                .clear();
        }
    }

    impl composable_rust_core::event_bus::EventBus for InMemoryEventBus {
        fn publish(
            &self,
            topic: &str,
            event: &composable_rust_core::event::SerializedEvent,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<(), composable_rust_core::event_bus::EventBusError>,
                    > + Send
                    + '_,
            >,
        > {
            // Clone data before moving into async block
            let topic = topic.to_string();
            let event = event.clone();

            Box::pin(async move {
                let subscribers_lock = self.subscribers.read().map_err(|e| {
                    composable_rust_core::event_bus::EventBusError::PublishFailed {
                        topic: topic.clone(),
                        reason: format!("Lock poisoned: {e}"),
                    }
                })?;

                if let Some(topic_subscribers) = subscribers_lock.get(&topic) {
                    // Send to all subscribers (at-least-once semantics)
                    for sender in topic_subscribers {
                        // Ignore send errors - subscriber might have dropped
                        // This mirrors real event bus behavior where subscribers can disconnect
                        let _ = sender.send(event.clone());
                    }
                }

                Ok(())
            })
        }

        fn subscribe(
            &self,
            topics: &[&str],
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            composable_rust_core::event_bus::EventStream,
                            composable_rust_core::event_bus::EventBusError,
                        >,
                    > + Send
                    + '_,
            >,
        > {
            // Clone topics before moving into async block
            let topics: Vec<String> = topics.iter().map(|s| (*s).to_string()).collect();

            Box::pin(async move {
                // Create a channel for this subscription
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<
                    composable_rust_core::event::SerializedEvent,
                >();

                // Register this subscriber for all requested topics
                {
                    let mut subscribers = self.subscribers.write().map_err(|e| {
                        composable_rust_core::event_bus::EventBusError::SubscriptionFailed {
                            topics: topics.clone(),
                            reason: format!("Lock poisoned: {e}"),
                        }
                    })?;

                    for topic in &topics {
                        subscribers
                            .entry(topic.clone())
                            .or_default()
                            .push(tx.clone());
                    }
                }

                // Create a stream from the receiver
                let stream = async_stream::stream! {
                    while let Some(event) = rx.recv().await {
                        yield Ok(event);
                    }
                };

                Ok(Box::pin(stream)
                    as std::pin::Pin<
                        Box<
                            dyn futures::Stream<
                                    Item = Result<
                                        composable_rust_core::event::SerializedEvent,
                                        composable_rust_core::event_bus::EventBusError,
                                    >,
                                > + Send,
                        >,
                    >)
            })
        }
    }
}

/// Test helpers and utilities
///
/// # Phase 1 Implementation
///
/// This module will contain:
/// - Builder patterns for common test scenarios
/// - Assertion helpers
/// - Test data generators
///
/// Test helpers and utilities.
pub mod helpers {
    // Placeholder for test helpers
}

/// Property-based testing utilities
///
/// # Phase 1 Implementation
///
/// This module will contain:
/// - proptest Arbitrary implementations
/// - Custom strategies for domain types
/// - Property test helpers
///
/// Property-based testing utilities using proptest.
pub mod properties {
    // Placeholder for property test utilities
}

/// `TestStore` - Test-specific store wrapper for effect tracking
///
/// # Phase 1 Implementation
///
/// Provides deterministic effect testing by queueing actions instead of
/// auto-feeding them back to the store.
///
/// `TestStore` for deterministic effect testing
#[allow(clippy::doc_markdown)] // Allow missing backticks in module docs
#[allow(clippy::unwrap_used)] // Test code uses unwrap for simplicity
#[allow(clippy::missing_panics_doc)] // Test utilities document panics where critical
#[allow(clippy::missing_errors_doc)] // Test utilities document errors where critical
#[allow(clippy::mismatching_type_param_order)] // Generic A conflicts with Vec<A>
pub mod test_store {
    use composable_rust_core::reducer::Reducer;
    use composable_rust_runtime::{EffectHandle, Store};
    use std::collections::VecDeque;
    use std::fmt::Debug;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use thiserror::Error;

    /// Errors that can occur during test store operations
    #[derive(Error, Debug)]
    pub enum TestStoreError {
        /// Expected action was not found in the queue
        #[error("Expected action not found. Queue: {queue:?}")]
        ActionNotFound {
            /// Current queue contents
            queue: String,
        },

        /// Actions were not in expected order
        #[error("Actions not in expected order. Expected {expected:?} but queue: {queue:?}")]
        WrongOrder {
            /// Expected pattern
            expected: String,
            /// Current queue
            queue: String,
        },

        /// Timeout waiting for effects
        #[error("Timeout waiting for effects to complete")]
        Timeout,

        /// Action was found but in wrong position (for ordered receive)
        #[error("Action found at wrong position. Expected at front, found at index {index}")]
        WrongPosition {
            /// Index where action was found
            index: usize,
        },
    }

    /// Trait for expected actions dispatch
    ///
    /// Implemented for both single actions (`A`) and ordered lists (`Vec<A>`).
    /// This enables type-based method dispatch for `receive()`.
    pub trait ExpectedActions<A> {
        /// Match and remove actions from queue
        ///
        /// # Returns
        ///
        /// Ok(()) if actions match and were removed, Err otherwise
        fn match_and_remove(&self, queue: &mut VecDeque<A>) -> Result<(), TestStoreError>
        where
            A: Debug + PartialEq;
    }

    impl<A> ExpectedActions<A> for A
    where
        A: Debug + PartialEq,
    {
        fn match_and_remove(&self, queue: &mut VecDeque<A>) -> Result<(), TestStoreError> {
            // Single action: must be at front
            if let Some(front) = queue.front() {
                if front == self {
                    queue.pop_front();
                    return Ok(());
                }
            }

            // Not at front - check if it exists elsewhere
            if let Some((index, _)) = queue.iter().enumerate().find(|(_, a)| *a == self) {
                return Err(TestStoreError::WrongPosition { index });
            }

            Err(TestStoreError::ActionNotFound {
                queue: format!("{queue:?}"),
            })
        }
    }

    impl<A> ExpectedActions<A> for Vec<A>
    where
        A: Debug + PartialEq,
    {
        fn match_and_remove(&self, queue: &mut VecDeque<A>) -> Result<(), TestStoreError> {
            // Vec: ordered matching
            if queue.len() < self.len() {
                return Err(TestStoreError::ActionNotFound {
                    queue: format!("{queue:?}"),
                });
            }

            // Check if front of queue matches expected order
            for (i, expected) in self.iter().enumerate() {
                if queue.get(i) != Some(expected) {
                    return Err(TestStoreError::WrongOrder {
                        expected: format!("{self:?}"),
                        queue: format!("{queue:?}"),
                    });
                }
            }

            // All matched - remove them
            for _ in 0..self.len() {
                queue.pop_front();
            }

            Ok(())
        }
    }

    /// Test-specific store wrapper for deterministic effect testing
    ///
    /// # Purpose
    ///
    /// TestStore queues actions produced by effects instead of automatically
    /// feeding them back to the store. This allows tests to:
    /// - Assert on intermediate actions
    /// - Inspect state between cascading actions
    /// - Control when feedback happens
    /// - Test effects deterministically
    ///
    /// # Example
    ///
    /// ```ignore
    /// #[tokio::test(start_paused = true)]
    /// async fn test_workflow() {
    ///     let store = TestStore::new(reducer, env, State::default());
    ///
    ///     let h1 = store.send(Action::Start).await;
    ///     tokio::time::advance(Duration::from_millis(100)).await;
    ///
    ///     let h2 = store.receive_after(Action::Middle, h1).await.unwrap();
    ///     assert_eq!(store.state(|s| s.step).await, Step::MiddleComplete);
    ///
    ///     tokio::time::advance(Duration::from_millis(100)).await;
    ///     store.receive_after(Action::End, h2).await.unwrap();
    ///
    ///     store.assert_no_pending_actions();
    /// }
    /// ```
    pub struct TestStore<S, A, E, R>
    where
        R: Reducer<State = S, Action = A, Environment = E>,
        A: Debug,
    {
        store: Store<S, A, E, R>,
        pub(crate) effect_queue: Arc<Mutex<VecDeque<A>>>,
    }

    impl<S, A, E, R> TestStore<S, A, E, R>
    where
        R: Reducer<State = S, Action = A, Environment = E> + Send + Sync + Clone + 'static,
        A: Send + Clone + Debug + PartialEq + 'static,
        S: Send + Sync + 'static,
        E: Send + Sync + Clone + 'static,
    {
        /// Create a new test store
        ///
        /// # Arguments
        ///
        /// - `reducer`: The reducer implementation
        /// - `environment`: Injected dependencies
        /// - `initial_state`: Starting state
        ///
        /// # Returns
        ///
        /// A new TestStore instance
        #[must_use]
        pub fn new(reducer: R, environment: E, initial_state: S) -> Self {
            let store = Store::new(initial_state, reducer, environment);
            let effect_queue = Arc::new(Mutex::new(VecDeque::new()));

            Self {
                store,
                effect_queue,
            }
        }

        /// Send an action to the store with Direct tracking
        ///
        /// Actions produced by effects are queued instead of auto-fed back.
        ///
        /// # Arguments
        ///
        /// - `action`: The action to process
        ///
        /// # Returns
        ///
        /// An [`EffectHandle`] for waiting on effect completion
        pub async fn send(&self, action: A) -> EffectHandle {
            // TODO: Use queued feedback destination
            // For now, just use normal store send
            self.store.send(action).await
        }

        /// Read current state via a closure
        ///
        /// # Arguments
        ///
        /// - `f`: Closure that receives a reference to state
        ///
        /// # Returns
        ///
        /// The value returned by the closure
        pub async fn state<F, T>(&self, f: F) -> T
        where
            F: FnOnce(&S) -> T,
        {
            self.store.state(f).await
        }

        /// Receive an expected action from the effect queue
        ///
        /// # Type-Based Dispatch
        ///
        /// - `receive(Action::Foo)` - matches single action at front
        /// - `receive(vec![Action::A, Action::B])` - matches ordered sequence
        ///
        /// # Arguments
        ///
        /// - `expected`: The expected action(s)
        ///
        /// # Returns
        ///
        /// - `Ok(EffectHandle)` - matched and removed, returns handle for subsequent actions
        /// - `Err(TestStoreError)` - mismatch or not found
        pub async fn receive<Exp>(&self, expected: Exp) -> Result<EffectHandle, TestStoreError>
        where
            Exp: ExpectedActions<A>,
        {
            // Yield to allow any pending tasks to complete
            tokio::task::yield_now().await;

            let mut queue = self.effect_queue.lock().unwrap();
            expected.match_and_remove(&mut *queue)?;

            // Return handle for any subsequent actions from this receive
            Ok(EffectHandle::completed())
        }

        /// Receive and wait for handle first, then match
        ///
        /// Ergonomic API that waits for the handle to complete before
        /// attempting to receive.
        ///
        /// # Arguments
        ///
        /// - `expected`: The expected action(s)
        /// - `handle`: Handle to wait on before receiving
        ///
        /// # Returns
        ///
        /// Handle for subsequent actions, or error
        pub async fn receive_after<Exp>(
            &self,
            expected: Exp,
            mut handle: EffectHandle,
        ) -> Result<EffectHandle, TestStoreError>
        where
            Exp: ExpectedActions<A>,
        {
            handle
                .wait_with_timeout(Duration::from_secs(30))
                .await
                .map_err(|()| TestStoreError::Timeout)?;

            self.receive(expected).await
        }

        /// Receive actions in any order (unordered matching)
        ///
        /// # Arguments
        ///
        /// - `expected`: Vec of actions to match (order doesn't matter)
        ///
        /// # Returns
        ///
        /// Handle for subsequent actions, or error
        pub async fn receive_unordered(
            &self,
            expected: Vec<A>,
        ) -> Result<EffectHandle, TestStoreError> {
            // Yield to allow any pending tasks to complete
            tokio::task::yield_now().await;

            let mut queue = self.effect_queue.lock().unwrap();

            if queue.len() < expected.len() {
                return Err(TestStoreError::ActionNotFound {
                    queue: format!("{queue:?}"),
                });
            }

            // Try to find and remove each expected action
            for exp in &expected {
                if let Some(pos) = queue.iter().position(|a| a == exp) {
                    queue.remove(pos);
                } else {
                    return Err(TestStoreError::ActionNotFound {
                        queue: format!("{queue:?}"),
                    });
                }
            }

            Ok(EffectHandle::completed())
        }

        /// Receive unordered after waiting on handle
        ///
        /// # Arguments
        ///
        /// - `expected`: Vec of actions (order independent)
        /// - `handle`: Handle to wait on first
        ///
        /// # Returns
        ///
        /// Handle for subsequent actions, or error
        pub async fn receive_unordered_after(
            &self,
            expected: Vec<A>,
            mut handle: EffectHandle,
        ) -> Result<EffectHandle, TestStoreError> {
            handle
                .wait_with_timeout(Duration::from_secs(30))
                .await
                .map_err(|()| TestStoreError::Timeout)?;

            self.receive_unordered(expected).await
        }

        /// Assert that there are no pending actions in the queue
        ///
        /// # Panics
        ///
        /// Panics if queue is not empty
        #[allow(clippy::unwrap_used)] // Test infrastructure, mutex poison is unrecoverable
        pub fn assert_no_pending_actions(&self) {
            let queue = self.effect_queue.lock().unwrap();
            assert!(
                queue.is_empty(),
                "Expected no pending actions, but found {} in queue: {:?}",
                queue.len(),
                queue
            );
        }

        /// Peek at the next action without removing it
        ///
        /// # Returns
        ///
        /// Reference to the front action if any
        #[must_use]
        pub fn peek_next(&self) -> Option<A> {
            let queue = self.effect_queue.lock().unwrap();
            queue.front().cloned()
        }
    }

    impl<S, A, E, R> Drop for TestStore<S, A, E, R>
    where
        R: Reducer<State = S, Action = A, Environment = E>,
        A: Debug,
    {
        #[allow(clippy::unwrap_used)] // Test infrastructure, mutex poison is unrecoverable
        #[allow(clippy::panic)] // Intentional: Drop panics if test leaves unprocessed actions
        fn drop(&mut self) {
            // Don't panic if we're already panicking
            if std::thread::panicking() {
                return;
            }

            let queue = self.effect_queue.lock().unwrap();
            assert!(
                queue.is_empty(),
                "TestStore dropped with {} unprocessed actions: {:?}",
                queue.len(),
                *queue
            );
        }
    }
}

// Re-export commonly used items
pub use mocks::{FixedClock, test_clock};
pub use test_store::{ExpectedActions, TestStore, TestStoreError};

// Placeholder test module
#[cfg(test)]
#[allow(clippy::unwrap_used)] // Tests can unwrap
#[allow(clippy::panic)] // Tests can panic
#[allow(dead_code)] // Test types may have unused variants
mod tests {
    use super::*;
    use composable_rust_core::{effect::Effect, reducer::Reducer};

    #[test]
    fn test_fixed_clock() {
        let clock = test_clock();
        let time1 = clock.now();
        let time2 = clock.now();
        assert_eq!(time1, time2);
    }

    #[test]
    fn test_fixed_clock_advance() {
        use chrono::Duration;

        let clock = test_clock();
        let start = clock.now();

        // Advance by 1 hour
        clock.advance(Duration::hours(1));
        let after_hour = clock.now();
        assert_eq!(after_hour, start + Duration::hours(1));

        // Advance by 30 seconds
        clock.advance(Duration::seconds(30));
        let after_seconds = clock.now();
        assert_eq!(
            after_seconds,
            start + Duration::hours(1) + Duration::seconds(30)
        );

        // Advance by negative duration (go backwards)
        clock.advance(Duration::seconds(-30));
        let after_backwards = clock.now();
        assert_eq!(after_backwards, start + Duration::hours(1));
    }

    #[test]
    fn test_fixed_clock_set() {
        use chrono::DateTime;

        let clock = test_clock();
        let original = clock.now();

        // Set to a specific time
        let new_time = DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        clock.set(new_time);
        assert_eq!(clock.now(), new_time);

        // Setting doesn't affect previous value
        assert_ne!(original, new_time);

        // Can set again
        let another_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        clock.set(another_time);
        assert_eq!(clock.now(), another_time);
    }

    // TestStore tests
    #[derive(Debug, Clone, PartialEq)]
    enum TestAction {
        Action1,
        Action2,
        Action3,
        ProduceAction(Box<TestAction>),
        ProduceMultiple(Vec<TestAction>),
    }

    #[derive(Debug, Clone)]
    struct TestState {
        value: i32,
    }

    #[derive(Debug, Clone)]
    struct TestEnv;

    #[derive(Debug, Clone)]
    struct TestReducer;

    impl Reducer for TestReducer {
        type State = TestState;
        type Action = TestAction;
        type Environment = TestEnv;

        fn reduce(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            _env: &Self::Environment,
        ) -> Vec<Effect<Self::Action>> {
            match action {
                TestAction::Action1 => {
                    state.value += 1;
                    vec![]
                },
                TestAction::Action2 => {
                    state.value += 2;
                    vec![]
                },
                TestAction::Action3 => {
                    state.value += 3;
                    vec![]
                },
                TestAction::ProduceAction(action) => {
                    vec![Effect::Future(Box::pin(async move { Some(*action) }))]
                },
                TestAction::ProduceMultiple(actions) => {
                    vec![Effect::Parallel(
                        actions
                            .into_iter()
                            .map(|a| Effect::Future(Box::pin(async move { Some(a) })))
                            .collect(),
                    )]
                },
            }
        }
    }

    #[tokio::test]
    async fn test_teststore_basic_send_and_state() {
        let store = TestStore::new(TestReducer, TestEnv, TestState { value: 0 });

        let _ = store.send(TestAction::Action1).await;
        let value = store.state(|s| s.value).await;
        assert_eq!(value, 1);

        store.assert_no_pending_actions();
    }

    #[tokio::test]
    async fn test_teststore_receive_single_action() {
        let store = TestStore::new(TestReducer, TestEnv, TestState { value: 0 });

        // Manually add action to queue for testing receive
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.push_back(TestAction::Action1);
        }

        // Receive should match and remove
        let result = store.receive(TestAction::Action1).await;
        assert!(result.is_ok());

        // Queue should be empty now
        store.assert_no_pending_actions();
    }

    #[tokio::test]
    async fn test_teststore_receive_ordered_vec() {
        let store = TestStore::new(TestReducer, TestEnv, TestState { value: 0 });

        // Add multiple actions in order
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.push_back(TestAction::Action1);
            queue.push_back(TestAction::Action2);
            queue.push_back(TestAction::Action3);
        }

        // Receive ordered sequence
        let result = store
            .receive(vec![TestAction::Action1, TestAction::Action2])
            .await;
        assert!(result.is_ok());

        // Action3 should still be in queue
        let next = store.peek_next();
        assert_eq!(next, Some(TestAction::Action3));

        // Clean up remaining action to avoid Drop panic
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.clear();
        }
    }

    #[tokio::test]
    async fn test_teststore_receive_unordered() {
        let store = TestStore::new(TestReducer, TestEnv, TestState { value: 0 });

        // Add actions in one order
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.push_back(TestAction::Action1);
            queue.push_back(TestAction::Action2);
            queue.push_back(TestAction::Action3);
        }

        // Receive in different order - should work
        let result = store
            .receive_unordered(vec![TestAction::Action3, TestAction::Action1])
            .await;
        assert!(result.is_ok());

        // Action2 should remain
        let next = store.peek_next();
        assert_eq!(next, Some(TestAction::Action2));

        // Clean up
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.clear();
        }
    }

    #[tokio::test]
    async fn test_teststore_receive_wrong_action_error() {
        let store = TestStore::new(TestReducer, TestEnv, TestState { value: 0 });

        // Add Action1
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.push_back(TestAction::Action1);
        }

        // Try to receive Action2 which doesn't exist - should fail with ActionNotFound
        let result = store.receive(TestAction::Action2).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            TestStoreError::ActionNotFound { .. } => {},
            _ => panic!("Expected ActionNotFound error"),
        }

        // Clean up
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.clear();
        }
    }

    #[tokio::test]
    async fn test_teststore_receive_wrong_position_error() {
        let store = TestStore::new(TestReducer, TestEnv, TestState { value: 0 });

        // Add two actions
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.push_back(TestAction::Action1);
            queue.push_back(TestAction::Action2);
        }

        // Try to receive Action2 which exists but not at front
        let result = store.receive(TestAction::Action2).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            TestStoreError::WrongPosition { index } => {
                assert_eq!(index, 1); // Action2 is at index 1
            },
            _ => panic!("Expected WrongPosition error"),
        }

        // Clean up
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.clear();
        }
    }

    #[tokio::test]
    async fn test_teststore_receive_wrong_order_error() {
        let store = TestStore::new(TestReducer, TestEnv, TestState { value: 0 });

        // Add actions
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.push_back(TestAction::Action1);
            queue.push_back(TestAction::Action2);
        }

        // Try to receive in wrong order
        let result = store
            .receive(vec![TestAction::Action2, TestAction::Action1])
            .await;
        assert!(result.is_err());

        match result.unwrap_err() {
            TestStoreError::WrongOrder { .. } => {},
            _ => panic!("Expected WrongOrder error"),
        }

        // Clean up
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.clear();
        }
    }

    #[tokio::test]
    async fn test_teststore_receive_action_not_found() {
        let store = TestStore::new(TestReducer, TestEnv, TestState { value: 0 });

        // Empty queue
        let result = store.receive(TestAction::Action1).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            TestStoreError::ActionNotFound { .. } => {},
            _ => panic!("Expected ActionNotFound error"),
        }
    }

    #[tokio::test]
    async fn test_teststore_receive_after_with_handle() {
        let store = TestStore::new(TestReducer, TestEnv, TestState { value: 0 });

        // Add action to queue
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.push_back(TestAction::Action1);
        }

        // Create a completed handle
        let handle = composable_rust_runtime::EffectHandle::completed();

        // receive_after should wait on handle, then receive
        let result = store.receive_after(TestAction::Action1, handle).await;
        assert!(result.is_ok());

        store.assert_no_pending_actions();
    }

    #[tokio::test]
    async fn test_teststore_peek_next() {
        let store = TestStore::new(TestReducer, TestEnv, TestState { value: 0 });

        // Initially empty
        assert_eq!(store.peek_next(), None);

        // Add action
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.push_back(TestAction::Action1);
            queue.push_back(TestAction::Action2);
        }

        // Peek should show Action1 without removing
        assert_eq!(store.peek_next(), Some(TestAction::Action1));
        assert_eq!(store.peek_next(), Some(TestAction::Action1)); // Still there

        // Receive removes it
        let _ = store.receive(TestAction::Action1).await;
        assert_eq!(store.peek_next(), Some(TestAction::Action2));

        // Clean up
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.clear();
        }
    }

    #[tokio::test]
    async fn test_teststore_assert_no_pending_actions_success() {
        let store = TestStore::new(TestReducer, TestEnv, TestState { value: 0 });

        // Should not panic when empty
        store.assert_no_pending_actions();
    }

    #[tokio::test]
    #[should_panic(expected = "Expected no pending actions")]
    async fn test_teststore_assert_no_pending_actions_panic() {
        let store = TestStore::new(TestReducer, TestEnv, TestState { value: 0 });

        // Add action
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.push_back(TestAction::Action1);
        }

        // Should panic
        store.assert_no_pending_actions();
    }

    #[tokio::test]
    #[should_panic(expected = "TestStore dropped with 1 unprocessed actions")]
    async fn test_teststore_drop_panic() {
        let store = TestStore::new(TestReducer, TestEnv, TestState { value: 0 });

        // Add unprocessed action
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.push_back(TestAction::Action1);
        }

        // Drop store - should panic
        drop(store);
    }

    #[tokio::test]
    async fn test_teststore_receive_unordered_duplicates() {
        let store = TestStore::new(TestReducer, TestEnv, TestState { value: 0 });

        // Add duplicate actions
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.push_back(TestAction::Action1);
            queue.push_back(TestAction::Action1);
            queue.push_back(TestAction::Action2);
        }

        // Receive two Action1s unordered
        let result = store
            .receive_unordered(vec![TestAction::Action1, TestAction::Action1])
            .await;
        assert!(result.is_ok());

        // Action2 should remain
        assert_eq!(store.peek_next(), Some(TestAction::Action2));

        // Clean up
        {
            let mut queue = store.effect_queue.lock().unwrap();
            queue.clear();
        }
    }
}
