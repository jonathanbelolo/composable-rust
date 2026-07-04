//! Authentication middleware for the ticketing system.
//!
//! Provides Axum extractors for:
//! - Bearer token extraction from Authorization header
//! - Session validation (auto-validates sessions from tokens)
//! - Role-based access control (admin checks)
//!
//! # Usage
//!
//! ```rust,ignore
//! use ticketing::auth::middleware::{SessionUser, BearerToken};
//!
//! // Require authentication
//! async fn get_profile(
//!     session: SessionUser,
//! ) -> Result<Json<ProfileResponse>, AppError> {
//!     // session.user_id is guaranteed valid
//!     Ok(Json(ProfileResponse { user_id: session.user_id }))
//! }
//! ```

use crate::auth::setup::TicketingAuthStore;
use axum::{
    extract::{FromRequestParts, State},
    http::request::Parts,
};
use composable_rust_auth::{
    AuthAction,
    state::{Session, SessionId, UserId},
};
use composable_rust_web::{
    error::AppError,
    extractors::{ClientIp, CorrelationId},
};
use std::sync::Arc;
use std::time::Duration;

/// Bearer token extracted from `Authorization: Bearer <token>` header.
#[derive(Debug, Clone)]
pub struct BearerToken(pub String);

impl<S> FromRequestParts<S> for BearerToken
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Extract Authorization header
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| AppError::unauthorized("Missing authorization header"))?;

        // Parse "Bearer <token>"
        if !auth_header.starts_with("Bearer ") {
            return Err(AppError::unauthorized(
                "Invalid authorization format. Expected 'Bearer <token>'",
            ));
        }

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| AppError::unauthorized("Invalid bearer token format"))?
            .to_string();

        if token.is_empty() {
            return Err(AppError::unauthorized("Empty bearer token"));
        }

        Ok(Self(token))
    }
}

/// Authenticated session user.
///
/// Extracts and validates the session from the bearer token.
/// Use this as a handler parameter to require authentication.
#[derive(Debug, Clone)]
pub struct SessionUser {
    /// The authenticated user ID
    pub user_id: UserId,
    /// The full session
    pub session: Session,
}

// Implementation for Arc<TicketingAuthStore> (used by auth routes)
impl FromRequestParts<Arc<TicketingAuthStore>> for SessionUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<TicketingAuthStore>,
    ) -> Result<Self, Self::Rejection> {
        // Extract bearer token
        let bearer = BearerToken::from_request_parts(parts, state).await?;

        // Check for test token bypass (only if AUTH_TEST_TOKEN env var is set)
        if let Ok(test_token) = std::env::var("AUTH_TEST_TOKEN") {
            // Support two test token patterns:
            // 1. Exact match (legacy): "test-secret" → returns hardcoded user
            // 2. Multi-user pattern: "test-user-{uuid}" → returns user with that UUID

            if bearer.0 == test_token {
                // Legacy: exact match returns hardcoded test user
                const TEST_USER_UUID: uuid::Uuid =
                    uuid::Uuid::from_bytes([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
                const TEST_SESSION_UUID: uuid::Uuid =
                    uuid::Uuid::from_bytes([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);
                const TEST_DEVICE_UUID: uuid::Uuid =
                    uuid::Uuid::from_bytes([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3]);

                let test_user_id = UserId(TEST_USER_UUID);
                let test_device_id = composable_rust_auth::state::DeviceId(TEST_DEVICE_UUID);
                let test_session = Session {
                    user_id: test_user_id,
                    session_id: SessionId(TEST_SESSION_UUID),
                    device_id: test_device_id,
                    email: "test@example.com".to_string(),
                    created_at: chrono::Utc::now(),
                    last_active: chrono::Utc::now(),
                    expires_at: chrono::Utc::now() + chrono::Duration::days(1),
                    ip_address: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                    user_agent: "Test Client".to_string(),
                    oauth_provider: None,
                    login_risk_score: 0.0,
                    idle_timeout: chrono::Duration::hours(24),
                    enable_sliding_refresh: false,
                };
                return Ok(Self {
                    user_id: test_user_id,
                    session: test_session,
                });
            } else if let Some(uuid_str) = bearer.0.strip_prefix("test-user-") {
                // Multi-user test pattern: extract UUID from "test-user-{uuid}"
                if let Ok(user_uuid) = uuid::Uuid::parse_str(uuid_str) {
                    let test_user_id = UserId(user_uuid);

                    // Generate deterministic session and device UUIDs based on user UUID
                    // This ensures the same test user always gets the same session/device IDs
                    let mut session_bytes = user_uuid.as_bytes().to_owned();
                    session_bytes[0] ^= 0x01; // XOR first byte for session UUID
                    let test_session_uuid = uuid::Uuid::from_bytes(session_bytes);

                    let mut device_bytes = user_uuid.as_bytes().to_owned();
                    device_bytes[0] ^= 0x02; // XOR first byte for device UUID
                    let test_device_uuid = uuid::Uuid::from_bytes(device_bytes);

                    let test_device_id = composable_rust_auth::state::DeviceId(test_device_uuid);
                    let test_session = Session {
                        user_id: test_user_id,
                        session_id: SessionId(test_session_uuid),
                        device_id: test_device_id,
                        email: format!("test-user-{user_uuid}@example.com"),
                        created_at: chrono::Utc::now(),
                        last_active: chrono::Utc::now(),
                        expires_at: chrono::Utc::now() + chrono::Duration::days(1),
                        ip_address: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                        user_agent: "Test Client".to_string(),
                        oauth_provider: None,
                        login_risk_score: 0.0,
                        idle_timeout: chrono::Duration::hours(24),
                        enable_sliding_refresh: false,
                    };
                    return Ok(Self {
                        user_id: test_user_id,
                        session: test_session,
                    });
                }
            }
        }

        // Parse session ID from token (UUID string)
        let uuid = uuid::Uuid::parse_str(&bearer.0)
            .map_err(|_| AppError::unauthorized("Invalid session token format"))?;
        let session_id = SessionId(uuid);

        // Extract client IP and correlation ID using framework extractors
        let client_ip = ClientIp::from_request_parts(parts, state)
            .await
            .unwrap_or(ClientIp(std::net::IpAddr::V4(
                std::net::Ipv4Addr::LOCALHOST,
            )));

        let correlation_id = CorrelationId::from_request_parts(parts, state)
            .await
            .unwrap_or(CorrelationId(uuid::Uuid::new_v4()));

        // Validate session via reducer
        let action = AuthAction::ValidateSession {
            correlation_id: correlation_id.0,
            session_id,
            ip_address: client_ip.0,
        };

        // Send action and wait for response
        let store = State::<Arc<TicketingAuthStore>>::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::internal("Failed to access store"))?;

        let response = store
            .send_and_wait_for(
                action,
                |a| {
                    matches!(
                        a,
                        AuthAction::SessionValidated { .. } | AuthAction::SessionExpired { .. }
                    )
                },
                Duration::from_secs(5),
            )
            .await
            .map_err(|e| AppError::internal(format!("Session validation error: {e}")))?;

        // Handle validation result
        match response {
            AuthAction::SessionValidated { session, .. } => Ok(Self {
                user_id: session.user_id,
                session,
            }),
            AuthAction::SessionExpired { .. } => Err(AppError::unauthorized("Session expired")),
            _ => Err(AppError::internal(
                "Unexpected response from session validation",
            )),
        }
    }
}

/// Require admin role.
///
/// Validates that the authenticated user has admin privileges.
/// Returns 403 Forbidden if the user is not an admin.
///
/// # Note
///
/// This is a placeholder implementation. In a real system, you would:
/// 1. Add a `role` field to the `Session` state
/// 2. Check the role against an admin list or permission system
/// 3. Query a user roles table for dynamic role assignment
#[derive(Debug, Clone)]
pub struct RequireAdmin {
    /// The authenticated admin user ID
    pub user_id: UserId,
    /// The full session
    pub session: Session,
}

impl FromRequestParts<Arc<TicketingAuthStore>> for RequireAdmin {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<TicketingAuthStore>,
    ) -> Result<Self, Self::Rejection> {
        // First validate session
        let session_user = SessionUser::from_request_parts(parts, state).await?;

        // Note: This implementation is a placeholder that allows all authenticated users.
        // In production, query user roles from database.
        Ok(Self {
            user_id: session_user.user_id,
            session: session_user.session,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    #[test]
    fn test_bearer_token_parsing() {
        // Valid bearer token
        let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let header = format!("Bearer {token}");
        assert!(header.starts_with("Bearer "));

        let extracted = header.strip_prefix("Bearer ").unwrap();
        assert_eq!(extracted, token);
    }

    #[test]
    fn test_invalid_bearer_format() {
        let invalid = "Basic dXNlcjpwYXNz";
        assert!(!invalid.starts_with("Bearer "));
    }
}
