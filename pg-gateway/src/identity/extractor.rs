//! Axum extractors for identity.
//!
//! This module provides Axum extractors to authenticate requests and extract
//! user identity from JWT tokens or session cookies.
//!
//! # Authentication Flow
//!
//! ```text
//! Request arrives
//!     │
//!     ▼
//! Check Authorization header for "Bearer <token>"
//!     │
//!     ├─── Found: Validate JWT → Identity
//!     │
//!     ▼
//! Check session cookie
//!     │
//!     ├─── Found: Validate session in DB → Identity
//!     │
//!     ▼
//! No credentials: Return 401 Unauthorized
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use composable_rust_pg_gateway::identity::Identity;
//! use axum::{routing::get, Router};
//!
//! async fn handler(identity: Identity) -> String {
//!     format!("Hello, user {}", identity.user_id)
//! }
//!
//! let app = Router::new()
//!     .route("/protected", get(handler));
//! ```

use super::jwt::{JwtError, JwtValidator};
use super::types::Identity;
use crate::ApiError;
use axum::extract::FromRef;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use sqlx::PgPool;
use std::sync::Arc;

/// Application state required for identity extraction.
///
/// Your application state must implement this trait (or contain a type that does)
/// for the [`Identity`] extractor to work.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::identity::{IdentityState, JwtValidator, JwtConfig};
/// use sqlx::PgPool;
/// use std::sync::Arc;
///
/// struct AppState {
///     pool: PgPool,
///     jwt: Arc<JwtValidator>,
/// }
///
/// impl IdentityState for AppState {
///     fn jwt_validator(&self) -> &JwtValidator {
///         &self.jwt
///     }
///
///     fn pool(&self) -> &PgPool {
///         &self.pool
///     }
///
///     fn session_cookie_name(&self) -> &str {
///         "session"
///     }
/// }
/// ```
pub trait IdentityState: Send + Sync {
    /// Get the JWT validator for token validation.
    fn jwt_validator(&self) -> &JwtValidator;

    /// Get the database pool for session validation.
    fn pool(&self) -> &PgPool;

    /// Get the session cookie name.
    ///
    /// Default: `"session"`
    #[allow(clippy::unnecessary_literal_bound)]
    fn session_cookie_name(&self) -> &str {
        "session"
    }
}

/// Configuration for identity extraction.
///
/// This struct holds the components needed to extract identity from requests.
/// It can be used directly as application state or embedded in your own state.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::identity::{IdentityConfig, JwtConfig};
/// use sqlx::PgPool;
///
/// let jwt_config = JwtConfig::from_env()?;
/// let identity_config = IdentityConfig::new(pool.clone(), jwt_config);
///
/// // Use as state
/// let app = Router::new()
///     .route("/protected", get(handler))
///     .with_state(identity_config);
/// ```
#[derive(Clone)]
pub struct IdentityConfig {
    pool: PgPool,
    jwt_validator: Arc<JwtValidator>,
    session_cookie_name: String,
}

impl IdentityConfig {
    /// Create a new identity configuration.
    #[must_use]
    pub fn new(pool: PgPool, jwt_config: super::jwt::JwtConfig) -> Self {
        Self {
            pool,
            jwt_validator: Arc::new(JwtValidator::new(jwt_config)),
            session_cookie_name: "session".to_string(),
        }
    }

    /// Set a custom session cookie name.
    #[must_use]
    pub fn with_session_cookie_name(mut self, name: impl Into<String>) -> Self {
        self.session_cookie_name = name.into();
        self
    }
}

impl IdentityState for IdentityConfig {
    fn jwt_validator(&self) -> &JwtValidator {
        &self.jwt_validator
    }

    fn pool(&self) -> &PgPool {
        &self.pool
    }

    fn session_cookie_name(&self) -> &str {
        &self.session_cookie_name
    }
}

impl std::fmt::Debug for IdentityConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityConfig")
            .field("session_cookie_name", &self.session_cookie_name)
            .field("pool", &"[PgPool]")
            .field("jwt_validator", &"[JwtValidator]")
            .finish()
    }
}

/// Axum extractor for [`Identity`].
///
/// Extracts user identity from the request by checking:
/// 1. `Authorization: Bearer <token>` header (JWT)
/// 2. Session cookie (if JWT not present)
///
/// Returns `401 Unauthorized` if neither is present or valid.
///
/// # Requirements
///
/// Your application state must implement [`IdentityState`] or contain an
/// [`IdentityConfig`] that can be extracted via [`FromRef`].
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::identity::Identity;
/// use axum::{routing::get, Router, Json};
///
/// async fn get_profile(identity: Identity) -> Json<Profile> {
///     // identity is guaranteed to be authenticated
///     let profile = fetch_profile(&identity.user_id).await;
///     Json(profile)
/// }
///
/// let app = Router::new()
///     .route("/profile", get(get_profile))
///     .with_state(identity_config);
/// ```
impl<S> axum::extract::FromRequestParts<S> for Identity
where
    S: Send + Sync,
    IdentityConfig: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let config = IdentityConfig::from_ref(state);

        // Try JWT from Authorization header first
        if let Some(auth_header) = parts.headers.get(AUTHORIZATION) {
            if let Ok(header_value) = auth_header.to_str() {
                if let Some(token) = header_value.strip_prefix("Bearer ") {
                    return validate_jwt(token, config.jwt_validator());
                }
            }
        }

        // Try session cookie
        if let Some(cookie_header) = parts.headers.get(axum::http::header::COOKIE) {
            if let Ok(cookies) = cookie_header.to_str() {
                if let Some(session_id) = extract_cookie(cookies, config.session_cookie_name()) {
                    return validate_session(session_id, config.pool()).await;
                }
            }
        }

        // No valid credentials found
        Err(ApiError::Unauthorized)
    }
}

/// Validate a JWT token and return identity.
fn validate_jwt(token: &str, validator: &JwtValidator) -> Result<Identity, ApiError> {
    validator.validate(token).map_err(|e| {
        tracing::debug!(error = %e, "JWT validation failed");
        match e {
            JwtError::Expired
            | JwtError::InvalidSignature
            | JwtError::InvalidFormat(_)
            | JwtError::InvalidClaims(_) => ApiError::Unauthorized,
            JwtError::Configuration(_) => ApiError::Internal,
        }
    })
}

/// Validate a session ID against the database.
async fn validate_session(session_id: &str, pool: &PgPool) -> Result<Identity, ApiError> {
    // Query the auth.sessions table
    let session: Option<SessionRow> = sqlx::query_as(
        r"
        SELECT user_id, tenant_id, roles
        FROM auth.sessions
        WHERE session_id = $1 AND expires_at > now()
        ",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Session validation query failed");
        ApiError::Internal
    })?;

    if let Some(row) = session {
        tracing::trace!(user_id = %row.user_id, "Session validated");
        Ok(Identity {
            user_id: row.user_id,
            tenant_id: row.tenant_id,
            roles: row.roles,
            session_id: Some(session_id.to_string()),
        })
    } else {
        tracing::debug!("Session not found or expired");
        Err(ApiError::Unauthorized)
    }
}

/// Session row from auth.sessions table.
#[derive(Debug, sqlx::FromRow)]
struct SessionRow {
    user_id: String,
    tenant_id: String,
    roles: Vec<String>,
}

/// Extract a cookie value from a cookie header string.
fn extract_cookie<'a>(cookies: &'a str, name: &str) -> Option<&'a str> {
    for cookie in cookies.split(';') {
        let cookie = cookie.trim();
        if let Some(value) = cookie.strip_prefix(name) {
            if let Some(value) = value.strip_prefix('=') {
                return Some(value);
            }
        }
    }
    None
}

/// Optional identity extractor.
///
/// Similar to [`Identity`], but returns `None` instead of `401 Unauthorized`
/// when no valid credentials are present.
///
/// Useful for endpoints that work differently for authenticated vs anonymous users.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::identity::OptionalIdentity;
///
/// async fn get_content(OptionalIdentity(identity): OptionalIdentity) -> String {
///     match identity {
///         Some(id) => format!("Hello, {}", id.user_id),
///         None => "Hello, anonymous".to_string(),
///     }
/// }
/// ```
#[derive(Debug, Clone)]
pub struct OptionalIdentity(pub Option<Identity>);

impl<S> axum::extract::FromRequestParts<S> for OptionalIdentity
where
    S: Send + Sync,
    IdentityConfig: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let config = IdentityConfig::from_ref(state);

        // Try JWT from Authorization header first
        if let Some(auth_header) = parts.headers.get(AUTHORIZATION) {
            if let Ok(header_value) = auth_header.to_str() {
                if let Some(token) = header_value.strip_prefix("Bearer ") {
                    match validate_jwt(token, config.jwt_validator()) {
                        Ok(identity) => return Ok(Self(Some(identity))),
                        Err(ApiError::Internal) => return Err(ApiError::Internal),
                        Err(_) => {} // Continue to try session cookie
                    }
                }
            }
        }

        // Try session cookie
        if let Some(cookie_header) = parts.headers.get(axum::http::header::COOKIE) {
            if let Ok(cookies) = cookie_header.to_str() {
                if let Some(session_id) = extract_cookie(cookies, config.session_cookie_name()) {
                    match validate_session(session_id, config.pool()).await {
                        Ok(identity) => return Ok(Self(Some(identity))),
                        Err(ApiError::Internal) => return Err(ApiError::Internal),
                        Err(_) => {} // No valid session
                    }
                }
            }
        }

        // No credentials - that's OK for optional
        Ok(Self(None))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn extract_cookie_single() {
        let cookies = "session=abc123";
        assert_eq!(extract_cookie(cookies, "session"), Some("abc123"));
    }

    #[test]
    fn extract_cookie_multiple() {
        let cookies = "foo=bar; session=abc123; baz=qux";
        assert_eq!(extract_cookie(cookies, "session"), Some("abc123"));
    }

    #[test]
    fn extract_cookie_not_found() {
        let cookies = "foo=bar; baz=qux";
        assert_eq!(extract_cookie(cookies, "session"), None);
    }

    #[test]
    fn extract_cookie_similar_name() {
        let cookies = "my_session=wrong; session=correct";
        assert_eq!(extract_cookie(cookies, "session"), Some("correct"));
    }

    #[test]
    fn extract_cookie_empty() {
        assert_eq!(extract_cookie("", "session"), None);
    }

    #[tokio::test]
    async fn identity_config_debug_redacts() {
        let config = IdentityConfig {
            pool: sqlx::PgPool::connect_lazy("postgres://localhost/test").unwrap(),
            jwt_validator: Arc::new(JwtValidator::new(super::super::jwt::JwtConfig::with_secret(
                "secret",
            ))),
            session_cookie_name: "session".to_string(),
        };

        let debug = format!("{config:?}");
        assert!(debug.contains("[PgPool]"));
        assert!(debug.contains("[JwtValidator]"));
    }
}
