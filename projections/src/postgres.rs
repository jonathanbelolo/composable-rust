//! `PostgreSQL` implementations for projections.
//!
//! # Overview
//!
//! Provides `PostgreSQL`-backed storage for projections with:
//! - Generic key-value storage (`projection_data` table)
//! - Custom queryable projections (create your own tables)
//! - Checkpoint tracking for resumption
//! - Separate database support (true CQRS)
//!
//! # Architecture
//!
//! ```text
//! Write Side (Event Store)          Read Side (Projections)
//! ┌─────────────────────┐          ┌─────────────────────┐
//! │  PostgreSQL DB #1   │          │  PostgreSQL DB #2   │
//! │                     │          │                     │
//! │  events             │          │  projection_data    │
//! │  snapshots          │   →→→    │  order_projections  │
//! │                     │  Events  │  customer_views     │
//! └─────────────────────┘          └─────────────────────┘
//! ```
//!
//! # Examples
//!
//! ## Simple Projection (Generic Table)
//!
//! ```ignore
//! use composable_rust_projections::postgres::*;
//!
//! let store = PostgresProjectionStore::new(pool, "projection_data".to_string());
//! store.save("customer:123", &customer_data).await?;
//! ```
//!
//! ## Separate Database (CQRS)
//!
//! ```ignore
//! // Event store on one database
//! let event_store = PostgresEventStore::new("postgres://localhost/events").await?;
//!
//! // Projections on separate database
//! let projection_store = PostgresProjectionStore::new_with_separate_db(
//!     "postgres://localhost/projections",
//!     "projection_data".to_string(),
//! ).await?;
//! ```

use composable_rust_core::projection::{
    EventPosition, ProjectionCheckpoint, ProjectionError, ProjectionStore, Result,
};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::future::Future;
use std::pin::Pin;

/// PostgreSQL-backed projection store.
///
/// Provides persistent storage for projection data using `PostgreSQL`.
/// Supports both generic key-value storage and custom queryable tables.
///
/// # Generic Storage
///
/// Uses the `projection_data` table for simple key-value storage:
///
/// ```sql
/// CREATE TABLE projection_data (
///     key TEXT PRIMARY KEY,
///     data BYTEA NOT NULL,
///     updated_at TIMESTAMPTZ NOT NULL
/// );
/// ```
///
/// # Custom Storage
///
/// For queryable projections, create custom tables with proper indexes:
///
/// ```sql
/// CREATE TABLE order_projections (
///     id TEXT PRIMARY KEY,
///     customer_id TEXT NOT NULL,
///     data JSONB NOT NULL,
///     total DECIMAL(10,2),
///     status TEXT,
///     created_at TIMESTAMPTZ NOT NULL
/// );
/// CREATE INDEX idx_customer ON order_projections(customer_id);
/// ```
///
/// # CQRS Separation
///
/// For true CQRS, use [`PostgresProjectionStore::new_with_separate_db`] to connect
/// to a different database than the event store.
///
/// # Example
///
/// ```ignore
/// use composable_rust_projections::postgres::*;
///
/// // Share pool with event store (simple setup)
/// let store = PostgresProjectionStore::new(pool, "projection_data".to_string());
///
/// // Or use separate database (CQRS best practice)
/// let store = PostgresProjectionStore::new_with_separate_db(
///     "postgres://localhost/projections",
///     "projection_data".to_string(),
/// ).await?;
///
/// // Store projection data
/// store.save("customer:123", &data).await?;
/// let retrieved = store.get("customer:123").await?;
/// ```
#[derive(Clone)]
pub struct PostgresProjectionStore {
    pool: PgPool,
    table_name: String,
}

impl PostgresProjectionStore {
    /// Create a new projection store using an existing connection pool.
    ///
    /// # Arguments
    ///
    /// - `pool`: Existing `PostgreSQL` connection pool
    /// - `table_name`: Name of the table for projection data
    ///
    /// # Example
    ///
    /// ```ignore
    /// let store = PostgresProjectionStore::new(pool, "projection_data".to_string());
    /// ```
    #[must_use]
    pub const fn new(pool: PgPool, table_name: String) -> Self {
        Self { pool, table_name }
    }

    /// Create a new projection store with a separate database connection.
    ///
    /// This is the recommended approach for **true CQRS** - keeping write-side
    /// (event store) and read-side (projections) in separate databases.
    ///
    /// # Arguments
    ///
    /// - `database_url`: `PostgreSQL` connection string for projection database
    /// - `table_name`: Name of the table for projection data
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError::Storage`] if connection fails.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Event store on one database
    /// let event_store = PostgresEventStore::new("postgres://localhost/events").await?;
    ///
    /// // Projections on separate database
    /// let projection_store = PostgresProjectionStore::new_with_separate_db(
    ///     "postgres://localhost/projections",
    ///     "projection_data".to_string(),
    /// ).await?;
    /// ```
    pub async fn new_with_separate_db(
        database_url: &str,
        table_name: String,
    ) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(10) // Reasonable default for projection queries
            .connect(database_url)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to connect: {e}")))?;

        Ok(Self::new(pool, table_name))
    }

    /// Run database migrations for projection tables.
    ///
    /// This creates the necessary tables (`projection_data`, `projection_checkpoints`, etc.)
    /// if they don't already exist.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError::Storage`] if migration fails.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let store = PostgresProjectionStore::new_with_separate_db(
    ///     "postgres://localhost/projections",
    ///     "projection_data".to_string(),
    /// ).await?;
    ///
    /// store.migrate().await?;
    /// ```
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Migration failed: {e}")))?;
        Ok(())
    }

    /// Get the underlying connection pool.
    ///
    /// Useful for custom queries or transactions.
    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Get the table name for this projection store.
    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.table_name
    }
}

impl ProjectionStore for PostgresProjectionStore {
    async fn save(&self, key: &str, data: &[u8]) -> Result<()> {
        // Use dynamic SQL since table name can vary
        let query = format!(
            "INSERT INTO {} (key, data, updated_at)
             VALUES ($1, $2, now())
             ON CONFLICT (key) DO UPDATE
             SET data = EXCLUDED.data, updated_at = now()",
            self.table_name
        );

        sqlx::query(&query)
            .bind(key)
            .bind(data)
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to save: {e}")))?;

        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let query = format!(
            "SELECT data FROM {} WHERE key = $1",
            self.table_name
        );

        let result: Option<(Vec<u8>,)> = sqlx::query_as(&query)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to get: {e}")))?;

        Ok(result.map(|(data,)| data))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let query = format!(
            "DELETE FROM {} WHERE key = $1",
            self.table_name
        );

        sqlx::query(&query)
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to delete: {e}")))?;

        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let query = format!(
            "SELECT EXISTS(SELECT 1 FROM {} WHERE key = $1)",
            self.table_name
        );

        let (exists,): (bool,) = sqlx::query_as(&query)
            .bind(key)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to check exists: {e}")))?;

        Ok(exists)
    }
}

/// PostgreSQL-backed checkpoint tracking.
///
/// Tracks where each projection has processed up to in the event stream,
/// enabling resumption after restarts or failures.
///
/// # Schema
///
/// ```sql
/// CREATE TABLE projection_checkpoints (
///     projection_name TEXT PRIMARY KEY,
///     event_offset BIGINT NOT NULL,
///     event_timestamp TIMESTAMPTZ NOT NULL,
///     updated_at TIMESTAMPTZ NOT NULL
/// );
/// ```
///
/// # Example
///
/// ```ignore
/// use composable_rust_projections::postgres::*;
///
/// let checkpoint = PostgresProjectionCheckpoint::new(pool);
///
/// // Save progress
/// checkpoint.save_position("order_summary", EventPosition {
///     offset: 1000,
///     timestamp: Utc::now(),
/// }).await?;
///
/// // Resume from last position
/// let last_position = checkpoint.load_position("order_summary").await?;
/// if let Some(position) = last_position {
///     println!("Resuming from offset {}", position.offset);
/// } else {
///     println!("Starting from beginning");
/// }
/// ```
#[derive(Clone)]
pub struct PostgresProjectionCheckpoint {
    pool: PgPool,
}

impl PostgresProjectionCheckpoint {
    /// Create a new checkpoint tracker using an existing connection pool.
    ///
    /// # Arguments
    ///
    /// - `pool`: `PostgreSQL` connection pool
    ///
    /// # Example
    ///
    /// ```ignore
    /// let checkpoint = PostgresProjectionCheckpoint::new(pool);
    /// ```
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new checkpoint tracker with a separate database connection.
    ///
    /// # Arguments
    ///
    /// - `database_url`: `PostgreSQL` connection string
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError::Checkpoint`] if connection fails.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let checkpoint = PostgresProjectionCheckpoint::new_with_separate_db(
    ///     "postgres://localhost/projections"
    /// ).await?;
    /// ```
    pub async fn new_with_separate_db(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5) // Checkpoints are low-volume
            .connect(database_url)
            .await
            .map_err(|e| ProjectionError::Checkpoint(format!("Failed to connect: {e}")))?;

        Ok(Self::new(pool))
    }

    /// Get the underlying connection pool.
    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl ProjectionCheckpoint for PostgresProjectionCheckpoint {
    fn save_position(
        &self,
        projection_name: &str,
        position: EventPosition,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let projection_name = projection_name.to_string();
        Box::pin(async move {
            // EventPosition uses u64 for offset, but PostgreSQL BIGINT is i64.
            // Wrapping would occur at 2^63 events (~9 quintillion), which is acceptable.
            // At 1 million events/sec, this would take 292,471 years to wrap.
            #[allow(clippy::cast_possible_wrap)]
            let offset_i64 = position.offset as i64;

            sqlx::query(
                "INSERT INTO projection_checkpoints (projection_name, event_offset, event_timestamp, updated_at)
                 VALUES ($1, $2, $3, now())
                 ON CONFLICT (projection_name) DO UPDATE
                 SET event_offset = EXCLUDED.event_offset,
                     event_timestamp = EXCLUDED.event_timestamp,
                     updated_at = now()"
            )
            .bind(projection_name)
            .bind(offset_i64)
            .bind(position.timestamp)
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Checkpoint(format!("Failed to save checkpoint: {e}")))?;

            Ok(())
        })
    }

    fn load_position(
        &self,
        projection_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<EventPosition>>> + Send + '_>> {
        let projection_name = projection_name.to_string();
        Box::pin(async move {
            let result: Option<(i64, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
                "SELECT event_offset, event_timestamp
                 FROM projection_checkpoints
                 WHERE projection_name = $1"
            )
            .bind(projection_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ProjectionError::Checkpoint(format!("Failed to load checkpoint: {e}")))?;

            Ok(result.map(|(offset, timestamp)| {
                #[allow(clippy::cast_sign_loss)] // Offset is always positive in our system
                EventPosition {
                    offset: offset as u64,
                    timestamp,
                }
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use composable_rust_core::projection::EventPosition;

    // Note: These are unit tests for the structure.
    // Integration tests with real Postgres are in the tests/ directory.

    #[test]
    fn test_event_position_creation() {
        let position = EventPosition::new(100, chrono::Utc::now());
        assert_eq!(position.offset, 100);
    }

    #[test]
    fn test_event_position_beginning() {
        let position = EventPosition::beginning();
        assert_eq!(position.offset, 0);
    }
}
