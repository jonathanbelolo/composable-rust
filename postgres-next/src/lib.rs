//! `PostgreSQL` event store implementation for `composable-rust-next`.
//!
//! This crate provides a production-ready `PostgreSQL`-based event store that implements
//! the [`EventStore`] trait from `composable-rust-next`.
//!
//! # Features
//!
//! - Event persistence with optimistic concurrency control
//! - Connection pooling for efficient resource usage
//! - Tracing and metrics for observability
//!
//! # Example
//!
//! ```no_run
//! use composable_rust_postgres_next::PostgresEventStore;
//!
//! async fn example() -> Result<(), Box<dyn std::error::Error>> {
//!     let event_store = PostgresEventStore::new("postgres://localhost/mydb").await?;
//!     Ok(())
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use composable_rust_next::{
    AtomicError, CALL_COMPLETED_EVENT_TYPE, CALL_DISPATCHED_EVENT_TYPE, CallCompleted,
    CallDispatched, DynAtomicPersist, EventMetadata, EventStore, EventStoreError, ProjectionError,
    Projector, SagaInstanceRecord, SagaRegistry, SerializedEvent, StreamId, Version,
};
use sqlx::Row;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;
use tracing::Instrument;

/// Connection pool statistics for monitoring and observability.
#[derive(Debug, Clone, Copy)]
pub struct PoolStats {
    /// Total number of connections in the pool (active + idle)
    pub size: u32,
    /// Number of idle connections available for use
    pub idle: usize,
}

impl PoolStats {
    /// Check if the connection pool is saturated (no idle connections).
    #[must_use]
    pub const fn is_saturated(&self) -> bool {
        self.idle == 0
    }

    /// Get the utilization percentage of the pool (0.0 to 1.0).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn utilization(&self) -> f64 {
        if self.size == 0 {
            0.0
        } else {
            1.0 - (self.idle as f64 / f64::from(self.size))
        }
    }
}

/// PostgreSQL-based event store implementation.
///
/// This implementation uses `PostgreSQL` for durable event storage with:
/// - Optimistic concurrency control via version numbers
/// - Connection pooling for efficient resource usage
/// - Tracing and metrics for observability
#[derive(Clone)]
pub struct PostgresEventStore {
    pool: PgPool,
}

impl PostgresEventStore {
    /// Create a new `PostgreSQL` event store from a database URL.
    ///
    /// Creates a connection pool with default settings (max 5 connections).
    ///
    /// # Errors
    ///
    /// Returns an error if connection to the database fails.
    pub async fn new(database_url: &str) -> Result<Self, EventStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|e| EventStoreError::Connection(e.to_string()))?;

        Ok(Self { pool })
    }

    /// Create a new `PostgreSQL` event store with custom pool options.
    ///
    /// # Arguments
    ///
    /// * `database_url` - `PostgreSQL` connection URL
    /// * `max_connections` - Maximum number of connections in the pool
    /// * `min_connections` - Minimum number of idle connections to maintain
    /// * `connect_timeout_secs` - Connection timeout in seconds
    /// * `idle_timeout_secs` - Idle connection timeout in seconds
    /// * `max_lifetime_secs` - Maximum connection lifetime in seconds
    ///
    /// # Errors
    ///
    /// Returns an error if connection to the database fails.
    pub async fn with_options(
        database_url: &str,
        max_connections: u32,
        min_connections: u32,
        connect_timeout_secs: u64,
        idle_timeout_secs: u64,
        max_lifetime_secs: u64,
    ) -> Result<Self, EventStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .min_connections(min_connections)
            .acquire_timeout(Duration::from_secs(connect_timeout_secs))
            .idle_timeout(Some(Duration::from_secs(idle_timeout_secs)))
            .max_lifetime(Some(Duration::from_secs(max_lifetime_secs)))
            .connect(database_url)
            .await
            .map_err(|e| EventStoreError::Connection(e.to_string()))?;

        Ok(Self { pool })
    }

    /// Create a new `PostgreSQL` event store from an existing connection pool.
    ///
    /// Useful when you want to share a connection pool across services.
    #[must_use]
    pub const fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get a reference to the underlying connection pool.
    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Get connection pool statistics for monitoring.
    #[must_use]
    pub fn pool_stats(&self) -> PoolStats {
        PoolStats {
            size: self.pool.size(),
            idle: self.pool.num_idle(),
        }
    }

    /// Run database migrations.
    ///
    /// This runs all pending SQL migrations from the `migrations/` directory.
    ///
    /// # Errors
    ///
    /// Returns an error if migrations fail.
    pub async fn run_migrations(&self) -> Result<(), EventStoreError> {
        sqlx::migrate!("../migrations")
            .run(&self.pool)
            .await
            .map_err(|e| EventStoreError::Connection(format!("Migration failed: {e}")))?;

        tracing::info!("Database migrations completed successfully");
        Ok(())
    }

    /// Append events and run an in-transaction projection atomically.
    ///
    /// The events are inserted and `projector.project_in_tx` runs inside the **same**
    /// transaction; it commits only if both succeed, so the projection can never
    /// diverge from the event stream. A projection error rolls the append back.
    ///
    /// This is the single-database counterpart to the [`EventStore::append`] +
    /// `Projector::project` two-step used by aggregates; sagas use it (via
    /// `DynAtomicPersist`) to keep their durable state consistent with their stream.
    ///
    /// # Errors
    ///
    /// Returns [`AtomicError::Append`] (including [`EventStoreError::VersionConflict`])
    /// if the append fails, or [`AtomicError::Projection`] (after rollback) if the
    /// projection fails.
    pub async fn append_with_projection<P: PgTransactionalProjector>(
        &self,
        stream_id: &StreamId,
        expected_version: Option<Version>,
        events: Vec<SerializedEvent>,
        projector: &P,
    ) -> Result<Version, AtomicError> {
        if events.is_empty() {
            return Err(AtomicError::Append(EventStoreError::Connection(
                "Cannot append empty event list".to_string(),
            )));
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AtomicError::Append(EventStoreError::Connection(e.to_string())))?;

        let final_version =
            insert_events_tx(&mut tx, stream_id.as_str(), expected_version, &events)
                .await
                .map_err(AtomicError::Append)?;

        // Stamp versions so the projector sees the same versioned events the caller
        // will broadcast.
        let versioned = stamp_versions(events, final_version);

        projector
            .project_in_tx(&mut tx, final_version, &versioned)
            .await
            .map_err(AtomicError::Projection)?;

        tx.commit()
            .await
            .map_err(|e| AtomicError::Append(EventStoreError::Connection(e.to_string())))?;

        Ok(final_version)
    }
}

/// A projection that runs INSIDE the event-append transaction.
///
/// Implemented by infrastructure that must update a read model atomically with the
/// event append (see [`PostgresEventStore::append_with_projection`]). All borrows
/// share `'a`, so the returned future may hold the connection borrow until it is
/// awaited; `append_with_projection` retains ownership of the transaction and commits
/// afterward.
pub trait PgTransactionalProjector: Send + Sync {
    /// Apply the just-appended `events` to the read model within the open transaction.
    ///
    /// `final_version` is the stream's new `MAX(version)` after this append. Use the
    /// provided `conn` (the transaction's connection) for all writes so they commit
    /// or roll back atomically with the events.
    ///
    /// # Errors
    ///
    /// Return [`ProjectionError`] to abort and roll back the entire transaction.
    fn project_in_tx<'a>(
        &'a self,
        conn: &'a mut sqlx::PgConnection,
        final_version: Version,
        events: &'a [SerializedEvent],
    ) -> impl std::future::Future<Output = Result<(), ProjectionError>> + Send + 'a;
}

/// Classify an event-`INSERT` failure into a domain [`EventStoreError`].
///
/// A `23505` unique-constraint violation means a concurrent writer already
/// committed `attempted_version` for this stream — an optimistic-concurrency
/// conflict. We deliberately do **not** re-query the actual version here: after
/// any statement error `PostgreSQL` aborts the surrounding transaction (`25P02`),
/// so a follow-up `SELECT` on the same connection would itself fail and mask the
/// real conflict as a [`EventStoreError::Connection`] — which the caller's retry
/// loop does not recognize. `attempted_version` (the version we tried to insert,
/// which the collision proves already exists) is a truthful `actual`.
fn classify_insert_error(
    e: &sqlx::Error,
    expected_version: Option<Version>,
    attempted_version: Version,
) -> EventStoreError {
    if let Some(db_err) = e.as_database_error() {
        if db_err.code().as_deref() == Some("23505") {
            return EventStoreError::VersionConflict {
                expected: expected_version,
                actual: attempted_version,
            };
        }
    }
    EventStoreError::Connection(e.to_string())
}

/// Version-check and insert `events` on an open transaction/connection (no commit).
///
/// Shared core used by [`PostgresEventStore::append_with_projection`]; mirrors the
/// concurrency check and insert loop of [`EventStore::append`]. Returns the final
/// stream version.
async fn insert_events_tx(
    conn: &mut sqlx::PgConnection,
    stream_id: &str,
    expected_version: Option<Version>,
    events: &[SerializedEvent],
) -> Result<Version, EventStoreError> {
    let current_version_i64: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), -1) FROM events WHERE stream_id = $1")
            .bind(stream_id)
            .fetch_one(&mut *conn)
            .await
            .map_err(|e| EventStoreError::Connection(e.to_string()))?;

    let current_version = if current_version_i64 == -1 {
        Version::initial()
    } else {
        Version::new(current_version_i64.try_into().unwrap_or(0))
    };

    if let Some(expected) = expected_version {
        if current_version != expected {
            return Err(EventStoreError::VersionConflict {
                expected: Some(expected),
                actual: current_version,
            });
        }
    }

    let mut next_version = current_version.next();
    for event in events {
        let version_i64 = i64::try_from(next_version.as_u64())
            .map_err(|e| EventStoreError::Connection(format!("Version overflow: {e}")))?;

        let metadata_json = metadata_to_json(event.metadata.as_ref())?;

        let result = sqlx::query(
            r"
            INSERT INTO events (stream_id, version, event_type, event_version, event_data, metadata, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, now())
            ",
        )
        .bind(stream_id)
        .bind(version_i64)
        .bind(&event.event_type)
        .bind(1i32)
        .bind(&event.payload)
        .bind(metadata_json)
        .execute(&mut *conn)
        .await;

        if let Err(e) = result {
            // Do not re-query here: the transaction is aborted after this error.
            return Err(classify_insert_error(&e, expected_version, next_version));
        }

        next_version = next_version.next();
    }

    Ok(next_version.prev())
}

/// Serialize an event's metadata to a JSON value for the `metadata` JSONB column.
///
/// `None` metadata binds SQL `NULL`, matching every row written before
/// metadata persistence existed.
fn metadata_to_json(
    metadata: Option<&EventMetadata>,
) -> Result<Option<serde_json::Value>, EventStoreError> {
    metadata
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| EventStoreError::Serialization(format!("metadata: {e}")))
}

/// Deserialize the `metadata` JSONB column back into [`EventMetadata`].
///
/// Tolerant by design: `NULL` (legacy rows) and undecodable JSON both load as
/// `None` — a malformed value is logged (with the row's stream and version so
/// the offending row is findable), never an error, so old streams stay
/// replayable.
fn metadata_from_json(
    stream_id: &str,
    version: Version,
    value: Option<serde_json::Value>,
) -> Option<EventMetadata> {
    value.and_then(|value| match serde_json::from_value(value) {
        Ok(metadata) => Some(metadata),
        Err(e) => {
            tracing::warn!(
                stream_id = %stream_id,
                version = version.as_u64(),
                error = %e,
                "Undecodable event metadata JSON; loading as None"
            );
            None
        },
    })
}

/// Stamp sequential stream versions onto freshly-appended events.
///
/// The last event gets `final_version`; earlier events count back from it.
fn stamp_versions(
    mut events: Vec<SerializedEvent>,
    final_version: Version,
) -> Vec<SerializedEvent> {
    let count = events.len() as u64;
    let start = final_version.as_u64() - count + 1;
    for (i, event) in events.iter_mut().enumerate() {
        event.version = Some(Version::new(start + i as u64));
    }
    events
}

impl EventStore for PostgresEventStore {
    fn load(
        &self,
        stream_id: &StreamId,
        from_version: Option<Version>,
    ) -> impl std::future::Future<Output = Result<Vec<SerializedEvent>, EventStoreError>> + Send
    {
        let stream_id_str = stream_id.as_str().to_string();
        let pool = self.pool.clone();

        let span = tracing::info_span!(
            "event_store.load",
            stream_id = %stream_id_str,
            from_version = ?from_version,
        );

        async move {
            let start = std::time::Instant::now();

            tracing::debug!(
                stream_id = %stream_id_str,
                from_version = ?from_version,
                "Loading events from stream"
            );

            let rows = if let Some(from_ver) = from_version {
                let from_version_i64 = i64::try_from(from_ver.as_u64())
                    .map_err(|e| EventStoreError::Connection(format!("Version overflow: {e}")))?;

                sqlx::query(
                    r"
                    SELECT version, event_type, event_data, metadata
                    FROM events
                    WHERE stream_id = $1 AND version >= $2
                    ORDER BY version ASC
                    ",
                )
                .bind(&stream_id_str)
                .bind(from_version_i64)
                .fetch_all(&pool)
                .await
            } else {
                sqlx::query(
                    r"
                    SELECT version, event_type, event_data, metadata
                    FROM events
                    WHERE stream_id = $1
                    ORDER BY version ASC
                    ",
                )
                .bind(&stream_id_str)
                .fetch_all(&pool)
                .await
            }
            .map_err(|e| EventStoreError::Connection(e.to_string()))?;

            let events: Vec<SerializedEvent> = rows
                .into_iter()
                .map(|row| {
                    let version_i64: i64 = row.get("version");
                    let version = Version::new(version_i64.try_into().unwrap_or(0));

                    SerializedEvent {
                        event_type: row.get("event_type"),
                        payload: row.get("event_data"),
                        metadata: metadata_from_json(&stream_id_str, version, row.get("metadata")),
                        version: Some(version),
                    }
                })
                .collect();

            tracing::debug!(
                stream_id = %stream_id_str,
                event_count = events.len(),
                "Loaded events from stream"
            );

            // Metrics
            let duration = start.elapsed();
            metrics::histogram!("event_store.load.duration_seconds").record(duration.as_secs_f64());
            #[allow(clippy::cast_precision_loss)]
            metrics::histogram!("event_store.load.event_count").record(events.len() as f64);

            Ok(events)
        }
        .instrument(span)
    }

    #[allow(clippy::too_many_lines)] // Complex database transaction logic
    fn append(
        &self,
        stream_id: &StreamId,
        expected_version: Option<Version>,
        events: Vec<SerializedEvent>,
    ) -> impl std::future::Future<Output = Result<Version, EventStoreError>> + Send {
        let stream_id_str = stream_id.as_str().to_string();
        let pool = self.pool.clone();

        let span = tracing::info_span!(
            "event_store.append",
            stream_id = %stream_id_str,
            expected_version = ?expected_version,
            event_count = events.len(),
        );

        async move {
            let start = std::time::Instant::now();

            if events.is_empty() {
                return Err(EventStoreError::Connection(
                    "Cannot append empty event list".to_string(),
                ));
            }

            tracing::debug!(
                stream_id = %stream_id_str,
                expected_version = ?expected_version,
                event_count = events.len(),
                "Appending events to stream"
            );

            #[allow(clippy::cast_precision_loss)]
            metrics::histogram!("event_store.append.event_count").record(events.len() as f64);

            // Start transaction
            let mut tx = pool
                .begin()
                .await
                .map_err(|e| EventStoreError::Connection(e.to_string()))?;

            // Get current version
            let current_version_i64: i64 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(version), -1) FROM events WHERE stream_id = $1",
            )
            .bind(&stream_id_str)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| EventStoreError::Connection(e.to_string()))?;

            let current_version = if current_version_i64 == -1 {
                Version::initial()
            } else {
                Version::new(current_version_i64.try_into().unwrap_or(0))
            };

            // Check optimistic concurrency
            if let Some(expected) = expected_version {
                if current_version != expected {
                    tracing::warn!(
                        stream_id = %stream_id_str,
                        expected = ?expected,
                        actual = ?current_version,
                        "Optimistic concurrency conflict"
                    );
                    return Err(EventStoreError::VersionConflict {
                        expected: Some(expected),
                        actual: current_version,
                    });
                }
            }

            // Insert events
            let mut next_version = current_version.next();
            for event in events {
                let version_i64 = i64::try_from(next_version.as_u64()).map_err(|e| {
                    EventStoreError::Connection(format!("Version overflow: {e}"))
                })?;

                let metadata_json = metadata_to_json(event.metadata.as_ref())?;

                let result = sqlx::query(
                    r"
                    INSERT INTO events (stream_id, version, event_type, event_version, event_data, metadata, created_at)
                    VALUES ($1, $2, $3, $4, $5, $6, now())
                    ",
                )
                .bind(&stream_id_str)
                .bind(version_i64)
                .bind(&event.event_type)
                .bind(1i32) // Default event_version
                .bind(&event.payload)
                .bind(metadata_json)
                .execute(&mut *tx)
                .await;

                if let Err(e) = result {
                    // Do not re-query here: the transaction is aborted after this error,
                    // so a follow-up SELECT would mask the conflict as a Connection error.
                    return Err(classify_insert_error(&e, expected_version, next_version));
                }

                next_version = next_version.next();
            }

            // Commit transaction
            tx.commit()
                .await
                .map_err(|e| EventStoreError::Connection(e.to_string()))?;

            let final_version = next_version.prev();

            tracing::debug!(
                stream_id = %stream_id_str,
                final_version = ?final_version,
                "Successfully appended events"
            );

            // Metrics
            let duration = start.elapsed();
            metrics::histogram!("event_store.append.duration_seconds")
                .record(duration.as_secs_f64());
            metrics::counter!("event_store.append.total", "result" => "success").increment(1);

            Ok(final_version)
        }
        .instrument(span)
    }
}

/// Run two transactional projectors sequentially on the same transaction.
///
/// Lets a consumer compose their domain projector with the
/// [`SagaJournalProjector`] on the atomic path:
/// `&(my_projector, journal_projector)`. The first error short-circuits and
/// rolls back the entire transaction — which is the point. Nest tuples for
/// more than two.
impl<A: PgTransactionalProjector, B: PgTransactionalProjector> PgTransactionalProjector for (A, B) {
    async fn project_in_tx(
        &self,
        conn: &mut sqlx::PgConnection,
        final_version: Version,
        events: &[SerializedEvent],
    ) -> Result<(), ProjectionError> {
        self.0
            .project_in_tx(&mut *conn, final_version, events)
            .await?;
        self.1
            .project_in_tx(&mut *conn, final_version, events)
            .await
    }
}

// ═══════════════════════════════════════════════════════════════════
// Durable-saga call journal (registry projection + recovery queries)
// ═══════════════════════════════════════════════════════════════════

/// Projects the durable-saga call journal markers into `saga_call_journal`.
///
/// Consumes `$saga.call_dispatched` / `$saga.call_completed` events and
/// maintains one row per dispatched call (`completed_at IS NULL` =
/// outstanding). All statements are idempotent upserts, so replay and
/// [`rebuild_from_store`](Self::rebuild_from_store) are safe.
///
/// It reads `stream_id` from the marker **payloads** (projector traits
/// deliver events without stream context) and each call's ID from the
/// event's stamped stream version.
///
/// # Wiring
///
/// Implements both projector traits:
///
/// - [`PgTransactionalProjector`] — the **recommended** path: compose with
///   your domain projector as `&(domain_projector, journal_projector)` in
///   [`PostgresEventStore::append_with_projection`] /
///   [`PostgresAtomicPersist`], so journal rows commit atomically with the
///   events. Registry-based recovery is only trustworthy on this path.
/// - [`Projector`] — the non-atomic path. Caveat: a crash between append
///   and project loses journal rows **permanently** (there is no catch-up
///   subscription), in either direction: a lost dispatch row hides an
///   instance from the recovery sweep; a lost completion row leaves a
///   zombie the sweep retries forever (harmlessly — `resume` returns
///   `NoOutstandingCalls`). If a registry-listed instance repeatedly
///   resolves to `NoOutstandingCalls`, run
///   [`rebuild_from_store`](Self::rebuild_from_store) to heal it.
///
/// # Logic tag
///
/// Construct with the saga type's `DurableBusinessLogic::LOGIC_TAG` so the
/// tag stamped on journal rows can never drift from the logic:
///
/// ```rust,ignore
/// let journal = SagaJournalProjector::new(pool.clone(), MySagaLogic::LOGIC_TAG);
/// ```
#[derive(Clone)]
pub struct SagaJournalProjector {
    pool: PgPool,
    logic_tag: &'static str,
}

impl SagaJournalProjector {
    /// Create a journal projector for one saga type.
    #[must_use]
    pub const fn new(pool: PgPool, logic_tag: &'static str) -> Self {
        Self { pool, logic_tag }
    }

    /// Apply marker events on an open connection (shared by both paths).
    async fn project_on_conn(
        &self,
        conn: &mut sqlx::PgConnection,
        events: &[SerializedEvent],
    ) -> Result<(), ProjectionError> {
        for event in events {
            match event.event_type.as_str() {
                CALL_DISPATCHED_EVENT_TYPE => {
                    let marker: CallDispatched =
                        bincode::deserialize(&event.payload).map_err(|e| {
                            ProjectionError::Deserialization(format!(
                                "{CALL_DISPATCHED_EVENT_TYPE}: {e}"
                            ))
                        })?;
                    let call_id = event.version.ok_or_else(|| {
                        ProjectionError::Custom(format!(
                            "{CALL_DISPATCHED_EVENT_TYPE} marker missing stream version"
                        ))
                    })?;
                    let call_id = i64::try_from(call_id.as_u64())
                        .map_err(|e| ProjectionError::Custom(format!("call_id overflow: {e}")))?;
                    let dispatched_at = event
                        .metadata
                        .as_ref()
                        .map_or_else(chrono::Utc::now, |m| m.timestamp);

                    sqlx::query(
                        r"
                        INSERT INTO saga_call_journal (stream_id, call_id, logic_tag, dispatched_at)
                        VALUES ($1, $2, $3, $4)
                        ON CONFLICT (stream_id, call_id) DO NOTHING
                        ",
                    )
                    .bind(&marker.stream_id)
                    .bind(call_id)
                    .bind(self.logic_tag)
                    .bind(dispatched_at)
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| ProjectionError::Database(e.to_string()))?;
                },
                CALL_COMPLETED_EVENT_TYPE => {
                    let marker: CallCompleted =
                        bincode::deserialize(&event.payload).map_err(|e| {
                            ProjectionError::Deserialization(format!(
                                "{CALL_COMPLETED_EVENT_TYPE}: {e}"
                            ))
                        })?;
                    let call_id = i64::try_from(marker.call_id.as_u64())
                        .map_err(|e| ProjectionError::Custom(format!("call_id overflow: {e}")))?;
                    let completed_at = event
                        .metadata
                        .as_ref()
                        .map_or_else(chrono::Utc::now, |m| m.timestamp);

                    // Upsert: covers a completion racing ahead of its
                    // dispatched row during rebuilds, and COALESCE keeps the
                    // first completion time under duplicate markers
                    // (at-least-once double-resume).
                    sqlx::query(
                        r"
                        INSERT INTO saga_call_journal
                            (stream_id, call_id, logic_tag, dispatched_at, completed_at)
                        VALUES ($1, $2, $3, $4, $4)
                        ON CONFLICT (stream_id, call_id) DO UPDATE
                        SET completed_at =
                            COALESCE(saga_call_journal.completed_at, EXCLUDED.completed_at)
                        ",
                    )
                    .bind(&marker.stream_id)
                    .bind(call_id)
                    .bind(self.logic_tag)
                    .bind(completed_at)
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| ProjectionError::Database(e.to_string()))?;
                },
                _ => {},
            }
        }
        Ok(())
    }

    /// Rebuild journal rows from the `events` table (disaster recovery, or
    /// healing after non-atomic-path projection loss).
    ///
    /// Replays every `$saga.*` marker whose **row** `stream_id` starts with
    /// `stream_id_prefix` through the same idempotent statements. Pass your
    /// saga type's stream prefix (e.g. `"spec-saga-"`) — the events table
    /// carries no logic tag, so the prefix is what scopes the rebuild to
    /// streams this projector's tag actually owns.
    ///
    /// The query filters by exact marker event types (index-served by
    /// `idx_events_type`); the prefix filter runs in Rust.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError`] on query failure or undecodable marker
    /// payloads.
    pub async fn rebuild_from_store(&self, stream_id_prefix: &str) -> Result<(), ProjectionError> {
        let rows = sqlx::query(
            r"
            SELECT stream_id, version, event_type, event_data, metadata
            FROM events
            WHERE event_type IN ($1, $2)
            ORDER BY stream_id, version ASC
            ",
        )
        .bind(CALL_DISPATCHED_EVENT_TYPE)
        .bind(CALL_COMPLETED_EVENT_TYPE)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ProjectionError::Database(e.to_string()))?;

        let events: Vec<SerializedEvent> = rows
            .into_iter()
            .filter(|row| {
                row.get::<String, _>("stream_id")
                    .starts_with(stream_id_prefix)
            })
            .map(|row| {
                let stream_id: String = row.get("stream_id");
                let version_i64: i64 = row.get("version");
                let version = Version::new(version_i64.try_into().unwrap_or(0));
                SerializedEvent {
                    event_type: row.get("event_type"),
                    payload: row.get("event_data"),
                    metadata: metadata_from_json(&stream_id, version, row.get("metadata")),
                    version: Some(version),
                }
            })
            .collect();

        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| ProjectionError::Database(e.to_string()))?;
        self.project_on_conn(&mut conn, &events).await
    }
}

impl std::fmt::Debug for SagaJournalProjector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SagaJournalProjector")
            .field("logic_tag", &self.logic_tag)
            .finish_non_exhaustive()
    }
}

impl PgTransactionalProjector for SagaJournalProjector {
    fn project_in_tx<'a>(
        &'a self,
        conn: &'a mut sqlx::PgConnection,
        _final_version: Version,
        events: &'a [SerializedEvent],
    ) -> impl std::future::Future<Output = Result<(), ProjectionError>> + Send + 'a {
        self.project_on_conn(conn, events)
    }
}

impl Projector for SagaJournalProjector {
    async fn project(&self, events: &[SerializedEvent]) -> Result<(), ProjectionError> {
        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| ProjectionError::Database(e.to_string()))?;
        self.project_on_conn(&mut conn, events).await
    }
}

/// [`SagaRegistry`] over the `saga_call_journal` table.
///
/// The recovery sweep's query side: instances with at least one
/// outstanding call, filtered by logic tag, served by the partial index
/// (`O(outstanding)`).
#[derive(Clone)]
pub struct PostgresSagaRegistry {
    pool: PgPool,
}

impl PostgresSagaRegistry {
    /// Create a registry over the given pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl std::fmt::Debug for PostgresSagaRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresSagaRegistry")
            .finish_non_exhaustive()
    }
}

impl SagaRegistry for PostgresSagaRegistry {
    type Error = sqlx::Error;

    async fn instances_with_outstanding_calls(
        &self,
        logic_tag: &str,
    ) -> Result<Vec<SagaInstanceRecord>, sqlx::Error> {
        let rows = sqlx::query(
            r"
            SELECT stream_id, logic_tag,
                   COUNT(*) AS outstanding,
                   MIN(dispatched_at) AS oldest
            FROM saga_call_journal
            WHERE completed_at IS NULL AND logic_tag = $1
            GROUP BY stream_id, logic_tag
            ORDER BY oldest ASC
            ",
        )
        .bind(logic_tag)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| SagaInstanceRecord {
                stream_id: StreamId::new(row.get::<String, _>("stream_id")),
                logic_tag: row.get("logic_tag"),
                outstanding_calls: row
                    .get::<i64, _>("outstanding")
                    .try_into()
                    .unwrap_or_default(),
                oldest_dispatched_at: row.get("oldest"),
            })
            .collect())
    }
}

/// [`DynAtomicPersist`] over [`PostgresEventStore::append_with_projection`].
///
/// Wire this into a `HandlerEnvironment::atomic_persist` so events and
/// projections (including the [`SagaJournalProjector`]) commit in one
/// transaction:
///
/// ```rust,ignore
/// let atomic = PostgresAtomicPersist::new(
///     store.clone(),
///     (my_projector, SagaJournalProjector::new(pool, MySagaLogic::LOGIC_TAG)),
/// );
/// ```
///
/// # Invariant
///
/// Must wrap the **same** store the environment's `event_store()` returns
/// (see `HandlerEnvironment::atomic_persist`).
pub struct PostgresAtomicPersist<P: PgTransactionalProjector> {
    store: PostgresEventStore,
    projector: P,
}

impl<P: PgTransactionalProjector> PostgresAtomicPersist<P> {
    /// Create an atomic persist over the given store and transactional
    /// projector (compose several with tuples).
    #[must_use]
    pub const fn new(store: PostgresEventStore, projector: P) -> Self {
        Self { store, projector }
    }
}

impl<P: PgTransactionalProjector> std::fmt::Debug for PostgresAtomicPersist<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresAtomicPersist")
            .finish_non_exhaustive()
    }
}

impl<P: PgTransactionalProjector> DynAtomicPersist for PostgresAtomicPersist<P> {
    fn append_and_project<'a>(
        &'a self,
        stream_id: &'a StreamId,
        expected_version: Option<Version>,
        events: Vec<SerializedEvent>,
    ) -> futures::future::BoxFuture<'a, Result<Version, AtomicError>> {
        Box::pin(async move {
            self.store
                .append_with_projection(stream_id, expected_version, events, &self.projector)
                .await
        })
    }
}

// ═══════════════════════════════════════════════════════════════════
// Multi-node sweep coordination
// ═══════════════════════════════════════════════════════════════════

/// A per-saga-instance `PostgreSQL` advisory lock for recovery sweeps.
///
/// Multi-node deployments use this to skip instances another node is
/// already resuming: acquire before `Handler::resume`, release after.
/// Without it a double-resume is *safe* (at-least-once) but wasteful —
/// each in-flight call may execute twice.
///
/// # Leak-proof by construction
///
/// Advisory locks are **session-scoped**, and pooled connections return to
/// the pool with their session (and locks!) intact — the classic leak. This
/// guard therefore [`detach`es](sqlx::pool::PoolConnection::detach) the
/// connection from the pool before locking: the session belongs to the
/// guard alone, so even if the guard is dropped without
/// [`release`](Self::release) (including on panic), closing the connection
/// ends the session and `PostgreSQL` frees the lock. It can never leak into
/// the pool.
///
/// # Example
///
/// ```rust,ignore
/// match SagaSweepLock::try_acquire(&pool, &record.stream_id).await? {
///     None => continue, // another node owns this instance
///     Some(lock) => {
///         let outcome = handler.resume(&record.stream_id, token).await;
///         lock.release().await?;
///         // handle outcome...
///     }
/// }
/// ```
pub struct SagaSweepLock {
    conn: Option<sqlx::PgConnection>,
    stream_id: String,
}

impl SagaSweepLock {
    /// Try to acquire the advisory lock for one saga instance.
    ///
    /// Returns `Ok(None)` when another session holds the lock (skip the
    /// instance — someone else is resuming it).
    ///
    /// # Errors
    ///
    /// Returns the underlying `sqlx` error on connection/query failure.
    pub async fn try_acquire(
        pool: &PgPool,
        stream_id: &StreamId,
    ) -> Result<Option<Self>, sqlx::Error> {
        // Detach: the session must belong to this guard, never the pool.
        let mut conn = pool.acquire().await?.detach();

        // hashtextextended: 64-bit key space (fewer collisions than the
        // 32-bit hashtext).
        let locked: bool =
            sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtextextended($1, 0))")
                .bind(stream_id.as_str())
                .fetch_one(&mut conn)
                .await?;

        if locked {
            Ok(Some(Self {
                conn: Some(conn),
                stream_id: stream_id.as_str().to_string(),
            }))
        } else {
            // No lock held; close the detached session eagerly.
            let _ = sqlx::Connection::close(conn).await;
            Ok(None)
        }
    }

    /// Release the lock and close the session (the graceful path).
    ///
    /// Dropping the guard without calling this also releases the lock
    /// (the detached session dies with the connection), just less tidily.
    ///
    /// # Errors
    ///
    /// Returns the underlying `sqlx` error if the unlock or close fails —
    /// the session still ends either way, so the lock is freed regardless.
    pub async fn release(mut self) -> Result<(), sqlx::Error> {
        if let Some(mut conn) = self.conn.take() {
            sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
                .bind(&self.stream_id)
                .execute(&mut conn)
                .await?;
            sqlx::Connection::close(conn).await?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for SagaSweepLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SagaSweepLock")
            .field("stream_id", &self.stream_id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_event_store_is_send_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}

        assert_send::<PostgresEventStore>();
        assert_sync::<PostgresEventStore>();
    }

    #[test]
    fn pool_stats_utilization() {
        let stats = PoolStats { size: 10, idle: 3 };
        assert!((stats.utilization() - 0.7).abs() < 0.001);
        assert!(!stats.is_saturated());

        let saturated = PoolStats { size: 10, idle: 0 };
        assert!(saturated.is_saturated());
        assert!((saturated.utilization() - 1.0).abs() < 0.001);

        let empty = PoolStats { size: 0, idle: 0 };
        assert!((empty.utilization() - 0.0).abs() < 0.001);
    }
}
