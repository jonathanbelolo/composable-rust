//! Console email provider for ticketing system
//!
//! Prints emails to stdout for demo/development purposes.
//! In production, replace with SMTP or cloud email service.

use composable_rust_auth::{providers::EmailProvider, Result};
use chrono::{DateTime, Utc};
use tracing::info;

/// Console email provider (prints to stdout for demo purposes)
#[derive(Debug, Clone)]
pub struct ConsoleEmailProvider {
    /// Base URL for constructing links
    #[allow(dead_code)] // Used by EmailProvider trait methods
    base_url: String,
}

impl ConsoleEmailProvider {
    /// Create a new console email provider
    ///
    /// # Arguments
    /// * `base_url` - Base URL for constructing magic links (e.g., `http://localhost:8080`)
    #[must_use]
    pub fn new(base_url: String) -> Self {
        Self { base_url }
    }
}

impl EmailProvider for ConsoleEmailProvider {
    async fn send_magic_link(
        &self,
        to: &str,
        token: &str,
        base_url: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        let magic_link = format!("{}/auth/magic-link/verify?token={}", base_url, token);
        let expires_in = (expires_at - Utc::now()).num_minutes();

        info!(
            "\n\n\
            ┌────────────────────────────────────────────────────────────────┐\n\
            │                     Magic Link Email                           │\n\
            ├────────────────────────────────────────────────────────────────┤\n\
            │ To: {:<58} │\n\
            │                                                                │\n\
            │ Click the link below to sign in to Ticketing System:          │\n\
            │                                                                │\n\
            │ {}  \n\
            │                                                                │\n\
            │ This link will expire in {} minutes.                          │\n\
            │                                                                │\n\
            │ If you didn't request this link, you can safely ignore it.    │\n\
            └────────────────────────────────────────────────────────────────┘\n",
            to, magic_link, expires_in
        );

        Ok(())
    }

    async fn send_password_reset(
        &self,
        to: &str,
        token: &str,
        base_url: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        let reset_link = format!("{}/auth/reset-password?token={}", base_url, token);
        let expires_in = (expires_at - Utc::now()).num_minutes();

        info!(
            "\n\n\
            ┌────────────────────────────────────────────────────────────────┐\n\
            │                  Password Reset Email                          │\n\
            ├────────────────────────────────────────────────────────────────┤\n\
            │ To: {:<58} │\n\
            │                                                                │\n\
            │ Click the link below to reset your password:                  │\n\
            │                                                                │\n\
            │ {}  \n\
            │                                                                │\n\
            │ This link will expire in {} minutes.                          │\n\
            │                                                                │\n\
            │ If you didn't request this, please ignore this email.         │\n\
            └────────────────────────────────────────────────────────────────┘\n",
            to, reset_link, expires_in
        );

        Ok(())
    }

    async fn send_verification_email(
        &self,
        to: &str,
        token: &str,
        base_url: &str,
    ) -> Result<()> {
        let verification_link = format!("{}/auth/verify-email?token={}", base_url, token);

        info!(
            "\n\n\
            ┌────────────────────────────────────────────────────────────────┐\n\
            │                  Email Verification                            │\n\
            ├────────────────────────────────────────────────────────────────┤\n\
            │ To: {:<58} │\n\
            │                                                                │\n\
            │ Please verify your email address by clicking the link below:  │\n\
            │                                                                │\n\
            │ {}  \n\
            │                                                                │\n\
            │ Thank you for registering!                                    │\n\
            └────────────────────────────────────────────────────────────────┘\n",
            to, verification_link
        );

        Ok(())
    }

    async fn send_security_alert(
        &self,
        to: &str,
        subject: &str,
        message: &str,
    ) -> Result<()> {
        info!(
            "\n\n\
            ┌────────────────────────────────────────────────────────────────┐\n\
            │                     Security Alert                             │\n\
            ├────────────────────────────────────────────────────────────────┤\n\
            │ To: {:<58} │\n\
            │ Subject: {:<54} │\n\
            │                                                                │\n\
            │ {}                                                             │\n\
            │                                                                │\n\
            │ If this wasn't you, please secure your account immediately.   │\n\
            └────────────────────────────────────────────────────────────────┘\n",
            to, subject, message
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[allow(clippy::unwrap_used)] // Test code can use unwrap
    async fn test_console_email_provider() {
        let provider = ConsoleEmailProvider::new("http://localhost:8080".to_string());

        let expires_at = Utc::now() + chrono::Duration::minutes(15);

        let result = provider
            .send_magic_link(
                "test@example.com",
                "test-token-123",
                "http://localhost:8080",
                expires_at,
            )
            .await;

        assert!(result.is_ok());
    }
}
