//! Magic Link authentication handlers.
//!
//! Implements passwordless authentication via email magic links.

use crate::{AuthAction, AuthEnvironment, AuthReducer, AuthState};
use axum::{extract::State, http::StatusCode, Json};
use composable_rust_runtime::Store;
use composable_rust_web::{AppError, ClientIp, CorrelationId, UserAgent};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Request to send a magic link.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SendMagicLinkRequest {
    /// Email address to send magic link to.
    pub email: String,
}

/// Response after sending magic link.
#[derive(Debug, Clone, Serialize)]
pub struct SendMagicLinkResponse {
    /// Confirmation message.
    pub message: String,

    /// Email address (for confirmation).
    pub email: String,
}

/// Request to verify a magic link.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VerifyMagicLinkRequest {
    /// Magic link token from email.
    pub token: String,
}

/// Response after successful magic link verification.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyMagicLinkResponse {
    /// Session ID.
    pub session_id: String,

    /// Session token for authentication.
    pub session_token: String,

    /// User's email.
    pub email: String,

    /// Session expiration timestamp (ISO 8601).
    pub expires_at: String,
}

/// Send magic link to user's email.
///
/// # Endpoint
///
/// ```text
/// POST /api/v1/auth/magic-link/send
/// Content-Type: application/json
///
/// {
///   "email": "user@example.com"
/// }
/// ```
///
/// # Response
///
/// ```json
/// {
///   "message": "Magic link sent. Check your email.",
///   "email": "user@example.com"
/// }
/// ```
///
/// # Flow
///
/// 1. Extract correlation_id, client_ip, user_agent from request
/// 2. Build `SendMagicLink` action
/// 3. Dispatch and wait for `MagicLinkSent` or `MagicLinkFailed`
/// 4. Return success/error response
pub async fn send_magic_link<O, E, W, S, T, U, D, R, OT, C, RL>(
    State(store): State<Arc<Store<AuthState, AuthAction, AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>, AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>>>>,
    correlation_id: CorrelationId,
    client_ip: ClientIp,
    user_agent: UserAgent,
    Json(request): Json<SendMagicLinkRequest>,
) -> Result<(StatusCode, Json<SendMagicLinkResponse>), AppError>
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
    // Build action from request
    let action = AuthAction::SendMagicLink {
        correlation_id: correlation_id.0,
        email: request.email.clone(),
        ip_address: client_ip.0,
        user_agent: user_agent.0,
    };

    // Dispatch and wait for terminal action
    let result = store
        .send_and_wait_for(
            action,
            |a| matches!(a, AuthAction::MagicLinkSent { .. } | AuthAction::MagicLinkFailed { .. }),
            Duration::from_secs(10),
        )
        .await
        .map_err(|_| AppError::timeout("Magic link request timed out"))?;

    // Map result to HTTP response
    match result {
        AuthAction::MagicLinkSent { email, .. } => Ok((
            StatusCode::OK,
            Json(SendMagicLinkResponse {
                message: "Magic link sent. Check your email.".to_string(),
                email,
            }),
        )),
        AuthAction::MagicLinkFailed { error, .. } => {
            Err(AppError::internal(format!("Failed to send magic link: {error}")))
        }
        _ => Err(AppError::internal("Unexpected action received")),
    }
}

/// Verify magic link token and create session.
///
/// # Endpoint
///
/// ```text
/// POST /api/v1/auth/magic-link/verify
/// Content-Type: application/json
///
/// {
///   "token": "abc123..."
/// }
/// ```
///
/// # Response (Success)
///
/// ```json
/// {
///   "`session_id`": "550e8400-e29b-41d4-a716-446655440000",
///   "session_token": "eyJhbGc...",
///   "email": "user@example.com",
///   "expires_at": "2025-11-09T18:00:00Z"
/// }
/// ```
///
/// # Response (Error)
///
/// - `401 Unauthorized`: Invalid or expired token
/// - `429 Too Many Requests`: Rate limit exceeded
///
/// # Flow
///
/// 1. Extract correlation_id, client_ip, user_agent from request
/// 2. Build `VerifyMagicLink` action
/// 3. Dispatch and wait for `SessionCreated` or error actions
/// 4. Return session info or error
pub async fn verify_magic_link<O, E, W, S, T, U, D, R, OT, C, RL>(
    State(store): State<Arc<Store<AuthState, AuthAction, AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>, AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>>>>,
    correlation_id: CorrelationId,
    client_ip: ClientIp,
    user_agent: UserAgent,
    Json(request): Json<VerifyMagicLinkRequest>,
) -> Result<(StatusCode, Json<VerifyMagicLinkResponse>), AppError>
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
    // Build action from request
    let action = AuthAction::VerifyMagicLink {
        correlation_id: correlation_id.0,
        token: request.token,
        ip_address: client_ip.0,
        user_agent: user_agent.0,
        fingerprint: None, // TODO: Extract from request header if available
    };

    // Dispatch and wait for terminal action
    let result = store
        .send_and_wait_for(
            action,
            |a| {
                matches!(
                    a,
                    AuthAction::SessionCreated { .. }
                        | AuthAction::MagicLinkFailed { .. }
                        | AuthAction::SessionCreationFailed { .. }
                )
            },
            Duration::from_secs(10),
        )
        .await
        .map_err(|_| AppError::timeout("Magic link verification timed out"))?;

    // Map result to HTTP response
    match result {
        AuthAction::SessionCreated { session, .. } => {
            // Session ID acts as the bearer token for authentication
            let session_token = session.session_id.0.to_string();
            Ok((
                StatusCode::OK,
                Json(VerifyMagicLinkResponse {
                    session_id: session.session_id.0.to_string(),
                    session_token,
                    email: session.email.clone(),
                    expires_at: session.expires_at.to_rfc3339(),
                }),
            ))
        }
        AuthAction::MagicLinkFailed { error, .. } => Err(AppError::unauthorized(error)),
        AuthAction::SessionCreationFailed { error, .. } => {
            Err(AppError::internal(format!("Session creation failed: {error}")))
        }
        _ => Err(AppError::internal("Unexpected action received")),
    }
}

// Integration tests are in auth/tests/magic_link_integration.rs
