//! API error types with `PostgreSQL` error code mapping.
//!
//! This module provides [`ApiError`] which automatically maps `PostgreSQL`
//! error codes to appropriate HTTP status codes.
//!
//! # `PostgreSQL` Error Code Conventions
//!
//! The thin server pattern uses custom `PostgreSQL` error codes for HTTP semantics:
//!
//! | Code | HTTP | Meaning |
//! |------|------|---------|
//! | P0001 | 400 | Bad Request (generic validation error) |
//! | P0401 | 401 | Unauthorized (not authenticated) |
//! | P0403 | 403 | Forbidden (authenticated but not allowed) |
//! | P0404 | 404 | Not Found |
//! | P0409 | 409 | Conflict (business rule violation) |
//! | 23505 | 409 | Unique violation (duplicate key) |
//! | 23503 | 400 | Foreign key violation |
//!
//! # Example: `PostgreSQL` Function
//!
//! ```sql
//! CREATE FUNCTION api.submit_order(p_order_id uuid, p_items jsonb)
//! RETURNS jsonb AS $$
//! BEGIN
//!     -- Validate order exists
//!     IF NOT EXISTS (SELECT 1 FROM orders WHERE id = p_order_id) THEN
//!         RAISE EXCEPTION 'Order not found'
//!             USING ERRCODE = 'P0404';
//!     END IF;
//!
//!     -- Check authorization
//!     IF NOT has_permission('submit_order') THEN
//!         RAISE EXCEPTION 'Not authorized to submit orders'
//!             USING ERRCODE = 'P0403';
//!     END IF;
//!
//!     -- Business logic...
//! END;
//! $$ LANGUAGE plpgsql;
//! ```
//!
//! # Example: Rust Handler
//!
//! ```rust,ignore
//! use composable_rust_pg_gateway::ApiError;
//!
//! async fn submit_order(
//!     pool: &PgPool,
//!     req: SubmitOrderRequest,
//! ) -> Result<Json<OrderResult>, ApiError> {
//!     let result = sqlx::query_as("SELECT * FROM api.submit_order($1, $2)")
//!         .bind(&req.order_id)
//!         .bind(&req.items)
//!         .fetch_one(pool)
//!         .await?;  // ApiError automatically maps PostgreSQL errors
//!
//!     Ok(Json(result))
//! }
//! ```

use serde::Serialize;

/// API error with automatic `PostgreSQL` error code mapping.
///
/// This error type is designed for the thin server pattern where `PostgreSQL`
/// functions raise exceptions with specific error codes to indicate HTTP
/// status codes.
///
/// # Automatic Conversion
///
/// Implements `From<sqlx::Error>` to automatically map `PostgreSQL` error codes
/// to the appropriate HTTP status. Unknown errors become 500 Internal Server Error.
#[derive(Debug, Clone)]
pub enum ApiError {
    /// 400 Bad Request - Validation error or invalid input.
    ///
    /// Mapped from `PostgreSQL` codes: P0001, 23503 (FK violation)
    BadRequest(String),

    /// 401 Unauthorized - Authentication required.
    ///
    /// Mapped from `PostgreSQL` code: P0401
    Unauthorized,

    /// 403 Forbidden - Authenticated but not authorized.
    ///
    /// Mapped from `PostgreSQL` code: P0403
    Forbidden(String),

    /// 404 Not Found - Resource does not exist.
    ///
    /// Mapped from `PostgreSQL` code: P0404
    NotFound,

    /// 409 Conflict - Business rule violation or duplicate.
    ///
    /// Mapped from `PostgreSQL` codes: P0409, 23505 (unique violation)
    Conflict(String),

    /// 500 Internal Server Error - Unexpected error.
    ///
    /// Used for database connection errors, unknown `PostgreSQL` errors, etc.
    Internal,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadRequest(msg) => write!(f, "Bad Request: {msg}"),
            Self::Unauthorized => write!(f, "Unauthorized"),
            Self::Forbidden(msg) => write!(f, "Forbidden: {msg}"),
            Self::NotFound => write!(f, "Not Found"),
            Self::Conflict(msg) => write!(f, "Conflict: {msg}"),
            Self::Internal => write!(f, "Internal Server Error"),
        }
    }
}

impl std::error::Error for ApiError {}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        match &err {
            sqlx::Error::Database(db_err) => {
                let code = db_err.code().map(std::borrow::Cow::into_owned);
                let message = db_err.message().to_string();

                match code.as_deref() {
                    // Custom application codes (P0xxx pattern)
                    Some("P0401") => Self::Unauthorized,
                    Some("P0403") => Self::Forbidden(message),
                    Some("P0404") => Self::NotFound,

                    // Conflict errors (combined)
                    Some("P0409" | "23505") => Self::Conflict(message),

                    // Bad request errors (combined)
                    Some("P0001" | "23503" | "23502" | "23514") => Self::BadRequest(message),

                    // Unknown database error - log and return internal
                    _ => {
                        tracing::error!(
                            error = %err,
                            code = ?code,
                            "Unhandled database error"
                        );
                        Self::Internal
                    }
                }
            }
            sqlx::Error::RowNotFound => Self::NotFound,
            _ => {
                tracing::error!(error = %err, "Database error");
                Self::Internal
            }
        }
    }
}

/// JSON error response body.
#[derive(Debug, Clone, Serialize)]
struct ErrorResponse {
    error: ErrorBody,
}

#[derive(Debug, Clone, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

#[cfg(feature = "http")]
mod http_impl {
    use super::{ApiError, ErrorBody, ErrorResponse};
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Response};
    use axum::Json;

    impl IntoResponse for ApiError {
        fn into_response(self) -> Response {
            let (status, code, message) = match &self {
                ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "BAD_REQUEST", msg.clone()),
                ApiError::Unauthorized => {
                    (StatusCode::UNAUTHORIZED, "UNAUTHORIZED", String::new())
                }
                ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, "FORBIDDEN", msg.clone()),
                ApiError::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND", String::new()),
                ApiError::Conflict(msg) => (StatusCode::CONFLICT, "CONFLICT", msg.clone()),
                ApiError::Internal => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_ERROR",
                    String::new(),
                ),
            };

            let body = ErrorResponse {
                error: ErrorBody { code, message },
            };

            (status, Json(body)).into_response()
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::ApiError;

    #[test]
    fn display_bad_request() {
        let err = ApiError::BadRequest("Invalid input".to_string());
        assert_eq!(err.to_string(), "Bad Request: Invalid input");
    }

    #[test]
    fn display_unauthorized() {
        let err = ApiError::Unauthorized;
        assert_eq!(err.to_string(), "Unauthorized");
    }

    #[test]
    fn display_forbidden() {
        let err = ApiError::Forbidden("Access denied".to_string());
        assert_eq!(err.to_string(), "Forbidden: Access denied");
    }

    #[test]
    fn display_not_found() {
        let err = ApiError::NotFound;
        assert_eq!(err.to_string(), "Not Found");
    }

    #[test]
    fn display_conflict() {
        let err = ApiError::Conflict("Already exists".to_string());
        assert_eq!(err.to_string(), "Conflict: Already exists");
    }

    #[test]
    fn display_internal() {
        let err = ApiError::Internal;
        assert_eq!(err.to_string(), "Internal Server Error");
    }
}
