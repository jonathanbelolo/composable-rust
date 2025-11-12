//! Mock rate limiter for testing.

use crate::error::{AuthError, Result};
use crate::providers::RateLimiter;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// In-memory rate limiter for testing.
///
/// Uses in-memory storage with sliding window algorithm.
///
/// # Memory Management
///
/// ⚠️ **WARNING**: This mock does not implement automatic background cleanup.
/// Old entries are only removed during `check_and_record()` calls for that specific key.
///
/// **For long-running tests**, call `reset(key)` periodically to prevent memory leaks:
///
/// ```rust
/// # use composable_rust_auth::mocks::MockRateLimiter;
/// # use composable_rust_auth::providers::RateLimiter;
/// # async fn example() {
/// let limiter = MockRateLimiter::new();
///
/// // After many attempts with different keys:
/// limiter.reset("test@example.com").await.unwrap();
/// // Or clear all keys by creating a new instance
/// # }
/// ```
///
/// **Production**: Use `RedisRateLimiter` which has automatic `TTL`-based cleanup.
#[derive(Debug, Clone)]
pub struct MockRateLimiter {
    /// Map of key -> Vec<timestamp_ms>
    attempts: Arc<Mutex<HashMap<String, Vec<u64>>>>,
}

impl MockRateLimiter {
    /// Create a new mock rate limiter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get current timestamp in milliseconds.
    ///
    /// # Panics
    ///
    /// This function uses `unwrap_or` to handle clock errors.
    #[allow(clippy::cast_possible_truncation)]
    fn current_timestamp_ms() -> u64 {
        // Casting u128 to u64 is safe here: timestamps won't exceed u64::MAX until year 584,554,531 AD
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis() as u64
    }

    /// Remove old entries outside the window.
    fn cleanup_old_entries(timestamps: &mut Vec<u64>, window_start: u64) {
        timestamps.retain(|&ts| ts >= window_start);
    }
}

impl Default for MockRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter for MockRateLimiter {
    async fn check_rate_limit(&self, key: &str, window: Duration) -> Result<()> {
        let attempts_guard = self
            .attempts
            .lock()
            .map_err(|_| AuthError::InternalError("Mutex lock failed".into()))?;

        let now_ms = Self::current_timestamp_ms();
        #[allow(clippy::cast_possible_truncation)]
        let window_ms = window.as_millis() as u64; // Safe: window is typically seconds/minutes
        let window_start = now_ms.saturating_sub(window_ms);

        if let Some(timestamps) = attempts_guard.get(key) {
            let count = timestamps.iter().filter(|&&ts| ts >= window_start).count();
            tracing::debug!(
                key = %key,
                count = count,
                window_ms = window_ms,
                "Mock rate limit check"
            );
        }

        Ok(())
    }

    async fn record_attempt(&self, key: &str) -> Result<()> {
        let mut attempts_guard = self
            .attempts
            .lock()
            .map_err(|_| AuthError::InternalError("Mutex lock failed".into()))?;

        let now_ms = Self::current_timestamp_ms();

        attempts_guard
            .entry(key.to_string())
            .or_insert_with(Vec::new)
            .push(now_ms);

        tracing::debug!(
            key = %key,
            timestamp_ms = now_ms,
            "Mock recorded rate limit attempt"
        );

        Ok(())
    }

    async fn check_and_record(&self, key: &str, max_attempts: u32, window: Duration) -> Result<()> {
        let mut attempts_guard = self
            .attempts
            .lock()
            .map_err(|_| AuthError::InternalError("Mutex lock failed".into()))?;

        let now_ms = Self::current_timestamp_ms();
        #[allow(clippy::cast_possible_truncation)]
        let window_ms = window.as_millis() as u64; // Safe: window is typically seconds/minutes
        let window_start = now_ms.saturating_sub(window_ms);

        // Get or create entry
        let timestamps = attempts_guard
            .entry(key.to_string())
            .or_insert_with(Vec::new);

        // Remove old entries
        Self::cleanup_old_entries(timestamps, window_start);

        // Check if over limit
        #[allow(clippy::cast_possible_truncation)]
        if timestamps.len() >= max_attempts as usize {
            let retry_after = Duration::from_millis(window_ms);

            tracing::warn!(
                rate_limit_exceeded = true,
                key = %key,
                attempts = timestamps.len() + 1,
                max_attempts = max_attempts,
                window_ms = window_ms,
                "Mock rate limit exceeded"
            );

            return Err(AuthError::TooManyAttempts { retry_after });
        }

        // Record attempt
        timestamps.push(now_ms);

        tracing::debug!(
            key = %key,
            attempts = timestamps.len(),
            max_attempts = max_attempts,
            window_ms = window_ms,
            "Mock rate limit check passed"
        );

        Ok(())
    }

    async fn reset(&self, key: &str) -> Result<()> {
        let mut attempts_guard = self
            .attempts
            .lock()
            .map_err(|_| AuthError::InternalError("Mutex lock failed".into()))?;

        attempts_guard.remove(key);

        tracing::info!(
            key = %key,
            "Mock reset rate limit"
        );

        Ok(())
    }

    async fn get_attempts(&self, key: &str) -> Result<u32> {
        let attempts_guard = self
            .attempts
            .lock()
            .map_err(|_| AuthError::InternalError("Mutex lock failed".into()))?;

        #[allow(clippy::cast_possible_truncation)]
        let count = attempts_guard
            .get(key)
            .map_or(0, |timestamps| timestamps.len() as u32);

        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_rate_limit_allows_within_limit() {
        let limiter = MockRateLimiter::new();

        // Should allow 5 attempts
        for i in 1..=5 {
            let result = limiter
                .check_and_record("test@example.com", 5, Duration::from_secs(60))
                .await;
            assert!(result.is_ok(), "Attempt {} should succeed", i);
        }
    }

    #[tokio::test]
    async fn test_mock_rate_limit_blocks_over_limit() {
        let limiter = MockRateLimiter::new();

        // First 5 attempts should succeed
        for _ in 0..5 {
            limiter
                .check_and_record("test@example.com", 5, Duration::from_secs(60))
                .await
                .unwrap();
        }

        // 6th attempt should fail
        let result = limiter
            .check_and_record("test@example.com", 5, Duration::from_secs(60))
            .await;

        assert!(
            matches!(result, Err(AuthError::TooManyAttempts { .. })),
            "6th attempt should be rate limited"
        );
    }

    #[tokio::test]
    async fn test_mock_rate_limit_reset() {
        let limiter = MockRateLimiter::new();

        // Fill up the limit
        for _ in 0..5 {
            limiter
                .check_and_record("test@example.com", 5, Duration::from_secs(60))
                .await
                .unwrap();
        }

        // Verify we're blocked
        let result = limiter
            .check_and_record("test@example.com", 5, Duration::from_secs(60))
            .await;
        assert!(matches!(result, Err(AuthError::TooManyAttempts { .. })));

        // Reset
        limiter.reset("test@example.com").await.unwrap();

        // Should be allowed again
        let result = limiter
            .check_and_record("test@example.com", 5, Duration::from_secs(60))
            .await;
        assert!(result.is_ok(), "Should be allowed after reset");
    }

    #[tokio::test]
    async fn test_mock_rate_limit_get_attempts() {
        let limiter = MockRateLimiter::new();

        // Initially 0
        let count = limiter.get_attempts("test@example.com").await.unwrap();
        assert_eq!(count, 0);

        // Add 3 attempts
        for _ in 0..3 {
            limiter
                .check_and_record("test@example.com", 5, Duration::from_secs(60))
                .await
                .unwrap();
        }

        let count = limiter.get_attempts("test@example.com").await.unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn test_mock_rate_limit_sliding_window() {
        let limiter = MockRateLimiter::new();

        // Use 3 attempts per 1 second for faster test
        for _ in 0..3 {
            limiter
                .check_and_record("test@example.com", 3, Duration::from_millis(500))
                .await
                .unwrap();
        }

        // Should be blocked now
        let result = limiter
            .check_and_record("test@example.com", 3, Duration::from_millis(500))
            .await;
        assert!(matches!(result, Err(AuthError::TooManyAttempts { .. })));

        // Wait for window to pass
        tokio::time::sleep(tokio::time::Duration::from_millis(600)).await;

        // Should be allowed again (old entries cleaned up)
        let result = limiter
            .check_and_record("test@example.com", 3, Duration::from_millis(500))
            .await;
        assert!(result.is_ok(), "Should be allowed after window expires");
    }
}
