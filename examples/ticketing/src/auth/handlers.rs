//! Custom authentication handlers for the ticketing system.
//!
//! These handlers wrap the `composable-rust-auth` handlers with additional
//! functionality specific to the ticketing system, such as testing support.

use crate::auth::setup::TicketingAuthStore;
use crate::config::Config;
use axum::{extract::State, http::StatusCode, Json};
use composable_rust_auth::AuthAction;
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

    /// **TESTING ONLY**: The magic link token.
    ///
    /// # Security Warning
    ///
    /// This field is ONLY included when `AUTH_EXPOSE_MAGIC_LINKS_FOR_TESTING=true`
    /// in the environment configuration. This defeats the security purpose of
    /// magic links (email ownership verification) but allows automated testing.
    ///
    /// **MUST be `None` in production!**
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magic_link_token: Option<String>,
}

/// Send magic link to user's email.
///
/// This is a custom handler that wraps the auth library's functionality
/// with testing support for automated integration tests.
///
/// # Endpoint
///
/// ```text
/// POST /auth/magic-link/request
/// Content-Type: application/json
///
/// {
///   "email": "user@example.com"
/// }
/// ```
///
/// # Response (Production)
///
/// ```json
/// {
///   "message": "Magic link sent. Check your email.",
///   "email": "user@example.com"
/// }
/// ```
///
/// # Response (Testing Mode)
///
/// When `AUTH_EXPOSE_MAGIC_LINKS_FOR_TESTING=true`:
///
/// ```json
/// {
///   "message": "Magic link sent. Check your email.",
///   "email": "user@example.com",
///   "magic_link_token": "abc123..."
/// }
/// ```
///
/// # Flow
///
/// 1. Extract `correlation_id`, `client_ip`, `user_agent` from request
/// 2. Build `SendMagicLink` action
/// 3. Dispatch and wait for `MagicLinkSent` or `MagicLinkFailed`
/// 4. If testing mode enabled, extract token from `MagicLinkSent`
/// 5. Return response with optional token
pub async fn send_magic_link(
    State(store): State<Arc<TicketingAuthStore>>,
    State(config): State<Arc<Config>>,
    correlation_id: CorrelationId,
    client_ip: ClientIp,
    user_agent: UserAgent,
    Json(request): Json<SendMagicLinkRequest>,
) -> Result<(StatusCode, Json<SendMagicLinkResponse>), AppError> {
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
        AuthAction::MagicLinkSent { email, token, .. } => {
            // Extract token if testing mode is enabled
            let magic_link_token = if config.auth.expose_magic_links_for_testing {
                Some(token)
            } else {
                None
            };

            Ok((
                StatusCode::OK,
                Json(SendMagicLinkResponse {
                    message: "Magic link sent. Check your email.".to_string(),
                    email,
                    magic_link_token,
                }),
            ))
        }
        AuthAction::MagicLinkFailed { error, .. } => {
            Err(AppError::internal(format!("Failed to send magic link: {error}")))
        }
        _ => Err(AppError::internal("Unexpected action received")),
    }
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

/// Verify magic link token and create session.
///
/// # Endpoint
///
/// ```text
/// POST /auth/magic-link/verify
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
///   "session_id": "550e8400-e29b-41d4-a716-446655440000",
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
/// 1. Extract `correlation_id`, `client_ip`, `user_agent` from request
/// 2. Build `VerifyMagicLink` action
/// 3. Dispatch and wait for `SessionCreated` or error actions
/// 4. Return session info or error
pub async fn verify_magic_link(
    State(store): State<Arc<TicketingAuthStore>>,
    correlation_id: CorrelationId,
    client_ip: ClientIp,
    user_agent: UserAgent,
    Json(request): Json<VerifyMagicLinkRequest>,
) -> Result<(StatusCode, Json<VerifyMagicLinkResponse>), AppError> {
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
