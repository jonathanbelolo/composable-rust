//! Console email provider for development and testing.

use crate::error::Result;
use crate::providers::EmailProvider;
use chrono::{DateTime, Utc};
use tracing::{info, warn};

/// Console email provider.
///
/// This provider logs emails to the console instead of sending them.
/// Useful for development and testing where you don't want to send real emails.
///
/// # Examples
///
/// ```ignore
/// use composable_rust_auth::providers::ConsoleEmailProvider;
///
/// let provider = ConsoleEmailProvider::new();
/// provider.send_magic_link(
///     "user@example.com",
///     "abc123",
///     `<https://app.example.com/auth/verify>`,
///     Utc::now() + chrono::Duration::minutes(15),
/// ).await?;
/// ```
#[derive(Clone, Debug, Default)]
pub struct ConsoleEmailProvider;

impl ConsoleEmailProvider {
    /// Create a new console email provider.
    #[must_use]
    pub const fn new() -> Self {
        Self
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
        let magic_link = format!("{base_url}?token={token}");
        let expires_minutes = (expires_at - Utc::now()).num_minutes();

        info!(
            to = %to,
            token = %token,
            expires_in = %expires_minutes,
            "📧 Magic Link Email (Development Mode)"
        );
        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║                   MAGIC LINK EMAIL                           ║");
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║ To: {to:<57}║");
        println!("║ Subject: Sign in to your account{:<30}║", "");
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║                                                              ║");
        println!("║ Click the link below to sign in to your account.            ║");
        println!("║ This link will expire in {expires_minutes} minutes.{:<23}║", "");
        println!("║                                                              ║");
        println!("║ Magic Link:                                                  ║");
        println!("║ {magic_link:<61}║");
        println!("║                                                              ║");
        println!("╚══════════════════════════════════════════════════════════════╝\n");

        Ok(())
    }

    async fn send_password_reset(
        &self,
        to: &str,
        token: &str,
        base_url: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        let reset_link = format!("{base_url}?token={token}");
        let expires_minutes = (expires_at - Utc::now()).num_minutes();

        info!(
            to = %to,
            token = %token,
            expires_in = %expires_minutes,
            "📧 Password Reset Email (Development Mode)"
        );
        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║                PASSWORD RESET EMAIL                          ║");
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║ To: {to:<57}║");
        println!("║ Subject: Reset your password{:<34}║", "");
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║                                                              ║");
        println!("║ Click the link below to reset your password.                ║");
        println!("║ This link will expire in {expires_minutes} minutes.{:<23}║", "");
        println!("║                                                              ║");
        println!("║ Reset Link:                                                  ║");
        println!("║ {reset_link:<61}║");
        println!("║                                                              ║");
        println!("╚══════════════════════════════════════════════════════════════╝\n");

        Ok(())
    }

    async fn send_verification_email(&self, to: &str, token: &str, base_url: &str) -> Result<()> {
        let verification_link = format!("{base_url}?token={token}");

        info!(
            to = %to,
            token = %token,
            "📧 Verification Email (Development Mode)"
        );
        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║               EMAIL VERIFICATION                             ║");
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║ To: {to:<57}║");
        println!("║ Subject: Verify your email address{:<27}║", "");
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║                                                              ║");
        println!("║ Welcome! Please verify your email address by clicking       ║");
        println!("║ the link below:                                              ║");
        println!("║                                                              ║");
        println!("║ Verification Link:                                           ║");
        println!("║ {verification_link:<61}║");
        println!("║                                                              ║");
        println!("╚══════════════════════════════════════════════════════════════╝\n");

        Ok(())
    }

    async fn send_security_alert(&self, to: &str, subject: &str, message: &str) -> Result<()> {
        warn!(
            to = %to,
            subject = %subject,
            "🚨 Security Alert Email (Development Mode)"
        );
        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║                   SECURITY ALERT                             ║");
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║ To: {to:<57}║");
        println!("║ Subject: {subject:<51}║");
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║                                                              ║");

        // Word wrap message to fit in box
        for line in message.lines() {
            let mut remaining = line;
            while !remaining.is_empty() {
                let chunk_len = remaining.len().min(60);
                let chunk = &remaining[..chunk_len];
                println!("║ {chunk:<61}║");
                remaining = &remaining[chunk_len..];
            }
        }

        println!("║                                                              ║");
        println!("╚══════════════════════════════════════════════════════════════╝\n");

        Ok(())
    }
}
