//! End-to-end (real Postgres) proof that a long-running saga's durable state
//! survives a process "restart" and that the saga stream has optimistic
//! concurrency — both because `saga_state` is written in the SAME transaction as
//! the saga's events via `PgAtomicPersist` / `PgReservationSagaStateProjector`.
//!
//! Requires Docker (testcontainers).

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use chrono::Utc;
use composable_rust_next::{BusinessLogic, DynAtomicPersist, SerializedEvent, StreamId, Version};
use composable_rust_postgres_next::PostgresEventStore;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use ticketing::next::{
    PgAtomicPersist, ReservationSagaEvent, ReservationSagaLogic, ReservationSagaPhase,
    ReservationSagaProjectionQueries,
};
use ticketing::types::{CustomerId, EventId, Money, ReservationId, SeatId};

/// Wrap a saga event as a `SerializedEvent` (bincode payload, like the Handler).
fn serialized(event: &ReservationSagaEvent) -> SerializedEvent {
    SerializedEvent {
        event_type: ReservationSagaLogic::event_type_name(event).to_string(),
        payload: bincode::serialize(event).expect("serialize saga event"),
        metadata: None,
        version: None,
    }
}

/// Apply the merged single-DB migrations to a fresh database.
async fn migrate(pool: &PgPool) {
    for sql in [
        include_str!("../migrations/001_events_log.sql"),
        include_str!("../migrations/002_projections.sql"),
        include_str!("../migrations/003_seats.sql"),
        include_str!("../migrations/004_saga_state.sql"),
        include_str!("../migrations/005_idempotency_keys.sql"),
    ] {
        sqlx::raw_sql(sql).execute(pool).await.expect("migration");
    }
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

    assert_eq!(version, Version::new(2), "rehydrated version == stream version");
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
    assert_eq!(version_after, Version::new(2), "conflict left state untouched");
}
