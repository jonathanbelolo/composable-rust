//! Authentication actions.
//!
//! This module defines all possible actions in the authentication system.
//! Actions follow the CQRS pattern: Commands (user intent) and Events (what happened).

use crate::providers::DeviceFingerprint;
use crate::state::{DeviceId, OAuthProvider, Session, SessionId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Authentication action.
///
/// This enum represents all possible inputs to the auth reducer:
/// - **Commands**: User requests (`InitiateOAuth`, `SendMagicLink`, etc.)
/// - **Events**: Results of async operations (`OAuthSuccess`, `EmailSent`, etc.)
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
    /// Initiate `OAuth` login flow.
    ///
    /// # Flow
    ///
    /// 1. Reducer generates `CSRF` state
    /// 2. Returns `Effect::RedirectToOAuthProvider`
    /// 3. User authorizes at provider
    /// 4. Provider redirects to callback with code
    InitiateOAuth {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// `OAuth` provider to use.
        provider: OAuthProvider,

        /// Client IP address for risk assessment.
        ip_address: IpAddr,

        /// User agent string for device fingerprinting.
        user_agent: String,

        /// Device fingerprint (optional, from client-side `FingerprintJS` or similar).
        fingerprint: Option<crate::providers::DeviceFingerprint>,
    },

    /// Handle `OAuth` callback.
    ///
    /// # Flow
    ///
    /// 1. Validate state parameter (`CSRF` protection)
    /// 2. Exchange code for access token
    /// 3. Fetch user info from provider
    /// 4. Create or link user account
    /// 5. Create session
    OAuthCallback {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// Authorization code from provider.
        code: String,

        /// State parameter (must match stored state).
        state: String,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,

        /// Device fingerprint (optional, from client-side `FingerprintJS` or similar).
        fingerprint: Option<crate::providers::DeviceFingerprint>,
    },

    /// `OAuth` token exchange succeeded.
    ///
    /// This is an **event** produced by the effect executor.
    OAuthSuccess {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User's email from provider.
        email: String,

        /// User's name from provider (if available).
        name: Option<String>,

        /// `OAuth` provider.
        provider: OAuthProvider,

        /// Provider's unique user ID (e.g., Google sub claim, GitHub user ID).
        provider_user_id: String,

        /// Access token from provider (for future API calls).
        access_token: String,

        /// Refresh token from provider (if available).
        refresh_token: Option<String>,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,

        /// Device fingerprint (if provided).
        fingerprint: Option<crate::providers::DeviceFingerprint>,
    },

    /// `OAuth` flow failed.
    OAuthFailed {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// Error code from provider.
        error: String,

        /// Human-readable error description.
        error_description: Option<String>,
    },

    /// `OAuth` authorization URL ready.
    ///
    /// This is an **event** produced by the reducer after `OAuth` state is stored.
    /// The web framework should intercept this action and perform an HTTP redirect (302).
    ///
    /// # Web Framework Integration
    ///
    /// When you receive this action, return HTTP 302 redirect:
    /// ```ignore
    /// HTTP/1.1 302 Found
    /// Location: <authorization_url>
    /// ```
    OAuthAuthorizationUrlReady {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// `OAuth` provider.
        provider: OAuthProvider,

        /// Full authorization URL to redirect to.
        authorization_url: String,
    },

    /// Refresh `OAuth` access token.
    ///
    /// This action triggers token refresh using the stored refresh token.
    ///
    /// # Flow
    ///
    /// 1. Get stored tokens from `OAuthTokenStore`
    /// 2. Call `OAuth` provider's `refresh_token()` method
    /// 3. Update stored tokens with new access token
    /// 4. Emit `OAuthTokenRefreshed` event
    ///
    /// # Errors
    ///
    /// - No refresh token exists
    /// - Refresh token is expired/invalid
    /// - Provider token endpoint fails
    RefreshOAuthToken {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User ID.
        user_id: UserId,

        /// `OAuth` provider.
        provider: OAuthProvider,
    },

    /// `OAuth` token refreshed successfully.
    ///
    /// This is an **event** produced by the effect executor.
    OAuthTokenRefreshed {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User ID.
        user_id: UserId,

        /// `OAuth` provider.
        provider: OAuthProvider,

        /// New access token.
        access_token: String,

        /// New expiration time (if provided).
        expires_at: Option<DateTime<Utc>>,
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
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

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
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

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
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// Token from email link.
        token: String,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,

        /// Browser fingerprint (optional, for risk scoring).
        fingerprint: Option<DeviceFingerprint>,
    },

    /// Magic link verified successfully.
    MagicLinkVerified {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User's email.
        email: String,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,

        /// Browser fingerprint (optional, for risk scoring).
        fingerprint: Option<DeviceFingerprint>,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // WebAuthn / Passkey Flow
    // ═══════════════════════════════════════════════════════════════════════
    /// Initiate passkey registration.
    ///
    /// # Flow
    ///
    /// 1. Generate `WebAuthn` challenge
    /// 2. Return challenge to client
    /// 3. Client calls `navigator.credentials.create()`
    /// 4. Client sends attestation response
    InitiatePasskeyRegistration {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

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
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User ID.
        user_id: UserId,

        /// Device ID.
        device_id: DeviceId,

        /// Credential ID from `WebAuthn`.
        credential_id: String,

        /// Public key (bytes).
        public_key: Vec<u8>,

        /// Attestation response (`JSON`).
        attestation_response: String,
    },

    /// Initiate passkey login.
    ///
    /// # Flow
    ///
    /// 1. User provides username/email
    /// 2. Look up user's passkeys
    /// 3. Generate `WebAuthn` challenge
    /// 4. Return challenge + credential IDs
    InitiatePasskeyLogin {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

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
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// Credential ID used.
        credential_id: String,

        /// Assertion response (`JSON`).
        assertion_response: String,

        /// Client IP address.
        ip_address: IpAddr,

        /// User agent string.
        user_agent: String,

        /// Browser fingerprint (optional, for risk scoring).
        fingerprint: Option<DeviceFingerprint>,
    },

    /// Passkey registration challenge generated.
    ///
    /// Returned after `InitiatePasskeyRegistration`.
    /// Contains `WebAuthn` challenge for client to use in `navigator.credentials.create()`.
    PasskeyRegistrationChallengeGenerated {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User ID.
        user_id: UserId,

        /// `WebAuthn` challenge (base64-encoded).
        challenge: String,

        /// Relying Party ID.
        rp_id: String,

        /// User email for `WebAuthn`.
        user_email: String,

        /// User display name for `WebAuthn`.
        user_display_name: String,
    },

    /// Passkey registration succeeded.
    ///
    /// Returned after `CompletePasskeyRegistration` successfully verifies attestation.
    PasskeyRegistrationSuccess {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User ID.
        user_id: UserId,

        /// Device ID.
        device_id: DeviceId,

        /// Credential ID.
        credential_id: String,
    },

    /// Passkey registration failed.
    ///
    /// Returned when attestation verification fails.
    PasskeyRegistrationFailed {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// Error message.
        error: String,
    },

    /// Passkey login challenge generated.
    ///
    /// Returned after `InitiatePasskeyLogin`.
    /// Contains `WebAuthn` challenge for client to use in `navigator.credentials.get()`.
    PasskeyLoginChallengeGenerated {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// `WebAuthn` challenge (base64-encoded).
        challenge: String,

        /// Allowed credential IDs for this user.
        allowed_credentials: Vec<String>,
    },

    /// Passkey login succeeded.
    PasskeyLoginSuccess {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

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

        /// Browser fingerprint (optional, for risk scoring).
        fingerprint: Option<DeviceFingerprint>,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Passkey Credential Management
    // ═══════════════════════════════════════════════════════════════════════
    /// List all passkey credentials for the authenticated user.
    ///
    /// # Returns
    ///
    /// Effect that queries the database and returns `PasskeyCredentialsListed` event.
    ///
    /// # Security
    ///
    /// This action should only be callable by authenticated users for their own credentials.
    /// The session validation ensures users can only list their own passkeys.
    ListPasskeyCredentials {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User ID (from authenticated session).
        user_id: UserId,
    },

    /// Passkey credentials list retrieved.
    ///
    /// This is an **event** produced by the effect executor.
    PasskeyCredentialsListed {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User ID.
        user_id: UserId,

        /// List of passkey credentials.
        credentials: Vec<crate::providers::PasskeyCredential>,
    },

    /// Delete a specific passkey credential.
    ///
    /// # Security
    ///
    /// - Only the credential owner can delete it (enforced by DB query with `user_id`)
    /// - Prevents users from deleting other users' credentials
    /// - Credential ID must belong to the `user_id`
    ///
    /// # Note
    ///
    /// This does NOT delete the device record - only the passkey credential.
    /// The device may still exist with other authentication methods.
    DeletePasskeyCredential {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User ID (from authenticated session).
        user_id: UserId,

        /// Credential ID to delete.
        credential_id: String,
    },

    /// Passkey credential deleted successfully.
    ///
    /// This is an **event** produced by the effect executor.
    PasskeyCredentialDeleted {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User ID.
        user_id: UserId,

        /// Credential ID that was deleted.
        credential_id: String,
    },

    /// Passkey credential deletion failed.
    ///
    /// This is an **event** for error cases.
    PasskeyCredentialDeletionFailed {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User ID.
        user_id: UserId,

        /// Credential ID that failed to delete.
        credential_id: String,

        /// Error message.
        error: String,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Session Management
    // ═══════════════════════════════════════════════════════════════════════
    /// Session created successfully.
    ///
    /// This is the final **event** for all successful auth flows.
    SessionCreated {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// The new session.
        session: Session,
    },

    /// Validate existing session.
    ///
    /// Called on every authenticated request to:
    /// 1. Verify session exists in Redis
    /// 2. Check not expired
    /// 3. Update `last_active` timestamp
    /// 4. Refresh sliding expiration
    ValidateSession {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// Session ID from cookie.
        session_id: SessionId,

        /// Client IP address (for anomaly detection).
        ip_address: IpAddr,
    },

    /// Session validated successfully.
    SessionValidated {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// The session.
        session: Session,
    },

    /// Session expired (`TTL` reached or explicit expiration).
    SessionExpired {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

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
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// Session ID to revoke.
        session_id: SessionId,
    },

    /// Logout successful.
    LogoutSuccess {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

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
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User ID.
        user_id: UserId,
    },

    /// Revoke specific device.
    ///
    /// Removes all sessions for a device and marks device as untrusted.
    RevokeDevice {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User ID.
        user_id: UserId,

        /// Device ID to revoke.
        device_id: DeviceId,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Error Actions
    // ═══════════════════════════════════════════════════════════════════════
    /// Magic link email failed to send.
    ///
    /// Triggered when the email provider returns an error.
    MagicLinkFailed {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// Email that failed.
        email: String,

        /// Error message from email provider.
        error: String,
    },

    /// Session creation failed.
    ///
    /// Triggered when `Redis` session store fails to create a session.
    SessionCreationFailed {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// User ID for which session creation failed.
        user_id: UserId,

        /// Device ID.
        device_id: DeviceId,

        /// Error message.
        error: String,
    },

    /// Event persistence failed.
    ///
    /// Triggered when event store fails to append events.
    EventPersistenceFailed {
        /// Stream ID that failed.
        stream_id: String,

        /// Error message.
        error: String,
    },

    /// Passkey authentication failed.
    ///
    /// Triggered when passkey authentication fails (validation, verification, etc.).
    PasskeyAuthenticationFailed {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// Error message.
        error: String,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Advanced Features (Phase 6B/6C)
    // ═══════════════════════════════════════════════════════════════════════
    /// Request step-up authentication.
    ///
    /// Used for sensitive operations (change password, view billing, etc.).
    RequestStepUp {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

        /// Session ID.
        session_id: SessionId,

        /// Required authentication level.
        required_level: AuthLevel,

        /// Action being protected.
        protected_action: String,
    },

    /// Step-up authentication completed.
    StepUpCompleted {
        /// Correlation ID for request tracing.
        correlation_id: uuid::Uuid,

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

    // ═══════════════════════════════════════════════════════════════════════
    // Event Sourcing
    // ═══════════════════════════════════════════════════════════════════════
    /// Event was persisted to the event store.
    ///
    /// This action is dispatched after successfully appending events to the event store.
    /// It triggers state updates by applying the event to the current state.
    EventPersisted {
        /// The event that was persisted.
        event: crate::events::AuthEvent,

        /// Version of the stream after persistence.
        version: u64,
    },
}

/// Authentication level for step-up auth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AuthLevel {
    /// Basic authentication (password, magic link, `OAuth`).
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
#[cfg_attr(feature = "postgres", derive(sqlx::Type))]
#[cfg_attr(feature = "postgres", sqlx(type_name = "device_trust_level", rename_all = "snake_case"))]
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
            correlation_id: uuid::Uuid::new_v4(),
            provider: OAuthProvider::Google,
            ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            user_agent: "Mozilla/5.0".to_string(),
            fingerprint: None,
        };

        // Should serialize/deserialize
        let json = serde_json::to_string(&action).expect("serialize");
        let deserialized: AuthAction = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(action, deserialized);
    }
}
