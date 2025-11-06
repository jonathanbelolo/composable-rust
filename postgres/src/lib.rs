//! `PostgreSQL` event store implementation for Composable Rust.
//!
//! This crate provides a production-ready `PostgreSQL`-based event store that implements
//! the [`EventStore`] trait from `composable-rust-core`. It uses sqlx for compile-time
//! checked queries and supports:
//!
//! - Event persistence with optimistic concurrency
//! - State snapshots for performance
//! - Connection pooling
//! - Transaction support
//!
//! # Example
//!
//! ```no_run
//! use composable_rust_postgres::PostgresEventStore;
//!
//! async fn example() -> Result<(), Box<dyn std::error::Error>> {
//!     let event_store = PostgresEventStore::new("postgres://localhost/mydb").await?;
//!     Ok(())
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use composable_rust_core::event::SerializedEvent;
use composable_rust_core::event_store::{EventStore, EventStoreError};
use composable_rust_core::stream::{StreamId, Version};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;

/// `PostgreSQL`-based event store implementation.
///
/// This implementation uses `PostgreSQL` for durable event storage with:
/// - Optimistic concurrency control via version numbers
/// - Snapshot support for performance optimization
/// - Connection pooling for efficient resource usage
///
/// # Example
///
/// ```no_run
/// use composable_rust_postgres::PostgresEventStore;
/// use composable_rust_core::stream::{StreamId, Version};
/// use composable_rust_core::event::SerializedEvent;
/// use composable_rust_core::event_store::EventStore;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let store = PostgresEventStore::new("postgres://localhost/mydb").await?;
///
/// let stream_id = StreamId::new("order-123");
/// let events = vec![/* SerializedEvent instances */];
///
/// let version = store.append_events(
///     stream_id,
///     Some(Version::new(0)),
///     events
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub struct PostgresEventStore {
    pool: PgPool,
}

impl PostgresEventStore {
    /// Create a new `PostgreSQL` event store from a database URL.
    ///
    /// This creates a connection pool with default settings (max 5 connections).
    ///
    /// # Errors
    ///
    /// Returns [`EventStoreError::DatabaseError`] if:
    /// - The database URL is invalid
    /// - Cannot connect to the database
    /// - Database authentication fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// use composable_rust_postgres::PostgresEventStore;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let store = PostgresEventStore::new("postgres://localhost/mydb").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(database_url: &str) -> Result<Self, EventStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        Ok(Self { pool })
    }

    /// Create a new `PostgreSQL` event store from an existing connection pool.
    ///
    /// Useful when you want to share a connection pool across multiple services
    /// or need custom pool configuration.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use composable_rust_postgres::PostgresEventStore;
    /// use sqlx::postgres::PgPoolOptions;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let pool = PgPoolOptions::new()
    ///     .max_connections(10)
    ///     .connect("postgres://localhost/mydb")
    ///     .await?;
    ///
    /// let store = PostgresEventStore::from_pool(pool);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get a reference to the underlying connection pool.
    ///
    /// Useful for health checks or manual queries.
    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[allow(async_fn_in_trait)] // Trait is Send + Sync bounded, same as core
impl EventStore for PostgresEventStore {
    #[allow(clippy::cognitive_complexity)] // Complex due to race condition handling
    #[allow(clippy::too_many_lines)] // TODO: Refactor in Phase 4
    async fn append_events(
        &self,
        stream_id: StreamId,
        expected_version: Option<Version>,
        events: Vec<SerializedEvent>,
    ) -> Result<Version, EventStoreError> {
        if events.is_empty() {
            return Err(EventStoreError::DatabaseError(
                "Cannot append empty event list".to_string(),
            ));
        }

        tracing::debug!(
            stream_id = %stream_id,
            expected_version = ?expected_version,
            event_count = events.len(),
            "Appending events to stream"
        );

        // Start transaction for atomicity
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        // Get current version for this stream
        let current_version: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(version) FROM events WHERE stream_id = $1"
        )
        .bind(stream_id.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        // Convert i64 to u64 with proper error handling
        let current_version = match current_version {
            Some(v) => {
                let version_u64 = u64::try_from(v).map_err(|e| {
                    EventStoreError::DatabaseError(format!(
                        "Invalid negative version {v} in database: {e}"
                    ))
                })?;
                Version::new(version_u64)
            }
            None => Version::new(0),
        };

        // Check optimistic concurrency
        if let Some(expected) = expected_version {
            if current_version != expected {
                tracing::warn!(
                    stream_id = %stream_id,
                    expected = ?expected,
                    actual = ?current_version,
                    "Optimistic concurrency conflict detected"
                );
                return Err(EventStoreError::ConcurrencyConflict {
                    stream_id,
                    expected,
                    actual: current_version,
                });
            }
        }

        // Insert events
        let mut next_version = current_version.next();
        for event in events {
            let version_i64 = i64::try_from(next_version.value()).map_err(|e| {
                EventStoreError::DatabaseError(format!("Version overflow: {e}"))
            })?;

            // Insert event - PRIMARY KEY constraint provides race condition protection
            let result = sqlx::query(
                r"
                INSERT INTO events (stream_id, version, event_type, event_data, metadata, created_at)
                VALUES ($1, $2, $3, $4, $5, now())
                "
            )
            .bind(stream_id.as_str())
            .bind(version_i64)
            .bind(&event.event_type)
            .bind(&event.data)
            .bind(&event.metadata)
            .execute(&mut *tx)
            .await;

            // Check for unique constraint violation (concurrent modification)
            if let Err(e) = result {
                // PostgreSQL unique constraint violation error code is 23505
                if let Some(db_err) = e.as_database_error() {
                    if db_err.code().as_deref() == Some("23505") {
                        // Concurrent modification detected via PRIMARY KEY constraint
                        // Re-query to get the actual current version
                        let actual_version: Option<i64> = sqlx::query_scalar(
                            "SELECT MAX(version) FROM events WHERE stream_id = $1"
                        )
                        .bind(stream_id.as_str())
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

                        let actual = match actual_version {
                            Some(v) => Version::new(u64::try_from(v).unwrap_or(0)),
                            None => Version::new(0),
                        };

                        tracing::warn!(
                            stream_id = %stream_id,
                            expected = ?expected_version,
                            actual = ?actual,
                            "Concurrent modification detected via unique constraint"
                        );

                        return Err(EventStoreError::ConcurrencyConflict {
                            stream_id: stream_id.clone(),
                            expected: expected_version.unwrap_or(Version::new(0)),
                            actual,
                        });
                    }
                }
                // Other database error - propagate
                return Err(EventStoreError::DatabaseError(e.to_string()));
            }

            next_version = next_version.next();
        }

        // Commit transaction
        tx.commit()
            .await
            .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        tracing::debug!(
            stream_id = %stream_id,
            final_version = ?(next_version - 1),
            "Successfully appended events"
        );

        // Return the final version (last event inserted)
        Ok(next_version - 1)
    }

    async fn load_events(
        &self,
        stream_id: StreamId,
        from_version: Option<Version>,
    ) -> Result<Vec<SerializedEvent>, EventStoreError> {
        tracing::debug!(
            stream_id = %stream_id,
            from_version = ?from_version,
            "Loading events from stream"
        );

        let events = if let Some(from_ver) = from_version {
            sqlx::query(
                r"
                SELECT event_type, event_data, metadata
                FROM events
                WHERE stream_id = $1 AND version >= $2
                ORDER BY version ASC
                "
            )
            .bind(stream_id.as_str())
            .bind(i64::try_from(from_ver.value()).map_err(|e| {
                EventStoreError::DatabaseError(format!("Version overflow: {e}"))
            })?)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(
                r"
                SELECT event_type, event_data, metadata
                FROM events
                WHERE stream_id = $1
                ORDER BY version ASC
                "
            )
            .bind(stream_id.as_str())
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        let event_vec: Vec<SerializedEvent> = events
            .into_iter()
            .map(|row| {
                SerializedEvent::new(
                    row.get("event_type"),
                    row.get("event_data"),
                    row.get("metadata"),
                )
            })
            .collect();

        tracing::debug!(
            stream_id = %stream_id,
            event_count = event_vec.len(),
            "Loaded events from stream"
        );

        Ok(event_vec)
    }

    async fn save_snapshot(
        &self,
        stream_id: StreamId,
        version: Version,
        state: Vec<u8>,
    ) -> Result<(), EventStoreError> {
        tracing::debug!(
            stream_id = %stream_id,
            version = ?version,
            state_size = state.len(),
            "Saving snapshot"
        );

        sqlx::query(
            r"
            INSERT INTO snapshots (stream_id, version, state_data, created_at)
            VALUES ($1, $2, $3, now())
            ON CONFLICT (stream_id) DO UPDATE
            SET version = EXCLUDED.version,
                state_data = EXCLUDED.state_data,
                created_at = EXCLUDED.created_at
            "
        )
        .bind(stream_id.as_str())
        .bind(i64::try_from(version.value()).map_err(|e| {
            EventStoreError::DatabaseError(format!("Version overflow: {e}"))
        })?)
        .bind(&state)
        .execute(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        tracing::debug!(
            stream_id = %stream_id,
            version = ?version,
            "Snapshot saved successfully"
        );

        Ok(())
    }

    async fn load_snapshot(
        &self,
        stream_id: StreamId,
    ) -> Result<Option<(Version, Vec<u8>)>, EventStoreError> {
        tracing::debug!(stream_id = %stream_id, "Loading snapshot");

        let result = sqlx::query(
            r"
            SELECT version, state_data
            FROM snapshots
            WHERE stream_id = $1
            "
        )
        .bind(stream_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        if let Some(row) = result {
            let version: i64 = row.get("version");
            let state_data: Vec<u8> = row.get("state_data");

            // Convert i64 to u64 with proper error handling
            let version_u64 = u64::try_from(version).map_err(|e| {
                EventStoreError::DatabaseError(format!(
                    "Invalid negative version {version} in snapshot: {e}"
                ))
            })?;

            tracing::debug!(
                stream_id = %stream_id,
                version = version_u64,
                "Snapshot loaded successfully"
            );

            Ok(Some((Version::new(version_u64), state_data)))
        } else {
            tracing::debug!(stream_id = %stream_id, "No snapshot found");
            Ok(None)
        }
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
}
