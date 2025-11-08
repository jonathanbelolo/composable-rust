//! Email provider trait.

use crate::error::Result;
use chrono::{DateTime, Utc};

/// Email provider.
///
/// This trait abstracts over email delivery services
/// (SendGrid, AWS SES, Postmark, etc.).
pub trait EmailProvider: Send + Sync {
    /// Send magic link email.
    ///
    /// # Arguments
    ///
    /// - `to`: Recipient email address
    /// - `token`: Magic link token
    /// - `base_url`: Base URL for magic link (e.g., "https://app.example.com/auth/verify")
    /// - `expires_at`: Token expiration timestamp
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Network request fails
    /// - Email provider rejects the request
    /// - Email is invalid
    async fn send_magic_link(
        &self,
        to: &str,
        token: &str,
        base_url: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()>;

    /// Send password reset email (future).
    ///
    /// Not implemented in Phase 6 (passwordless-first).
    async fn send_password_reset(
        &self,
        to: &str,
        token: &str,
        base_url: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()>;

    /// Send account verification email.
    ///
    /// # Arguments
    ///
    /// - `to`: Recipient email address
    /// - `token`: Verification token
    /// - `base_url`: Base URL for verification link
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Network request fails
    /// - Email provider rejects the request
    async fn send_verification_email(
        &self,
        to: &str,
        token: &str,
        base_url: &str,
    ) -> Result<()>;

    /// Send security alert email.
    ///
    /// # Arguments
    ///
    /// - `to`: Recipient email address
    /// - `subject`: Alert subject
    /// - `message`: Alert message
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Network request fails
    /// - Email provider rejects the request
    async fn send_security_alert(
        &self,
        to: &str,
        subject: &str,
        message: &str,
    ) -> Result<()>;
}
