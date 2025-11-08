//! Authentication effects.
//!
//! This module defines all side effects that the auth reducer can produce.
//! Effects are **values**, not execution. The effect executor is responsible
//! for interpreting these values and performing actual I/O.

use crate::actions::AuthAction;
use crate::state::{DeviceId, OAuthProvider, Session, SessionId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Authentication effect.
///
/// Effects are descriptions of side effects, not the effects themselves.
/// The reducer returns `Vec<AuthEffect>`, and the effect executor
/// interprets them.
///
/// # Architecture Note
///
/// Effects are **composable**:
/// - `Parallel`: Execute effects concurrently
/// - `Sequential`: Execute effects in order
/// - `Conditional`: Execute effect only if condition holds
///
/// This allows complex workflows while keeping the reducer pure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AuthEffect {
    // ═══════════════════════════════════════════════════════════════════════
    // Core Effect Composition (from composable-rust-core)
    // ═══════════════════════════════════════════════════════════════════════
    /// No effect.
    None,

    /// Execute effects in parallel.
    Parallel(Vec<AuthEffect>),

    /// Execute effects sequentially.
    Sequential(Vec<AuthEffect>),

    /// Future that produces an action.
    ///
    /// The executor will await this future and dispatch the resulting action.
    Future {
        /// Future that produces an action.
        ///
        /// Note: We can't store the actual future here (not serializable),
        /// so we use a description that the executor interprets.
        description: String,

        /// Callback to dispatch when future completes.
        on_success: Option<Box<AuthAction>>,

        /// Callback to dispatch if future fails.
        on_error: Option<Box<AuthAction>>,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // OAuth2 / OIDC Effects
    // ═══════════════════════════════════════════════════════════════════════
    /// Redirect user to OAuth provider.
    ///
    /// # Executor Responsibility
    ///
    /// 1. Build authorization URL with state parameter
    /// 2. Return HTTP redirect response (302)
    RedirectToOAuthProvider {
        /// OAuth provider.
        provider: OAuthProvider,

        /// CSRF state parameter.
        state_param: String,

        /// Redirect URI (callback URL).
        redirect_uri: String,
    },

    /// Exchange OAuth authorization code for access token.
    ///
    /// # Executor Responsibility
    ///
    /// 1. HTTP POST to provider's token endpoint
    /// 2. Validate response
    /// 3. Dispatch `OAuthSuccess` or `OAuthFailed`
    ExchangeOAuthCode {
        /// OAuth provider.
        provider: OAuthProvider,

        /// Authorization code from callback.
        code: String,

        /// Redirect URI (must match initial request).
        redirect_uri: String,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,
    },

    /// Fetch user info from OAuth provider.
    ///
    /// # Executor Responsibility
    ///
    /// 1. HTTP GET to provider's userinfo endpoint
    /// 2. Parse response (email, name, etc.)
    /// 3. Dispatch action with user info
    FetchOAuthUserInfo {
        /// OAuth provider.
        provider: OAuthProvider,

        /// Access token from token exchange.
        access_token: String,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Magic Link Effects
    // ═══════════════════════════════════════════════════════════════════════
    /// Generate cryptographically secure magic link token.
    ///
    /// # Executor Responsibility
    ///
    /// 1. Generate 32 bytes of cryptographic randomness
    /// 2. Base64-encode token
    /// 3. Hash token with SHA-256 for database storage
    /// 4. Dispatch action with token and hash
    GenerateMagicLinkToken {
        /// Email address.
        email: String,

        /// TTL for token (typically 5-15 minutes).
        ttl_minutes: u32,
    },

    /// Send magic link email.
    ///
    /// # Executor Responsibility
    ///
    /// 1. Build email with magic link URL
    /// 2. Send via email provider (SendGrid, AWS SES, etc.)
    /// 3. Dispatch `MagicLinkSent` or error action
    SendMagicLinkEmail {
        /// Email address.
        email: String,

        /// Magic link token.
        token: String,

        /// Base URL for magic link (e.g., "https://app.example.com/auth/verify").
        base_url: String,

        /// Expiration timestamp.
        expires_at: DateTime<Utc>,
    },

    /// Store magic link token (hashed) in database.
    StoreMagicLinkToken {
        /// Email address.
        email: String,

        /// Token hash (SHA-256).
        token_hash: String,

        /// Expiration timestamp.
        expires_at: DateTime<Utc>,
    },

    /// Verify magic link token.
    ///
    /// # Executor Responsibility
    ///
    /// 1. Hash provided token
    /// 2. Look up in database
    /// 3. Verify not expired
    /// 4. Verify not already used
    /// 5. Mark as used
    /// 6. Dispatch `MagicLinkVerified` or error
    VerifyMagicLinkToken {
        /// Token from email link.
        token: String,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // WebAuthn / Passkey Effects
    // ═══════════════════════════════════════════════════════════════════════
    /// Generate WebAuthn challenge.
    ///
    /// # Executor Responsibility
    ///
    /// 1. Generate 32 bytes of cryptographic randomness
    /// 2. Store challenge in Redis (5-minute TTL)
    /// 3. Return challenge to client
    GenerateWebAuthnChallenge {
        /// User ID.
        user_id: UserId,

        /// Challenge type (registration or authentication).
        challenge_type: WebAuthnChallengeType,
    },

    /// Verify WebAuthn attestation (registration).
    ///
    /// # Executor Responsibility
    ///
    /// 1. Parse attestation response
    /// 2. Verify signature
    /// 3. Extract public key
    /// 4. Verify origin and RP ID
    /// 5. Dispatch success or error action
    VerifyWebAuthnAttestation {
        /// User ID.
        user_id: UserId,

        /// Device ID.
        device_id: DeviceId,

        /// Attestation response (JSON from client).
        attestation_response: String,

        /// Expected origin (e.g., "https://app.example.com").
        expected_origin: String,

        /// Expected RP ID (e.g., "app.example.com").
        expected_rp_id: String,
    },

    /// Verify WebAuthn assertion (login).
    ///
    /// # Executor Responsibility
    ///
    /// 1. Parse assertion response
    /// 2. Look up public key for credential ID
    /// 3. Verify signature
    /// 4. Verify origin and RP ID
    /// 5. Update credential counter (replay protection)
    /// 6. Dispatch `PasskeyLoginSuccess` or error
    VerifyWebAuthnAssertion {
        /// Credential ID.
        credential_id: String,

        /// Assertion response (JSON from client).
        assertion_response: String,

        /// Expected origin.
        expected_origin: String,

        /// Expected RP ID.
        expected_rp_id: String,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Session Management Effects (Redis)
    // ═══════════════════════════════════════════════════════════════════════
    /// Create session in Redis.
    ///
    /// # Executor Responsibility
    ///
    /// 1. Serialize session
    /// 2. Store in Redis with TTL
    /// 3. Set up sliding expiration
    /// 4. Dispatch `SessionCreated`
    CreateSession {
        /// Session to create.
        session: Session,

        /// TTL in seconds (typically 24 hours).
        ttl_seconds: u32,
    },

    /// Get session from Redis.
    ///
    /// # Executor Responsibility
    ///
    /// 1. Look up session by ID
    /// 2. Deserialize
    /// 3. Dispatch `SessionValidated` or `SessionExpired`
    GetSession {
        /// Session ID.
        session_id: SessionId,
    },

    /// Update session in Redis.
    ///
    /// # Executor Responsibility
    ///
    /// 1. Update session fields (e.g., last_active)
    /// 2. Refresh TTL (sliding expiration)
    /// 3. Save back to Redis
    UpdateSession {
        /// Session to update.
        session: Session,
    },

    /// Delete session from Redis.
    ///
    /// # Executor Responsibility
    ///
    /// 1. Remove from Redis
    /// 2. Dispatch `LogoutSuccess`
    DeleteSession {
        /// Session ID to delete.
        session_id: SessionId,
    },

    /// Delete all sessions for a user.
    ///
    /// # Executor Responsibility
    ///
    /// 1. Scan Redis for user's sessions
    /// 2. Delete all matches
    /// 3. Optionally publish logout events
    DeleteAllUserSessions {
        /// User ID.
        user_id: UserId,
    },

    /// Delete all sessions for a device.
    DeleteDeviceSessions {
        /// Device ID.
        device_id: DeviceId,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Database Effects (PostgreSQL)
    // ═══════════════════════════════════════════════════════════════════════
    /// Get user from database.
    GetUser {
        /// User ID.
        user_id: Option<UserId>,

        /// Email address.
        email: Option<String>,
    },

    /// Create user in database.
    CreateUser {
        /// User ID.
        user_id: UserId,

        /// Email address.
        email: String,

        /// Display name.
        name: Option<String>,

        /// Email verified flag.
        email_verified: bool,
    },

    /// Update user in database.
    UpdateUser {
        /// User ID.
        user_id: UserId,

        /// New email (if changed).
        email: Option<String>,

        /// New name (if changed).
        name: Option<String>,

        /// Email verified flag.
        email_verified: Option<bool>,
    },

    /// Get device from database.
    GetDevice {
        /// Device ID.
        device_id: DeviceId,
    },

    /// Create device in database.
    CreateDevice {
        /// Device ID.
        device_id: DeviceId,

        /// User ID.
        user_id: UserId,

        /// Device name (e.g., "iPhone 15 Pro").
        name: String,

        /// Device type (Mobile, Desktop, Tablet).
        device_type: DeviceType,

        /// Platform (e.g., "iOS 17.2").
        platform: String,

        /// IP address.
        ip_address: IpAddr,

        /// User agent.
        user_agent: String,

        /// Trust level.
        trust_level: crate::actions::DeviceTrustLevel,
    },

    /// Update device in database.
    UpdateDevice {
        /// Device ID.
        device_id: DeviceId,

        /// Last seen timestamp.
        last_seen: DateTime<Utc>,

        /// Trust level.
        trust_level: Option<crate::actions::DeviceTrustLevel>,

        /// Passkey credential ID (if registered).
        passkey_credential_id: Option<String>,

        /// Public key (if passkey registered).
        public_key: Option<Vec<u8>>,
    },

    /// Get passkey credential.
    GetPasskey {
        /// Credential ID.
        credential_id: String,
    },

    /// Create passkey credential.
    CreatePasskey {
        /// Credential ID.
        credential_id: String,

        /// User ID.
        user_id: UserId,

        /// Device ID.
        device_id: DeviceId,

        /// Public key.
        public_key: Vec<u8>,

        /// Signature counter (replay protection).
        counter: u32,
    },

    /// Update passkey counter.
    UpdatePasskeyCounter {
        /// Credential ID.
        credential_id: String,

        /// New counter value.
        counter: u32,
    },

    /// Store OAuth link (user ↔ provider).
    StoreOAuthLink {
        /// User ID.
        user_id: UserId,

        /// OAuth provider.
        provider: OAuthProvider,

        /// Provider user ID.
        provider_user_id: String,

        /// Access token.
        access_token: String,

        /// Refresh token (if available).
        refresh_token: Option<String>,

        /// Token expiration.
        expires_at: Option<DateTime<Utc>>,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Risk & Analytics Effects
    // ═══════════════════════════════════════════════════════════════════════
    /// Calculate login risk score.
    ///
    /// # Executor Responsibility
    ///
    /// 1. Analyze IP address (geolocation, VPN detection)
    /// 2. Check device fingerprint
    /// 3. Detect impossible travel
    /// 4. Check against known bad actors
    /// 5. Return risk score (0.0-1.0)
    CalculateLoginRisk {
        /// User ID.
        user_id: UserId,

        /// IP address.
        ip_address: IpAddr,

        /// User agent.
        user_agent: String,

        /// Last login location (for impossible travel detection).
        last_login_location: Option<String>,

        /// Last login timestamp.
        last_login_at: Option<DateTime<Utc>>,
    },

    /// Record login attempt.
    ///
    /// For rate limiting and analytics.
    RecordLoginAttempt {
        /// User ID (if known).
        user_id: Option<UserId>,

        /// Email address.
        email: String,

        /// IP address.
        ip_address: IpAddr,

        /// Success flag.
        success: bool,

        /// Authentication method.
        auth_method: AuthMethod,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Event Publishing Effects
    // ═══════════════════════════════════════════════════════════════════════
    /// Publish authentication event.
    ///
    /// For event sourcing, audit logs, and cross-system notifications.
    PublishEvent {
        /// Event type.
        event_type: String,

        /// Event payload (JSON).
        payload: String,

        /// Aggregate ID (user_id or session_id).
        aggregate_id: String,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // HTTP Response Effects
    // ═══════════════════════════════════════════════════════════════════════
    /// Set HTTP cookie.
    SetCookie {
        /// Cookie name.
        name: String,

        /// Cookie value.
        value: String,

        /// Max age in seconds.
        max_age: u32,

        /// HTTP-only flag.
        http_only: bool,

        /// Secure flag.
        secure: bool,

        /// SameSite policy.
        same_site: SameSitePolicy,

        /// Cookie path.
        path: String,
    },

    /// Delete HTTP cookie.
    DeleteCookie {
        /// Cookie name.
        name: String,
    },

    /// Return HTTP redirect.
    Redirect {
        /// URL to redirect to.
        url: String,

        /// HTTP status code (302 or 303).
        status_code: u16,
    },
}

/// WebAuthn challenge type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebAuthnChallengeType {
    /// Registration (creating a passkey).
    Registration,

    /// Authentication (logging in with passkey).
    Authentication,
}

/// Device type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceType {
    /// Mobile device (phone, tablet).
    Mobile,

    /// Desktop computer.
    Desktop,

    /// Tablet.
    Tablet,

    /// Other/unknown.
    Other,
}

/// Authentication method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthMethod {
    /// OAuth2/OIDC.
    OAuth,

    /// Magic link.
    MagicLink,

    /// WebAuthn/Passkey.
    Passkey,
}

/// SameSite cookie policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SameSitePolicy {
    /// Strict (same-site only).
    Strict,

    /// Lax (cross-site GET allowed).
    Lax,

    /// None (cross-site allowed, requires Secure).
    None,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_effect_serialization() {
        let effect = AuthEffect::RedirectToOAuthProvider {
            provider: OAuthProvider::Google,
            state_param: "abc123".to_string(),
            redirect_uri: "https://example.com/callback".to_string(),
        };

        // Should serialize/deserialize
        let json = serde_json::to_string(&effect).expect("serialize");
        let deserialized: AuthEffect = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(effect, deserialized);
    }

    #[test]
    fn test_parallel_effects() {
        let effect1 = AuthEffect::CreateSession {
            session: Session {
                session_id: SessionId::new(),
                user_id: UserId::new(),
                device_id: crate::state::DeviceId::new(),
                email: "test@example.com".to_string(),
                created_at: chrono::Utc::now(),
                last_active: chrono::Utc::now(),
                expires_at: chrono::Utc::now(),
                ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                user_agent: "test".to_string(),
                oauth_provider: None,
                login_risk_score: 0.1,
            },
            ttl_seconds: 86400,
        };

        let effect2 = AuthEffect::PublishEvent {
            event_type: "session.created".to_string(),
            payload: "{}".to_string(),
            aggregate_id: "user-123".to_string(),
        };

        let parallel = AuthEffect::Parallel(vec![effect1, effect2]);

        // Should be able to nest parallel effects
        assert!(matches!(parallel, AuthEffect::Parallel(_)));
    }
}
