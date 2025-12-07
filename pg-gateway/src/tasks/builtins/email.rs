//! Email task executor using lettre.
//!
//! This module provides an email executor that sends emails via SMTP.
//! It supports template rendering using Tera.

use crate::tasks::{TaskError, TaskExecutor};
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::Deserialize;
use std::sync::Arc;
use tera::{Context, Tera};

/// Email task payload.
#[derive(Debug, Clone, Deserialize)]
pub struct EmailTask {
    /// Template name to render.
    pub template: String,
    /// Recipient email address.
    pub to: String,
    /// Email subject.
    pub subject: String,
    /// Template data.
    #[serde(default)]
    pub data: serde_json::Value,
}

/// Configuration for the email executor.
#[derive(Debug, Clone)]
pub struct EmailConfig {
    /// SMTP server host.
    pub smtp_host: String,
    /// SMTP server port.
    pub smtp_port: u16,
    /// SMTP username.
    pub smtp_username: String,
    /// SMTP password.
    pub smtp_password: String,
    /// Use TLS (STARTTLS).
    pub use_tls: bool,
    /// From email address.
    pub from_email: String,
    /// From name (optional).
    pub from_name: Option<String>,
}

impl EmailConfig {
    /// Create a new email configuration.
    #[must_use]
    pub fn new(
        smtp_host: impl Into<String>,
        smtp_username: impl Into<String>,
        smtp_password: impl Into<String>,
        from_email: impl Into<String>,
    ) -> Self {
        Self {
            smtp_host: smtp_host.into(),
            smtp_port: 587,
            smtp_username: smtp_username.into(),
            smtp_password: smtp_password.into(),
            use_tls: true,
            from_email: from_email.into(),
            from_name: None,
        }
    }

    /// Create configuration from environment variables.
    ///
    /// Reads from:
    /// - `SMTP_HOST`
    /// - `SMTP_PORT` (optional, default 587)
    /// - `SMTP_USERNAME`
    /// - `SMTP_PASSWORD`
    /// - `SMTP_FROM_EMAIL`
    /// - `SMTP_FROM_NAME` (optional)
    /// - `SMTP_USE_TLS` (optional, default true)
    ///
    /// # Errors
    ///
    /// Returns an error if required environment variables are missing.
    pub fn from_env() -> Result<Self, String> {
        let smtp_host =
            std::env::var("SMTP_HOST").map_err(|_| "SMTP_HOST not set")?;
        let smtp_port = std::env::var("SMTP_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(587);
        let smtp_username =
            std::env::var("SMTP_USERNAME").map_err(|_| "SMTP_USERNAME not set")?;
        let smtp_password =
            std::env::var("SMTP_PASSWORD").map_err(|_| "SMTP_PASSWORD not set")?;
        let from_email =
            std::env::var("SMTP_FROM_EMAIL").map_err(|_| "SMTP_FROM_EMAIL not set")?;
        let from_name = std::env::var("SMTP_FROM_NAME").ok();
        let use_tls = std::env::var("SMTP_USE_TLS")
            .map(|v| v != "false" && v != "0")
            .unwrap_or(true);

        Ok(Self {
            smtp_host,
            smtp_port,
            smtp_username,
            smtp_password,
            use_tls,
            from_email,
            from_name,
        })
    }

    /// Set the SMTP port.
    #[must_use]
    pub const fn with_port(mut self, port: u16) -> Self {
        self.smtp_port = port;
        self
    }

    /// Set TLS mode.
    #[must_use]
    pub const fn with_tls(mut self, use_tls: bool) -> Self {
        self.use_tls = use_tls;
        self
    }

    /// Set the from name.
    #[must_use]
    pub fn with_from_name(mut self, name: impl Into<String>) -> Self {
        self.from_name = Some(name.into());
        self
    }
}

/// Email executor that sends emails via SMTP.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::tasks::builtins::{EmailExecutor, EmailConfig};
/// use tera::Tera;
///
/// let config = EmailConfig::from_env()?;
/// let mut templates = Tera::default();
/// templates.add_raw_template("magic_link", "Click here: {{ link }}")?;
///
/// let executor = EmailExecutor::new(config, templates)?;
/// ```
pub struct EmailExecutor {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    templates: Arc<Tera>,
}

impl EmailExecutor {
    /// Create a new email executor.
    ///
    /// # Errors
    ///
    /// Returns an error if the SMTP transport cannot be created or
    /// the from email address is invalid.
    pub fn new(config: EmailConfig, templates: Tera) -> Result<Self, String> {
        let creds = Credentials::new(config.smtp_username, config.smtp_password);

        let transport = if config.use_tls {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.smtp_host)
                .map_err(|e| format!("Failed to create SMTP transport: {e}"))?
                .credentials(creds)
                .port(config.smtp_port)
                .build()
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.smtp_host)
                .credentials(creds)
                .port(config.smtp_port)
                .build()
        };

        let from: Mailbox = if let Some(name) = config.from_name {
            format!("{name} <{}>", config.from_email)
                .parse()
                .map_err(|e| format!("Invalid from email: {e}"))?
        } else {
            config
                .from_email
                .parse()
                .map_err(|e| format!("Invalid from email: {e}"))?
        };

        Ok(Self {
            transport,
            from,
            templates: Arc::new(templates),
        })
    }

    /// Create an email executor with shared templates.
    ///
    /// # Errors
    ///
    /// Returns an error if the SMTP transport cannot be created or
    /// the from email address is invalid.
    pub fn with_shared_templates(
        config: EmailConfig,
        templates: Arc<Tera>,
    ) -> Result<Self, String> {
        let creds = Credentials::new(config.smtp_username, config.smtp_password);

        let transport = if config.use_tls {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.smtp_host)
                .map_err(|e| format!("Failed to create SMTP transport: {e}"))?
                .credentials(creds)
                .port(config.smtp_port)
                .build()
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.smtp_host)
                .credentials(creds)
                .port(config.smtp_port)
                .build()
        };

        let from: Mailbox = if let Some(name) = config.from_name {
            format!("{name} <{}>", config.from_email)
                .parse()
                .map_err(|e| format!("Invalid from email: {e}"))?
        } else {
            config
                .from_email
                .parse()
                .map_err(|e| format!("Invalid from email: {e}"))?
        };

        Ok(Self {
            transport,
            from,
            templates,
        })
    }
}

impl TaskExecutor for EmailExecutor {
    fn task_type(&self) -> &'static str {
        "send_email"
    }

    fn execute(
        &self,
        payload: serde_json::Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), TaskError>> + Send + '_>,
    > {
        Box::pin(async move {
            let task: EmailTask = serde_json::from_value(payload)
                .map_err(|e| TaskError::permanent(format!("Invalid payload: {e}")))?;

            // Render template
            let context = Context::from_value(task.data)
                .map_err(|e| TaskError::permanent(format!("Invalid template data: {e}")))?;

            let body = self
                .templates
                .render(&task.template, &context)
                .map_err(|e| TaskError::permanent(format!("Template error: {e}")))?;

            // Parse recipient
            let to: Mailbox = task
                .to
                .parse()
                .map_err(|e| TaskError::permanent(format!("Invalid recipient email: {e}")))?;

            // Build email
            let email = Message::builder()
                .from(self.from.clone())
                .to(to)
                .subject(task.subject)
                .body(body)
                .map_err(|e| TaskError::permanent(format!("Failed to build email: {e}")))?;

            // Send email
            self.transport.send(email).await.map_err(|e| {
                // SMTP errors are usually temporary (connection issues, rate limiting)
                TaskError::temporary(format!("SMTP error: {e}"))
            })?;

            Ok(())
        })
    }
}

impl std::fmt::Debug for EmailExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmailExecutor")
            .field("from", &self.from)
            .field("templates", &"[Tera]")
            .field("transport", &"[SmtpTransport]")
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn email_task_deserialize() {
        let json = serde_json::json!({
            "template": "welcome",
            "to": "user@example.com",
            "subject": "Welcome!",
            "data": {
                "name": "John"
            }
        });

        let task: EmailTask = serde_json::from_value(json).unwrap();
        assert_eq!(task.template, "welcome");
        assert_eq!(task.to, "user@example.com");
        assert_eq!(task.subject, "Welcome!");
    }

    #[test]
    fn email_task_default_data() {
        let json = serde_json::json!({
            "template": "simple",
            "to": "user@example.com",
            "subject": "Hello"
        });

        let task: EmailTask = serde_json::from_value(json).unwrap();
        assert!(task.data.is_null());
    }

    #[test]
    fn email_config_new() {
        let config = EmailConfig::new(
            "smtp.example.com",
            "user",
            "pass",
            "noreply@example.com",
        );

        assert_eq!(config.smtp_host, "smtp.example.com");
        assert_eq!(config.smtp_port, 587);
        assert!(config.use_tls);
    }

    #[test]
    fn email_config_builder() {
        let config = EmailConfig::new("smtp.example.com", "user", "pass", "noreply@example.com")
            .with_port(465)
            .with_tls(false)
            .with_from_name("My App");

        assert_eq!(config.smtp_port, 465);
        assert!(!config.use_tls);
        assert_eq!(config.from_name, Some("My App".to_string()));
    }
}
