//! Projection system for building and maintaining read models from events.
//!
//! # Overview
//!
//! Projections are the **query side of CQRS** (Command Query Responsibility Segregation).
//! While events and event stores handle the write side (commands → events → state),
//! projections handle the read side (events → denormalized views for queries).
//!
//! ## Key Concepts
//!
//! - **Projection**: Transforms events into optimized read models
//! - **Projection Store**: Backend storage for projection data (Postgres, Redis, etc.)
//! - **Checkpoint**: Tracks projection progress through the event stream
//! - **Catch-up**: Replaying events to rebuild or update projections
//!
//! ## CQRS Separation
//!
//! ```text
//! Write Side:                  Read Side:
//! ┌─────────────────┐         ┌─────────────────┐
//! │  Event Store    │         │  Projections    │
//! │  (Postgres 1)   │         │  (Postgres 2)   │
//! │                 │         │                 │
//! │  events table   │         │  customer_view  │
//! │  snapshots      │         │  order_summary  │
//! └─────────────────┘         │  product_search │
//!                             └─────────────────┘
//!         │                            ▲
//!         │ Events published           │ Updated by
//!         │ to Event Bus               │ projections
//!         ▼                            │
//! ┌──────────────────────────────────────┐
//! │         Event Bus (Redpanda)         │
//! └──────────────────────────────────────┘
//! ```
//!
//! ## Example
//!
//! ```ignore
//! use composable_rust_core::projection::*;
//!
//! struct CustomerOrderHistoryProjection {
//!     store: Arc<dyn ProjectionStore>,
//! }
//!
//! impl Projection for CustomerOrderHistoryProjection {
//!     type Event = OrderEvent;
//!
//!     fn name(&self) -> &str {
//!         "customer_order_history"
//!     }
//!
//!     async fn apply_event(&self, event: &Self::Event) -> Result<()> {
//!         match event {
//!             OrderEvent::OrderPlaced { order_id, customer_id, items, total } => {
//!                 // Update projection store
//!                 let summary = OrderSummary { /* ... */ };
//!                 self.store.save(&format!("order:{}", order_id), &summary).await?;
//!                 Ok(())
//!             }
//!             _ => Ok(()),
//!         }
//!     }
//! }
//! ```

use crate::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

/// Error type for projection operations.
#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    /// Storage backend error
    #[error("Storage error: {0}")]
    Storage(String),

    /// Serialization/deserialization error
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Checkpoint error
    #[error("Checkpoint error: {0}")]
    Checkpoint(String),

    /// Event processing error
    #[error("Event processing error: {0}")]
    EventProcessing(String),

    /// Generic error
    #[error("Projection error: {0}")]
    Other(String),
}

/// Result type for projection operations.
pub type Result<T> = std::result::Result<T, ProjectionError>;

/// A projection builds and maintains a read model from events.
///
/// Projections subscribe to events from the event bus and update denormalized
/// views optimized for querying. This is the query side of CQRS.
///
/// # Philosophy
///
/// - **Eventually Consistent**: Projections lag behind events (typically 10-100ms)
/// - **Optimized for Reads**: Schema designed for query patterns, not writes
/// - **Rebuildable**: Can be dropped and rebuilt from events at any time
/// - **Separate Storage**: Often uses different database than event store
///
/// # Example
///
/// ```ignore
/// struct OrderSummaryProjection {
///     store: Arc<PostgresProjectionStore>,
/// }
///
/// impl Projection for OrderSummaryProjection {
///     type Event = OrderEvent;
///
///     fn name(&self) -> &str {
///         "order_summary"
///     }
///
///     async fn apply_event(&self, event: &Self::Event) -> Result<()> {
///         match event {
///             OrderEvent::OrderPlaced { order_id, total, .. } => {
///                 // Store in queryable format
///                 self.store.save(&format!("order:{}", order_id), &total).await
///             }
///             _ => Ok(()),
///         }
///     }
///
///     async fn rebuild(&self) -> Result<()> {
///         // Drop projection data
///         self.store.clear("order:*").await
///     }
/// }
/// ```
pub trait Projection: Send + Sync {
    /// The event type this projection listens to.
    ///
    /// Must be deserializable from the event stream.
    type Event: for<'de> Deserialize<'de> + Send;

    /// Get the projection name (used for checkpointing and identification).
    ///
    /// Should be unique across all projections in the system.
    ///
    /// # Example
    ///
    /// ```ignore
    /// fn name(&self) -> &str {
    ///     "customer_order_history"
    /// }
    /// ```
    fn name(&self) -> &str;

    /// Apply an event to update the projection.
    ///
    /// This is called for each event consumed from the event bus.
    /// The projection should extract relevant data and update its storage.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError`] if event processing or storage fails.
    ///
    /// # Idempotency
    ///
    /// Implementations should be idempotent where possible, as events
    /// may be replayed during catch-up or error recovery.
    ///
    /// # Example
    ///
    /// ```ignore
    /// async fn apply_event(&self, event: &Self::Event) -> Result<()> {
    ///     match event {
    ///         OrderEvent::OrderPlaced { order_id, customer_id, total } => {
    ///             let key = format!("customer:{}:orders", customer_id);
    ///             self.store.append(&key, order_id).await?;
    ///             Ok(())
    ///         }
    ///         _ => Ok(()), // Ignore other events
    ///     }
    /// }
    /// ```
    fn apply_event(&self, event: &Self::Event) -> impl Future<Output = Result<()>> + Send;

    /// Rebuild projection from scratch (optional).
    ///
    /// This drops current projection data and prepares for a full replay
    /// of all events. Called before catch-up when rebuilding.
    ///
    /// Default implementation is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError`] if rebuild fails.
    ///
    /// # Example
    ///
    /// ```ignore
    /// async fn rebuild(&self) -> Result<()> {
    ///     // Drop projection table
    ///     sqlx::query("TRUNCATE order_projections")
    ///         .execute(&self.pool)
    ///         .await
    ///         .map_err(|e| ProjectionError::Storage(e.to_string()))?;
    ///     Ok(())
    /// }
    /// ```
    fn rebuild(&self) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }
}

/// Storage backend for projection data.
///
/// Projections can use different storage from the event store, optimized
/// for query patterns. Common backends: Postgres (JSONB), Redis (cache), Elasticsearch (search).
///
/// # CQRS Separation
///
/// For true CQRS, use a **separate database** from the event store:
/// - **Event Store DB**: Optimized for append-only writes
/// - **Projection DB**: Optimized for reads and complex queries
///
/// # Example
///
/// ```ignore
/// // Simple key-value storage
/// let store = PostgresProjectionStore::new(pool, "projections");
/// store.save("customer:123", &customer_data).await?;
/// let data = store.get("customer:123").await?;
/// ```
pub trait ProjectionStore: Send + Sync {
    /// Save projection data to storage.
    ///
    /// Implementations should handle upserts (insert or update).
    ///
    /// # Arguments
    ///
    /// - `key`: Unique identifier for this projection data
    /// - `data`: Serialized projection data (bincode or JSON)
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError::Storage`] if save fails.
    fn save(
        &self,
        key: &str,
        data: &[u8],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Get projection data by key.
    ///
    /// # Arguments
    ///
    /// - `key`: Unique identifier for the projection data
    ///
    /// # Returns
    ///
    /// - `Some(data)` if found
    /// - `None` if not found
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError::Storage`] if retrieval fails.
    fn get(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Delete projection data by key.
    ///
    /// # Arguments
    ///
    /// - `key`: Unique identifier for the projection data
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError::Storage`] if deletion fails.
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send;

    /// Check if projection data exists.
    ///
    /// Default implementation uses [`ProjectionStore::get`].
    ///
    /// # Arguments
    ///
    /// - `key`: Unique identifier for the projection data
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError::Storage`] if check fails.
    fn exists(&self, key: &str) -> impl Future<Output = Result<bool>> + Send {
        async move { Ok(self.get(key).await?.is_some()) }
    }
}

/// Checkpoint tracking for projection progress through the event stream.
///
/// Checkpoints allow projections to resume from where they left off after
/// restarts, crashes, or rebuilds.
///
/// # Checkpoint Strategy
///
/// - Save checkpoint every N events (e.g., every 100)
/// - Save on graceful shutdown
/// - Use event offset or position for resumption
///
/// # Example
///
/// ```ignore
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
/// ```
///
/// # Dyn Compatibility
///
/// This trait uses explicit `Pin<Box<dyn Future>>` returns instead of `impl Future`
/// to enable trait object usage (`Arc<dyn ProjectionCheckpoint>`). This is required
/// for the projection manager where checkpoints are passed as dependencies.
pub trait ProjectionCheckpoint: Send + Sync {
    /// Save the current position in the event stream.
    ///
    /// # Arguments
    ///
    /// - `projection_name`: Unique projection identifier
    /// - `position`: Current event position (offset, timestamp)
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError::Checkpoint`] if save fails.
    fn save_position(
        &self,
        projection_name: &str,
        position: EventPosition,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;

    /// Load the last saved position for a projection.
    ///
    /// # Arguments
    ///
    /// - `projection_name`: Unique projection identifier
    ///
    /// # Returns
    ///
    /// - `Some(position)` if checkpoint exists
    /// - `None` if this is a new projection (start from beginning)
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError::Checkpoint`] if load fails.
    fn load_position(
        &self,
        projection_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<EventPosition>>> + Send + '_>>;
}

/// Position in the event stream (for checkpoint resumption).
///
/// Represents where a projection has processed up to in the event stream.
/// Used for resuming after restarts and tracking projection lag.
///
/// # Example
///
/// ```
/// use composable_rust_core::projection::EventPosition;
/// use chrono::Utc;
///
/// let position = EventPosition {
///     offset: 1000,
///     timestamp: Utc::now(),
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPosition {
    /// Offset in the event stream (Kafka offset, event sequence number, etc.)
    pub offset: u64,

    /// Timestamp when this position was reached
    pub timestamp: DateTime<Utc>,
}

impl EventPosition {
    /// Create a new event position.
    ///
    /// # Example
    ///
    /// ```
    /// use composable_rust_core::projection::EventPosition;
    /// use chrono::Utc;
    ///
    /// let position = EventPosition::new(1000, Utc::now());
    /// ```
    #[must_use]
    pub const fn new(offset: u64, timestamp: DateTime<Utc>) -> Self {
        Self { offset, timestamp }
    }

    /// Create a position at the beginning of the stream.
    ///
    /// # Example
    ///
    /// ```
    /// use composable_rust_core::projection::EventPosition;
    ///
    /// let start = EventPosition::beginning();
    /// assert_eq!(start.offset, 0);
    /// ```
    #[must_use]
    pub fn beginning() -> Self {
        Self {
            offset: 0,
            timestamp: Utc::now(),
        }
    }
}
