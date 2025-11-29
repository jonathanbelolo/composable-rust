//! Error types for the handler and serialization

use crate::{EventBusError, EventStoreError};

/// Errors that can occur during projection
///
/// Projections update read models (query-side databases) from events.
/// These errors indicate issues with that process.
#[derive(Debug, Clone, thiserror::Error)]
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
        matches!(
            self,
            Self::Persist(EventStoreError::VersionConflict { .. })
        )
    }
}

/// Serialization errors for event encoding/decoding
#[derive(Debug, Clone, thiserror::Error)]
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
