//! Error types for authentication and authorization operations.

use thiserror::Error;

/// Result type alias for authentication operations.
pub type Result<T> = std::result::Result<T, AuthError>;

/// Comprehensive error taxonomy for authentication and authorization.
///
/// This enum covers all possible failure modes in the auth system,
/// organized by category for clear error handling and user feedback.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum AuthError {
    // ═══════════════════════════════════════════════════════════
    // Authentication Errors
    // ═══════════════════════════════════════════════════════════

    /// Invalid credentials provided.
    #[error("Invalid credentials")]
    InvalidCredentials,

    /// Passkey not found for this device.
    #[error("Passkey not found")]
    PasskeyNotFound,

    /// Passkey verification failed.
    #[error("Passkey verification failed: {reason}")]
    PasskeyVerificationFailed {
        /// Reason for failure
        reason: String
    },

    /// Magic link has expired.
    #[error("Magic link has expired")]
    MagicLinkExpired,

    /// Magic link token is invalid.
    #[error("Invalid magic link token")]
    MagicLinkInvalid,

    /// Magic link has already been used.
    #[error("Magic link has already been used")]
    MagicLinkAlreadyUsed,

    /// OAuth authorization code is invalid.
    #[error("Invalid OAuth authorization code")]
    OAuthCodeInvalid,

    /// OAuth state parameter is invalid (CSRF protection).
    #[error("Invalid OAuth state parameter")]
    OAuthStateInvalid,

    // ═══════════════════════════════════════════════════════════
    // Authorization Errors
    // ═══════════════════════════════════════════════════════════

    /// User lacks required permissions.
    #[error("Insufficient permissions: {required}")]
    InsufficientPermissions {
        /// Required permission that was missing
        required: String
    },

    /// Requested resource not found.
    #[error("Resource not found")]
    ResourceNotFound,

    // ═══════════════════════════════════════════════════════════
    // Session Errors
    // ═══════════════════════════════════════════════════════════

    /// Session has expired.
    #[error("Session has expired")]
    SessionExpired,

    /// Session not found.
    #[error("Session not found")]
    SessionNotFound,

    /// Session has been revoked.
    #[error("Session has been revoked")]
    SessionRevoked,

    /// Refresh token is invalid.
    #[error("Invalid refresh token")]
    InvalidRefreshToken,

    // ═══════════════════════════════════════════════════════════
    // Rate Limiting
    // ═══════════════════════════════════════════════════════════

    /// Too many authentication attempts.
    #[error("Too many attempts, please retry after {retry_after:?}")]
    TooManyAttempts {
        /// Duration to wait before retrying
        retry_after: std::time::Duration
    },

    // ═══════════════════════════════════════════════════════════
    // WebAuthn Specific
    // ═══════════════════════════════════════════════════════════

    /// WebAuthn challenge has expired.
    #[error("WebAuthn challenge has expired")]
    ChallengeExpired,

    /// WebAuthn challenge not found.
    #[error("WebAuthn challenge not found")]
    ChallengeNotFound,

    /// WebAuthn origin mismatch (phishing protection).
    #[error("WebAuthn origin mismatch")]
    OriginMismatch,

    /// WebAuthn RP ID mismatch.
    #[error("WebAuthn RP ID mismatch")]
    RpIdMismatch,

    // ═══════════════════════════════════════════════════════════
    // System Errors
    // ═══════════════════════════════════════════════════════════

    /// Database operation failed.
    #[error("Database error: {0}")]
    DatabaseError(String),

    /// Email delivery failed.
    #[error("Failed to send email")]
    EmailDeliveryFailed,

    /// Internal server error (should not be exposed to users).
    #[error("Internal error")]
    InternalError,
}

impl AuthError {
    /// Returns `true` if this error is due to invalid user input.
    ///
    /// # Examples
    ///
    /// ```
    /// # use composable_rust_auth::AuthError;
    /// assert!(AuthError::InvalidCredentials.is_user_error());
    /// assert!(!AuthError::InternalError.is_user_error());
    /// ```
    pub const fn is_user_error(&self) -> bool {
        matches!(
            self,
            Self::InvalidCredentials
                | Self::MagicLinkInvalid
                | Self::OAuthCodeInvalid
                | Self::OAuthStateInvalid
                | Self::InsufficientPermissions { .. }
        )
    }

    /// Returns `true` if this error indicates a security issue.
    ///
    /// # Examples
    ///
    /// ```
    /// # use composable_rust_auth::AuthError;
    /// assert!(AuthError::OriginMismatch.is_security_issue());
    /// assert!(!AuthError::SessionExpired.is_security_issue());
    /// ```
    pub const fn is_security_issue(&self) -> bool {
        matches!(
            self,
            Self::OriginMismatch
                | Self::RpIdMismatch
                | Self::OAuthStateInvalid
                | Self::TooManyAttempts { .. }
        )
    }
}
