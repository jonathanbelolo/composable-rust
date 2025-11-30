//! Projection queries trait for CQRS read models
//!
//! This module defines the trait that business logic uses to query read models.
//! Projections are async because they typically involve database access.
//!
//! # Example
//!
//! ```rust,ignore
//! pub struct PostgresProjectionQueries {
//!     pool: sqlx::PgPool,
//! }
//!
//! impl ProjectionQueries for PostgresProjectionQueries {
//!     type Error = sqlx::Error;
//!
//!     async fn get_event(&self, id: EventId) -> Result<Option<EventDto>, Self::Error> {
//!         sqlx::query_as!(EventRow, "SELECT * FROM events WHERE id = $1", id.as_uuid())
//!             .fetch_optional(&self.pool)
//!             .await
//!             .map(|row| row.map(EventDto::from))
//!     }
//! }
//! ```

use std::future::Future;

/// Trait for querying projection data (read models)
///
/// This trait is used by business logic to query read models for CQRS queries.
/// Implementations typically access a database or other read-optimized storage.
///
/// # Async
///
/// All query methods are async because they typically involve I/O operations.
/// This is why `BusinessLogic::process()` must be async when supporting queries.
///
/// # Implementation Notes
///
/// - Implement this trait in your application layer (not the framework)
/// - Return `Ok(None)` when an entity is not found (not an error)
/// - Return `Err` only for infrastructure failures (DB connection, etc.)
///
/// # Example: In-Memory Implementation (for testing)
///
/// ```rust,ignore
/// use std::collections::HashMap;
/// use std::sync::RwLock;
///
/// pub struct InMemoryProjectionQueries {
///     events: RwLock<HashMap<EventId, EventDto>>,
/// }
///
/// impl ProjectionQueries for InMemoryProjectionQueries {
///     type Error = std::convert::Infallible;
///
///     async fn get_event(&self, id: EventId) -> Result<Option<EventDto>, Self::Error> {
///         Ok(self.events.read().unwrap().get(&id).cloned())
///     }
/// }
/// ```
pub trait ProjectionQueries: Send + Sync {
    /// Error type for query operations
    ///
    /// Use `std::convert::Infallible` for implementations that never fail
    /// (like in-memory implementations).
    type Error: std::error::Error + Send + Sync + 'static;
}

/// No-op implementation for aggregates that don't support queries
///
/// Use this when an aggregate doesn't have any query operations.
/// All query methods will never be called since the aggregate only
/// returns `BusinessResult::Done` or `BusinessResult::Continue`.
#[derive(Debug, Clone, Default)]
pub struct NoOpProjectionQueries;

impl ProjectionQueries for NoOpProjectionQueries {
    type Error = std::convert::Infallible;
}

/// Extension trait for projections that can return items by ID
///
/// This is a marker trait showing a common pattern. Applications
/// should implement domain-specific query methods on their
/// `ProjectionQueries` implementation.
pub trait GetById<Id, T>: ProjectionQueries {
    /// Get an entity by ID
    ///
    /// Returns `Ok(None)` if not found, `Err` only for infrastructure errors.
    fn get_by_id(&self, id: Id) -> impl Future<Output = Result<Option<T>, Self::Error>> + Send;
}
