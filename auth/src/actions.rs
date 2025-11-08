//! Authentication actions.
//!
//! This module defines all possible actions in the authentication system.
//! Actions follow the CQRS pattern: Commands (user intent) and Events (what happened).

use crate::state::{DeviceId, OAuthProvider, Session, SessionId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Authentication action.
///
/// This enum represents all possible inputs to the auth reducer:
/// - **Commands**: User requests (InitiateOAuth, SendMagicLink, etc.)
/// - **Events**: Results of async operations (OAuthSuccess, EmailSent, etc.)
///
/// # Architecture Note
///
/// Actions are the **only** way to communicate with the auth system.
/// The reducer is a pure function: `(State, Action, Env) → (State, Effects)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AuthAction {
    // ═══════════════════════════════════════════════════════════════════════
    // OAuth2 / OIDC Flow
    // ═══════════════════════════════════════════════════════════════════════
    /// Initiate OAuth login flow.
    ///
    /// # Flow
    ///
    /// 1. Reducer generates CSRF state
    /// 2. Returns `Effect::RedirectToOAuthProvider`
    /// 3. User authorizes at provider
    /// 4. Provider redirects to callback with code
    InitiateOAuth {
        /// OAuth provider to use.
        provider: OAuthProvider,

        /// Client IP address for risk assessment.
        ip_address: IpAddr,

        /// User agent string for device fingerprinting.
        user_agent: String,
    },

    /// Handle OAuth callback.
    ///
    /// # Flow
    ///
    /// 1. Validate state parameter (CSRF protection)
    /// 2. Exchange code for access token
    /// 3. Fetch user info from provider
    /// 4. Create or link user account
    /// 5. Create session
    OAuthCallback {
        /// Authorization code from provider.
        code: String,

        /// State parameter (must match stored state).
        state: String,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,
    },

    /// OAuth token exchange succeeded.
    ///
    /// This is an **event** produced by the effect executor.
    OAuthSuccess {
        /// User's email from provider.
        email: String,

        /// User's name from provider (if available).
        name: Option<String>,

        /// OAuth provider.
        provider: OAuthProvider,

        /// Access token from provider (for future API calls).
        access_token: String,

        /// Refresh token from provider (if available).
        refresh_token: Option<String>,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,
    },

    /// OAuth flow failed.
    OAuthFailed {
        /// Error code from provider.
        error: String,

        /// Human-readable error description.
        error_description: Option<String>,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Magic Link Flow
    // ═══════════════════════════════════════════════════════════════════════
    /// Send magic link to email.
    ///
    /// # Flow
    ///
    /// 1. Validate email format
    /// 2. Generate cryptographically secure token
    /// 3. Store hashed token in database
    /// 4. Send email with link
    SendMagicLink {
        /// Email address to send link to.
        email: String,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,
    },

    /// Magic link email sent successfully.
    ///
    /// This is an **event** from the email provider.
    MagicLinkSent {
        /// Email address.
        email: String,

        /// Token (stored hashed in DB, but kept in state for tests).
        token: String,

        /// Expiration timestamp.
        expires_at: DateTime<Utc>,
    },

    /// Verify magic link token.
    ///
    /// # Flow
    ///
    /// 1. Look up token in database
    /// 2. Verify not expired
    /// 3. Verify not already used
    /// 4. Create or get user
    /// 5. Create session
    /// 6. Invalidate token
    VerifyMagicLink {
        /// Token from email link.
        token: String,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,
    },

    /// Magic link verified successfully.
    MagicLinkVerified {
        /// User's email.
        email: String,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // WebAuthn / Passkey Flow
    // ═══════════════════════════════════════════════════════════════════════
    /// Initiate passkey registration.
    ///
    /// # Flow
    ///
    /// 1. Generate WebAuthn challenge
    /// 2. Return challenge to client
    /// 3. Client calls `navigator.credentials.create()`
    /// 4. Client sends attestation response
    InitiatePasskeyRegistration {
        /// User ID (must be logged in).
        user_id: UserId,

        /// Device name (e.g., "iPhone 15 Pro").
        device_name: String,
    },

    /// Complete passkey registration.
    ///
    /// # Flow
    ///
    /// 1. Verify attestation response
    /// 2. Extract public key
    /// 3. Store credential in database
    /// 4. Update device as passkey-enabled
    CompletePasskeyRegistration {
        /// User ID.
        user_id: UserId,

        /// Device ID.
        device_id: DeviceId,

        /// Credential ID from WebAuthn.
        credential_id: String,

        /// Public key (bytes).
        public_key: Vec<u8>,

        /// Attestation response (JSON).
        attestation_response: String,
    },

    /// Initiate passkey login.
    ///
    /// # Flow
    ///
    /// 1. User provides username/email
    /// 2. Look up user's passkeys
    /// 3. Generate WebAuthn challenge
    /// 4. Return challenge + credential IDs
    InitiatePasskeyLogin {
        /// Username or email.
        username: String,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,
    },

    /// Complete passkey login.
    ///
    /// # Flow
    ///
    /// 1. Verify assertion response
    /// 2. Verify signature with stored public key
    /// 3. Create session
    CompletePasskeyLogin {
        /// Credential ID used.
        credential_id: String,

        /// Assertion response (JSON).
        assertion_response: String,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,
    },

    /// Passkey login succeeded.
    PasskeyLoginSuccess {
        /// User ID.
        user_id: UserId,

        /// Device ID.
        device_id: DeviceId,

        /// Email.
        email: String,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Session Management
    // ═══════════════════════════════════════════════════════════════════════
    /// Session created successfully.
    ///
    /// This is the final **event** for all successful auth flows.
    SessionCreated {
        /// The new session.
        session: Session,
    },

    /// Validate existing session.
    ///
    /// Called on every authenticated request to:
    /// 1. Verify session exists in Redis
    /// 2. Check not expired
    /// 3. Update last_active timestamp
    /// 4. Refresh sliding expiration
    ValidateSession {
        /// Session ID from cookie.
        session_id: SessionId,

        /// Client IP address (for anomaly detection).
        ip_address: IpAddr,
    },

    /// Session validated successfully.
    SessionValidated {
        /// The session.
        session: Session,
    },

    /// Session expired (TTL reached or explicit expiration).
    SessionExpired {
        /// Session ID that expired.
        session_id: SessionId,
    },

    /// Logout (revoke session).
    ///
    /// # Flow
    ///
    /// 1. Remove session from Redis
    /// 2. Clear session cookie
    /// 3. Optionally publish logout event
    Logout {
        /// Session ID to revoke.
        session_id: SessionId,
    },

    /// Logout successful.
    LogoutSuccess {
        /// Session ID that was revoked.
        session_id: SessionId,
    },

    /// Revoke all sessions for a user.
    ///
    /// Used when:
    /// - User changes password
    /// - User reports account compromise
    /// - Admin action
    RevokeAllSessions {
        /// User ID.
        user_id: UserId,
    },

    /// Revoke specific device.
    ///
    /// Removes all sessions for a device and marks device as untrusted.
    RevokeDevice {
        /// User ID.
        user_id: UserId,

        /// Device ID to revoke.
        device_id: DeviceId,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Advanced Features (Phase 6B/6C)
    // ═══════════════════════════════════════════════════════════════════════
    /// Request step-up authentication.
    ///
    /// Used for sensitive operations (change password, view billing, etc.).
    RequestStepUp {
        /// Session ID.
        session_id: SessionId,

        /// Required authentication level.
        required_level: AuthLevel,

        /// Action being protected.
        protected_action: String,
    },

    /// Step-up authentication completed.
    StepUpCompleted {
        /// Session ID.
        session_id: SessionId,

        /// New authentication level.
        new_level: AuthLevel,

        /// Timestamp when step-up expires.
        expires_at: DateTime<Utc>,
    },

    /// Update device trust level.
    ///
    /// Progressive trust based on usage patterns.
    UpdateDeviceTrust {
        /// Device ID.
        device_id: DeviceId,

        /// New trust level.
        trust_level: DeviceTrustLevel,

        /// Reason for change.
        reason: String,
    },
}

/// Authentication level for step-up auth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AuthLevel {
    /// Basic authentication (password, magic link, OAuth).
    Basic = 1,

    /// Multi-factor authentication.
    MultiFactor = 2,

    /// Hardware-backed authentication (passkey, hardware token).
    HardwareBacked = 3,

    /// Biometric authentication.
    Biometric = 4,
}

/// Device trust level (progressive trust).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DeviceTrustLevel {
    /// Unknown device (first login).
    Unknown = 0,

    /// Recognized device (has logged in before).
    Recognized = 1,

    /// Familiar device (regular usage pattern, same location).
    Familiar = 2,

    /// Trusted device (passkey registered, long history).
    Trusted = 3,

    /// Highly trusted device (admin-approved, corporate device).
    HighlyTrusted = 4,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_level_ordering() {
        assert!(AuthLevel::MultiFactor > AuthLevel::Basic);
        assert!(AuthLevel::HardwareBacked > AuthLevel::MultiFactor);
        assert!(AuthLevel::Biometric > AuthLevel::HardwareBacked);
    }

    #[test]
    fn test_device_trust_ordering() {
        assert!(DeviceTrustLevel::Recognized > DeviceTrustLevel::Unknown);
        assert!(DeviceTrustLevel::Familiar > DeviceTrustLevel::Recognized);
        assert!(DeviceTrustLevel::Trusted > DeviceTrustLevel::Familiar);
        assert!(DeviceTrustLevel::HighlyTrusted > DeviceTrustLevel::Trusted);
    }

    #[test]
    fn test_action_serialization() {
        use std::net::Ipv4Addr;

        let action = AuthAction::InitiateOAuth {
            provider: OAuthProvider::Google,
            ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            user_agent: "Mozilla/5.0".to_string(),
        };

        // Should serialize/deserialize
        let json = serde_json::to_string(&action).expect("serialize");
        let deserialized: AuthAction = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(action, deserialized);
    }
}
