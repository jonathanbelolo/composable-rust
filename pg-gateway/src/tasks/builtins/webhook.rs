//! Webhook task executor.
//!
//! This module provides an HTTP webhook executor for sending
//! event notifications to external services.

use crate::tasks::{TaskError, TaskExecutor};
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Webhook task payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookTask {
    /// Target URL for the webhook.
    pub url: String,
    /// HTTP method (default: POST).
    #[serde(default = "default_method")]
    pub method: String,
    /// Request headers.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Request body (JSON).
    #[serde(default)]
    pub body: serde_json::Value,
    /// Timeout in seconds (default: 30).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_method() -> String {
    "POST".to_string()
}

const fn default_timeout() -> u64 {
    30
}

/// Configuration for the webhook executor.
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    /// Default timeout for requests.
    pub default_timeout: Duration,
    /// Maximum number of redirects to follow.
    pub max_redirects: usize,
    /// User-Agent header value.
    pub user_agent: String,
    /// Default headers to include in all requests.
    pub default_headers: HashMap<String, String>,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(30),
            max_redirects: 5,
            user_agent: "composable-rust-pg-gateway/1.0".to_string(),
            default_headers: HashMap::new(),
        }
    }
}

impl WebhookConfig {
    /// Create a new configuration with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the default timeout.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Set the maximum number of redirects.
    #[must_use]
    pub const fn with_max_redirects(mut self, max: usize) -> Self {
        self.max_redirects = max;
        self
    }

    /// Set the User-Agent header.
    #[must_use]
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Add a default header.
    #[must_use]
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.default_headers.insert(key.into(), value.into());
        self
    }
}

/// Webhook executor that sends HTTP requests.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::tasks::builtins::{WebhookExecutor, WebhookConfig};
///
/// let config = WebhookConfig::new()
///     .with_timeout(Duration::from_secs(10))
///     .with_user_agent("my-service/1.0");
///
/// let executor = WebhookExecutor::new(config)?;
/// ```
pub struct WebhookExecutor {
    client: Client,
    config: WebhookConfig,
}

impl WebhookExecutor {
    /// Create a new webhook executor.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be created.
    pub fn new(config: WebhookConfig) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(config.default_timeout)
            .redirect(reqwest::redirect::Policy::limited(config.max_redirects))
            .user_agent(&config.user_agent)
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

        Ok(Self { client, config })
    }

    /// Create a webhook executor with default configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be created.
    pub fn with_defaults() -> Result<Self, String> {
        Self::new(WebhookConfig::default())
    }

    /// Get a reference to the configuration.
    #[must_use]
    pub const fn config(&self) -> &WebhookConfig {
        &self.config
    }
}

impl TaskExecutor for WebhookExecutor {
    fn task_type(&self) -> &'static str {
        "webhook"
    }

    fn execute(
        &self,
        payload: serde_json::Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), TaskError>> + Send + '_>>
    {
        Box::pin(async move {
            let task: WebhookTask = serde_json::from_value(payload)
                .map_err(|e| TaskError::permanent(format!("Invalid payload: {e}")))?;

            // Validate URL
            if task.url.is_empty() {
                return Err(TaskError::permanent("Empty webhook URL"));
            }

            // Parse method
            let method = task
                .method
                .to_uppercase()
                .parse::<Method>()
                .map_err(|e| TaskError::permanent(format!("Invalid HTTP method: {e}")))?;

            // Build request
            let mut request = self.client.request(method, &task.url);

            // Add default headers
            for (key, value) in &self.config.default_headers {
                request = request.header(key, value);
            }

            // Add task-specific headers
            for (key, value) in &task.headers {
                request = request.header(key, value);
            }

            // Add body for methods that support it
            if !task.body.is_null() {
                request = request
                    .header("Content-Type", "application/json")
                    .json(&task.body);
            }

            // Set timeout from task if specified
            if task.timeout_secs > 0 {
                request = request.timeout(Duration::from_secs(task.timeout_secs));
            }

            // Send request
            let response = request.send().await.map_err(|e| {
                if e.is_timeout() {
                    TaskError::temporary(format!("Request timeout: {e}"))
                } else if e.is_connect() {
                    TaskError::temporary(format!("Connection error: {e}"))
                } else {
                    TaskError::temporary(format!("Request failed: {e}"))
                }
            })?;

            // Check response status
            let status = response.status();

            if status.is_success() {
                Ok(())
            } else if is_retryable_status(status) {
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "<failed to read body>".to_string());
                Err(TaskError::temporary(format!("HTTP {status}: {body}")))
            } else {
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "<failed to read body>".to_string());
                Err(TaskError::permanent(format!("HTTP {status}: {body}")))
            }
        })
    }
}

/// Check if an HTTP status code indicates a retryable error.
const fn is_retryable_status(status: StatusCode) -> bool {
    matches!(
        status.as_u16(),
        408 | // Request Timeout
        429 | // Too Many Requests
        500 | // Internal Server Error
        502 | // Bad Gateway
        503 | // Service Unavailable
        504 // Gateway Timeout
    )
}

impl std::fmt::Debug for WebhookExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookExecutor")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn webhook_task_deserialize() {
        let json = serde_json::json!({
            "url": "https://example.com/webhook",
            "body": {"event": "test"}
        });

        let task: WebhookTask = serde_json::from_value(json).unwrap();
        assert_eq!(task.url, "https://example.com/webhook");
        assert_eq!(task.method, "POST");
        assert_eq!(task.timeout_secs, 30);
    }

    #[test]
    fn webhook_task_with_method() {
        let json = serde_json::json!({
            "url": "https://example.com/api",
            "method": "PUT",
            "headers": {"X-Custom": "value"},
            "body": {"data": 123}
        });

        let task: WebhookTask = serde_json::from_value(json).unwrap();
        assert_eq!(task.method, "PUT");
        assert_eq!(task.headers.get("X-Custom"), Some(&"value".to_string()));
    }

    #[test]
    fn webhook_config_builder() {
        let config = WebhookConfig::new()
            .with_timeout(Duration::from_secs(60))
            .with_max_redirects(10)
            .with_user_agent("test-agent/1.0")
            .with_header("X-Api-Key", "secret");

        assert_eq!(config.default_timeout, Duration::from_secs(60));
        assert_eq!(config.max_redirects, 10);
        assert_eq!(config.user_agent, "test-agent/1.0");
        assert_eq!(
            config.default_headers.get("X-Api-Key"),
            Some(&"secret".to_string())
        );
    }

    #[test]
    fn executor_creation() {
        let executor = WebhookExecutor::with_defaults().unwrap();
        assert_eq!(executor.task_type(), "webhook");
    }

    #[tokio::test]
    async fn executor_rejects_empty_url() {
        let executor = WebhookExecutor::with_defaults().unwrap();
        let payload = serde_json::json!({
            "url": "",
            "body": {}
        });

        let result = executor.execute(payload).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().is_permanent());
    }

    #[tokio::test]
    async fn executor_rejects_invalid_method() {
        let executor = WebhookExecutor::with_defaults().unwrap();
        let payload = serde_json::json!({
            "url": "https://example.com",
            "method": "INVALID"
        });

        let result = executor.execute(payload).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().is_permanent());
    }

    #[test]
    fn retryable_status_codes() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_retryable_status(StatusCode::GATEWAY_TIMEOUT));
        assert!(!is_retryable_status(StatusCode::BAD_REQUEST));
        assert!(!is_retryable_status(StatusCode::NOT_FOUND));
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED));
    }
}
