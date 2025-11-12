//! Redis-based rate limiter implementation.
//!
//! Uses sliding window algorithm for accurate rate limiting.
//!
//! # Algorithm
//!
//! Sliding window with sorted sets:
//! 1. Store timestamps in Redis sorted set (ZADD)
//! 2. Remove old entries outside window (ZREMRANGEBYSCORE)
//! 3. Count remaining entries (ZCARD)
//! 4. Compare against limit
//!
//! # Security
//!
//! Prevents brute force attacks by limiting authentication attempts per time window.

use crate::error::{AuthError, Result};
use crate::providers::RateLimiter;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `Redis`-based rate limiter using sliding window algorithm.
///
/// # Example
///
/// ```no_run
/// use composable_rust_auth::stores::RedisRateLimiter;
/// use composable_rust_auth::providers::RateLimiter;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let limiter = RedisRateLimiter::new("redis://127.0.0.1:6379").await?;
///
/// // Check and record with 5 attempts per 15 minutes
/// limiter.check_and_record("user@example.com", 5, std::time::Duration::from_secs(900)).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct RedisRateLimiter {
    /// Connection manager for connection pooling.
    conn_manager: ConnectionManager,
}

impl RedisRateLimiter {
    /// Create a new `Redis` rate limiter.
    ///
    /// # Arguments
    ///
    /// * `redis_url` - `Redis` connection URL (e.g., "<redis://127.0.0.1:6379>")
    ///
    /// # Errors
    ///
    /// Returns error if connection to `Redis` fails.
    pub async fn new(redis_url: &str) -> Result<Self> {
        let client = Client::open(redis_url).map_err(|e| {
            AuthError::InternalError(format!("Failed to create Redis client: {e}"))
        })?;

        let conn_manager = ConnectionManager::new(client).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to create Redis connection manager: {e}"))
        })?;

        Ok(Self { conn_manager })
    }

    /// Get the `Redis` key for rate limiting.
    fn rate_limit_key(key: &str) -> String {
        format!("rate_limit:{key}")
    }

    /// Get current timestamp in milliseconds.
    #[allow(clippy::cast_possible_truncation)] // Safe: timestamps fit in u64 until year 2554
    fn current_timestamp_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis() as u64
    }
}

impl RateLimiter for RedisRateLimiter {
    async fn check_rate_limit(&self, key: &str, window: Duration) -> Result<()> {
        let mut conn = self.conn_manager.clone();
        let rate_key = Self::rate_limit_key(key);
        let now_ms = Self::current_timestamp_ms();
        #[allow(clippy::cast_possible_truncation)] // Safe: rate limit windows are small durations
        let window_ms = window.as_millis() as u64;
        let window_start = now_ms.saturating_sub(window_ms);

        // Remove entries outside the window
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Safe: Redis zrembyscore accepts isize scores
        let _: () = conn
            .zrembyscore(&rate_key, 0, window_start as isize)
            .await
            .map_err(|e| {
                AuthError::InternalError(format!("Failed to clean old rate limit entries: {e}"))
            })?;

        // Count remaining entries
        let count: u64 = conn.zcard(&rate_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to get rate limit count: {e}"))
        })?;

        // If we have entries, check if we're over the limit
        // Note: This is a check-only operation, actual limit is enforced in check_and_record
        if count > 0 {
            tracing::debug!(
                key = %key,
                count = count,
                window_ms = window_ms,
                "Rate limit check"
            );
        }

        Ok(())
    }

    async fn record_attempt(&self, key: &str) -> Result<()> {
        let mut conn = self.conn_manager.clone();
        let rate_key = Self::rate_limit_key(key);
        let now_ms = Self::current_timestamp_ms();

        // Add current timestamp to sorted set
        let _: () = conn
            .zadd(&rate_key, now_ms, now_ms)
            .await
            .map_err(|e| {
                AuthError::InternalError(format!("Failed to record rate limit attempt: {e}"))
            })?;

        // Set expiration on the key (window + buffer for cleanup)
        let _: () = conn
            .expire(&rate_key, 3600) // 1 hour expiration
            .await
            .map_err(|e| {
                AuthError::InternalError(format!("Failed to set rate limit key expiration: {e}"))
            })?;

        tracing::debug!(
            key = %key,
            timestamp_ms = now_ms,
            "Recorded rate limit attempt"
        );

        Ok(())
    }

    async fn check_and_record(&self, key: &str, max_attempts: u32, window: Duration) -> Result<()> {
        let mut conn = self.conn_manager.clone();
        let rate_key = Self::rate_limit_key(key);
        let now_ms = Self::current_timestamp_ms();
        #[allow(clippy::cast_possible_truncation)] // Safe: rate limit windows are small durations
        let window_ms = window.as_millis() as u64;
        let window_start = now_ms.saturating_sub(window_ms);

        // ✅ SECURITY: Use Redis pipeline for atomic check-and-record
        //
        // This prevents race conditions where two concurrent requests could both
        // pass the check before either records, bypassing the rate limit.
        //
        // Pipeline ensures (atomically):
        // 1. Remove old entries outside the window
        // 2. Count current entries in window
        // 3. Add new entry for this attempt
        // 4. Set TTL for automatic cleanup
        //
        // IMPORTANT: Pipeline is atomic - either ALL operations succeed or ALL fail.
        // If expire() fails, the entire pipeline fails → safe default (deny access).
        //
        // Note: .ignore() means "don't return this value", NOT "ignore errors".
        // All operations are still executed and errors still propagate.

        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Safe: Redis zrembyscore accepts isize scores
        let (count,): (u64,) = redis::pipe()
            .atomic()
            .zrembyscore(&rate_key, 0, window_start as isize)
            .ignore() // Don't return count of removed items (not needed)
            .zcard(&rate_key) // Return: current count in window
            .zadd(&rate_key, now_ms, now_ms)
            .ignore() // Don't return zadd result (not needed)
            .expire(&rate_key, 3600) // 1 hour cleanup (prevent memory leak)
            .ignore() // Don't return expire result (not needed)
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                // Pipeline failed - safe default: deny access
                tracing::error!(
                    error = %e,
                    key = %key,
                    "Redis pipeline failed during rate limit check (safe default: deny)"
                );
                AuthError::InternalError(format!("Failed to check and record rate limit: {e}"))
            })?;

        // Check if over limit (count is BEFORE adding current attempt)
        if count >= u64::from(max_attempts) {
            let retry_after = Duration::from_millis(window_ms);

            tracing::warn!(
                rate_limit_exceeded = true,
                key = %key,
                attempts = count + 1,
                max_attempts = max_attempts,
                window_ms = window_ms,
                "Rate limit exceeded"
            );

            return Err(AuthError::TooManyAttempts { retry_after });
        }

        tracing::debug!(
            key = %key,
            attempts = count + 1,
            max_attempts = max_attempts,
            window_ms = window_ms,
            "Rate limit check passed"
        );

        Ok(())
    }

    async fn reset(&self, key: &str) -> Result<()> {
        let mut conn = self.conn_manager.clone();
        let rate_key = Self::rate_limit_key(key);

        let _: () = conn.del(&rate_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to reset rate limit: {e}"))
        })?;

        tracing::info!(
            key = %key,
            "Reset rate limit"
        );

        Ok(())
    }

    async fn get_attempts(&self, key: &str) -> Result<u32> {
        let mut conn = self.conn_manager.clone();
        let rate_key = Self::rate_limit_key(key);

        let count: u64 = conn.zcard(&rate_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to get rate limit attempts: {e}"))
        })?;

        #[allow(clippy::cast_possible_truncation)]
        Ok(count as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require a running Redis instance
    // Run with: docker run -d -p 6379:6379 redis:7-alpine

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_rate_limit_allows_within_limit() {
        let limiter = RedisRateLimiter::new("redis://127.0.0.1:6379")
            .await
            .unwrap();

        let key = format!("test:allow:{}", uuid::Uuid::new_v4());

        // Should allow 5 attempts
        for i in 1..=5 {
            let result = limiter
                .check_and_record(&key, 5, Duration::from_secs(60))
                .await;
            assert!(result.is_ok(), "Attempt {} should succeed", i);
        }

        // Cleanup
        limiter.reset(&key).await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_rate_limit_blocks_over_limit() {
        let limiter = RedisRateLimiter::new("redis://127.0.0.1:6379")
            .await
            .unwrap();

        let key = format!("test:block:{}", uuid::Uuid::new_v4());

        // First 5 attempts should succeed
        for i in 1..=5 {
            let result = limiter
                .check_and_record(&key, 5, Duration::from_secs(60))
                .await;
            assert!(result.is_ok(), "Attempt {} should succeed", i);
        }

        // 6th attempt should fail
        let result = limiter
            .check_and_record(&key, 5, Duration::from_secs(60))
            .await;

        assert!(
            matches!(result, Err(AuthError::TooManyAttempts { .. })),
            "6th attempt should be rate limited"
        );

        // Cleanup
        limiter.reset(&key).await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_rate_limit_sliding_window() {
        let limiter = RedisRateLimiter::new("redis://127.0.0.1:6379")
            .await
            .unwrap();

        let key = format!("test:sliding:{}", uuid::Uuid::new_v4());

        // Use 3 attempts per 2 seconds for faster test
        for _ in 0..3 {
            limiter
                .check_and_record(&key, 3, Duration::from_secs(2))
                .await
                .unwrap();
        }

        // Should be blocked now
        let result = limiter
            .check_and_record(&key, 3, Duration::from_secs(2))
            .await;
        assert!(matches!(result, Err(AuthError::TooManyAttempts { .. })));

        // Wait for window to pass
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        // Should be allowed again
        let result = limiter
            .check_and_record(&key, 3, Duration::from_secs(2))
            .await;
        assert!(result.is_ok(), "Should be allowed after window expires");

        // Cleanup
        limiter.reset(&key).await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_rate_limit_reset() {
        let limiter = RedisRateLimiter::new("redis://127.0.0.1:6379")
            .await
            .unwrap();

        let key = format!("test:reset:{}", uuid::Uuid::new_v4());

        // Fill up the limit
        for _ in 0..5 {
            limiter
                .check_and_record(&key, 5, Duration::from_secs(60))
                .await
                .unwrap();
        }

        // Verify we're blocked
        let result = limiter
            .check_and_record(&key, 5, Duration::from_secs(60))
            .await;
        assert!(matches!(result, Err(AuthError::TooManyAttempts { .. })));

        // Reset
        limiter.reset(&key).await.unwrap();

        // Should be allowed again
        let result = limiter
            .check_and_record(&key, 5, Duration::from_secs(60))
            .await;
        assert!(result.is_ok(), "Should be allowed after reset");

        // Cleanup
        limiter.reset(&key).await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_rate_limit_get_attempts() {
        let limiter = RedisRateLimiter::new("redis://127.0.0.1:6379")
            .await
            .unwrap();

        let key = format!("test:count:{}", uuid::Uuid::new_v4());

        // Initially 0
        let count = limiter.get_attempts(&key).await.unwrap();
        assert_eq!(count, 0);

        // Add 3 attempts
        for _ in 0..3 {
            limiter
                .check_and_record(&key, 5, Duration::from_secs(60))
                .await
                .unwrap();
        }

        let count = limiter.get_attempts(&key).await.unwrap();
        assert_eq!(count, 3);

        // Cleanup
        limiter.reset(&key).await.unwrap();
    }
}
