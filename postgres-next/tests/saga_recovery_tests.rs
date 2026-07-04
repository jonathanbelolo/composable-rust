//! Integration tests for durable-saga recovery against real `PostgreSQL`:
//! the `SagaJournalProjector`, `PostgresSagaRegistry`, `PostgresAtomicPersist`,
//! and the full kill-and-resume drill (the consumer's M0 exit drill).
//!
//! # Architecture
//!
//! - One container for the entire test suite
//! - Database reset (TRUNCATE) between sections
//! - Run with: `cargo test --test saga_recovery_tests`
//!
//! # Requirements
//!
//! Docker must be running.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
// Test-harness noise: these lints target production-code concerns, not integration tests.
#![allow(clippy::panic)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::single_match)]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use composable_rust_next::testing::RecordingUnitExecutor;
use composable_rust_next::{
    BusinessLogic, BusinessResult, CALL_COMPLETED_EVENT_TYPE, CALL_DISPATCHED_EVENT_TYPE,
    CallDispatched, CallId, CancellationToken, Clock, DurableBusinessLogic, DurableOutcome,
    DynAtomicPersist, FixedClock, Handler, HandlerEnvironment, InProcessEventBus, MetadataContext,
    NoOpProjectionQueries, NoOpQueryFetcher, ProjectionError, SagaRegistry, SerializedEvent,
    StreamId, Version,
};
use composable_rust_postgres_next::{
    PgTransactionalProjector, PostgresAtomicPersist, PostgresEventStore, PostgresSagaRegistry,
    SagaJournalProjector, SagaSweepLock,
};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;
use tokio::sync::oneshot;

// ═══════════════════════════════════════════════════════════════════════════
// Test Infrastructure
// ═══════════════════════════════════════════════════════════════════════════

struct TestDb {
    _container: ContainerAsync<Postgres>,
    pool: PgPool,
}

impl TestDb {
    async fn new() -> Self {
        let container = Postgres::default()
            .start()
            .await
            .expect("Failed to start postgres");

        let port = container.get_host_port_ipv4(5432).await.expect("No port");
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

        let pool = loop {
            match PgPoolOptions::new()
                .max_connections(10)
                .acquire_timeout(std::time::Duration::from_secs(5))
                .connect(&url)
                .await
            {
                Ok(p) => {
                    if sqlx::query("SELECT 1").execute(&p).await.is_ok() {
                        break p;
                    }
                },
                Err(_) => {},
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        };

        sqlx::migrate!("../migrations")
            .run(&pool)
            .await
            .expect("Migrations failed");

        Self {
            _container: container,
            pool,
        }
    }

    async fn reset(&self) {
        sqlx::query("TRUNCATE TABLE events RESTART IDENTITY CASCADE")
            .execute(&self.pool)
            .await
            .expect("Truncate events failed");
        sqlx::query("TRUNCATE TABLE saga_call_journal RESTART IDENTITY CASCADE")
            .execute(&self.pool)
            .await
            .expect("Truncate saga_call_journal failed");
    }

    fn store(&self) -> PostgresEventStore {
        PostgresEventStore::from_pool(self.pool.clone())
    }
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.expect(sql)
}

async fn wait_for_count(pool: &PgPool, sql: &str, expected: i64) {
    let deadline = tokio::time::timeout(Duration::from_secs(10), async {
        while count(pool, sql).await != expected {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await;
    assert!(
        deadline.is_ok(),
        "timeout waiting for `{sql}` == {expected} (actual: {})",
        count(pool, sql).await
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Environment wiring: real store, atomic persist, no projections
// ═══════════════════════════════════════════════════════════════════════════

struct PgTestEnv {
    clock: FixedClock,
    store: PostgresEventStore,
    projections: NoOpProjectionQueries,
    metadata: MetadataContext,
    atomic: Arc<dyn DynAtomicPersist>,
}

impl PgTestEnv {
    fn new(store: PostgresEventStore, atomic: Arc<dyn DynAtomicPersist>) -> Self {
        Self {
            clock: FixedClock::new(Utc::now()),
            store,
            projections: NoOpProjectionQueries,
            metadata: MetadataContext::new(),
            atomic,
        }
    }
}

impl HandlerEnvironment for PgTestEnv {
    type Clock = FixedClock;
    type EventStore = PostgresEventStore;
    type Projector = SagaJournalProjector; // type-level only; atomic path projects
    type EventBus = InProcessEventBus;
    type Projections = NoOpProjectionQueries;

    fn clock(&self) -> &FixedClock {
        &self.clock
    }
    fn event_store(&self) -> &PostgresEventStore {
        &self.store
    }
    fn projector(&self) -> Option<&SagaJournalProjector> {
        None
    }
    fn event_bus(&self) -> Option<&InProcessEventBus> {
        None
    }
    fn broadcast_topic(&self) -> &'static str {
        "saga-recovery-tests"
    }
    fn projections(&self) -> &NoOpProjectionQueries {
        &self.projections
    }
    fn metadata(&self) -> &MetadataContext {
        &self.metadata
    }
    fn atomic_persist(&self) -> Option<&dyn DynAtomicPersist> {
        Some(self.atomic.as_ref())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Saga fixtures (resume-safe: completion state in a shared handle that
// stands in for the saga's projection, surviving the "crash")
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Clone)]
#[allow(dead_code)] // `call_id` carried for realism
enum SagaIn {
    Start {
        id: u64,
    },
    Completion {
        id: u64,
        call_id: CallId,
        result: u64,
    },
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum SagaEv {
    Started(u64),
    Progress(u64),
    Finished(u64),
}

#[derive(Debug, thiserror::Error)]
#[error("saga error")]
struct SagaErr;

#[derive(Default)]
struct SagaState;

fn saga_event_type_name(event: &SagaEv) -> &'static str {
    match event {
        SagaEv::Started(_) => "SStarted",
        SagaEv::Progress(_) => "SProgress",
        SagaEv::Finished(_) => "SFinished",
    }
}

fn saga_input_id(input: &SagaIn) -> u64 {
    match input {
        SagaIn::Start { id } | SagaIn::Completion { id, .. } => *id,
    }
}

struct ResumableSaga {
    total: u64,
    completed: Arc<Mutex<HashSet<CallId>>>,
}

impl BusinessLogic for ResumableSaga {
    type State = SagaState;
    type Input = SagaIn;
    type Event = SagaEv;
    type Error = SagaErr;
    type Call = u64;
    type CallResult = u64;
    type Response = ();

    fn stream_id(input: &SagaIn) -> StreamId {
        StreamId::new(format!("rsaga-{}", saga_input_id(input)))
    }

    fn process(
        &self,
        input: SagaIn,
        _clock: &dyn Clock,
    ) -> Result<BusinessResult<SagaEv, u64, ()>, SagaErr> {
        match input {
            SagaIn::Start { id } => Ok(BusinessResult::Continue {
                events: vec![SagaEv::Started(id)],
                calls: (1..=self.total).collect(),
            }),
            SagaIn::Completion {
                id,
                call_id,
                result,
            } => {
                let mut done = self.completed.lock().unwrap();
                done.insert(call_id);
                if done.len() as u64 == self.total {
                    Ok(BusinessResult::Done(vec![SagaEv::Finished(id)]))
                } else {
                    Ok(BusinessResult::Continue {
                        events: vec![SagaEv::Progress(result)],
                        calls: Vec::new(),
                    })
                }
            },
        }
    }

    fn apply(&self, _state: &mut SagaState, _event: &SagaEv) {}

    fn event_type_name(event: &SagaEv) -> &'static str {
        saga_event_type_name(event)
    }
}

impl DurableBusinessLogic for ResumableSaga {
    const LOGIC_TAG: &'static str = "rsaga";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> SagaIn {
        let id = stream_id
            .as_str()
            .strip_prefix("rsaga-")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        SagaIn::Completion {
            id,
            call_id,
            result,
        }
    }
}

/// Parks (returns `Done`) after its single call completes.
struct GateSaga;

impl BusinessLogic for GateSaga {
    type State = SagaState;
    type Input = SagaIn;
    type Event = SagaEv;
    type Error = SagaErr;
    type Call = u64;
    type CallResult = u64;
    type Response = ();

    fn stream_id(input: &SagaIn) -> StreamId {
        StreamId::new(format!("gate-{}", saga_input_id(input)))
    }

    fn process(
        &self,
        input: SagaIn,
        _clock: &dyn Clock,
    ) -> Result<BusinessResult<SagaEv, u64, ()>, SagaErr> {
        match input {
            SagaIn::Start { id } => Ok(BusinessResult::Continue {
                events: vec![SagaEv::Started(id)],
                calls: vec![1],
            }),
            SagaIn::Completion { id, .. } => Ok(BusinessResult::Done(vec![SagaEv::Finished(id)])),
        }
    }

    fn apply(&self, _state: &mut SagaState, _event: &SagaEv) {}

    fn event_type_name(event: &SagaEv) -> &'static str {
        saga_event_type_name(event)
    }
}

impl DurableBusinessLogic for GateSaga {
    const LOGIC_TAG: &'static str = "gate";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> SagaIn {
        let id = stream_id
            .as_str()
            .strip_prefix("gate-")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        SagaIn::Completion {
            id,
            call_id,
            result,
        }
    }
}

type Gates = Arc<Mutex<HashMap<u64, oneshot::Receiver<u64>>>>;

fn gated_executor(gates: Gates) -> RecordingUnitExecutor<u64, u64> {
    RecordingUnitExecutor::new(move |call: u64| {
        let gate = gates.lock().unwrap().remove(&call);
        async move {
            match gate {
                Some(rx) => rx.await.unwrap_or(call),
                None => call,
            }
        }
    })
}

fn echo_executor() -> RecordingUnitExecutor<u64, u64> {
    RecordingUnitExecutor::new(|call: u64| async move { call })
}

/// A transactional projector that always fails (for rollback tests).
struct FailingProjector;

impl PgTransactionalProjector for FailingProjector {
    async fn project_in_tx(
        &self,
        _conn: &mut sqlx::PgConnection,
        _final_version: Version,
        _events: &[SerializedEvent],
    ) -> Result<(), ProjectionError> {
        Err(ProjectionError::Custom("boom".to_string()))
    }
}

fn dispatched_marker(stream_id: &str, call: u64) -> SerializedEvent {
    let marker = CallDispatched {
        stream_id: stream_id.to_string(),
        call: bincode::serialize(&call).unwrap(),
    };
    SerializedEvent {
        event_type: CALL_DISPATCHED_EVENT_TYPE.to_string(),
        payload: bincode::serialize(&marker).unwrap(),
        metadata: None,
        version: None,
    }
}

fn completed_marker(stream_id: &str, call_id: u64) -> SerializedEvent {
    let marker = composable_rust_next::CallCompleted {
        stream_id: stream_id.to_string(),
        call_id: CallId::new(call_id),
    };
    SerializedEvent {
        event_type: CALL_COMPLETED_EVENT_TYPE.to_string(),
        payload: bincode::serialize(&marker).unwrap(),
        metadata: None,
        version: None,
    }
}

fn domain_event(typ: &str) -> SerializedEvent {
    SerializedEvent {
        event_type: typ.to_string(),
        payload: vec![1, 2, 3],
        metadata: None,
        version: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// All Tests in One Function (Single Container)
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn saga_recovery_integration() {
    let db = TestDb::new().await;

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 1: Journal projector through the atomic path
    // ═══════════════════════════════════════════════════════════════════════

    // Test 1: markers project into journal rows atomically with the append
    {
        db.reset().await;
        let store = db.store();
        let journal = SagaJournalProjector::new(db.pool.clone(), "rsaga");
        let id = StreamId::new("rsaga-1");

        store
            .append_with_projection(
                &id,
                None,
                vec![
                    domain_event("SStarted"),
                    dispatched_marker("rsaga-1", 10),
                    dispatched_marker("rsaga-1", 20),
                ],
                &journal,
            )
            .await
            .unwrap();

        assert_eq!(
            count(
                &db.pool,
                "SELECT COUNT(*) FROM saga_call_journal WHERE completed_at IS NULL"
            )
            .await,
            2,
            "both dispatched markers become outstanding rows"
        );
        // CallIds are the markers' stream versions (2 and 3; the domain
        // event took version 1).
        assert_eq!(
            count(
                &db.pool,
                "SELECT COUNT(*) FROM saga_call_journal WHERE call_id IN (2, 3)"
            )
            .await,
            2
        );

        // Completing call 2 flips its row.
        store
            .append_with_projection(&id, None, vec![completed_marker("rsaga-1", 2)], &journal)
            .await
            .unwrap();
        assert_eq!(
            count(
                &db.pool,
                "SELECT COUNT(*) FROM saga_call_journal WHERE completed_at IS NULL"
            )
            .await,
            1
        );
        println!("  [PASS] journal_projector_atomic_path");
    }

    // Test 2: replay is idempotent (rebuild through the Projector impl)
    {
        let journal = SagaJournalProjector::new(db.pool.clone(), "rsaga");
        let store = db.store();
        // Re-project the whole stream as a rebuild would.
        let events =
            composable_rust_next::EventStore::load(&store, &StreamId::new("rsaga-1"), None)
                .await
                .unwrap();
        composable_rust_next::Projector::project(&journal, &events)
            .await
            .unwrap();

        assert_eq!(
            count(&db.pool, "SELECT COUNT(*) FROM saga_call_journal").await,
            2,
            "replay must not duplicate rows"
        );
        assert_eq!(
            count(
                &db.pool,
                "SELECT COUNT(*) FROM saga_call_journal WHERE completed_at IS NULL"
            )
            .await,
            1,
            "replay must not resurrect or re-complete calls"
        );
        println!("  [PASS] journal_projection_idempotent_replay");
    }

    // Test 3: projection failure rolls back events AND journal rows
    {
        db.reset().await;
        let store = db.store();
        let journal = SagaJournalProjector::new(db.pool.clone(), "rsaga");
        let id = StreamId::new("rsaga-2");

        let result = store
            .append_with_projection(
                &id,
                None,
                vec![dispatched_marker("rsaga-2", 1)],
                &(journal, FailingProjector),
            )
            .await;
        assert!(matches!(
            result,
            Err(composable_rust_next::AtomicError::Projection(_))
        ));

        assert_eq!(
            count(&db.pool, "SELECT COUNT(*) FROM events").await,
            0,
            "append must roll back"
        );
        assert_eq!(
            count(&db.pool, "SELECT COUNT(*) FROM saga_call_journal").await,
            0,
            "journal rows must roll back with the events"
        );
        println!("  [PASS] projection_failure_rolls_back_events_and_journal");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 2: Registry query
    // ═══════════════════════════════════════════════════════════════════════

    {
        db.reset().await;
        let store = db.store();
        let mine = SagaJournalProjector::new(db.pool.clone(), "rsaga");
        let other = SagaJournalProjector::new(db.pool.clone(), "other");

        // Stream A: outstanding (2 dispatched, 1 completed), tag rsaga.
        store
            .append_with_projection(
                &StreamId::new("rsaga-a"),
                None,
                vec![
                    dispatched_marker("rsaga-a", 1),
                    dispatched_marker("rsaga-a", 2),
                ],
                &mine,
            )
            .await
            .unwrap();
        store
            .append_with_projection(
                &StreamId::new("rsaga-a"),
                None,
                vec![completed_marker("rsaga-a", 1)],
                &mine,
            )
            .await
            .unwrap();

        // Stream B: balanced, tag rsaga.
        store
            .append_with_projection(
                &StreamId::new("rsaga-b"),
                None,
                vec![dispatched_marker("rsaga-b", 1)],
                &mine,
            )
            .await
            .unwrap();
        store
            .append_with_projection(
                &StreamId::new("rsaga-b"),
                None,
                vec![completed_marker("rsaga-b", 1)],
                &mine,
            )
            .await
            .unwrap();

        // Stream C: outstanding, but a DIFFERENT logic tag.
        store
            .append_with_projection(
                &StreamId::new("other-c"),
                None,
                vec![dispatched_marker("other-c", 1)],
                &other,
            )
            .await
            .unwrap();

        let registry = PostgresSagaRegistry::new(db.pool.clone());
        let records = registry
            .instances_with_outstanding_calls("rsaga")
            .await
            .unwrap();

        assert_eq!(records.len(), 1, "only stream A needs recovery");
        assert_eq!(records[0].stream_id, StreamId::new("rsaga-a"));
        assert_eq!(records[0].logic_tag, "rsaga");
        assert_eq!(records[0].outstanding_calls, 1);
        assert!(records[0].oldest_dispatched_at <= Utc::now());
        println!("  [PASS] registry_query_filters_by_tag_and_outstanding");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 3: Kill-and-resume against real PG (the M0 exit drill)
    // ═══════════════════════════════════════════════════════════════════════

    {
        db.reset().await;
        let store = db.store();
        let journal = SagaJournalProjector::new(db.pool.clone(), ResumableSaga::LOGIC_TAG);
        let atomic: Arc<dyn DynAtomicPersist> =
            Arc::new(PostgresAtomicPersist::new(store.clone(), journal));

        // Window of 3: calls 1 and 2 complete on cue; call 3 hangs.
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        let (_tx3, rx3) = oneshot::channel();
        let gates: Gates = Arc::new(Mutex::new(HashMap::from([(1, rx1), (2, rx2), (3, rx3)])));
        let first_executor = gated_executor(gates);

        let completed_state = Arc::new(Mutex::new(HashSet::new()));
        let saga = ResumableSaga {
            total: 3,
            completed: Arc::clone(&completed_state),
        };
        let env = PgTestEnv::new(store.clone(), Arc::clone(&atomic));
        let handler = Arc::new(Handler::new(saga, first_executor, NoOpQueryFetcher, env));
        let h = Arc::clone(&handler);
        let run = tokio::spawn(async move {
            h.handle_durable(SagaIn::Start { id: 7 }, CancellationToken::new())
                .await
        });

        // Two completions land durably (journal rows flip inside the same
        // transactions as the completion markers), one call in flight...
        tx1.send(101).unwrap();
        tx2.send(102).unwrap();
        wait_for_count(
            &db.pool,
            "SELECT COUNT(*) FROM saga_call_journal WHERE completed_at IS NOT NULL",
            2,
        )
        .await;

        // ...and the process crashes.
        run.abort();
        assert!(run.await.unwrap_err().is_cancelled());

        // The recovery driver on the restarted process: registry lists the
        // instance, a fresh handler resumes it.
        let registry = PostgresSagaRegistry::new(db.pool.clone());
        let records = registry
            .instances_with_outstanding_calls(ResumableSaga::LOGIC_TAG)
            .await
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].stream_id, StreamId::new("rsaga-7"));
        assert_eq!(records[0].outstanding_calls, 1);

        let second_executor = echo_executor();
        let second_executor_handle = second_executor.clone();
        let saga = ResumableSaga {
            total: 3,
            completed: completed_state,
        };
        let env = PgTestEnv::new(store.clone(), atomic);
        let handler = Handler::new(saga, second_executor, NoOpQueryFetcher, env);

        let outcome = handler
            .resume(&records[0].stream_id, CancellationToken::new())
            .await
            .unwrap();
        assert!(
            matches!(outcome, DurableOutcome::Completed { .. }),
            "resume must complete the saga, got {outcome:?}"
        );

        // Exactly the one incomplete call re-ran; no completed call re-paid.
        assert_eq!(second_executor_handle.dispatched(), vec![3]);

        // Journal fully balanced; registry empty; saga finished.
        assert_eq!(
            count(
                &db.pool,
                "SELECT COUNT(*) FROM saga_call_journal WHERE completed_at IS NULL"
            )
            .await,
            0
        );
        assert!(
            registry
                .instances_with_outstanding_calls(ResumableSaga::LOGIC_TAG)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            count(
                &db.pool,
                "SELECT COUNT(*) FROM events WHERE event_type = 'SFinished'"
            )
            .await,
            1
        );
        println!("  [PASS] kill_and_resume_m0_drill");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 4: Parked saga untouched by sweep and by resume
    // ═══════════════════════════════════════════════════════════════════════

    {
        db.reset().await;
        let store = db.store();
        let journal = SagaJournalProjector::new(db.pool.clone(), GateSaga::LOGIC_TAG);
        let atomic: Arc<dyn DynAtomicPersist> =
            Arc::new(PostgresAtomicPersist::new(store.clone(), journal));

        // Run to the review gate (Done with a balanced journal).
        let env = PgTestEnv::new(store.clone(), Arc::clone(&atomic));
        let handler = Handler::new(GateSaga, echo_executor(), NoOpQueryFetcher, env);
        let outcome = handler
            .handle_durable(SagaIn::Start { id: 9 }, CancellationToken::new())
            .await
            .unwrap();
        assert!(matches!(outcome, DurableOutcome::Completed { .. }));
        let events_before = count(&db.pool, "SELECT COUNT(*) FROM events").await;

        // The sweep query does not list it.
        let registry = PostgresSagaRegistry::new(db.pool.clone());
        assert!(
            registry
                .instances_with_outstanding_calls(GateSaga::LOGIC_TAG)
                .await
                .unwrap()
                .is_empty(),
            "a parked instance must not appear in the recovery sweep"
        );

        // A (hypothetical, stale-list) resume leaves it alone.
        let sweep_executor = echo_executor();
        let sweep_executor_handle = sweep_executor.clone();
        let env = PgTestEnv::new(store.clone(), atomic);
        let handler = Handler::new(GateSaga, sweep_executor, NoOpQueryFetcher, env);
        let outcome = handler
            .resume(&StreamId::new("gate-9"), CancellationToken::new())
            .await
            .unwrap();
        assert!(matches!(outcome, DurableOutcome::NoOutstandingCalls));
        assert_eq!(sweep_executor_handle.dispatch_count(), 0);
        assert_eq!(
            count(&db.pool, "SELECT COUNT(*) FROM events").await,
            events_before,
            "resume must not persist anything on a parked instance"
        );
        println!("  [PASS] parked_saga_untouched");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 5: rebuild_from_store heals a lost journal
    // ═══════════════════════════════════════════════════════════════════════

    {
        db.reset().await;
        let store = db.store();
        let journal = SagaJournalProjector::new(db.pool.clone(), "rsaga");

        // Real journal state: 3 dispatched, 2 completed, on two streams.
        store
            .append_with_projection(
                &StreamId::new("rsaga-r1"),
                None,
                vec![
                    dispatched_marker("rsaga-r1", 1),
                    dispatched_marker("rsaga-r1", 2),
                ],
                &journal,
            )
            .await
            .unwrap();
        store
            .append_with_projection(
                &StreamId::new("rsaga-r1"),
                None,
                vec![completed_marker("rsaga-r1", 1)],
                &journal,
            )
            .await
            .unwrap();
        store
            .append_with_projection(
                &StreamId::new("rsaga-r2"),
                None,
                vec![
                    dispatched_marker("rsaga-r2", 1),
                    completed_marker("rsaga-r2", 1),
                ],
                &journal,
            )
            .await
            .unwrap();

        let snapshot: Vec<(String, i64, bool)> = sqlx::query_as(
            "SELECT stream_id, call_id, completed_at IS NOT NULL
             FROM saga_call_journal ORDER BY stream_id, call_id",
        )
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert_eq!(snapshot.len(), 3);

        // Disaster: the journal table is lost.
        sqlx::query("TRUNCATE TABLE saga_call_journal")
            .execute(&db.pool)
            .await
            .unwrap();

        journal.rebuild_from_store("rsaga-").await.unwrap();

        let rebuilt: Vec<(String, i64, bool)> = sqlx::query_as(
            "SELECT stream_id, call_id, completed_at IS NOT NULL
             FROM saga_call_journal ORDER BY stream_id, call_id",
        )
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert_eq!(
            rebuilt, snapshot,
            "rebuild must reproduce the journal exactly (dispatch/completion state per call)"
        );
        println!("  [PASS] rebuild_from_store_heals_journal");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 6: SagaSweepLock — advisory-lock exclusion without pool leaks
    // ═══════════════════════════════════════════════════════════════════════

    {
        db.reset().await;
        let stream = StreamId::new("lock-1");

        let lock = SagaSweepLock::try_acquire(&db.pool, &stream)
            .await
            .unwrap()
            .expect("first acquire must succeed");
        assert!(
            SagaSweepLock::try_acquire(&db.pool, &stream)
                .await
                .unwrap()
                .is_none(),
            "a second acquire on the same instance must be blocked"
        );

        // Other instances are unaffected.
        let other = SagaSweepLock::try_acquire(&db.pool, &StreamId::new("lock-2"))
            .await
            .unwrap()
            .expect("a different instance must be lockable");
        other.release().await.unwrap();

        // Graceful release frees the lock immediately.
        lock.release().await.unwrap();
        let reacquired = SagaSweepLock::try_acquire(&db.pool, &stream)
            .await
            .unwrap()
            .expect("released lock must be reacquirable");

        // Drop-without-release also frees it: the guard's detached session
        // dies with the connection (may take a moment for the server to
        // notice the socket close).
        drop(reacquired);
        let after_drop = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(lock) = SagaSweepLock::try_acquire(&db.pool, &stream).await.unwrap() {
                    break lock;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("dropping the guard must release the lock");
        after_drop.release().await.unwrap();
        println!("  [PASS] saga_sweep_lock_exclusion_and_release");
    }

    println!("\nAll saga recovery integration tests passed!");
}
