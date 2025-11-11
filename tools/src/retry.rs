//! Retry policies and timeout handling for tool execution
//!
//! Provides configurable retry policies with timeouts:
//! - No retry (fail immediately)
//! - Fixed retry (constant delay between attempts)
//! - Exponential backoff (increasing delay between attempts)

use composable_rust_core::agent::{ToolError, ToolResult};
use std::future::Future;
use std::time::Duration;

/// Retry policy for tool execution
#[derive(Debug, Clone)]
pub enum RetryPolicy {
    /// No retry - fail immediately on error
    None,

    /// Fixed retry with constant delay
    ///
    /// Retries the operation `attempts` times with a fixed `delay` between attempts.
    Fixed {
        /// Number of attempts (including the initial attempt)
        attempts: u32,
        /// Delay between attempts
        delay: Duration,
    },

    /// Exponential backoff
    ///
    /// Retries with exponentially increasing delays: initial_delay * (multiplier ^ attempt)
    Exponential {
        /// Number of attempts (including the initial attempt)
        attempts: u32,
        /// Initial delay (doubled each retry by default)
        initial_delay: Duration,
        /// Multiplier for exponential growth (typically 2.0)
        multiplier: f64,
    },
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::Fixed {
            attempts: 3,
            delay: Duration::from_millis(100),
        }
    }
}

/// Tool configuration with retry policy and timeout
#[derive(Debug, Clone)]
pub struct ToolConfig {
    /// Retry policy for this tool
    pub retry_policy: RetryPolicy,
    /// Maximum execution time for a single attempt
    pub timeout: Duration,
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            retry_policy: RetryPolicy::default(),
            timeout: Duration::from_secs(30),
        }
    }
}

impl ToolConfig {
    /// Create a config with no retry
    #[must_use]
    pub fn no_retry() -> Self {
        Self {
            retry_policy: RetryPolicy::None,
            timeout: Duration::from_secs(30),
        }
    }

    /// Create a config with fixed retry
    #[must_use]
    pub const fn fixed_retry(attempts: u32, delay: Duration) -> Self {
        Self {
            retry_policy: RetryPolicy::Fixed { attempts, delay },
            timeout: Duration::from_secs(30),
        }
    }

    /// Create a config with exponential backoff
    #[must_use]
    pub const fn exponential_backoff(attempts: u32, initial_delay: Duration) -> Self {
        Self {
            retry_policy: RetryPolicy::Exponential {
                attempts,
                initial_delay,
                multiplier: 2.0,
            },
            timeout: Duration::from_secs(30),
        }
    }

    /// Set timeout duration
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Execute a tool with retry policy and timeout
///
/// This function wraps tool execution with:
/// - Timeout enforcement (per attempt)
/// - Retry logic based on configured policy
/// - Exponential backoff for transient failures
///
/// ## Example
///
/// ```ignore
/// use composable_rust_tools::retry::{execute_with_retry, ToolConfig};
/// use std::time::Duration;
///
/// let config = ToolConfig::fixed_retry(3, Duration::from_millis(100));
/// let result = execute_with_retry(&config, || async {
///     // Your tool execution here
///     Ok("result".to_string())
/// }).await;
/// ```
///
/// # Errors
///
/// Returns `ToolError` if:
/// - All retry attempts fail
/// - Timeout is exceeded
/// - Underlying tool execution fails
pub async fn execute_with_retry<F, Fut>(config: &ToolConfig, executor: F) -> ToolResult
where
    F: Fn() -> Fut,
    Fut: Future<Output = ToolResult>,
{
    match &config.retry_policy {
        RetryPolicy::None => {
            // Single attempt with timeout
            tokio::time::timeout(config.timeout, executor())
                .await
                .map_err(|_| ToolError {
                    message: format!("Tool execution timed out after {:?}", config.timeout),
                })?
        }

        RetryPolicy::Fixed { attempts, delay } => {
            let mut last_error = None;

            for attempt in 0..*attempts {
                match tokio::time::timeout(config.timeout, executor()).await {
                    Ok(Ok(result)) => return Ok(result),
                    Ok(Err(e)) => {
                        last_error = Some(e);
                        if attempt < attempts - 1 {
                            tokio::time::sleep(*delay).await;
                        }
                    }
                    Err(_) => {
                        last_error = Some(ToolError {
                            message: format!("Tool execution timed out after {:?}", config.timeout),
                        });
                        if attempt < attempts - 1 {
                            tokio::time::sleep(*delay).await;
                        }
                    }
                }
            }

            Err(last_error.expect("At least one attempt should have occurred"))
        }

        RetryPolicy::Exponential {
            attempts,
            initial_delay,
            multiplier,
        } => {
            let mut last_error = None;
            let mut current_delay = *initial_delay;

            for attempt in 0..*attempts {
                match tokio::time::timeout(config.timeout, executor()).await {
                    Ok(Ok(result)) => return Ok(result),
                    Ok(Err(e)) => {
                        last_error = Some(e);
                        if attempt < attempts - 1 {
                            tokio::time::sleep(current_delay).await;
                            current_delay = current_delay.mul_f64(*multiplier);
                        }
                    }
                    Err(_) => {
                        last_error = Some(ToolError {
                            message: format!("Tool execution timed out after {:?}", config.timeout),
                        });
                        if attempt < attempts - 1 {
                            tokio::time::sleep(current_delay).await;
                            current_delay = current_delay.mul_f64(*multiplier);
                        }
                    }
                }
            }

            Err(last_error.expect("At least one attempt should have occurred"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_retry_policy_default() {
        let policy = RetryPolicy::default();
        match policy {
            RetryPolicy::Fixed { attempts, delay } => {
                assert_eq!(attempts, 3);
                assert_eq!(delay, Duration::from_millis(100));
            }
            _ => panic!("Expected Fixed policy"),
        }
    }

    #[test]
    fn test_tool_config_default() {
        let config = ToolConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_tool_config_no_retry() {
        let config = ToolConfig::no_retry();
        assert!(matches!(config.retry_policy, RetryPolicy::None));
    }

    #[test]
    fn test_tool_config_with_timeout() {
        let config = ToolConfig::default().with_timeout(Duration::from_secs(60));
        assert_eq!(config.timeout, Duration::from_secs(60));
    }

    #[tokio::test]
    async fn test_execute_with_retry_success_first_attempt() {
        let config = ToolConfig::fixed_retry(3, Duration::from_millis(10));
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = execute_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok("success".to_string())
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.expect("should succeed"), "success");
        assert_eq!(counter.load(Ordering::SeqCst), 1); // Only one attempt
    }

    #[tokio::test]
    async fn test_execute_with_retry_success_second_attempt() {
        let config = ToolConfig::fixed_retry(3, Duration::from_millis(10));
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = execute_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                let attempt = counter.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    Err(ToolError {
                        message: "First attempt failed".to_string(),
                    })
                } else {
                    Ok("success".to_string())
                }
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.expect("should succeed"), "success");
        assert_eq!(counter.load(Ordering::SeqCst), 2); // Two attempts
    }

    #[tokio::test]
    async fn test_execute_with_retry_all_attempts_fail() {
        let config = ToolConfig::fixed_retry(3, Duration::from_millis(10));
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = execute_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err(ToolError {
                    message: "Always fails".to_string(),
                })
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 3); // All three attempts
    }

    #[tokio::test]
    async fn test_execute_with_retry_no_retry_policy() {
        let config = ToolConfig::no_retry();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = execute_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err(ToolError {
                    message: "Fails".to_string(),
                })
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 1); // Only one attempt
    }

    #[tokio::test]
    async fn test_execute_with_retry_exponential_backoff() {
        let config = ToolConfig::exponential_backoff(3, Duration::from_millis(10));
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let start = std::time::Instant::now();
        let result = execute_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                let attempt = counter.fetch_add(1, Ordering::SeqCst);
                if attempt < 2 {
                    Err(ToolError {
                        message: "Not yet".to_string(),
                    })
                } else {
                    Ok("success".to_string())
                }
            }
        })
        .await;

        let elapsed = start.elapsed();

        assert!(result.is_ok());
        assert_eq!(counter.load(Ordering::SeqCst), 3); // Three attempts
        // Exponential: 10ms + 20ms = 30ms minimum
        assert!(elapsed >= Duration::from_millis(30));
    }

    #[tokio::test]
    async fn test_execute_with_retry_timeout() {
        let config = ToolConfig::no_retry().with_timeout(Duration::from_millis(50));

        let result = execute_with_retry(&config, || async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok("should timeout".to_string())
        })
        .await;

        assert!(result.is_err());
        assert!(result
            .expect_err("should timeout")
            .message
            .contains("timed out"));
    }
}
