//! Combinator for running several [`Projector`]s as one.
//!
//! [`Handler`](crate::Handler) drives exactly one projector per environment
//! (via [`HandlerEnvironment::projector`](crate::HandlerEnvironment::projector)).
//! When a single write needs to update more than one read model — for example an
//! aggregate's DTO projector plus one or more standalone measurement / read-model
//! projectors that own their own tables — [`MultiProjector`] fans the same event
//! batch out to each child projector, in order, behind one [`Projector`] impl.
//!
//! Because [`Projector::project`] returns a return-position `impl Future`, the
//! trait is **not** object-safe, so projectors of different concrete types cannot
//! be stored together as `dyn Projector`. [`DynProjector`] restores object safety
//! by boxing the returned future, and a blanket impl makes every [`Projector`] a
//! [`DynProjector`] automatically.
//!
//! # Example
//!
//! ```rust,ignore
//! use composable_rust_next::MultiProjector;
//!
//! let projector = MultiProjector::new()
//!     .with(CargoProjector::new(pool.clone()))          // aggregate DTO projector
//!     .with(CargoThroughputMeasurement::new(pool.clone())); // a separate read model
//! // ... used as the environment's `Projector`.
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::{ProjectionError, Projector, SerializedEvent};

/// Object-safe (dyn-compatible) counterpart to [`Projector`].
///
/// [`Projector::project`] returns an `impl Future` (return-position `impl Trait`
/// in a trait), which makes `Projector` **not** object-safe — a `dyn Projector`
/// cannot be formed, so projectors of different concrete types cannot be collected
/// together. `DynProjector` sidesteps this by returning a boxed
/// [`Future`](std::future::Future) instead, allowing heterogeneous projectors to
/// be stored in, for example, a [`MultiProjector`].
///
/// You should never need to implement this trait by hand: a blanket
/// `impl<P: Projector> DynProjector for P` boxes the future for you, so any
/// [`Projector`] can be used wherever a `DynProjector` is expected.
pub trait DynProjector: Send + Sync {
    /// Project `events` to the read model, returning a boxed future.
    ///
    /// This is the object-safe form of [`Projector::project`]; it carries the
    /// same "await to completion, then the read model is updated" contract.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError`] if the projection fails (database error, etc.).
    fn project_dyn<'a>(
        &'a self,
        events: &'a [SerializedEvent],
    ) -> Pin<Box<dyn Future<Output = Result<(), ProjectionError>> + Send + 'a>>;
}

impl<P: Projector> DynProjector for P {
    fn project_dyn<'a>(
        &'a self,
        events: &'a [SerializedEvent],
    ) -> Pin<Box<dyn Future<Output = Result<(), ProjectionError>> + Send + 'a>> {
        Box::pin(self.project(events))
    }
}

/// A [`Projector`] that fans one event batch out to several child projectors.
///
/// # Semantics
///
/// - **Ordering**: child projectors run in the order they were registered (via
///   [`with`](Self::with) / [`push`](Self::push)). Each is awaited to completion
///   before the next one starts; they do not run concurrently.
/// - **Short-circuit**: the first child that returns `Err` aborts the run.
///   Subsequent projectors are **not** invoked and the error propagates to the
///   caller unchanged.
/// - **Empty**: a `MultiProjector` with no children is a valid no-op that returns
///   `Ok(())`.
///
/// # Atomicity
///
/// There is **no transaction across child projectors**. Each child commits its
/// own writes independently, and a failure in one child does **not** roll back
/// children that already ran. If projectors `[A, B]` run and `B` fails, `A`'s
/// read model is already updated while `B`'s is not, and the error propagates —
/// leaving the read models mutually inconsistent until the write is retried.
///
/// Because [`Handler`](crate::Handler) projects *after* events are persisted, a
/// retry replays the same event batch through every child again. Child
/// projectors should therefore be **idempotent** (e.g. upsert by event/stream
/// version, or `ON CONFLICT DO NOTHING`) so that re-running `A` after `B` failed
/// does not double-apply. If you need stronger atomicity, have the children share
/// a single database transaction rather than composing them here.
///
/// # Cloning
///
/// `MultiProjector` is [`Clone`]; child projectors live behind an [`Arc`], so a
/// clone shares them rather than duplicating them. This lets it drop directly
/// into the [`HandlerEnvironment::Projector`](crate::HandlerEnvironment::Projector)
/// slot of environments that require their projector to be `Clone`.
///
/// # Example
///
/// Compose two *distinct* read-model projectors so a single write fans out to both.
/// (The behavior — registration-order fan-out, short-circuit on error, and use as a
/// real [`HandlerEnvironment::Projector`](crate::HandlerEnvironment::Projector) driven
/// through the [`Handler`](crate::Handler) — is covered by this module's tests.)
///
/// ```rust
/// use composable_rust_next::{MultiProjector, ProjectionError, Projector, SerializedEvent};
///
/// // Read model #1: the aggregate's DTO table.
/// struct ReservationDtoProjector;
/// impl Projector for ReservationDtoProjector {
///     async fn project(&self, _events: &[SerializedEvent]) -> Result<(), ProjectionError> {
///         Ok(()) // ... upsert the reservation DTO row(s) ...
///     }
/// }
///
/// // Read model #2: a separate throughput-measurement table that owns its own schema.
/// struct ThroughputMeasurementProjector;
/// impl Projector for ThroughputMeasurementProjector {
///     async fn project(&self, _events: &[SerializedEvent]) -> Result<(), ProjectionError> {
///         Ok(()) // ... record per-batch throughput ...
///     }
/// }
///
/// // One `Projector` that updates both read models, in registration order.
/// let projector = MultiProjector::new()
///     .with(ReservationDtoProjector)
///     .with(ThroughputMeasurementProjector);
/// let _ = projector; // drop into an environment's `Projector` slot
/// ```
#[derive(Clone, Default)]
pub struct MultiProjector {
    projectors: Vec<Arc<dyn DynProjector>>,
}

impl MultiProjector {
    /// Create an empty `MultiProjector`.
    ///
    /// An empty `MultiProjector` is a valid no-op projector: its
    /// [`project`](Projector::project) returns `Ok(())` without doing anything.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            projectors: Vec::new(),
        }
    }

    /// Add a projector, returning `self` for fluent chaining.
    ///
    /// Accepts any concrete [`Projector`], so heterogeneous projectors can be
    /// combined. Projectors run in the order they are added.
    #[must_use]
    pub fn with(mut self, projector: impl Projector + 'static) -> Self {
        self.push(projector);
        self
    }

    /// Add a projector in place.
    ///
    /// Accepts any concrete [`Projector`], so heterogeneous projectors can be
    /// combined. Projectors run in the order they are added.
    pub fn push(&mut self, projector: impl Projector + 'static) {
        let projector: Arc<dyn DynProjector> = Arc::new(projector);
        self.projectors.push(projector);
    }
}

impl std::fmt::Debug for MultiProjector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiProjector")
            .field("projectors", &self.projectors.len())
            .finish()
    }
}

impl Projector for MultiProjector {
    async fn project(&self, events: &[SerializedEvent]) -> Result<(), ProjectionError> {
        for projector in &self.projectors {
            projector.project_dyn(events).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{InMemoryEventBus, InMemoryEventStore};
    use crate::{
        BusinessLogic, BusinessResult, Clock, FixedClock, HandleResult, Handler,
        HandlerEnvironment, MetadataContext, NoOpCallExecutor, NoOpProjectionQueries,
        NoOpQueryFetcher, StreamId,
    };
    use chrono::Utc;
    use std::convert::Infallible;
    use std::sync::{Mutex, MutexGuard, PoisonError};

    type Log = Arc<Mutex<Vec<(&'static str, usize)>>>;

    /// Lock without `unwrap`, recovering the guard even if the lock is poisoned.
    fn lock<'a>(
        log: &'a Mutex<Vec<(&'static str, usize)>>,
    ) -> MutexGuard<'a, Vec<(&'static str, usize)>> {
        log.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Builds a minimal event for fan-out assertions.
    fn make_event(event_type: &str) -> SerializedEvent {
        SerializedEvent {
            event_type: event_type.to_string(),
            payload: Vec::new(),
            metadata: None,
            version: None,
            stream_id: None,
        }
    }

    /// Records `(name, events.len())` into a shared log when run, then succeeds.
    struct RecordingProjector {
        name: &'static str,
        log: Log,
    }

    impl Projector for RecordingProjector {
        async fn project(&self, events: &[SerializedEvent]) -> Result<(), ProjectionError> {
            lock(&self.log).push((self.name, events.len()));
            Ok(())
        }
    }

    /// Records its name (proving it ran), then fails.
    struct FailingProjector {
        name: &'static str,
        log: Log,
    }

    impl Projector for FailingProjector {
        async fn project(&self, events: &[SerializedEvent]) -> Result<(), ProjectionError> {
            lock(&self.log).push((self.name, events.len()));
            Err(ProjectionError::Custom("boom".to_string()))
        }
    }

    #[tokio::test]
    async fn runs_all_projectors_in_registration_order() {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let projector = MultiProjector::new()
            .with(RecordingProjector {
                name: "first",
                log: Arc::clone(&log),
            })
            .with(RecordingProjector {
                name: "second",
                log: Arc::clone(&log),
            });

        let events = [make_event("E1"), make_event("E2")];
        let result = projector.project(&events).await;
        assert!(result.is_ok(), "expected success, got {result:?}");

        // Both ran, in order, and each received the full batch.
        let recorded = lock(&log).clone();
        assert_eq!(recorded, vec![("first", 2), ("second", 2)]);
    }

    #[tokio::test]
    async fn failure_short_circuits_remaining_projectors() {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        // Heterogeneous projectors: recording, then failing, then recording.
        let projector = MultiProjector::new()
            .with(RecordingProjector {
                name: "before",
                log: Arc::clone(&log),
            })
            .with(FailingProjector {
                name: "boom",
                log: Arc::clone(&log),
            })
            .with(RecordingProjector {
                name: "after",
                log: Arc::clone(&log),
            });

        let events = [make_event("E1")];
        let result = projector.project(&events).await;

        // The error propagates unchanged.
        assert!(matches!(result, Err(ProjectionError::Custom(msg)) if msg == "boom"));

        // Only the first two ran; "after" was never invoked.
        let recorded = lock(&log).clone();
        assert_eq!(recorded, vec![("before", 1), ("boom", 1)]);
    }

    #[tokio::test]
    async fn empty_multi_projector_is_a_noop() {
        let projector = MultiProjector::new();
        assert!(projector.project(&[make_event("E1")]).await.is_ok());
        assert!(projector.project(&[]).await.is_ok());
    }

    #[test]
    fn multi_projector_implements_projector_and_clone() {
        // Compile-level proof that `MultiProjector: Projector`.
        fn requires_projector<P: Projector>(_: P) {}
        fn requires_clone<T: Clone>(_: &T) {}

        let projector = MultiProjector::new().with(RecordingProjector {
            name: "p",
            log: Arc::new(Mutex::new(Vec::new())),
        });
        requires_clone(&projector);
        let _clone = projector.clone();
        requires_projector(projector);
    }

    #[tokio::test]
    async fn clone_shares_children_via_arc() {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let original = MultiProjector::new().with(RecordingProjector {
            name: "p",
            log: Arc::clone(&log),
        });

        let clone = original.clone();

        // Structural: the clone shares the *same* `Arc`'d child (strong count 2),
        // rather than duplicating it.
        assert_eq!(Arc::strong_count(&original.projectors[0]), 2);

        // Behavioral: running the clone drives that shared child.
        assert!(clone.project(&[make_event("E1")]).await.is_ok());
        assert_eq!(lock(&log).clone(), vec![("p", 1)]);
    }

    // ── End-to-end: MultiProjector wired as a real `HandlerEnvironment::Projector` ──

    #[derive(Clone)]
    struct TestCommand;

    #[derive(Clone, serde::Serialize, serde::Deserialize)]
    enum TestEvent {
        Happened,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("test error")]
    struct TestError;

    struct TestLogic;

    impl BusinessLogic for TestLogic {
        type State = ();
        type Input = TestCommand;
        type Event = TestEvent;
        type Error = TestError;
        type Call = Infallible;
        type CallResult = Infallible;
        type Response = ();

        fn stream_id(_input: &TestCommand) -> StreamId {
            StreamId::new("test-stream")
        }

        fn process(
            &self,
            _input: TestCommand,
            _clock: &dyn Clock,
        ) -> Result<BusinessResult<TestEvent, Infallible, ()>, TestError> {
            Ok(BusinessResult::Done(vec![TestEvent::Happened]))
        }

        fn apply(&self, _state: &mut (), _event: &TestEvent) {}

        fn event_type_name(_event: &TestEvent) -> &'static str {
            "TestHappened"
        }
    }

    /// Minimal environment whose `Projector` associated type is `MultiProjector`.
    struct E2EEnv {
        clock: FixedClock,
        event_store: InMemoryEventStore,
        projector: MultiProjector,
        projections: NoOpProjectionQueries,
        topic: String,
        metadata: MetadataContext,
    }

    impl HandlerEnvironment for E2EEnv {
        type Clock = FixedClock;
        type EventStore = InMemoryEventStore;
        type Projector = MultiProjector;
        type EventBus = InMemoryEventBus;
        type Projections = NoOpProjectionQueries;

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
            None
        }
        fn broadcast_topic(&self) -> &str {
            &self.topic
        }
        fn projections(&self) -> &Self::Projections {
            &self.projections
        }
        fn metadata(&self) -> &MetadataContext {
            &self.metadata
        }
    }

    #[tokio::test]
    async fn runs_through_real_handler_as_environment_projector() {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let event_store = InMemoryEventStore::new();

        let env = E2EEnv {
            clock: FixedClock::new(Utc::now()),
            event_store: event_store.clone(),
            projector: MultiProjector::new()
                .with(RecordingProjector {
                    name: "dto",
                    log: Arc::clone(&log),
                })
                .with(RecordingProjector {
                    name: "measurement",
                    log: Arc::clone(&log),
                }),
            projections: NoOpProjectionQueries,
            topic: "test".to_string(),
            metadata: MetadataContext::new(),
        };

        let handler = Handler::new(TestLogic, NoOpCallExecutor, NoOpQueryFetcher, env);
        let result = handler.handle(TestCommand).await;

        // The Handler persisted one event and ran the MultiProjector synchronously.
        assert!(
            matches!(result, Ok(HandleResult::Command { event_count: 1, .. })),
            "expected one persisted event, got {result:?}"
        );

        // Both child projectors ran, in order, each receiving the persisted batch.
        assert_eq!(lock(&log).clone(), vec![("dto", 1), ("measurement", 1)]);

        // And the event really was persisted to the stream.
        assert_eq!(event_store.events_for_stream("test-stream").len(), 1);
    }
}
