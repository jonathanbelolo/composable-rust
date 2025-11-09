//! Error types for web handlers.
//!
//! This module defines error types that bridge between domain errors
//! and HTTP responses, implementing Axum's `IntoResponse` trait.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::fmt;

/// Application error type for web handlers.
///
/// This type wraps domain errors and provides HTTP-friendly error responses.
/// It implements Axum's `IntoResponse` trait to automatically convert errors
/// into HTTP responses.
///
/// # Examples
///
/// ```ignore
/// async fn handler() -> Result<Json<Data>, AppError> {
///     let user = find_user(id).await
///         .map_err(|e| AppError::not_found("User", id))?;
///     Ok(Json(user))
/// }
/// ```
#[derive(Debug)]
pub struct AppError {
    /// HTTP status code
    status: StatusCode,
    /// Error message (user-facing)
    message: String,
    /// Error code (for client error handling)
    code: String,
    /// Internal error (for logging, not exposed to client)
    #[allow(dead_code)]
    source: Option<anyhow::Error>,
}

impl AppError {
    /// Create a new application error.
    #[must_use]
    pub const fn new(status: StatusCode, message: String, code: String) -> Self {
        Self {
            status,
            message,
            code,
            source: None,
        }
    }

    /// Create a new error with a source error.
    #[must_use]
    pub fn with_source(mut self, source: anyhow::Error) -> Self {
        self.source = Some(source);
        self
    }

    /// Create a 400 Bad Request error.
    #[must_use]
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            message.into(),
            "BAD_REQUEST".to_string(),
        )
    }

    /// Create a 401 Unauthorized error.
    #[must_use]
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            message.into(),
            "UNAUTHORIZED".to_string(),
        )
    }

    /// Create a 403 Forbidden error.
    #[must_use]
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            message.into(),
            "FORBIDDEN".to_string(),
        )
    }

    /// Create a 404 Not Found error.
    #[must_use]
    pub fn not_found(resource: impl fmt::Display, id: impl fmt::Display) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            format!("{resource} with id {id} not found"),
            "NOT_FOUND".to_string(),
        )
    }

    /// Create a 409 Conflict error.
    #[must_use]
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            message.into(),
            "CONFLICT".to_string(),
        )
    }

    /// Create a 422 Unprocessable Entity error.
    #[must_use]
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            message.into(),
            "VALIDATION_ERROR".to_string(),
        )
    }

    /// Create a 408 Request Timeout error.
    #[must_use]
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::REQUEST_TIMEOUT,
            message.into(),
            "TIMEOUT".to_string(),
        )
    }

    /// Create a 500 Internal Server Error.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            message.into(),
            "INTERNAL_SERVER_ERROR".to_string(),
        )
    }

    /// Create a 503 Service Unavailable error.
    #[must_use]
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            message.into(),
            "SERVICE_UNAVAILABLE".to_string(),
        )
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

/// Error response body (JSON).
#[derive(Debug, Serialize)]
struct ErrorResponse {
    /// Error code (for client error handling).
    code: String,
    /// Human-readable error message.
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // Log internal errors
        if self.status.is_server_error() {
            if let Some(source) = &self.source {
                tracing::error!(
                    status = %self.status,
                    code = %self.code,
                    message = %self.message,
                    error = %source,
                    "Internal server error"
                );
            } else {
                tracing::error!(
                    status = %self.status,
                    code = %self.code,
                    message = %self.message,
                    "Internal server error"
                );
            }
        }

        let body = ErrorResponse {
            code: self.code,
            message: self.message,
        };

        (self.status, Json(body)).into_response()
    }
}

/// Convert `anyhow::Error` to `AppError`.
impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        Self::internal("An internal error occurred").with_source(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = AppError::bad_request("Invalid input");
        assert_eq!(err.to_string(), "[BAD_REQUEST] Invalid input");
    }

    #[test]
    fn test_not_found() {
        let err = AppError::not_found("User", "123");
        assert_eq!(err.to_string(), "[NOT_FOUND] User with id 123 not found");
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_validation() {
        let err = AppError::validation("Email is required");
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(err.code, "VALIDATION_ERROR");
    }

    #[test]
    fn test_timeout() {
        let err = AppError::timeout("Request timed out");
        assert_eq!(err.status, StatusCode::REQUEST_TIMEOUT);
        assert_eq!(err.code, "TIMEOUT");
    }
}
