//! Error types for the handler and serialization

use crate::{EventBusError, EventStoreError};

/// Errors that can occur during projection
///
/// Projections update read models (query-side databases) from events.
/// These errors indicate issues with that process.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum ProjectionError {
    /// Database error during projection
    #[error("database error: {0}")]
    Database(String),

    /// Failed to deserialize an event for projection
    #[error("deserialization error: {0}")]
    Deserialization(String),

    /// Projection timed out
    #[error("projection timeout")]
    Timeout,

    /// Custom projection error
    #[error("{0}")]
    Custom(String),
}

/// Error from an atomic append-and-project operation.
///
/// Distinguishes a failed event-store append (which may be a recoverable
/// [`EventStoreError::VersionConflict`]) from a failed in-transaction projection,
/// so the `Handler` can map each to the right [`HandlerError`] and preserve
/// optimistic-concurrency retries.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum AtomicError {
    /// The event-store append failed; carries `VersionConflict` for retry.
    #[error("append failed: {0}")]
    Append(#[from] EventStoreError),

    /// The in-transaction projection failed; the append was rolled back.
    #[error("projection failed: {0}")]
    Projection(#[from] ProjectionError),
}

/// Errors that can occur during handler execution
///
/// This enum distinguishes between business errors (from the `BusinessLogic` trait)
/// and infrastructure errors (event store, event bus, serialization).
///
/// # Type Parameter
///
/// - `E`: The business error type from `BusinessLogic::Error`
///
/// # Error Handling Strategy
///
/// - **Business errors**: Return to the caller; nothing is persisted
/// - **Infrastructure errors**: May require retry or manual intervention
///
/// # Examples
///
/// ```rust,ignore
/// match handler.handle(command).await {
///     Ok(result) => println!("Success: version {}", result.version),
///     Err(HandlerError::Business(e)) => {
///         // Business rule violation - return 400 Bad Request
///         println!("Validation failed: {}", e);
///     }
///     Err(HandlerError::Load(e)) => {
///         // Infrastructure issue - return 503 Service Unavailable
///         println!("Event store unavailable: {}", e);
///     }
///     Err(e) => {
///         // Other infrastructure errors
///         println!("Error: {}", e);
///     }
/// }
/// ```
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HandlerError<E: std::error::Error> {
    /// Business logic error (validation, invalid state transition)
    ///
    /// The handler returns this when `BusinessLogic::process()` returns `Err`.
    /// Nothing is persisted.
    #[error("business logic error: {0}")]
    Business(E),

    /// Failed to load events from the event store
    ///
    /// This typically indicates an infrastructure issue (database connection,
    /// network timeout) rather than a business error.
    #[error("failed to load events: {0}")]
    Load(EventStoreError),

    /// Failed to persist events to the event store
    ///
    /// This may be a version conflict (optimistic concurrency) or an
    /// infrastructure issue.
    #[error("failed to persist events: {0}")]
    Persist(EventStoreError),

    /// Failed to project events to the read model
    ///
    /// Events were persisted successfully, but projection failed.
    /// The event store is the source of truth; retry the projection.
    #[error("failed to project events: {0}")]
    Projection(ProjectionError),

    /// Failed to broadcast events to the event bus
    ///
    /// Events were persisted and projected, but broadcasting failed.
    /// This may require manual intervention or retry.
    #[error("failed to broadcast events: {0}")]
    Broadcast(EventBusError),

    /// Serialization or deserialization error
    ///
    /// This indicates a bug in the serialization code or schema mismatch.
    #[error("serialization error: {0}")]
    Serialization(SerializationError),

    /// Failed to fetch query data from projections
    ///
    /// This occurs when the `QueryFetcher` fails to retrieve data from
    /// the projection database before calling business logic.
    #[error("failed to fetch query data: {0}")]
    QueryFetch(String),

    /// Saga exceeded maximum iterations
    ///
    /// This occurs when a saga's `Continue` loop exceeds the configured
    /// maximum iterations. This is a safety guard against infinite loops
    /// in saga logic.
    ///
    /// If this error occurs, check the saga's business logic to ensure
    /// it eventually returns `Done`.
    #[error("saga exceeded maximum iterations ({max_iterations})")]
    SagaIterationsExceeded {
        /// The maximum iterations that were allowed
        max_iterations: u32,
    },

    /// Durable saga returned `Done` while calls were still outstanding
    ///
    /// Nothing was persisted: persisting `Done` would strand the
    /// outstanding journal entries and make recovery re-drive a saga that
    /// believes it is finished. A durable saga must consume every
    /// dispatched call's completion before returning `Done`.
    #[error("saga returned Done with {outstanding} outstanding call(s)")]
    DoneWithOutstandingCalls {
        /// Number of dispatched-but-uncompleted calls at the time of the error
        outstanding: usize,
    },

    /// Durable saga can never be woken again
    ///
    /// The saga returned `Continue` with no new calls while no calls were
    /// outstanding — no completion will ever arrive, so the loop would
    /// wait forever. Nothing was persisted. A saga that wants to park at a
    /// human-review gate must return `Done` instead.
    #[error("saga returned Continue with no calls and none outstanding; it can never be woken")]
    SagaStuck,

    /// Durable saga returned `Respond` in a feedback cycle
    ///
    /// A `Respond` persists nothing, so the completion that triggered the
    /// cycle could not be journaled and the call would silently re-run
    /// after a restart. Queries are only legal as the *initial* input of a
    /// durable run.
    #[error("saga returned Respond in a feedback cycle; the completion cannot be journaled")]
    RespondInFeedbackCycle,

    /// A durable saga call exceeded the configured watchdog duration
    ///
    /// The oldest in-flight call ran longer than the handler's
    /// `max_call_duration`. The run ended crash-equivalently: nothing was
    /// persisted at timeout, every in-flight call (the stuck one AND its
    /// siblings) stays outstanding in the journal, and `resume`
    /// re-dispatches them. This error is the alerting hook for hung calls.
    #[error("saga call {call_id} exceeded max_call_duration ({max_call_duration:?})")]
    CallStuck {
        /// The oldest in-flight call (the one that tripped the watchdog)
        call_id: crate::CallId,
        /// The configured watchdog duration
        max_call_duration: std::time::Duration,
    },

    /// Durable saga exceeded the maximum total dispatched calls
    ///
    /// This is durable mode's runaway guard (the analogue of
    /// [`SagaIterationsExceeded`](Self::SagaIterationsExceeded)): it counts
    /// **dispatched calls over the saga instance's lifetime** (resume seeds
    /// the counter from the journal), because completion cycles are bounded
    /// by dispatches and dispatches are the variable that can run away.
    #[error("saga exceeded maximum total calls ({max_total_calls})")]
    TotalCallsExceeded {
        /// The configured cap on lifetime dispatched calls
        max_total_calls: u32,
    },
}

impl<E: std::error::Error> HandlerError<E> {
    /// Check if this is a business logic error
    #[must_use]
    pub const fn is_business(&self) -> bool {
        matches!(self, Self::Business(_))
    }

    /// Check if this is an infrastructure error
    #[must_use]
    pub const fn is_infrastructure(&self) -> bool {
        !self.is_business()
    }

    /// Check if this is a version conflict (retryable)
    #[must_use]
    pub const fn is_version_conflict(&self) -> bool {
        matches!(self, Self::Persist(EventStoreError::VersionConflict { .. }))
    }
}

/// Serialization errors for event encoding/decoding
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum SerializationError {
    /// Failed to encode an event
    #[error("failed to encode event: {0}")]
    Encode(String),

    /// Failed to decode an event
    #[error("failed to decode event: {0}")]
    Decode(String),

    /// Unknown event type during deserialization
    #[error("unknown event type: {0}")]
    UnknownEventType(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("test error")]
    struct TestError;

    #[test]
    fn handler_error_is_business() {
        let err: HandlerError<TestError> = HandlerError::Business(TestError);
        assert!(err.is_business());
        assert!(!err.is_infrastructure());
    }

    #[test]
    fn handler_error_is_infrastructure() {
        let err: HandlerError<TestError> =
            HandlerError::Load(EventStoreError::Connection("test".into()));
        assert!(!err.is_business());
        assert!(err.is_infrastructure());
    }

    #[test]
    fn handler_error_is_version_conflict() {
        let err: HandlerError<TestError> =
            HandlerError::Persist(EventStoreError::VersionConflict {
                expected: Some(crate::Version::new(5)),
                actual: crate::Version::new(6),
            });
        assert!(err.is_version_conflict());

        let err2: HandlerError<TestError> =
            HandlerError::Persist(EventStoreError::Connection("test".into()));
        assert!(!err2.is_version_conflict());
    }
}
