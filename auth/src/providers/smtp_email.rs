//! SMTP email provider implementation using Lettre.

use crate::error::{AuthError, Result};
use crate::providers::EmailProvider;
use chrono::{DateTime, Utc};
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};

/// SMTP email provider using Lettre.
///
/// This provider sends real emails via SMTP, suitable for production use.
///
/// # Configuration
///
/// - `smtp_server`: SMTP server address (e.g., "smtp.gmail.com")
/// - `smtp_port`: SMTP server port (usually 587 for TLS, 465 for SSL)
/// - `smtp_username`: SMTP authentication username
/// - `smtp_password`: SMTP authentication password
/// - `from_email`: Sender email address
/// - `from_name`: Sender display name
///
/// # Examples
///
/// ```ignore
/// use composable_rust_auth::providers::SmtpEmailProvider;
///
/// let provider = SmtpEmailProvider::new(
///     "smtp.gmail.com".to_string(),
///     587,
///     "user@gmail.com".to_string(),
///     "app_password".to_string(),
///     "noreply@example.com".to_string(),
///     "Example App".to_string(),
/// )?;
/// ```
#[derive(Clone)]
pub struct SmtpEmailProvider {
    /// SMTP server address.
    smtp_server: String,

    /// SMTP server port.
    smtp_port: u16,

    /// SMTP credentials.
    credentials: Credentials,

    /// Sender email address.
    from_email: String,

    /// Sender display name.
    from_name: String,
}

impl SmtpEmailProvider {
    /// Create a new SMTP email provider.
    ///
    /// # Arguments
    ///
    /// - `smtp_server`: SMTP server address
    /// - `smtp_port`: SMTP server port
    /// - `smtp_username`: SMTP authentication username
    /// - `smtp_password`: SMTP authentication password
    /// - `from_email`: Sender email address
    /// - `from_name`: Sender display name
    ///
    /// # Errors
    ///
    /// Returns error if configuration is invalid.
    pub fn new(
        smtp_server: String,
        smtp_port: u16,
        smtp_username: String,
        smtp_password: String,
        from_email: String,
        from_name: String,
    ) -> Result<Self> {
        let credentials = Credentials::new(smtp_username, smtp_password);

        Ok(Self {
            smtp_server,
            smtp_port,
            credentials,
            from_email,
            from_name,
        })
    }

    /// Build SMTP transport for sending emails.
    ///
    /// Creates a new transport for each email to avoid connection pooling issues.
    ///
    /// # Errors
    ///
    /// Returns error if SMTP connection fails.
    fn build_transport(&self) -> Result<SmtpTransport> {
        SmtpTransport::relay(&self.smtp_server)
            .map_err(|e| AuthError::EmailError(format!("SMTP relay error: {e}")))?
            .port(self.smtp_port)
            .credentials(self.credentials.clone())
            .build()
            .pipe(Ok)
    }

    /// Build the "From" header.
    fn from_header(&self) -> String {
        format!("{} <{}>", self.from_name, self.from_email)
    }
}

impl EmailProvider for SmtpEmailProvider {
    async fn send_magic_link(
        &self,
        to: &str,
        token: &str,
        base_url: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        let magic_link = format!("{base_url}?token={token}");
        let expires_minutes = (expires_at - Utc::now()).num_minutes();

        let html_body = format!(
            r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>Sign in to your account</title>
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <div style="max-width: 600px; margin: 0 auto; padding: 20px;">
        <h2 style="color: #2563eb;">Sign in to your account</h2>
        <p>Click the link below to sign in to your account. This link will expire in {expires_minutes} minutes.</p>
        <p style="margin: 30px 0;">
            <a href="{magic_link}"
               style="display: inline-block; background-color: #2563eb; color: white; padding: 12px 24px; text-decoration: none; border-radius: 4px;">
                Sign In
            </a>
        </p>
        <p style="color: #666; font-size: 14px;">
            If you didn't request this email, you can safely ignore it.
        </p>
        <p style="color: #666; font-size: 12px; margin-top: 40px;">
            Or copy and paste this link into your browser:<br>
            {magic_link}
        </p>
    </div>
</body>
</html>
            "#
        );

        let email = Message::builder()
            .from(
                self.from_header()
                    .parse()
                    .map_err(|e| AuthError::EmailError(format!("Invalid from address: {e}")))?,
            )
            .to(to
                .parse()
                .map_err(|e| AuthError::EmailError(format!("Invalid to address: {e}")))?)
            .subject("Sign in to your account")
            .header(ContentType::TEXT_HTML)
            .body(html_body)
            .map_err(|e| AuthError::EmailError(format!("Failed to build email: {e}")))?;

        let mailer = self.build_transport()?;

        // Send email
        tokio::task::spawn_blocking(move || {
            mailer
                .send(&email)
                .map_err(|e| AuthError::EmailError(format!("Failed to send email: {e}")))
        })
        .await
        .map_err(|e| AuthError::EmailError(format!("Email task failed: {e}")))?
        .map(|_| ())
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

        let html_body = format!(
            r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>Reset your password</title>
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <div style="max-width: 600px; margin: 0 auto; padding: 20px;">
        <h2 style="color: #dc2626;">Reset your password</h2>
        <p>Click the link below to reset your password. This link will expire in {expires_minutes} minutes.</p>
        <p style="margin: 30px 0;">
            <a href="{reset_link}"
               style="display: inline-block; background-color: #dc2626; color: white; padding: 12px 24px; text-decoration: none; border-radius: 4px;">
                Reset Password
            </a>
        </p>
        <p style="color: #666; font-size: 14px;">
            If you didn't request this password reset, please ignore this email. Your password will not be changed.
        </p>
        <p style="color: #666; font-size: 12px; margin-top: 40px;">
            Or copy and paste this link into your browser:<br>
            {reset_link}
        </p>
    </div>
</body>
</html>
            "#
        );

        let email = Message::builder()
            .from(
                self.from_header()
                    .parse()
                    .map_err(|e| AuthError::EmailError(format!("Invalid from address: {e}")))?,
            )
            .to(to
                .parse()
                .map_err(|e| AuthError::EmailError(format!("Invalid to address: {e}")))?)
            .subject("Reset your password")
            .header(ContentType::TEXT_HTML)
            .body(html_body)
            .map_err(|e| AuthError::EmailError(format!("Failed to build email: {e}")))?;

        let mailer = self.build_transport()?;

        tokio::task::spawn_blocking(move || {
            mailer
                .send(&email)
                .map_err(|e| AuthError::EmailError(format!("Failed to send email: {e}")))
        })
        .await
        .map_err(|e| AuthError::EmailError(format!("Email task failed: {e}")))?
        .map(|_| ())
    }

    async fn send_verification_email(&self, to: &str, token: &str, base_url: &str) -> Result<()> {
        let verification_link = format!("{base_url}?token={token}");

        let html_body = format!(
            r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>Verify your email address</title>
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <div style="max-width: 600px; margin: 0 auto; padding: 20px;">
        <h2 style="color: #2563eb;">Verify your email address</h2>
        <p>Welcome! Please verify your email address by clicking the link below:</p>
        <p style="margin: 30px 0;">
            <a href="{verification_link}"
               style="display: inline-block; background-color: #2563eb; color: white; padding: 12px 24px; text-decoration: none; border-radius: 4px;">
                Verify Email
            </a>
        </p>
        <p style="color: #666; font-size: 14px;">
            If you didn't create an account, you can safely ignore this email.
        </p>
        <p style="color: #666; font-size: 12px; margin-top: 40px;">
            Or copy and paste this link into your browser:<br>
            {verification_link}
        </p>
    </div>
</body>
</html>
            "#
        );

        let email = Message::builder()
            .from(
                self.from_header()
                    .parse()
                    .map_err(|e| AuthError::EmailError(format!("Invalid from address: {e}")))?,
            )
            .to(to
                .parse()
                .map_err(|e| AuthError::EmailError(format!("Invalid to address: {e}")))?)
            .subject("Verify your email address")
            .header(ContentType::TEXT_HTML)
            .body(html_body)
            .map_err(|e| AuthError::EmailError(format!("Failed to build email: {e}")))?;

        let mailer = self.build_transport()?;

        tokio::task::spawn_blocking(move || {
            mailer
                .send(&email)
                .map_err(|e| AuthError::EmailError(format!("Failed to send email: {e}")))
        })
        .await
        .map_err(|e| AuthError::EmailError(format!("Email task failed: {e}")))?
        .map(|_| ())
    }

    async fn send_security_alert(&self, to: &str, subject: &str, message: &str) -> Result<()> {
        let html_body = format!(
            r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>Security Alert</title>
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <div style="max-width: 600px; margin: 0 auto; padding: 20px;">
        <h2 style="color: #dc2626;">Security Alert</h2>
        <div style="background-color: #fef2f2; border-left: 4px solid #dc2626; padding: 15px; margin: 20px 0;">
            <p style="margin: 0;">{message}</p>
        </div>
        <p style="color: #666; font-size: 14px;">
            If you didn't perform this action, please secure your account immediately.
        </p>
    </div>
</body>
</html>
            "#
        );

        let email = Message::builder()
            .from(
                self.from_header()
                    .parse()
                    .map_err(|e| AuthError::EmailError(format!("Invalid from address: {e}")))?,
            )
            .to(to
                .parse()
                .map_err(|e| AuthError::EmailError(format!("Invalid to address: {e}")))?)
            .subject(subject)
            .header(ContentType::TEXT_HTML)
            .body(html_body)
            .map_err(|e| AuthError::EmailError(format!("Failed to build email: {e}")))?;

        let mailer = self.build_transport()?;

        tokio::task::spawn_blocking(move || {
            mailer
                .send(&email)
                .map_err(|e| AuthError::EmailError(format!("Failed to send email: {e}")))
        })
        .await
        .map_err(|e| AuthError::EmailError(format!("Email task failed: {e}")))?
        .map(|_| ())
    }
}

/// Helper trait to enable `.pipe()` method for ergonomic Ok wrapping.
trait Pipe: Sized {
    /// Pipe a value through a function.
    fn pipe<F, T>(self, f: F) -> T
    where
        F: FnOnce(Self) -> T,
    {
        f(self)
    }
}

impl<T> Pipe for T {}
