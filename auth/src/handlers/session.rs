//! Session management handlers.
//!
//! Handles session validation, refresh, and logout.

use crate::{AuthAction, AuthEnvironment, AuthReducer, AuthState};
use crate::state::SessionId;
use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use composable_rust_runtime::Store;
use composable_rust_web::{AppError, ClientIp, CorrelationId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Request to validate/refresh a session.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GetSessionRequest {
    /// Session ID (bearer token).
    pub session_id: String,
}

/// Response with session info.
#[derive(Debug, Clone, Serialize)]
pub struct GetSessionResponse {
    /// Session ID.
    pub session_id: String,

    /// User's email.
    pub email: String,

    /// Session expiration timestamp (ISO 8601).
    pub expires_at: String,

    /// Last activity timestamp (ISO 8601).
    pub last_active: String,
}

/// Request to logout (destroy session).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogoutRequest {
    /// Session ID to destroy.
    pub session_id: String,
}

/// Response after successful logout.
#[derive(Debug, Clone, Serialize)]
pub struct LogoutResponse {
    /// Success message.
    pub message: String,
}

/// Get session info and refresh it.
///
/// # Endpoint
///
/// ```text
/// POST /api/v1/auth/session
/// Content-Type: application/json
///
/// {
///   "session_id": "uuid"
/// }
/// ```
///
/// # Response
///
/// ```json
/// {
///   "session_id": "uuid",
///   "email": "user@example.com",
///   "expires_at": "2024-01-01T00:00:00Z",
///   "last_active": "2024-01-01T00:00:00Z"
/// }
/// ```
pub async fn get_session<O, E, W, S, T, U, D, R, OT, C, RL>(
    State(store): State<Arc<Store<AuthState, AuthAction, AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>, AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>>>>,
    correlation_id: CorrelationId,
    client_ip: ClientIp,
    Json(request): Json<GetSessionRequest>,
) -> Result<(StatusCode, Json<GetSessionResponse>), AppError>
where
    O: crate::providers::OAuth2Provider + Clone + 'static,
    E: crate::providers::EmailProvider + Clone + 'static,
    W: crate::providers::WebAuthnProvider + Clone + 'static,
    S: crate::providers::SessionStore + Clone + 'static,
    T: crate::providers::TokenStore + Clone + 'static,
    U: crate::providers::UserRepository + Clone + 'static,
    D: crate::providers::DeviceRepository + Clone + 'static,
    R: crate::providers::RiskCalculator + Clone + 'static,
    OT: crate::providers::OAuthTokenStore + Clone + 'static,
    C: crate::providers::ChallengeStore + Clone + 'static,
    RL: crate::providers::RateLimiter + Clone + 'static,
{
    // Parse session ID
    let session_id = uuid::Uuid::parse_str(&request.session_id)
        .map_err(|_| AppError::bad_request("Invalid session ID format"))?;

    // Build action
    let action = AuthAction::ValidateSession {
        correlation_id: correlation_id.0,
        session_id: SessionId(session_id),
        ip_address: client_ip.0,
    };

    // Dispatch and wait for validation result
    let result = store
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
        .map_err(|_| AppError::timeout("Session validation timed out"))?;

    // Map result to HTTP response
    match result {
        AuthAction::SessionValidated { session, .. } => Ok((
            StatusCode::OK,
            Json(GetSessionResponse {
                session_id: session.session_id.0.to_string(),
                email: session.email.clone(),
                expires_at: session.expires_at.to_rfc3339(),
                last_active: session.last_active.to_rfc3339(),
            }),
        )),
        AuthAction::SessionExpired { .. } => Err(AppError::unauthorized("Session expired")),
        _ => Err(AppError::internal("Unexpected action received")),
    }
}

/// Logout (destroy session).
///
/// # Endpoint
///
/// ```text
/// POST /api/v1/auth/logout
/// Content-Type: application/json
///
/// {
///   "session_id": "uuid"
/// }
/// ```
///
/// # Response
///
/// ```json
/// {
///   "message": "Logged out successfully"
/// }
/// ```
pub async fn logout<O, E, W, S, T, U, D, R, OT, C, RL>(
    State(store): State<Arc<Store<AuthState, AuthAction, AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>, AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>>>>,
    correlation_id: CorrelationId,
    Json(request): Json<LogoutRequest>,
) -> Result<(StatusCode, Json<LogoutResponse>), AppError>
where
    O: crate::providers::OAuth2Provider + Clone + 'static,
    E: crate::providers::EmailProvider + Clone + 'static,
    W: crate::providers::WebAuthnProvider + Clone + 'static,
    S: crate::providers::SessionStore + Clone + 'static,
    T: crate::providers::TokenStore + Clone + 'static,
    U: crate::providers::UserRepository + Clone + 'static,
    D: crate::providers::DeviceRepository + Clone + 'static,
    R: crate::providers::RiskCalculator + Clone + 'static,
    OT: crate::providers::OAuthTokenStore + Clone + 'static,
    C: crate::providers::ChallengeStore + Clone + 'static,
    RL: crate::providers::RateLimiter + Clone + 'static,
{
    // Parse session ID
    let session_id = uuid::Uuid::parse_str(&request.session_id)
        .map_err(|_| AppError::bad_request("Invalid session ID format"))?;

    // Build action
    let action = AuthAction::Logout {
        correlation_id: correlation_id.0,
        session_id: SessionId(session_id),
    };

    // Dispatch and wait for logout result
    let result = store
        .send_and_wait_for(
            action,
            |a| matches!(a, AuthAction::LogoutSuccess { .. }),
            Duration::from_secs(5),
        )
        .await
        .map_err(|_| AppError::timeout("Logout timed out"))?;

    // Map result to HTTP response
    match result {
        AuthAction::LogoutSuccess { .. } => Ok((
            StatusCode::OK,
            Json(LogoutResponse {
                message: "Logged out successfully".to_string(),
            }),
        )),
        _ => Err(AppError::internal("Unexpected action received")),
    }
}
