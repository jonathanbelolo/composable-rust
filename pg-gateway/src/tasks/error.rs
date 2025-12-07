//! Task error types.
//!
//! This module defines error types for background task execution,
//! distinguishing between temporary (retryable) and permanent failures.

use std::fmt;

/// Error type for task execution.
///
/// Tasks can fail in two ways:
/// - **Temporary**: The failure is transient and the task should be retried
///   (e.g., network timeout, service unavailable)
/// - **Permanent**: The failure is permanent and retrying won't help
///   (e.g., invalid payload, rejected by service)
///
/// # Example
///
/// ```rust
/// use composable_rust_pg_gateway::tasks::TaskError;
///
/// fn send_email(to: &str) -> Result<(), TaskError> {
///     if to.is_empty() {
///         // Invalid input - don't retry
///         return Err(TaskError::Permanent("Empty recipient".to_string()));
///     }
///
///     // Simulate network error - should retry
///     Err(TaskError::Temporary("Connection timeout".to_string()))
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskError {
    /// Temporary failure - task will be retried with exponential backoff.
    ///
    /// Use this for transient errors like:
    /// - Network timeouts
    /// - Service temporarily unavailable (503)
    /// - Rate limiting (429)
    /// - Database connection issues
    Temporary(String),

    /// Permanent failure - task will be moved to dead letter queue.
    ///
    /// Use this for errors that won't be fixed by retrying:
    /// - Invalid payload/configuration
    /// - Authentication failures (401)
    /// - Bad request (400)
    /// - Resource not found (404)
    Permanent(String),
}

impl TaskError {
    /// Create a temporary error.
    #[must_use]
    pub fn temporary(message: impl Into<String>) -> Self {
        Self::Temporary(message.into())
    }

    /// Create a permanent error.
    #[must_use]
    pub fn permanent(message: impl Into<String>) -> Self {
        Self::Permanent(message.into())
    }

    /// Check if this error is temporary (retryable).
    #[must_use]
    pub const fn is_temporary(&self) -> bool {
        matches!(self, Self::Temporary(_))
    }

    /// Check if this error is permanent (not retryable).
    #[must_use]
    pub const fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent(_))
    }

    /// Get the error message.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Temporary(msg) | Self::Permanent(msg) => msg,
        }
    }
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Temporary(msg) => write!(f, "Temporary failure (will retry): {msg}"),
            Self::Permanent(msg) => write!(f, "Permanent failure (no retry): {msg}"),
        }
    }
}

impl std::error::Error for TaskError {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn temporary_error() {
        let err = TaskError::temporary("connection timeout");
        assert!(err.is_temporary());
        assert!(!err.is_permanent());
        assert_eq!(err.message(), "connection timeout");
    }

    #[test]
    fn permanent_error() {
        let err = TaskError::permanent("invalid payload");
        assert!(err.is_permanent());
        assert!(!err.is_temporary());
        assert_eq!(err.message(), "invalid payload");
    }

    #[test]
    fn display_temporary() {
        let err = TaskError::Temporary("timeout".to_string());
        assert_eq!(
            format!("{err}"),
            "Temporary failure (will retry): timeout"
        );
    }

    #[test]
    fn display_permanent() {
        let err = TaskError::Permanent("bad request".to_string());
        assert_eq!(
            format!("{err}"),
            "Permanent failure (no retry): bad request"
        );
    }

    #[test]
    fn clone_and_eq() {
        let err1 = TaskError::temporary("test");
        let err2 = err1.clone();
        assert_eq!(err1, err2);
    }
}
