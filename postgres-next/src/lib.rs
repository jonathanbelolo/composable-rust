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
    EventStore, EventStoreError, SerializedEvent, StreamId, Version,
};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;
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
                let from_version_i64 = i64::try_from(from_ver.as_u64()).map_err(|e| {
                    EventStoreError::Connection(format!("Version overflow: {e}"))
                })?;

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
            metrics::histogram!("event_store.load.duration_seconds")
                .record(duration.as_secs_f64());
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
                    // Check for unique constraint violation (concurrent modification)
                    if let Some(db_err) = e.as_database_error() {
                        if db_err.code().as_deref() == Some("23505") {
                            // Re-query actual version
                            let actual_i64: Option<i64> = sqlx::query_scalar(
                                "SELECT MAX(version) FROM events WHERE stream_id = $1",
                            )
                            .bind(&stream_id_str)
                            .fetch_optional(&mut *tx)
                            .await
                            .map_err(|e| EventStoreError::Connection(e.to_string()))?;

                            let actual = actual_i64.map_or_else(Version::initial, |v| {
                                Version::new(v.try_into().unwrap_or(0))
                            });

                            return Err(EventStoreError::VersionConflict {
                                expected: expected_version,
                                actual,
                            });
                        }
                    }
                    return Err(EventStoreError::Connection(e.to_string()));
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
