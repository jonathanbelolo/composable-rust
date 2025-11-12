//! Authentication state types.
//!
//! This module defines the core state types for the authentication system.
//! All types are `Clone` to support the functional architecture pattern.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

// ═══════════════════════════════════════════════════════════════════════
// ID Types
// ═══════════════════════════════════════════════════════════════════════

/// Unique identifier for a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub uuid::Uuid);

impl UserId {
    /// Generate a new random `UserId`.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for UserId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub uuid::Uuid);

impl SessionId {
    /// Generate a new cryptographically secure random `SessionId`.
    ///
    /// Uses 256 bits of randomness for security.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub uuid::Uuid);

impl DeviceId {
    /// Generate a new random `DeviceId`.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for DeviceId {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Core State Types
// ═══════════════════════════════════════════════════════════════════════

/// Root authentication state.
///
/// This is the state managed by the auth reducer. It represents the
/// in-memory state during an authentication flow.
///
/// # Examples
///
/// ```
/// # use composable_rust_auth::AuthState;
/// let mut state = AuthState::default();
/// assert!(state.session.is_none());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AuthState {
    /// Current session (if logged in).
    pub session: Option<Session>,

    /// `OAuth` state (during `OAuth` flow).
    pub oauth_state: Option<OAuthState>,

    /// Magic link state (during magic link flow).
    pub magic_link_state: Option<MagicLinkState>,

    /// `WebAuthn` challenge (during passkey flow).
    pub webauthn_challenge: Option<WebAuthnChallenge>,
}

/// User session.
///
/// Sessions are ephemeral (stored in `Redis` with `TTL`). They reference
/// permanent device records (stored in `PostgreSQL`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// Unique session identifier.
    pub session_id: SessionId,

    /// User ID (foreign key to `PostgreSQL` users table).
    pub user_id: UserId,

    /// Device ID (foreign key to `PostgreSQL` devices table).
    pub device_id: DeviceId,

    /// User's email (cached from `PostgreSQL`).
    pub email: String,

    /// Session creation timestamp.
    pub created_at: DateTime<Utc>,

    /// Last activity timestamp (updated on each request).
    pub last_active: DateTime<Utc>,

    /// Session expiration timestamp.
    pub expires_at: DateTime<Utc>,

    /// IP address from which the session was created.
    pub ip_address: IpAddr,

    /// User agent string.
    pub user_agent: String,

    /// `OAuth` provider (if authenticated via `OAuth`).
    pub oauth_provider: Option<OAuthProvider>,

    /// Risk assessment at login time.
    pub login_risk_score: f32,

    /// Idle timeout - max time between activity before session expires.
    ///
    /// This allows different authentication methods to have different
    /// idle timeout policies (e.g., passkeys might have longer timeouts
    /// than magic links).
    pub idle_timeout: chrono::Duration,

    /// Enable sliding window session refresh.
    ///
    /// When `true`, the `expires_at` timestamp is extended on each access,
    /// creating a sliding window for the absolute session lifetime.
    /// When `false`, the session expires at a fixed `expires_at` time.
    ///
    /// Default: false (fixed expiration for security)
    pub enable_sliding_refresh: bool,
}

/// Token pair for `JWT`-based authentication (optional feature).
///
/// Used for stateless API clients (mobile apps, SPAs).
/// The refresh token is actually just a session ID.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenPair {
    /// Short-lived access token (`JWT`, 15 minutes).
    pub access_token: String,

    /// Long-lived refresh token (session ID, 24 hours).
    pub refresh_token: String,

    /// Access token expiration timestamp.
    pub expires_at: DateTime<Utc>,
}

// ═══════════════════════════════════════════════════════════════════════
// OAuth State
// ═══════════════════════════════════════════════════════════════════════

/// `OAuth` provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OAuthProvider {
    /// Google `OAuth`.
    Google,
    /// GitHub `OAuth`.
    GitHub,
    /// Microsoft `OAuth`.
    Microsoft,
}

impl OAuthProvider {
    /// Get the provider name as a string.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::GitHub => "github",
            Self::Microsoft => "microsoft",
        }
    }

    /// Parse provider from string.
    ///
    /// # Errors
    ///
    /// Returns error if the provider string is not recognized.
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "google" => Ok(Self::Google),
            "github" => Ok(Self::GitHub),
            "microsoft" => Ok(Self::Microsoft),
            _ => Err(format!("Unknown OAuth provider: {s}")),
        }
    }
}

/// `OAuth` flow state.
///
/// Stored in `AuthState` during the `OAuth` authorization code flow
/// to prevent `CSRF` attacks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OAuthState {
    /// `CSRF` protection: random state parameter.
    ///
    /// Must be 256 bits of cryptographic randomness.
    pub state_param: String,

    /// `OAuth` provider.
    pub provider: OAuthProvider,

    /// Timestamp when the `OAuth` flow was initiated.
    pub initiated_at: DateTime<Utc>,
}

// ═══════════════════════════════════════════════════════════════════════
// Magic Link State
// ═══════════════════════════════════════════════════════════════════════

/// Magic link flow state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MagicLinkState {
    /// Email address the magic link was sent to.
    pub email: String,

    /// Token (stored hashed in database).
    pub token: String,

    /// Expiration timestamp (typically 5-15 minutes).
    pub expires_at: DateTime<Utc>,
}

// ═══════════════════════════════════════════════════════════════════════
// WebAuthn State
// ═══════════════════════════════════════════════════════════════════════

/// `WebAuthn` challenge.
///
/// Stored in `Redis` with short `TTL` (~5 minutes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebAuthnChallenge {
    /// Challenge ID.
    pub challenge_id: String,

    /// Challenge bytes (base64-encoded).
    pub challenge: String,

    /// User ID this challenge is for.
    pub user_id: UserId,

    /// Expiration timestamp.
    pub expires_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_id_generation() {
        let id1 = UserId::new();
        let id2 = UserId::new();

        // IDs should be unique
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_session_id_generation() {
        let id1 = SessionId::new();
        let id2 = SessionId::new();

        // Session IDs should be unique
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_oauth_provider_str() {
        assert_eq!(OAuthProvider::Google.as_str(), "google");
        assert_eq!(OAuthProvider::GitHub.as_str(), "github");
        assert_eq!(OAuthProvider::Microsoft.as_str(), "microsoft");
    }
}
