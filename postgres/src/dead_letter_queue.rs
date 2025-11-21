//! Dead Letter Queue (DLQ) for failed events.
//!
//! Provides persistent storage and management of events that failed processing
//! after exhausting retries. Enables observability, incident response, and
//! manual reprocessing workflows.

use chrono::{DateTime, Utc};
use composable_rust_core::event::SerializedEvent;
use composable_rust_core::event_store::EventStoreError;
use sqlx::{PgPool, Row};

/// Status of a failed event in the Dead Letter Queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DLQStatus {
    /// Event is pending investigation/reprocessing
    Pending,
    /// Event is currently being processed
    Processing,
    /// Event was successfully reprocessed
    Resolved,
    /// Event was permanently discarded (cannot be fixed)
    Discarded,
}

impl DLQStatus {
    /// Convert status to database string representation.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Processing => "processing",
            Self::Resolved => "resolved",
            Self::Discarded => "discarded",
        }
    }

    /// Parse status from database string.
    ///
    /// # Errors
    ///
    /// Returns error if the string doesn't match a known status.
    pub fn parse(s: &str) -> Result<Self, EventStoreError> {
        match s {
            "pending" => Ok(Self::Pending),
            "processing" => Ok(Self::Processing),
            "resolved" => Ok(Self::Resolved),
            "discarded" => Ok(Self::Discarded),
            _ => Err(EventStoreError::DatabaseError(format!(
                "Invalid DLQ status: {s}"
            ))),
        }
    }
}

/// An entry in the Dead Letter Queue.
///
/// Contains the failed event data plus failure metadata for troubleshooting.
#[derive(Debug, Clone)]
pub struct FailedEvent {
    /// Unique identifier for this DLQ entry
    pub id: i64,

    /// The stream ID this event belongs to
    pub stream_id: String,

    /// The event that failed
    pub event: SerializedEvent,

    /// When the event was originally created
    pub original_timestamp: DateTime<Utc>,

    /// Error message from the failure
    pub error_message: String,

    /// Full error details (debug output, stack trace, etc.)
    pub error_details: Option<String>,

    /// Number of times processing was retried
    pub retry_count: i32,

    /// When this event first failed
    pub first_failed_at: DateTime<Utc>,

    /// When this event most recently failed
    pub last_failed_at: DateTime<Utc>,

    /// Current processing status
    pub status: DLQStatus,

    /// When the failure was resolved (if applicable)
    pub resolved_at: Option<DateTime<Utc>>,

    /// Who/what resolved the failure
    pub resolved_by: Option<String>,

    /// Notes about the resolution
    pub resolution_notes: Option<String>,
}

/// `PostgreSQL`-based Dead Letter Queue for failed events.
///
/// Provides persistent storage for events that failed processing, enabling:
/// - Incident investigation and debugging
/// - Manual reprocessing workflows
/// - Failure trend analysis
/// - Compliance and audit trails
///
/// # Example
///
/// ```no_run
/// use composable_rust_postgres::DeadLetterQueue;
/// use composable_rust_core::event::SerializedEvent;
///
/// # async fn example(pool: sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
/// let dlq = DeadLetterQueue::new(pool);
///
/// // List pending failures
/// let pending = dlq.list_pending(100).await?;
/// println!("Pending failures: {}", pending.len());
///
/// // Mark one as processing
/// dlq.update_status(pending[0].id, composable_rust_postgres::DLQStatus::Processing).await?;
/// # Ok(())
/// # }
/// ```
pub struct DeadLetterQueue {
    pool: PgPool,
}

impl DeadLetterQueue {
    /// Create a new Dead Letter Queue with the given connection pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Add a failed event to the DLQ.
    ///
    /// # Arguments
    ///
    /// * `stream_id` - The stream ID this event belongs to
    /// * `event` - The event that failed
    /// * `original_timestamp` - When the event was originally created
    /// * `error_message` - Human-readable error message
    /// * `error_details` - Full error details (debug output, stack trace)
    /// * `retry_count` - Number of retries attempted before giving up
    ///
    /// # Returns
    ///
    /// The unique ID of the created DLQ entry.
    ///
    /// # Errors
    ///
    /// Returns [`EventStoreError::DatabaseError`] if the insert fails.
    pub async fn add_entry(
        &self,
        stream_id: &str,
        event: &SerializedEvent,
        original_timestamp: DateTime<Utc>,
        error_message: &str,
        error_details: Option<&str>,
        retry_count: i32,
    ) -> Result<i64, EventStoreError> {
        let id: (i64,) = sqlx::query_as(
            r"
            INSERT INTO failed_events (
                stream_id, event_type, event_version, event_data, metadata,
                original_timestamp, error_message, error_details, retry_count
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING id
            ",
        )
        .bind(stream_id)
        .bind(&event.event_type)
        .bind(event.event_version)
        .bind(&event.data)
        .bind(event.metadata.as_ref().map(composable_rust_core::event::EventMetadata::to_json))
        .bind(original_timestamp)
        .bind(error_message)
        .bind(error_details)
        .bind(retry_count)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        tracing::warn!(
            dlq_id = id.0,
            stream_id = stream_id,
            event_type = %event.event_type,
            error = error_message,
            retry_count = retry_count,
            "Event added to Dead Letter Queue"
        );

        metrics::counter!("event_store.dlq.added", "event_type" => event.event_type.clone())
            .increment(1);

        Ok(id.0)
    }

    /// List pending failed events.
    ///
    /// Returns events in order of oldest first (FIFO processing).
    ///
    /// # Arguments
    ///
    /// * `limit` - Maximum number of entries to return
    ///
    /// # Errors
    ///
    /// Returns [`EventStoreError::DatabaseError`] if the query fails.
    pub async fn list_pending(&self, limit: usize) -> Result<Vec<FailedEvent>, EventStoreError> {
        self.list_by_status(DLQStatus::Pending, limit).await
    }

    /// List failed events by status.
    ///
    /// # Arguments
    ///
    /// * `status` - The status to filter by
    /// * `limit` - Maximum number of entries to return
    ///
    /// # Errors
    ///
    /// Returns [`EventStoreError::DatabaseError`] if the query fails.
    pub async fn list_by_status(
        &self,
        status: DLQStatus,
        limit: usize,
    ) -> Result<Vec<FailedEvent>, EventStoreError> {
        #[allow(clippy::cast_possible_wrap)] // Limit is reasonable size, i64 is safe
        let rows = sqlx::query(
            r"
            SELECT
                id, stream_id, event_type, event_version, event_data, metadata,
                original_timestamp, error_message, error_details, retry_count,
                first_failed_at, last_failed_at, status,
                resolved_at, resolved_by, resolution_notes
            FROM failed_events
            WHERE status = $1
            ORDER BY first_failed_at ASC
            LIMIT $2
            ",
        )
        .bind(status.as_str())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        rows.iter()
            .map(Self::row_to_failed_event)
            .collect()
    }

    /// Get a specific failed event by ID.
    ///
    /// # Errors
    ///
    /// Returns [`EventStoreError::DatabaseError`] if the query fails or entry not found.
    pub async fn get_by_id(&self, id: i64) -> Result<FailedEvent, EventStoreError> {
        let row = sqlx::query(
            r"
            SELECT
                id, stream_id, event_type, event_version, event_data, metadata,
                original_timestamp, error_message, error_details, retry_count,
                first_failed_at, last_failed_at, status,
                resolved_at, resolved_by, resolution_notes
            FROM failed_events
            WHERE id = $1
            ",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        Self::row_to_failed_event(&row)
    }

    /// Update the status of a failed event.
    ///
    /// # Errors
    ///
    /// Returns [`EventStoreError::DatabaseError`] if the update fails.
    pub async fn update_status(
        &self,
        id: i64,
        status: DLQStatus,
    ) -> Result<(), EventStoreError> {
        sqlx::query(
            r"
            UPDATE failed_events
            SET status = $1, last_failed_at = NOW()
            WHERE id = $2
            ",
        )
        .bind(status.as_str())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        tracing::info!(dlq_id = id, status = status.as_str(), "DLQ entry status updated");

        Ok(())
    }

    /// Mark a failed event as resolved.
    ///
    /// # Arguments
    ///
    /// * `id` - The DLQ entry ID
    /// * `resolved_by` - Who/what resolved it (e.g., username, service name)
    /// * `notes` - Resolution notes (what was done, why it worked)
    ///
    /// # Errors
    ///
    /// Returns [`EventStoreError::DatabaseError`] if the update fails.
    pub async fn mark_resolved(
        &self,
        id: i64,
        resolved_by: &str,
        notes: Option<&str>,
    ) -> Result<(), EventStoreError> {
        sqlx::query(
            r"
            UPDATE failed_events
            SET status = 'resolved',
                resolved_at = NOW(),
                resolved_by = $1,
                resolution_notes = $2
            WHERE id = $3
            ",
        )
        .bind(resolved_by)
        .bind(notes)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        tracing::info!(
            dlq_id = id,
            resolved_by = resolved_by,
            "DLQ entry marked as resolved"
        );

        metrics::counter!("event_store.dlq.resolved").increment(1);

        Ok(())
    }

    /// Mark a failed event as discarded (permanently failed).
    ///
    /// Use this when a failure cannot be fixed (e.g., data corruption, schema mismatch).
    ///
    /// # Arguments
    ///
    /// * `id` - The DLQ entry ID
    /// * `reason` - Why it was discarded
    ///
    /// # Errors
    ///
    /// Returns [`EventStoreError::DatabaseError`] if the update fails.
    pub async fn mark_discarded(&self, id: i64, reason: &str) -> Result<(), EventStoreError> {
        sqlx::query(
            r"
            UPDATE failed_events
            SET status = 'discarded',
                resolved_at = NOW(),
                resolution_notes = $1
            WHERE id = $2
            ",
        )
        .bind(reason)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        tracing::warn!(dlq_id = id, reason = reason, "DLQ entry marked as discarded");

        metrics::counter!("event_store.dlq.discarded").increment(1);

        Ok(())
    }

    /// Get count of pending failures.
    ///
    /// Useful for monitoring and health checks.
    ///
    /// # Errors
    ///
    /// Returns [`EventStoreError::DatabaseError`] if the query fails.
    pub async fn count_pending(&self) -> Result<i64, EventStoreError> {
        let (count,): (i64,) = sqlx::query_as(
            r"
            SELECT COUNT(*)
            FROM failed_events
            WHERE status = 'pending'
            ",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        Ok(count)
    }

    /// Convert a database row to a `FailedEvent`.
    fn row_to_failed_event(row: &sqlx::postgres::PgRow) -> Result<FailedEvent, EventStoreError> {
        let metadata_json: Option<serde_json::Value> = row.get("metadata");
        let metadata = metadata_json.and_then(|json| {
            composable_rust_core::event::EventMetadata::from_json(&json).ok()
        });

        let status_str: String = row.get("status");
        let status = DLQStatus::parse(&status_str)?;

        Ok(FailedEvent {
            id: row.get("id"),
            stream_id: row.get("stream_id"),
            event: SerializedEvent {
                event_type: row.get("event_type"),
                event_version: row.get("event_version"),
                data: row.get("event_data"),
                metadata,
            },
            original_timestamp: row.get("original_timestamp"),
            error_message: row.get("error_message"),
            error_details: row.get("error_details"),
            retry_count: row.get("retry_count"),
            first_failed_at: row.get("first_failed_at"),
            last_failed_at: row.get("last_failed_at"),
            status,
            resolved_at: row.get("resolved_at"),
            resolved_by: row.get("resolved_by"),
            resolution_notes: row.get("resolution_notes"),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // Test code can use unwrap/expect
mod tests {
    use super::*;

    #[test]
    fn dlq_status_roundtrip() {
        for status in &[
            DLQStatus::Pending,
            DLQStatus::Processing,
            DLQStatus::Resolved,
            DLQStatus::Discarded,
        ] {
            let s = status.as_str();
            let parsed = DLQStatus::parse(s).expect("valid status should parse");
            assert_eq!(*status, parsed);
        }
    }

    #[test]
    fn dlq_status_invalid() {
        assert!(DLQStatus::parse("invalid").is_err());
    }
}
