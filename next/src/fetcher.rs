//! Query fetcher trait for pre-fetching projection data.
//!
//! The [`QueryFetcher`] trait enables the [`Handler`](crate::Handler) to fetch
//! projection data BEFORE calling `BusinessLogic::process()`. This is the core
//! of the CQRS read path - all validation data comes from projections, not the
//! event store.
//!
//! # Architecture
//!
//! ```text
//! Handler.handle(input)
//!     │
//!     ├─► QueryFetcher.fetch(input, projections)
//!     │       │
//!     │       ├─► Returns (prepared_input, expected_version)
//!     │       │   - prepared_input: Input with `fetched` field populated
//!     │       │   - expected_version: For optimistic concurrency on writes
//!     │       │
//!     │
//!     ├─► BusinessLogic.process(prepared_input) ──► Pure logic
//!     │                                              (validates against fetched data)
//!     │
//!     ├─► EventStore.append(expected_version) ──► Optimistic concurrency check
//!     │
//!     └─► On version conflict: RETRY with fresh projection data
//! ```
//!
//! # Version Tracking
//!
//! For writes, the fetcher returns the expected version from the projection.
//! This version is used for optimistic concurrency when appending to the event store.
//!
//! - **New entities**: Return `None` (new stream)
//! - **Existing entities**: Return `Some(version)` from projection
//!
//! # Example: Event Aggregate Query Fetcher
//!
//! ```rust,ignore
//! use composable_rust_next::{QueryFetcher, ProjectionQueries, Version};
//!
//! pub struct EventQueryFetcher;
//!
//! impl QueryFetcher<EventCommand, EventProjectionQueries> for EventQueryFetcher {
//!     type Error = sqlx::Error;
//!
//!     async fn fetch(
//!         &self,
//!         input: EventCommand,
//!         projections: &EventProjectionQueries,
//!     ) -> Result<FetchResult<EventCommand>, Self::Error> {
//!         match input {
//!             // Query: fetch data, no version needed
//!             EventCommand::GetEvent { event_id, requesting_user_id, .. } => {
//!                 let fetched = projections.get_event(event_id).await?;
//!                 Ok(FetchResult::new(
//!                     EventCommand::GetEvent { event_id, requesting_user_id, fetched },
//!                     None, // Queries don't need version
//!                 ))
//!             }
//!
//!             // Create: no existing data, new stream
//!             EventCommand::Create { event_id, name, .. } => {
//!                 Ok(FetchResult::new(
//!                     EventCommand::Create { event_id, name, fetched: None },
//!                     None, // New stream, no expected version
//!                 ))
//!             }
//!
//!             // Update: fetch existing data with version
//!             EventCommand::Publish { event_id, .. } => {
//!                 let fetched = projections.get_event_with_version(event_id).await?;
//!                 let version = fetched.as_ref().map(|f| f.version);
//!                 Ok(FetchResult::new(
//!                     EventCommand::Publish { event_id, fetched },
//!                     version, // Use version for optimistic concurrency
//!                 ))
//!             }
//!         }
//!     }
//! }
//! ```

use std::convert::Infallible;
use std::future::Future;

use crate::{ProjectionQueries, Version};

/// Result of fetching projection data.
///
/// Contains the prepared input and an optional expected version for
/// optimistic concurrency control.
#[derive(Debug, Clone)]
pub struct FetchResult<Input> {
    /// The prepared input with fetched data populated
    pub input: Input,

    /// Expected version for optimistic concurrency.
    ///
    /// - `None`: New stream (first write) or query (no write)
    /// - `Some(v)`: Existing entity at version v
    pub expected_version: Option<Version>,
}

impl<Input> FetchResult<Input> {
    /// Create a new fetch result.
    #[must_use]
    pub const fn new(input: Input, expected_version: Option<Version>) -> Self {
        Self { input, expected_version }
    }

    /// Create a fetch result for a new entity (no expected version).
    #[must_use]
    pub const fn new_entity(input: Input) -> Self {
        Self { input, expected_version: None }
    }

    /// Create a fetch result for an existing entity with a known version.
    #[must_use]
    pub const fn existing(input: Input, version: Version) -> Self {
        Self { input, expected_version: Some(version) }
    }
}

/// Trait for pre-fetching projection data.
///
/// This trait is the cornerstone of CQRS in this architecture:
///
/// - **Queries**: Fetch data from projections, return to caller
/// - **Commands**: Fetch validation data from projections, use for business logic
///
/// The event store is NEVER touched for reading validation state. Only for:
/// 1. Version check during append (optimistic concurrency)
/// 2. Projection rebuilding (disaster recovery)
///
/// # Type Parameters
///
/// - `Input`: The command/input type (same as `BusinessLogic::Input`)
/// - `Projections`: The projection queries type (same as `Environment::Projections`)
///
/// # Version Tracking
///
/// Implementations should return the entity's current version from the projection.
/// This enables optimistic concurrency without touching the event store.
///
/// # Aggregates Without Queries
///
/// Use [`NoOpQueryFetcher`] for simple aggregates. It passes input through
/// with `expected_version: None` (suitable for create-only commands).
pub trait QueryFetcher<Input, Projections>: Send + Sync
where
    Input: Send,
    Projections: ProjectionQueries,
{
    /// Error type for fetch operations
    ///
    /// Use `std::convert::Infallible` for implementations that never fail.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Fetch projection data and expected version.
    ///
    /// This method:
    /// 1. Examines the input to determine what data is needed
    /// 2. Fetches data from projections
    /// 3. Returns prepared input + expected version for concurrency control
    ///
    /// # Returns
    ///
    /// - `FetchResult.input`: The input with `fetched` field populated
    /// - `FetchResult.expected_version`: Version for optimistic concurrency
    ///   - `None` for new entities or queries
    ///   - `Some(version)` for existing entities
    ///
    /// # Errors
    ///
    /// Returns an error if the projection query fails (database error, etc.)
    fn fetch(
        &self,
        input: Input,
        projections: &Projections,
    ) -> impl Future<Output = Result<FetchResult<Input>, Self::Error>> + Send;
}

/// No-op query fetcher for simple aggregates.
///
/// This implementation passes all inputs through unchanged with no expected version.
/// Suitable for:
/// - Create-only aggregates
/// - Aggregates that handle version tracking differently
/// - Testing
///
/// For production use with updates/deletes, implement a custom [`QueryFetcher`]
/// that fetches version from projections.
///
/// # Example
///
/// ```rust,ignore
/// let handler = Handler::new(
///     MyBusinessLogic,
///     NoOpCallExecutor,
///     NoOpQueryFetcher,  // Simple pass-through
///     env,
/// );
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOpQueryFetcher;

impl<Input, Projections> QueryFetcher<Input, Projections> for NoOpQueryFetcher
where
    Input: Send,
    Projections: ProjectionQueries,
{
    type Error = Infallible;

    async fn fetch(
        &self,
        input: Input,
        _projections: &Projections,
    ) -> Result<FetchResult<Input>, Self::Error> {
        Ok(FetchResult::new_entity(input))
    }
}
