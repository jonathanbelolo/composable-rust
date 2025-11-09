//! Passkey/WebAuthn authentication handlers.
//!
//! Implements FIDO2/WebAuthn passwordless authentication.
//!
//! **Note**: These handlers are implemented but the underlying passkey reducers
//! return `None` in several places (see passkey.rs:208, 216, etc.). The reducers
//! need to be updated to return proper challenge/result actions before these
//! handlers will work end-to-end.

use crate::{AuthAction, AuthEnvironment, AuthReducer, AuthState};
use crate::state::UserId;
use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use composable_rust_runtime::Store;
use composable_rust_web::{AppError, ClientIp, CorrelationId, UserAgent};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Request to begin passkey registration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BeginPasskeyRegistrationRequest {
    /// User ID (must be authenticated).
    pub user_id: String,

    /// Device name for this passkey.
    pub device_name: String,
}

/// Response with WebAuthn challenge for registration.
#[derive(Debug, Clone, Serialize)]
pub struct BeginPasskeyRegistrationResponse {
    /// WebAuthn challenge (base64).
    pub challenge: String,

    /// RP ID.
    pub rp_id: String,

    /// User info for WebAuthn.
    pub user: WebAuthnUser,
}

/// WebAuthn user info.
#[derive(Debug, Clone, Serialize)]
pub struct WebAuthnUser {
    /// User ID (base64).
    pub id: String,

    /// User's email.
    pub name: String,

    /// Display name.
    pub display_name: String,
}

/// Request to complete passkey registration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CompletePasskeyRegistrationRequest {
    /// User ID.
    pub user_id: String,

    /// Device ID.
    pub device_id: String,

    /// Credential ID from WebAuthn.
    pub credential_id: String,

    /// Public key (base64).
    pub public_key: String,

    /// Attestation response from navigator.credentials.create().
    pub attestation_response: String,
}

/// Response after successful passkey registration.
#[derive(Debug, Clone, Serialize)]
pub struct CompletePasskeyRegistrationResponse {
    /// Success message.
    pub message: String,

    /// Credential ID.
    pub credential_id: String,
}

/// Request to begin passkey login.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BeginPasskeyLoginRequest {
    /// Email or username.
    pub email: String,
}

/// Response with WebAuthn challenge for login.
#[derive(Debug, Clone, Serialize)]
pub struct BeginPasskeyLoginResponse {
    /// WebAuthn challenge (base64).
    pub challenge: String,

    /// Allowed credential IDs.
    pub allowed_credentials: Vec<String>,
}

/// Request to complete passkey login.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CompletePasskeyLoginRequest {
    /// Email or username.
    pub email: String,

    /// Credential ID used.
    pub credential_id: String,

    /// Assertion response from navigator.credentials.get().
    pub assertion_response: String,
}

/// Response after successful passkey login.
#[derive(Debug, Clone, Serialize)]
pub struct CompletePasskeyLoginResponse {
    /// Session ID.
    pub session_id: String,

    /// Session token for authentication.
    pub session_token: String,

    /// User's email.
    pub email: String,

    /// Session expiration timestamp (ISO 8601).
    pub expires_at: String,
}

/// Begin passkey registration flow.
///
/// # Endpoint
///
/// ```text
/// POST /api/v1/auth/passkey/register/begin
/// Content-Type: application/json
///
/// {
///   "user_id": "uuid",
///   "device_name": "My Phone"
/// }
/// ```
///
/// # Response
///
/// Returns WebAuthn challenge for `navigator.credentials.create()`.
pub async fn begin_passkey_registration<O, E, W, S, T, U, D, R, OT, C, RL>(
    State(store): State<Arc<Store<AuthState, AuthAction, AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>, AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>>>>,
    correlation_id: CorrelationId,
    _client_ip: ClientIp,
    _user_agent: UserAgent,
    Json(request): Json<BeginPasskeyRegistrationRequest>,
) -> Result<(StatusCode, Json<BeginPasskeyRegistrationResponse>), AppError>
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
    // Parse user ID
    let user_id = uuid::Uuid::parse_str(&request.user_id)
        .map_err(|_| AppError::bad_request("Invalid user ID format"))?;

    // Build action
    let action = AuthAction::InitiatePasskeyRegistration {
        correlation_id: correlation_id.0,
        user_id: UserId(user_id),
        device_name: request.device_name,
    };

    // Dispatch and wait for challenge or failure
    let result = store
        .send_and_wait_for(
            action,
            |a| {
                matches!(
                    a,
                    AuthAction::PasskeyRegistrationChallengeGenerated { .. }
                        | AuthAction::PasskeyRegistrationFailed { .. }
                )
            },
            Duration::from_secs(10),
        )
        .await
        .map_err(|_| AppError::timeout("Passkey registration initiation timed out"))?;

    // Map result to HTTP response
    match result {
        AuthAction::PasskeyRegistrationChallengeGenerated {
            challenge,
            rp_id,
            user_email,
            user_display_name,
            ..
        } => Ok((
            StatusCode::OK,
            Json(BeginPasskeyRegistrationResponse {
                challenge,
                rp_id,
                user: WebAuthnUser {
                    id: request.user_id,
                    name: user_email,
                    display_name: user_display_name,
                },
            }),
        )),
        AuthAction::PasskeyRegistrationFailed { error, .. } => {
            Err(AppError::internal(format!("Passkey registration failed: {error}")))
        }
        _ => Err(AppError::internal("Unexpected action received")),
    }
}

/// Complete passkey registration.
///
/// # Endpoint
///
/// ```text
/// POST /api/v1/auth/passkey/register/complete
/// Content-Type: application/json
///
/// {
///   "user_id": "uuid",
///   "device_id": "uuid",
///   "credential_id": "base64...",
///   "public_key": "base64...",
///   "attestation_response": "{...}"
/// }
/// ```
pub async fn complete_passkey_registration<O, E, W, S, T, U, D, R, OT, C, RL>(
    State(store): State<Arc<Store<AuthState, AuthAction, AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>, AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>>>>,
    correlation_id: CorrelationId,
    _client_ip: ClientIp,
    _user_agent: UserAgent,
    Json(request): Json<CompletePasskeyRegistrationRequest>,
) -> Result<(StatusCode, Json<CompletePasskeyRegistrationResponse>), AppError>
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
    // Parse UUIDs
    let user_id = uuid::Uuid::parse_str(&request.user_id)
        .map_err(|_| AppError::bad_request("Invalid user ID format"))?;
    let device_id = uuid::Uuid::parse_str(&request.device_id)
        .map_err(|_| AppError::bad_request("Invalid device ID format"))?;

    // Decode base64 public key
    let public_key = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &request.public_key,
    )
    .map_err(|_| AppError::bad_request("Invalid base64 public key"))?;

    // Build action
    let action = AuthAction::CompletePasskeyRegistration {
        correlation_id: correlation_id.0,
        user_id: UserId(user_id),
        device_id: crate::state::DeviceId(device_id),
        credential_id: request.credential_id.clone(),
        public_key,
        attestation_response: request.attestation_response,
    };

    // Dispatch and wait for success or failure
    let result = store
        .send_and_wait_for(
            action,
            |a| {
                matches!(
                    a,
                    AuthAction::PasskeyRegistrationSuccess { .. }
                        | AuthAction::PasskeyRegistrationFailed { .. }
                )
            },
            Duration::from_secs(10),
        )
        .await
        .map_err(|_| AppError::timeout("Passkey registration completion timed out"))?;

    // Map result to HTTP response
    match result {
        AuthAction::PasskeyRegistrationSuccess { credential_id, .. } => Ok((
            StatusCode::OK,
            Json(CompletePasskeyRegistrationResponse {
                message: "Passkey registered successfully".to_string(),
                credential_id,
            }),
        )),
        AuthAction::PasskeyRegistrationFailed { error, .. } => {
            Err(AppError::internal(format!("Passkey registration failed: {error}")))
        }
        _ => Err(AppError::internal("Unexpected action received")),
    }
}

/// Begin passkey login flow.
///
/// # Endpoint
///
/// ```text
/// POST /api/v1/auth/passkey/login/begin
/// Content-Type: application/json
///
/// {
///   "email": "user@example.com"
/// }
/// ```
pub async fn begin_passkey_login<O, E, W, S, T, U, D, R, OT, C, RL>(
    State(store): State<Arc<Store<AuthState, AuthAction, AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>, AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>>>>,
    correlation_id: CorrelationId,
    client_ip: ClientIp,
    user_agent: UserAgent,
    Json(request): Json<BeginPasskeyLoginRequest>,
) -> Result<(StatusCode, Json<BeginPasskeyLoginResponse>), AppError>
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
    // Build action
    let action = AuthAction::InitiatePasskeyLogin {
        correlation_id: correlation_id.0,
        username: request.email,
        ip_address: client_ip.0,
        user_agent: user_agent.0,
    };

    // Dispatch and wait for challenge or failure
    let result = store
        .send_and_wait_for(
            action,
            |a| {
                matches!(
                    a,
                    AuthAction::PasskeyLoginChallengeGenerated { .. }
                        | AuthAction::PasskeyAuthenticationFailed { .. }
                )
            },
            Duration::from_secs(10),
        )
        .await
        .map_err(|_| AppError::timeout("Passkey login initiation timed out"))?;

    // Map result to HTTP response
    match result {
        AuthAction::PasskeyLoginChallengeGenerated { challenge, allowed_credentials, .. } => Ok((
            StatusCode::OK,
            Json(BeginPasskeyLoginResponse {
                challenge,
                allowed_credentials,
            }),
        )),
        AuthAction::PasskeyAuthenticationFailed { error, .. } => {
            Err(AppError::unauthorized(format!("Passkey login failed: {error}")))
        }
        _ => Err(AppError::internal("Unexpected action received")),
    }
}

/// Complete passkey login and create session.
///
/// # Endpoint
///
/// ```text
/// POST /api/v1/auth/passkey/login/complete
/// Content-Type: application/json
///
/// {
///   "email": "user@example.com",
///   "credential_id": "base64...",
///   "assertion_response": "{...}"
/// }
/// ```
pub async fn complete_passkey_login<O, E, W, S, T, U, D, R, OT, C, RL>(
    State(store): State<Arc<Store<AuthState, AuthAction, AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>, AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>>>>,
    correlation_id: CorrelationId,
    client_ip: ClientIp,
    user_agent: UserAgent,
    Json(request): Json<CompletePasskeyLoginRequest>,
) -> Result<(StatusCode, Json<CompletePasskeyLoginResponse>), AppError>
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
    // Build action
    let action = AuthAction::CompletePasskeyLogin {
        correlation_id: correlation_id.0,
        credential_id: request.credential_id,
        assertion_response: request.assertion_response,
        ip_address: client_ip.0,
        user_agent: user_agent.0.clone(),
        fingerprint: None,
    };

    // Dispatch and wait for session creation or failure
    // Note: Similar to OAuth callback, we wait for SessionCreated instead of PasskeyLoginSuccess
    let result = store
        .send_and_wait_for(
            action,
            |a| {
                matches!(
                    a,
                    AuthAction::SessionCreated { .. }
                        | AuthAction::PasskeyAuthenticationFailed { .. }
                )
            },
            Duration::from_secs(30),
        )
        .await
        .map_err(|_| AppError::timeout("Passkey login completion timed out"))?;

    // Map result to HTTP response
    match result {
        AuthAction::SessionCreated { session, .. } => {
            // Session ID acts as the bearer token for authentication
            let session_token = session.session_id.0.to_string();
            Ok((
                StatusCode::OK,
                Json(CompletePasskeyLoginResponse {
                    session_id: session.session_id.0.to_string(),
                    session_token,
                    email: session.email.clone(),
                    expires_at: session.expires_at.to_rfc3339(),
                }),
            ))
        }
        AuthAction::PasskeyAuthenticationFailed { error, .. } => {
            Err(AppError::unauthorized(format!("Passkey login failed: {error}")))
        }
        _ => Err(AppError::internal("Unexpected action received")),
    }
}
