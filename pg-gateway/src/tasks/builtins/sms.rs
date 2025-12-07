//! SMS task executor.
//!
//! This module provides a generic SMS executor that can be used with
//! various SMS providers (Twilio, AWS SNS, etc.).

use crate::tasks::{TaskError, TaskExecutor};
use serde::Deserialize;
use std::future::Future;
use std::pin::Pin;

/// SMS task payload.
#[derive(Debug, Clone, Deserialize)]
pub struct SmsTask {
    /// Recipient phone number (E.164 format recommended).
    pub to: String,
    /// Message body.
    pub body: String,
    /// Optional sender ID or phone number.
    #[serde(default)]
    pub from: Option<String>,
}

/// Trait for SMS providers.
///
/// Implement this trait to integrate with your SMS provider.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::tasks::builtins::{SmsProvider, SmsTask};
/// use composable_rust_pg_gateway::tasks::TaskError;
///
/// struct TwilioProvider {
///     client: reqwest::Client,
///     account_sid: String,
///     auth_token: String,
///     from_number: String,
/// }
///
/// impl SmsProvider for TwilioProvider {
///     async fn send(&self, task: &SmsTask) -> Result<(), TaskError> {
///         let response = self.client
///             .post(format!(
///                 "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
///                 self.account_sid
///             ))
///             .basic_auth(&self.account_sid, Some(&self.auth_token))
///             .form(&[
///                 ("From", task.from.as_deref().unwrap_or(&self.from_number)),
///                 ("To", &task.to),
///                 ("Body", &task.body),
///             ])
///             .send()
///             .await
///             .map_err(|e| TaskError::temporary(format!("HTTP error: {e}")))?;
///
///         if response.status().is_success() {
///             Ok(())
///         } else if response.status().is_server_error() {
///             Err(TaskError::temporary(format!("Twilio error: {}", response.status())))
///         } else {
///             Err(TaskError::permanent(format!("Twilio rejected: {}", response.status())))
///         }
///     }
/// }
/// ```
pub trait SmsProvider: Send + Sync {
    /// Send an SMS message.
    ///
    /// # Errors
    ///
    /// Returns [`TaskError::Temporary`] for transient failures (network issues,
    /// rate limiting) and [`TaskError::Permanent`] for permanent failures
    /// (invalid number, authentication error).
    fn send(&self, task: &SmsTask) -> impl Future<Output = Result<(), TaskError>> + Send;
}

/// SMS executor that uses a configurable provider.
///
/// This executor delegates actual SMS sending to an [`SmsProvider`] implementation,
/// allowing you to use any SMS service (Twilio, AWS SNS, Vonage, etc.).
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::tasks::builtins::{SmsExecutor, SmsProvider, SmsTask};
/// use composable_rust_pg_gateway::tasks::TaskError;
///
/// // Implement your provider
/// struct MyProvider { /* ... */ }
/// impl SmsProvider for MyProvider {
///     async fn send(&self, task: &SmsTask) -> Result<(), TaskError> {
///         // Send via your SMS service
///         Ok(())
///     }
/// }
///
/// // Create executor
/// let provider = MyProvider { /* ... */ };
/// let executor = SmsExecutor::new(provider);
/// ```
pub struct SmsExecutor<P: SmsProvider> {
    provider: P,
}

impl<P: SmsProvider> SmsExecutor<P> {
    /// Create a new SMS executor with the given provider.
    #[must_use]
    pub const fn new(provider: P) -> Self {
        Self { provider }
    }

    /// Get a reference to the provider.
    #[must_use]
    pub const fn provider(&self) -> &P {
        &self.provider
    }
}

impl<P: SmsProvider + 'static> TaskExecutor for SmsExecutor<P> {
    fn task_type(&self) -> &'static str {
        "send_sms"
    }

    fn execute(
        &self,
        payload: serde_json::Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), TaskError>> + Send + '_>,
    > {
        Box::pin(async move {
            let task: SmsTask = serde_json::from_value(payload)
                .map_err(|e| TaskError::permanent(format!("Invalid payload: {e}")))?;

            // Validate phone number format (basic check)
            if task.to.is_empty() {
                return Err(TaskError::permanent("Empty recipient phone number"));
            }

            self.provider.send(&task).await
        })
    }
}

impl<P: SmsProvider> std::fmt::Debug for SmsExecutor<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmsExecutor")
            .field("provider", &std::any::type_name::<P>())
            .finish()
    }
}

/// A console SMS provider for development/testing.
///
/// This provider logs SMS messages to the console instead of sending them.
/// Useful for local development and testing.
///
/// # Example
///
/// ```rust
/// use composable_rust_pg_gateway::tasks::builtins::{SmsExecutor, ConsoleSmsProvider};
///
/// let executor = SmsExecutor::new(ConsoleSmsProvider);
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct ConsoleSmsProvider;

impl SmsProvider for ConsoleSmsProvider {
    async fn send(&self, task: &SmsTask) -> Result<(), TaskError> {
        tracing::info!(
            to = %task.to,
            from = ?task.from,
            body = %task.body,
            "SMS (console): Would send message"
        );
        println!(
            "📱 SMS to {}: {}",
            task.to,
            task.body
        );
        Ok(())
    }
}

/// Type-erased SMS executor for use with `Arc<dyn TaskExecutor>`.
///
/// This wrapper allows any `SmsProvider` to be used as a `TaskExecutor`
/// trait object.
pub struct DynSmsExecutor {
    provider: Box<dyn DynSmsProvider>,
}

impl DynSmsExecutor {
    /// Create a new dynamic SMS executor.
    pub fn new<P: SmsProvider + 'static>(provider: P) -> Self {
        Self {
            provider: Box::new(provider),
        }
    }
}

impl TaskExecutor for DynSmsExecutor {
    fn task_type(&self) -> &'static str {
        "send_sms"
    }

    fn execute(
        &self,
        payload: serde_json::Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), TaskError>> + Send + '_>,
    > {
        Box::pin(async move {
            let task: SmsTask = serde_json::from_value(payload)
                .map_err(|e| TaskError::permanent(format!("Invalid payload: {e}")))?;

            if task.to.is_empty() {
                return Err(TaskError::permanent("Empty recipient phone number"));
            }

            self.provider.send_dyn(&task).await
        })
    }
}

impl std::fmt::Debug for DynSmsExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynSmsExecutor").finish_non_exhaustive()
    }
}

/// Internal trait for type-erased SMS providers.
trait DynSmsProvider: Send + Sync {
    fn send_dyn<'a>(
        &'a self,
        task: &'a SmsTask,
    ) -> Pin<Box<dyn Future<Output = Result<(), TaskError>> + Send + 'a>>;
}

impl<P: SmsProvider> DynSmsProvider for P {
    fn send_dyn<'a>(
        &'a self,
        task: &'a SmsTask,
    ) -> Pin<Box<dyn Future<Output = Result<(), TaskError>> + Send + 'a>> {
        Box::pin(self.send(task))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn sms_task_deserialize() {
        let json = serde_json::json!({
            "to": "+1234567890",
            "body": "Hello, world!"
        });

        let task: SmsTask = serde_json::from_value(json).unwrap();
        assert_eq!(task.to, "+1234567890");
        assert_eq!(task.body, "Hello, world!");
        assert!(task.from.is_none());
    }

    #[test]
    fn sms_task_with_from() {
        let json = serde_json::json!({
            "to": "+1234567890",
            "body": "Hello!",
            "from": "+0987654321"
        });

        let task: SmsTask = serde_json::from_value(json).unwrap();
        assert_eq!(task.from, Some("+0987654321".to_string()));
    }

    #[tokio::test]
    async fn console_provider_succeeds() {
        let provider = ConsoleSmsProvider;
        let task = SmsTask {
            to: "+1234567890".to_string(),
            body: "Test message".to_string(),
            from: None,
        };

        let result = provider.send(&task).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn executor_rejects_empty_number() {
        let executor = SmsExecutor::new(ConsoleSmsProvider);
        let payload = serde_json::json!({
            "to": "",
            "body": "Test"
        });

        let result = executor.execute(payload).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().is_permanent());
    }

    #[tokio::test]
    async fn executor_rejects_invalid_payload() {
        let executor = SmsExecutor::new(ConsoleSmsProvider);
        let payload = serde_json::json!({
            "invalid": "payload"
        });

        let result = executor.execute(payload).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().is_permanent());
    }

    #[test]
    fn executor_task_type() {
        let executor = SmsExecutor::new(ConsoleSmsProvider);
        assert_eq!(executor.task_type(), "send_sms");
    }
}
