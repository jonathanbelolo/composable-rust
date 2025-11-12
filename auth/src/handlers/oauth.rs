//! OAuth2/OIDC authentication handlers.
//!
//! Implements `OAuth2` authorization code flow with OIDC support.

use crate::{AuthAction, AuthEnvironment, AuthReducer, AuthState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    Json,
};
use composable_rust_runtime::Store;
use composable_rust_web::{AppError, ClientIp, CorrelationId, UserAgent};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Request to initiate `OAuth` flow (no body, provider in path).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OAuthAuthorizeRequest {
    /// Optional redirect URL after auth (defaults to /).
    pub redirect_to: Option<String>,
}

/// `OAuth` callback query parameters.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OAuthCallbackQuery {
    /// Authorization code from provider.
    pub code: String,

    /// State parameter (`CSRF` protection).
    pub state: String,

    /// Optional error from provider.
    pub error: Option<String>,

    /// Optional error description from provider.
    pub error_description: Option<String>,
}

/// Response after successful `OAuth` callback.
#[derive(Debug, Clone, Serialize)]
pub struct OAuthCallbackResponse {
    /// Session ID.
    pub session_id: String,

    /// Session token for authentication.
    pub session_token: String,

    /// User's email.
    pub email: String,

    /// Session expiration timestamp (ISO 8601).
    pub expires_at: String,
}

/// Initiate `OAuth` authorization flow.
///
/// # Endpoint
///
/// ```text
/// GET /api/v1/auth/oauth/:provider/authorize
/// ```
///
/// # Flow
///
/// 1. Extract provider from path (e.g., "google", "github")
/// 2. Generate `correlation_id`
/// 3. Send `InitiateOAuth` action
/// 4. Wait for `OAuthAuthorizationUrlReady`
/// 5. Redirect to `OAuth` provider
///
/// # Response
///
/// HTTP 302 redirect to `OAuth` provider's authorization page.
pub async fn oauth_authorize<O, E, W, S, T, U, D, R, OT, C, RL>(
    State(store): State<Arc<Store<AuthState, AuthAction, AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>, AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>>>>,
    Path(provider_str): Path<String>,
    correlation_id: CorrelationId,
    client_ip: ClientIp,
    user_agent: UserAgent,
) -> Result<Response, AppError>
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
    // Parse provider from path
    let provider = crate::state::OAuthProvider::from_str(&provider_str)
        .map_err(|_| AppError::bad_request(format!("Invalid OAuth provider: {provider_str}")))?;

    // Build action from request
    let action = AuthAction::InitiateOAuth {
        correlation_id: correlation_id.0,
        provider,
        ip_address: client_ip.0,
        user_agent: user_agent.0.clone(),
        fingerprint: None, // TODO: Extract from request if provided
    };

    // Dispatch and wait for authorization URL or failure
    let result = store
        .send_and_wait_for(
            action,
            |a| {
                matches!(
                    a,
                    AuthAction::OAuthAuthorizationUrlReady { .. } | AuthAction::OAuthFailed { .. }
                )
            },
            Duration::from_secs(10),
        )
        .await
        .map_err(|_| AppError::timeout("OAuth initiation timed out"))?;

    // Map result to HTTP response
    match result {
        AuthAction::OAuthAuthorizationUrlReady { authorization_url, .. } => {
            // Redirect to OAuth provider
            Ok(Redirect::to(&authorization_url).into_response())
        }
        AuthAction::OAuthFailed { error, error_description, .. } => {
            let message = error_description.unwrap_or(error);
            Err(AppError::internal(format!("OAuth failed: {message}")))
        }
        _ => Err(AppError::internal("Unexpected action received")),
    }
}

/// Handle `OAuth` callback after user authorizes.
///
/// # Endpoint
///
/// ```text
/// GET /api/v1/auth/oauth/:provider/callback?code=...&state=...
/// ```
///
/// # Flow
///
/// 1. Extract code and state from query parameters
/// 2. Send `OAuthCallback` action
/// 3. Wait for `OAuthSuccess` or `OAuthFailed`
/// 4. Return session info or error
///
/// # Response (Success)
///
/// ```json
/// {
///   "`session_id`": "uuid",
///   "session_token": "token",
///   "email": "user@example.com",
///   "expires_at": "2024-01-01T00:00:00Z"
/// }
/// ```
///
/// # Response (Error)
///
/// ```json
/// {
///   "code": "oauth_failed",
///   "message": "`OAuth` authentication failed: invalid_grant"
/// }
/// ```
pub async fn oauth_callback<O, E, W, S, T, U, D, R, OT, C, RL>(
    State(store): State<Arc<Store<AuthState, AuthAction, AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>, AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>>>>,
    Path(provider_str): Path<String>,
    Query(query): Query<OAuthCallbackQuery>,
    correlation_id: CorrelationId,
    client_ip: ClientIp,
    user_agent: UserAgent,
) -> Result<(StatusCode, Json<OAuthCallbackResponse>), AppError>
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
    // Check if provider sent an error
    if let Some(error) = query.error {
        let description = query.error_description.unwrap_or_default();
        return Err(AppError::bad_request(format!("OAuth error: {error} - {description}")));
    }

    // Parse provider from path (validates provider name)
    let _provider = crate::state::OAuthProvider::from_str(&provider_str)
        .map_err(|_| AppError::bad_request(format!("Invalid OAuth provider: {provider_str}")))?;

    // Build action from callback
    let action = AuthAction::OAuthCallback {
        correlation_id: correlation_id.0,
        code: query.code,
        state: query.state,
        ip_address: client_ip.0,
        user_agent: user_agent.0.clone(),
        fingerprint: None, // TODO: Extract from request if provided
    };

    // Dispatch and wait for session creation or failure (30 seconds for token exchange)
    let result = store
        .send_and_wait_for(
            action,
            |a| {
                matches!(
                    a,
                    AuthAction::SessionCreated { .. } | AuthAction::OAuthFailed { .. }
                )
            },
            Duration::from_secs(30),
        )
        .await
        .map_err(|_| AppError::timeout("OAuth callback timed out"))?;

    // Map result to HTTP response
    match result {
        AuthAction::SessionCreated { session, .. } => {
            // Session ID acts as the bearer token for authentication
            let session_token = session.session_id.0.to_string();
            // TODO: Set session cookie here (needs cookie middleware)
            Ok((
                StatusCode::OK,
                Json(OAuthCallbackResponse {
                    session_id: session.session_id.0.to_string(),
                    session_token,
                    email: session.email.clone(),
                    expires_at: session.expires_at.to_rfc3339(),
                }),
            ))
        }
        AuthAction::OAuthFailed { error, error_description, .. } => {
            let message = error_description.unwrap_or(error);
            Err(AppError::internal(format!("OAuth authentication failed: {message}")))
        }
        _ => Err(AppError::internal("Unexpected action received")),
    }
}
