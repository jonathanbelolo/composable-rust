//! Mock email provider for testing.

use crate::error::Result;
use crate::providers::EmailProvider;
use chrono::{DateTime, Utc};
use std::future::Future;

/// Mock email provider.
///
/// Simulates email delivery without actually sending emails.
#[derive(Debug, Clone, Default)]
pub struct MockEmailProvider {
    /// Whether to simulate success or failure.
    pub should_succeed: bool,
}

impl MockEmailProvider {
    /// Create a new mock email provider that succeeds.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            should_succeed: true,
        }
    }
}

impl EmailProvider for MockEmailProvider {
    fn send_magic_link(
        &self,
        _to: &str,
        _token: &str,
        _base_url: &str,
        _expires_at: DateTime<Utc>,
    ) -> impl Future<Output = Result<()>> + Send {
        async move { Ok(()) }
    }

    fn send_password_reset(
        &self,
        _to: &str,
        _token: &str,
        _base_url: &str,
        _expires_at: DateTime<Utc>,
    ) -> impl Future<Output = Result<()>> + Send {
        async move { Ok(()) }
    }

    fn send_verification_email(
        &self,
        _to: &str,
        _token: &str,
        _base_url: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        async move { Ok(()) }
    }

    fn send_security_alert(
        &self,
        _to: &str,
        _subject: &str,
        _message: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        async move { Ok(()) }
    }
}
