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
    AtomicError, EventStore, EventStoreError, ProjectionError, SerializedEvent, StreamId, Version,
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
        .bind::<Option<serde_json::Value>>(None)
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
                        metadata: None, // TODO: Parse metadata if needed
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
                .bind::<Option<serde_json::Value>>(None) // TODO: Serialize metadata if needed
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
