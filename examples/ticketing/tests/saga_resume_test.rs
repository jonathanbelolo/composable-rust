//! End-to-end (real Postgres) proof that a long-running saga's durable state
//! survives a process "restart" and that the saga stream has optimistic
//! concurrency — both because `saga_state` is written in the SAME transaction as
//! the saga's events via `PgAtomicPersist` / `PgReservationSagaStateProjector`.
//!
//! Requires Docker (testcontainers).

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::too_many_lines)] // integration-test bodies are naturally long

use chrono::{Duration, Utc};
use composable_rust_next::{
    AtomicError, BusinessLogic, CallExecutor, DynAtomicPersist, EventStore, EventStoreError,
    Handler, HandlerError, SerializedEvent, StreamId, SystemClock, Version,
};
use composable_rust_postgres_next::PostgresEventStore;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use ticketing::next::{
    EventInventorySagaEvent, EventInventorySagaLogic, EventInventorySagaPhase,
    EventInventorySagaProjectionQueries, NoOpEventBus, NoOpProjector, PgAtomicPersist,
    PgEventInventorySagaAtomicPersist, ReservationSagaCall, ReservationSagaCallResult,
    ReservationSagaEvent, ReservationSagaInput, ReservationSagaLogic, ReservationSagaPhase,
    ReservationSagaProjectionQueries, ReservationSagaQueryFetcher, TicketingEnvironment,
};
use ticketing::types::{CustomerId, EventId, Money, ReservationId, SeatId};
use uuid::Uuid;

/// Wrap a saga event as a `SerializedEvent` (bincode payload, like the Handler).
fn serialized(event: &ReservationSagaEvent) -> SerializedEvent {
    SerializedEvent {
        event_type: ReservationSagaLogic::event_type_name(event).to_string(),
        payload: bincode::serialize(event).expect("serialize saga event"),
        metadata: None,
        version: None,
        stream_id: None,
    }
}

/// Apply the merged single-DB migrations to a fresh database.
async fn migrate(pool: &PgPool) {
    for sql in [
        include_str!("../migrations/001_events_log.sql"),
        include_str!("../migrations/002_projections.sql"),
        include_str!("../migrations/003_seats.sql"),
        include_str!("../migrations/004_saga_state.sql"),
        include_str!("../migrations/005_saga_state_version.sql"),
        include_str!("../migrations/006_event_inventory_saga_state.sql"),
    ] {
        sqlx::raw_sql(sql).execute(pool).await.expect("migration");
    }
}

/// Start a fresh Postgres container, connect, and apply the migrations.
async fn fresh_db() -> (ContainerAsync<Postgres>, PgPool) {
    let container = Postgres::default().start().await.expect("start postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pool = loop {
        if let Ok(p) = PgPoolOptions::new().connect(&url).await {
            if sqlx::query("SELECT 1").execute(&p).await.is_ok() {
                break p;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    migrate(&pool).await;
    (container, pool)
}

#[tokio::test]
async fn saga_state_survives_restart_and_enforces_occ() {
    let container = Postgres::default().start().await.expect("start postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pool = loop {
        if let Ok(p) = PgPoolOptions::new().connect(&url).await {
            if sqlx::query("SELECT 1").execute(&p).await.is_ok() {
                break p;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    migrate(&pool).await;

    let reservation_id = ReservationId::new();
    let stream = StreamId::new(format!("saga-reservation-{}", reservation_id.as_uuid()));
    let now = Utc::now();

    // Drive the saga two phases through the transactional atomic-persist path.
    let ap = PgAtomicPersist::new(PostgresEventStore::from_pool(pool.clone()));

    let v1 = ap
        .append_and_project(
            &stream,
            None,
            vec![serialized(&ReservationSagaEvent::ReservationInitiated {
                reservation_id,
                event_id: EventId::new(),
                customer_id: CustomerId::new(),
                section: "A".to_string(),
                quantity: 2,
                expires_at: now + chrono::Duration::minutes(5),
                initiated_at: now,
            })],
        )
        .await
        .expect("append ReservationInitiated");
    assert_eq!(v1, Version::new(1));

    let v2 = ap
        .append_and_project(
            &stream,
            Some(Version::new(1)),
            vec![serialized(&ReservationSagaEvent::SeatsAllocated {
                reservation_id,
                seats: vec![SeatId::new(), SeatId::new()],
                total_amount: Money::from_cents(10_000),
                allocated_at: now,
            })],
        )
        .await
        .expect("append SeatsAllocated");
    assert_eq!(v2, Version::new(2));

    // "Restart": a brand-new reader over the same DB (no in-memory state at all).
    let queries = ReservationSagaProjectionQueries::new(pool.clone());
    let (state, version) = queries
        .get_saga_state(reservation_id)
        .await
        .expect("query saga_state")
        .expect("saga_state row must exist after restart");

    assert_eq!(
        version,
        Version::new(2),
        "rehydrated version == stream version"
    );
    assert_eq!(
        state.phase,
        ReservationSagaPhase::AwaitingPayment,
        "folded events rehydrate to AwaitingPayment"
    );
    assert_eq!(state.seats.len(), 2, "seats rehydrated");
    assert_eq!(
        state.total_amount,
        Some(Money::from_cents(10_000)),
        "amount rehydrated"
    );

    // The customer read model was updated in the SAME transaction.
    let status: String =
        sqlx::query_scalar("SELECT status FROM reservations_projection WHERE id = $1")
            .bind(reservation_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("reservations_projection row");
    assert_eq!(status, "seats_reserved");

    // Optimistic concurrency: a stale append (wrong expected version) conflicts.
    let conflict = ap
        .append_and_project(
            &stream,
            Some(Version::new(1)), // stale — stream is at version 2
            vec![serialized(&ReservationSagaEvent::ReservationCancelled {
                reservation_id,
                reason: "stale".to_string(),
                cancelled_at: now,
            })],
        )
        .await;
    assert!(
        conflict.is_err(),
        "a stale-version append must fail with a version conflict"
    );

    // The conflicting write rolled back: state is unchanged at version 2.
    let (_, version_after) = queries
        .get_saga_state(reservation_id)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(
        version_after,
        Version::new(2),
        "conflict left state untouched"
    );
}

#[tokio::test]
async fn create_time_occ_dedups_duplicate_initiation() {
    let (_container, pool) = fresh_db().await;

    let reservation_id = ReservationId::new();
    let stream = StreamId::new(format!("saga-reservation-{}", reservation_id.as_uuid()));
    let store = PostgresEventStore::from_pool(pool.clone());
    let ap = PgAtomicPersist::new(store.clone());
    let now = Utc::now();

    let initiated = || {
        serialized(&ReservationSagaEvent::ReservationInitiated {
            reservation_id,
            event_id: EventId::new(),
            customer_id: CustomerId::new(),
            section: "A".to_string(),
            quantity: 1,
            expires_at: now + Duration::minutes(5),
            initiated_at: now,
        })
    };

    // First initiation expects an empty stream (Version::initial()) — succeeds.
    let v = ap
        .append_and_project(&stream, Some(Version::initial()), vec![initiated()])
        .await
        .expect("first initiate");
    assert_eq!(v, Version::new(1));

    // A duplicate (same deterministic id → same stream) conflicts at create time,
    // exactly how idempotent initiation dedups a double-submit.
    let dup = ap
        .append_and_project(&stream, Some(Version::initial()), vec![initiated()])
        .await;
    assert!(
        matches!(
            dup,
            Err(AtomicError::Append(EventStoreError::VersionConflict { .. }))
        ),
        "duplicate initiation must conflict (create-time OCC)"
    );

    // Exactly one ReservationInitiated survived.
    assert_eq!(store.load(&stream, None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn expiration_query_selects_overdue_non_terminal_sagas() {
    let (_container, pool) = fresh_db().await;
    let now = Utc::now();

    let overdue = Uuid::new_v4(); // non-terminal, past due → selected
    let not_due = Uuid::new_v4(); // non-terminal, future → excluded
    let terminal = Uuid::new_v4(); // terminal, past due → excluded

    for (id, phase, expires_at) in [
        (overdue, "AwaitingPayment", now - Duration::minutes(1)),
        (not_due, "AwaitingPayment", now + Duration::minutes(10)),
        (terminal, "Failed", now - Duration::minutes(1)),
    ] {
        sqlx::query(
            "INSERT INTO saga_state (reservation_id, version, phase, expires_at, state) \
             VALUES ($1, 1, $2, $3, '{}'::jsonb)",
        )
        .bind(id)
        .bind(phase)
        .bind(expires_at)
        .execute(&pool)
        .await
        .expect("insert saga_state");
    }

    // The expiration worker's selection query, verbatim.
    let due: Vec<Uuid> = sqlx::query_scalar(
        "SELECT reservation_id FROM saga_state \
         WHERE phase NOT IN ('Completed', 'Failed') AND expires_at < now() \
         ORDER BY expires_at ASC LIMIT 100",
    )
    .fetch_all(&pool)
    .await
    .expect("query due sagas");

    assert_eq!(
        due,
        vec![overdue],
        "only the overdue, non-terminal saga is selected"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// End-to-end: the REAL ReservationSagaHandler driven through the durable Pg path.
//
// These drive the real `ReservationSagaLogic` through the real `Handler` +
// `PgAtomicPersist` (saga_state written transactionally) + the Pg `QueryFetcher`
// (rehydrate) + the feedback loop — the integration that the lib unit tests and the
// PgAtomicPersist-direct tests above do not exercise together. A trivial stub call
// executor returns successful-but-empty results, which the saga logic treats as "no
// seats reserved" and drives to a terminal state, so no concrete inventory/payment
// events need to be constructed (child-aggregate dispatch is covered by the in-memory
// harness in `src/next/testing`).
// ═══════════════════════════════════════════════════════════════════════════

/// A call executor returning a successful-but-empty result for every saga call.
/// The reservation saga treats an empty inventory reserve as "no seats" and fails fast
/// to a terminal state, and ignores results during compensation — so this drives the
/// real Handler loop to a terminal `saga_state` without building concrete child events.
struct EmptyOkCallExecutor;

impl CallExecutor<ReservationSagaCall, ReservationSagaCallResult> for EmptyOkCallExecutor {
    async fn execute(&self, calls: Vec<ReservationSagaCall>) -> Vec<ReservationSagaCallResult> {
        calls
            .into_iter()
            .map(|call| match call {
                ReservationSagaCall::Inventory(_) => ReservationSagaCallResult::Inventory {
                    result: Ok(Vec::new()),
                },
                ReservationSagaCall::Payment(_) => ReservationSagaCallResult::Payment {
                    result: Ok(Vec::new()),
                },
            })
            .collect()
    }
}

type TestSagaEnv = TicketingEnvironment<
    SystemClock,
    PostgresEventStore,
    NoOpProjector,
    NoOpEventBus,
    ReservationSagaProjectionQueries,
>;
type TestSagaHandler =
    Handler<ReservationSagaLogic, EmptyOkCallExecutor, ReservationSagaQueryFetcher, TestSagaEnv>;

/// Build the REAL reservation saga handler over Postgres (durable atomic persist + Pg
/// fetcher), as `bootstrap::builder` does but with the stub call executor.
fn build_saga_handler(pool: &PgPool) -> TestSagaHandler {
    let store = PostgresEventStore::from_pool(pool.clone());
    let env: TestSagaEnv = TicketingEnvironment::with_projections(
        SystemClock,
        store.clone(),
        None::<NoOpProjector>,
        None::<NoOpEventBus>,
        "test-reservation-sagas",
        ReservationSagaProjectionQueries::new(pool.clone()),
    )
    .with_atomic_persist(Arc::new(PgAtomicPersist::new(store)));
    Handler::new(
        ReservationSagaLogic,
        EmptyOkCallExecutor,
        ReservationSagaQueryFetcher::new(),
        env,
    )
}

fn saga_stream(reservation_id: ReservationId) -> StreamId {
    StreamId::new(format!("saga-reservation-{}", reservation_id.as_uuid()))
}

#[tokio::test]
async fn saga_runs_to_terminal_through_real_handler() {
    let (_container, pool) = fresh_db().await;
    let handler = build_saga_handler(&pool);
    let reservation_id = ReservationId::new();

    handler
        .handle(ReservationSagaInput::InitiateReservation {
            reservation_id,
            event_id: EventId::new(),
            customer_id: CustomerId::new(),
            section: "A".to_string(),
            quantity: 2,
        })
        .await
        .expect("handle initiate");

    // The real Handler drove the durable path across two transactional appends:
    // ReservationInitiated (v1), then InventoryReservationFailed (v2, from the empty
    // reserve result) → terminal saga_state, all via env.atomic_persist + feedback.
    let queries = ReservationSagaProjectionQueries::new(pool.clone());
    let (state, version) = queries
        .get_saga_state(reservation_id)
        .await
        .expect("query")
        .expect("saga_state row exists");
    assert_eq!(
        state.phase,
        ReservationSagaPhase::Failed,
        "saga reached terminal"
    );
    assert_eq!(
        version,
        Version::new(2),
        "two events appended via the Handler"
    );

    let events = PostgresEventStore::from_pool(pool)
        .load(&saga_stream(reservation_id), None)
        .await
        .expect("load");
    assert_eq!(events.len(), 2, "saga stream has both events");
}

#[tokio::test]
async fn duplicate_initiation_conflicts_through_real_handler() {
    let (_container, pool) = fresh_db().await;
    let handler = build_saga_handler(&pool);
    let reservation_id = ReservationId::new();

    let initiate = || ReservationSagaInput::InitiateReservation {
        reservation_id,
        event_id: EventId::new(),
        customer_id: CustomerId::new(),
        section: "A".to_string(),
        quantity: 1,
    };

    handler.handle(initiate()).await.expect("first initiate");

    // A second initiation for the SAME reservation_id (e.g. a duplicate idempotency
    // key) hits create-time OCC in the Pg fetcher and surfaces as VersionConflict
    // through the real Handler — the mapping the HTTP layer turns into an idempotent 200.
    let dup = handler.handle(initiate()).await;
    assert!(
        matches!(
            dup,
            Err(HandlerError::Persist(
                EventStoreError::VersionConflict { .. }
            ))
        ),
        "duplicate initiation must conflict, got {dup:?}"
    );
}

#[tokio::test]
async fn expiration_resumes_seeded_saga_to_terminal_through_real_handler() {
    let (_container, pool) = fresh_db().await;
    let reservation_id = ReservationId::new();
    let stream = saga_stream(reservation_id);
    let now = Utc::now();

    // Seed a non-terminal saga with a PAST deadline — exactly the state a crash would
    // leave behind — via the durable atomic-persist path.
    let seed = PgAtomicPersist::new(PostgresEventStore::from_pool(pool.clone()));
    seed.append_and_project(
        &stream,
        None,
        vec![serialized(&ReservationSagaEvent::ReservationInitiated {
            reservation_id,
            event_id: EventId::new(),
            customer_id: CustomerId::new(),
            section: "A".to_string(),
            quantity: 1,
            expires_at: now - Duration::minutes(1),
            initiated_at: now - Duration::minutes(6),
        })],
    )
    .await
    .expect("seed non-terminal saga");

    // A FRESH handler (no in-memory state) rehydrates the saga from durable saga_state
    // and expires it — restart-resume + the expiration action, end-to-end.
    let handler = build_saga_handler(&pool);
    handler
        .handle(ReservationSagaInput::ExpireReservation {
            reservation_id,
            fetched: None,
        })
        .await
        .expect("expire");

    let queries = ReservationSagaProjectionQueries::new(pool.clone());
    let (state, _version) = queries
        .get_saga_state(reservation_id)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(
        state.phase,
        ReservationSagaPhase::Failed,
        "expired saga reaches a terminal phase"
    );

    let events = PostgresEventStore::from_pool(pool)
        .load(&stream, None)
        .await
        .expect("load");
    assert!(
        events.iter().any(|e| e.event_type == "ReservationExpired"),
        "a ReservationExpired event was appended by the saga"
    );
}

// ── Event-Inventory saga: same durable-state guarantee as the reservation saga ──

fn serialized_event_inventory(event: &EventInventorySagaEvent) -> SerializedEvent {
    SerializedEvent {
        event_type: EventInventorySagaLogic::event_type_name(event).to_string(),
        payload: bincode::serialize(event).expect("serialize event-inventory saga event"),
        metadata: None,
        version: None,
        stream_id: None,
    }
}

#[tokio::test]
async fn event_inventory_saga_state_survives_restart_and_enforces_occ() {
    let (_container, pool) = fresh_db().await;
    let event_id = EventId::new();
    let stream = StreamId::new(format!("saga-event-inventory-{}", event_id.as_uuid()));
    let now = Utc::now();

    let ap = PgEventInventorySagaAtomicPersist::new(PostgresEventStore::from_pool(pool.clone()));

    let v1 = ap
        .append_and_project(
            &stream,
            None,
            vec![serialized_event_inventory(
                &EventInventorySagaEvent::Initiated {
                    event_id,
                    name: "Concert".to_string(),
                    sections: vec!["VIP".to_string(), "GA".to_string()],
                    initiated_at: now,
                },
            )],
        )
        .await
        .expect("append Initiated");
    assert_eq!(v1, Version::new(1));

    let v2 = ap
        .append_and_project(
            &stream,
            Some(Version::new(1)),
            vec![serialized_event_inventory(
                &EventInventorySagaEvent::EventCreated {
                    event_id,
                    created_at: now,
                },
            )],
        )
        .await
        .expect("append EventCreated");
    assert_eq!(v2, Version::new(2));

    // "Restart": a fresh reader rehydrates from durable saga_state_event_inventory.
    let queries = EventInventorySagaProjectionQueries::new(pool.clone());
    let (state, version) = queries
        .get_saga_state(event_id)
        .await
        .expect("query")
        .expect("saga_state_event_inventory row exists");
    assert_eq!(
        version,
        Version::new(2),
        "rehydrated version == stream version"
    );
    assert_eq!(
        state.phase,
        EventInventorySagaPhase::InitializingInventory,
        "folded events rehydrate to InitializingInventory"
    );

    // A stale append (wrong expected version) conflicts.
    let conflict = ap
        .append_and_project(
            &stream,
            Some(Version::new(1)),
            vec![serialized_event_inventory(
                &EventInventorySagaEvent::EventCreated {
                    event_id,
                    created_at: now,
                },
            )],
        )
        .await;
    assert!(
        matches!(
            conflict,
            Err(AtomicError::Append(EventStoreError::VersionConflict { .. }))
        ),
        "a stale-version append must conflict"
    );
}
