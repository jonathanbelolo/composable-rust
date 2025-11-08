//! Retry logic with exponential backoff for handling transient failures.
//!
//! This module provides utilities for retrying operations that may fail due to transient
//! errors (network issues, temporary service unavailability, etc.).
//!
//! # Example
//!
//! ```rust
//! use composable_rust_runtime::retry::{RetryPolicy, retry_with_backoff};
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let policy = RetryPolicy::builder()
//!     .max_retries(5)
//!     .initial_delay(Duration::from_millis(100))
//!     .max_delay(Duration::from_secs(10))
//!     .multiplier(2.0)
//!     .build();
//!
//! let result = retry_with_backoff(policy, || async {
//!     // Your fallible operation here
//!     Ok::<_, String>(42)
//! }).await?;
//! # Ok(())
//! # }
//! ```

use std::time::Duration;
use tokio::time::sleep;

/// Retry policy configuration for exponential backoff.
///
/// # Default Values
///
/// - `max_retries`: 3
/// - `initial_delay`: 100ms
/// - `max_delay`: 30 seconds
/// - `multiplier`: 2.0 (delay doubles each retry)
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts
    pub max_retries: usize,
    /// Initial delay before first retry
    pub initial_delay: Duration,
    /// Maximum delay between retries (cap for exponential backoff)
    pub max_delay: Duration,
    /// Multiplier for exponential backoff
    pub multiplier: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
        }
    }
}

impl RetryPolicy {
    /// Create a new policy builder.
    #[must_use]
    pub const fn builder() -> RetryPolicyBuilder {
        RetryPolicyBuilder {
            max_retries: Some(3),
            initial_delay: Some(Duration::from_millis(100)),
            max_delay: Some(Duration::from_secs(30)),
            multiplier: Some(2.0),
        }
    }

    /// Calculate delay for a given attempt number.
    ///
    /// Uses exponential backoff: delay = initial_delay * (multiplier ^ attempt)
    /// Capped at `max_delay`.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: usize) -> Duration {
        if attempt == 0 {
            return self.initial_delay;
        }

        let delay_ms = self.initial_delay.as_millis() as f64
            * self.multiplier.powi(attempt as i32);

        let delay = Duration::from_millis(delay_ms as u64);

        if delay > self.max_delay {
            self.max_delay
        } else {
            delay
        }
    }
}

/// Builder for [`RetryPolicy`].
#[derive(Debug, Clone)]
pub struct RetryPolicyBuilder {
    max_retries: Option<usize>,
    initial_delay: Option<Duration>,
    max_delay: Option<Duration>,
    multiplier: Option<f64>,
}

impl RetryPolicyBuilder {
    /// Set maximum number of retries.
    #[must_use]
    pub const fn max_retries(mut self, max_retries: usize) -> Self {
        self.max_retries = Some(max_retries);
        self
    }

    /// Set initial delay before first retry.
    #[must_use]
    pub const fn initial_delay(mut self, delay: Duration) -> Self {
        self.initial_delay = Some(delay);
        self
    }

    /// Set maximum delay (cap for exponential backoff).
    #[must_use]
    pub const fn max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = Some(delay);
        self
    }

    /// Set multiplier for exponential backoff.
    #[must_use]
    pub const fn multiplier(mut self, multiplier: f64) -> Self {
        self.multiplier = Some(multiplier);
        self
    }

    /// Build the [`RetryPolicy`].
    #[must_use]
    pub fn build(self) -> RetryPolicy {
        RetryPolicy {
            max_retries: self.max_retries.unwrap_or(3),
            initial_delay: self.initial_delay.unwrap_or(Duration::from_millis(100)),
            max_delay: self.max_delay.unwrap_or(Duration::from_secs(30)),
            multiplier: self.multiplier.unwrap_or(2.0),
        }
    }
}

/// Retry an async operation with exponential backoff.
///
/// # Arguments
///
/// * `policy` - Retry policy configuration
/// * `operation` - Async operation to retry (must be `FnMut` to allow multiple calls)
///
/// # Returns
///
/// Returns `Ok(T)` if the operation succeeds within the retry limit,
/// or `Err(E)` with the last error if all retries are exhausted.
///
/// # Example
///
/// ```rust
/// use composable_rust_runtime::retry::{RetryPolicy, retry_with_backoff};
///
/// # async fn example() -> Result<(), String> {
/// let policy = RetryPolicy::default();
///
/// let result = retry_with_backoff(policy, || async {
///     // Simulated fallible operation
///     Ok::<_, String>(42)
/// }).await?;
///
/// assert_eq!(result, 42);
/// # Ok(())
/// # }
/// ```
pub async fn retry_with_backoff<F, Fut, T, E>(
    policy: RetryPolicy,
    mut operation: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempt = 0;
    let mut last_error: Option<E> = None;

    loop {
        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    tracing::info!(
                        attempt,
                        "Operation succeeded after retry"
                    );
                }
                return Ok(result);
            }
            Err(err) => {
                if attempt >= policy.max_retries {
                    tracing::error!(
                        attempt,
                        error = %err,
                        "Operation failed after max retries"
                    );
                    return Err(last_error.unwrap_or(err));
                }

                let delay = policy.delay_for_attempt(attempt);
                tracing::warn!(
                    attempt,
                    delay_ms = delay.as_millis(),
                    error = %err,
                    "Operation failed, retrying..."
                );

                last_error = Some(err);
                sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

/// Retry an async operation with custom retry logic.
///
/// This function allows you to provide a predicate to determine whether
/// an error is retryable.
///
/// # Arguments
///
/// * `policy` - Retry policy configuration
/// * `operation` - Async operation to retry
/// * `is_retryable` - Predicate to determine if an error should trigger a retry
///
/// # Example
///
/// ```rust
/// use composable_rust_runtime::retry::{RetryPolicy, retry_with_predicate};
///
/// # async fn example() -> Result<(), String> {
/// let policy = RetryPolicy::default();
///
/// let result = retry_with_predicate(
///     policy,
///     || async { Ok::<_, String>(42) },
///     |err: &String| err.contains("transient"),
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn retry_with_predicate<F, Fut, T, E, P>(
    policy: RetryPolicy,
    mut operation: F,
    is_retryable: P,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
    P: Fn(&E) -> bool,
{
    let mut attempt = 0;
    let mut last_error: Option<E> = None;

    loop {
        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    tracing::info!(
                        attempt,
                        "Operation succeeded after retry"
                    );
                }
                return Ok(result);
            }
            Err(err) => {
                if !is_retryable(&err) {
                    tracing::warn!(
                        error = %err,
                        "Error is not retryable, failing immediately"
                    );
                    return Err(err);
                }

                if attempt >= policy.max_retries {
                    tracing::error!(
                        attempt,
                        error = %err,
                        "Operation failed after max retries"
                    );
                    return Err(last_error.unwrap_or(err));
                }

                let delay = policy.delay_for_attempt(attempt);
                tracing::warn!(
                    attempt,
                    delay_ms = delay.as_millis(),
                    error = %err,
                    "Operation failed, retrying..."
                );

                last_error = Some(err);
                sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_retry_policy_delay_calculation() {
        let policy = RetryPolicy::builder()
            .initial_delay(Duration::from_millis(100))
            .multiplier(2.0)
            .max_delay(Duration::from_secs(10))
            .build();

        assert_eq!(policy.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(400));
        assert_eq!(policy.delay_for_attempt(3), Duration::from_millis(800));
    }

    #[test]
    fn test_retry_policy_max_delay_cap() {
        let policy = RetryPolicy::builder()
            .initial_delay(Duration::from_millis(1000))
            .multiplier(10.0)
            .max_delay(Duration::from_secs(2))
            .build();

        // 1000ms * 10^5 = 100,000,000ms, but capped at 2000ms
        assert_eq!(policy.delay_for_attempt(5), Duration::from_secs(2));
    }

    #[tokio::test]
    async fn test_retry_succeeds_on_first_try() {
        let policy = RetryPolicy::default();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let result = retry_with_backoff(policy, || {
            let c = Arc::clone(&counter_clone);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>(42)
            }
        })
        .await;

        assert_eq!(result, Ok(42));
        assert_eq!(counter.load(Ordering::SeqCst), 1); // Only called once
    }

    #[tokio::test]
    async fn test_retry_succeeds_after_failures() {
        let policy = RetryPolicy::builder()
            .max_retries(3)
            .initial_delay(Duration::from_millis(10))
            .build();

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let result = retry_with_backoff(policy, || {
            let c = Arc::clone(&counter_clone);
            async move {
                let attempt = c.fetch_add(1, Ordering::SeqCst);
                if attempt < 2 {
                    Err(format!("Attempt {attempt} failed"))
                } else {
                    Ok(42)
                }
            }
        })
        .await;

        assert_eq!(result, Ok(42));
        assert_eq!(counter.load(Ordering::SeqCst), 3); // 2 failures + 1 success
    }

    #[tokio::test]
    async fn test_retry_exhausts_retries() {
        let policy = RetryPolicy::builder()
            .max_retries(2)
            .initial_delay(Duration::from_millis(10))
            .build();

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let result = retry_with_backoff(policy, || {
            let c = Arc::clone(&counter_clone);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>("Persistent failure")
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 3); // Initial + 2 retries
    }

    #[tokio::test]
    async fn test_retry_with_predicate_skips_non_retryable() {
        let policy = RetryPolicy::default();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let result = retry_with_predicate(
            policy,
            || {
                let c = Arc::clone(&counter_clone);
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err::<i32, _>("permanent error")
                }
            },
            |err: &&str| err.contains("transient"),
        )
        .await;

        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 1); // No retries for non-retryable error
    }
}
