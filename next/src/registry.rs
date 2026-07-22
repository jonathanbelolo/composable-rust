//! Saga instance registry for recovery sweeps
//!
//! A [`SagaRegistry`] answers one question: **which saga instances have
//! outstanding journal entries?** It is the query side of the durable-saga
//! call journal — typically a projection over the `$saga.call_dispatched` /
//! `$saga.call_completed` marker events (see the `composable-rust-postgres-next`
//! crate for the canonical `PostgreSQL` implementation).
//!
//! # Recovery predicate
//!
//! "Needs recovery" ≡ "has outstanding journal entries" — **nothing else**.
//! There is deliberately no "terminal" flag: the framework cannot know one
//! (a saga parked at a human-review gate returns `Done` exactly like a
//! finished saga does, and is re-entered later by a user command). A parked
//! instance has zero outstanding calls and therefore never appears in this
//! query; `Handler::resume` on it is a no-op either way.

use std::future::Future;

use crate::{Correlation, StreamId};

/// One saga instance with outstanding (dispatched-but-uncompleted) calls.
#[derive(Debug, Clone)]
pub struct SagaInstanceRecord {
    /// The instance's event stream.
    pub stream_id: StreamId,

    /// The saga type's stable tag
    /// ([`DurableBusinessLogic::LOGIC_TAG`](crate::DurableBusinessLogic::LOGIC_TAG)).
    pub logic_tag: String,

    /// Number of outstanding calls.
    pub outstanding_calls: u64,

    /// When the oldest still-outstanding call was dispatched — useful for
    /// alerting on instances stuck far longer than any sane call duration.
    pub oldest_dispatched_at: chrono::DateTime<chrono::Utc>,
}

/// Query surface over the saga call journal, for recovery sweeps.
///
/// # Logic-tag filtering is mandatory
///
/// `Handler::resume` deserializes journaled call payloads as the handler's
/// own `Call` type. A sweep that feeds saga type X's handler a stream
/// journaled by type Y decodes garbage (bincode may even succeed on the
/// wrong type). Always sweep with your own logic's
/// [`LOGIC_TAG`](crate::DurableBusinessLogic::LOGIC_TAG).
pub trait SagaRegistry: Send + Sync {
    /// Error type for registry queries.
    type Error: std::error::Error + Send + Sync + 'static;

    /// All instances of the given saga type with at least one outstanding
    /// call.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying query fails (database error,
    /// etc.).
    fn instances_with_outstanding_calls(
        &self,
        logic_tag: &str,
    ) -> impl Future<Output = Result<Vec<SagaInstanceRecord>, Self::Error>> + Send;
}

/// One parked saga instance whose await deadline has passed.
#[derive(Debug, Clone)]
pub struct ExpiredAwait {
    /// The instance's event stream.
    pub stream_id: StreamId,

    /// The correlation the instance is parked on (passed back to
    /// [`Handler::resume_awaiting`](crate::Handler::resume_awaiting) so the
    /// wake clears exactly this entry).
    pub correlation: Correlation,

    /// The saga-chosen tag identifying which await timed out.
    pub timeout_tag: String,
}

/// Query surface over the saga **correlation index**: "which parked instance
/// waits on this correlation?" and "which awaits have expired?".
///
/// # The write side is NOT here
///
/// Index rows are registered / deregistered by PROJECTING the `$saga.awaiting`
/// / `$saga.awaiting_cleared` marker events (see the
/// `composable-rust-postgres-next` `SagaCorrelationProjector`), so every row
/// commits ATOMICALLY with the saga's domain events in the same transaction.
/// An imperative `register()` after the append would be a non-atomic two-step
/// that could park a saga unfindably on a crash between persist and register —
/// so it is deliberately not offered.
pub trait CorrelationIndex: Send + Sync {
    /// Error type for index queries.
    type Error: std::error::Error + Send + Sync + 'static;

    /// The stream of the saga instance parked on `correlation`, if any.
    ///
    /// Scoped by `logic_tag` so two saga types may reuse a correlation key
    /// name without collision. Returns `None` when no instance is parked on it
    /// (never parked, or already woken — the wake's `$saga.awaiting_cleared`
    /// deregistered the row), which is what makes event delivery idempotent.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying query fails.
    fn find(
        &self,
        logic_tag: &str,
        correlation: &Correlation,
    ) -> impl Future<Output = Result<Option<StreamId>, Self::Error>> + Send;

    /// All instances of the given saga type whose await deadline is at or
    /// before `now` — the input to a timeout sweep.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying query fails.
    fn find_expired(
        &self,
        logic_tag: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> impl Future<Output = Result<Vec<ExpiredAwait>, Self::Error>> + Send;
}
