//! Rate Limiter using Token Bucket Algorithm (Phase 8.4 Part 3.2)
//!
//! Prevents resource exhaustion by limiting request rate per time window.
//!
//! ## Algorithm: Token Bucket
//!
//! ```text
//! Bucket (capacity: 100 tokens)
//! ├─ Tokens refill at constant rate (e.g., 10/second)
//! ├─ Each request consumes tokens
//! └─ If not enough tokens → request rejected
//!
//! Time:  0s    1s    2s    3s    4s
//! Tokens: 100 → 90 → 100 → 80 → 90
//!         ↓10   ↓0   ↓20  ↓10
//!       request request request request
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! use agent_patterns::resilience::rate_limiter::*;
//!
//! let config = RateLimiterConfig {
//!     capacity: 100,        // Max tokens
//!     refill_rate: 10.0,    // Tokens per second
//! };
//!
//! let rate_limiter = RateLimiter::new("api_requests".into(), config);
//!
//! // Try to acquire tokens
//! match rate_limiter.try_acquire(1).await {
//!     Ok(()) => {
//!         // Request allowed
//!         execute_request().await
//!     }
//!     Err(e) => {
//!         // Rate limit exceeded
//!         return_429_error()
//!     }
//! }
//! ```

use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::warn;

/// Rate limiter configuration using token bucket algorithm
#[derive(Debug, Clone)]
pub struct RateLimiterConfig {
    /// Maximum number of tokens (burst capacity)
    pub capacity: usize,
    /// Tokens refilled per second
    pub refill_rate: f64,
}

impl Default for RateLimiterConfig {
    fn default() -> Self {
        Self {
            capacity: 100,
            refill_rate: 10.0,
        }
    }
}

/// Internal state tracking
struct RateLimiterState {
    tokens: f64,
    last_refill: Instant,
}

/// Token bucket rate limiter
///
/// Limits request rate by maintaining a bucket of tokens that:
/// - Refill at a constant rate
/// - Are consumed by requests
/// - When empty, requests are rejected
pub struct RateLimiter {
    name: String,
    config: RateLimiterConfig,
    state: Arc<RwLock<RateLimiterState>>,
}

impl RateLimiter {
    /// Create new rate limiter
    ///
    /// # Arguments
    ///
    /// * `name` - Name for logging (e.g., "api_requests", "llm_calls")
    /// * `config` - Rate limiter configuration
    #[must_use]
    pub fn new(name: String, config: RateLimiterConfig) -> Self {
        Self {
            name,
            config: config.clone(),
            state: Arc::new(RwLock::new(RateLimiterState {
                tokens: config.capacity as f64,
                last_refill: Instant::now(),
            })),
        }
    }

    /// Attempt to acquire tokens
    ///
    /// # Arguments
    ///
    /// * `tokens` - Number of tokens to acquire (usually 1 per request)
    ///
    /// # Errors
    ///
    /// Returns error if not enough tokens available
    pub async fn try_acquire(&self, tokens: usize) -> Result<(), String> {
        let mut state = self.state.write().await;

        // Refill tokens based on elapsed time
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_refill).as_secs_f64();
        let new_tokens = elapsed * self.config.refill_rate;

        // Add new tokens (capped at capacity)
        state.tokens = (state.tokens + new_tokens).min(self.config.capacity as f64);
        state.last_refill = now;

        // Check if enough tokens available
        if state.tokens >= tokens as f64 {
            state.tokens -= tokens as f64;
            Ok(())
        } else {
            warn!(
                "Rate limit exceeded for {} (need: {}, available: {:.2})",
                self.name, tokens, state.tokens
            );
            Err(format!(
                "Rate limit exceeded for {} (need: {}, available: {:.2})",
                self.name, tokens, state.tokens
            ))
        }
    }

    /// Get current token count
    ///
    /// Useful for monitoring and metrics.
    pub async fn available_tokens(&self) -> f64 {
        let state = self.state.read().await;
        state.tokens
    }

    /// Get rate limiter name
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get capacity
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.config.capacity
    }

    /// Get refill rate
    #[must_use]
    pub fn refill_rate(&self) -> f64 {
        self.config.refill_rate
    }
}

/// Execute function with rate limiting
///
/// Convenience function that checks rate limit before executing.
///
/// # Example
///
/// ```ignore
/// let result = with_rate_limit(&rate_limiter, 1, async {
///     expensive_operation().await
/// }).await?;
/// ```
pub async fn with_rate_limit<F, T>(
    rate_limiter: &RateLimiter,
    tokens: usize,
    f: F,
) -> Result<T, String>
where
    F: std::future::Future<Output = T>,
{
    // Try to acquire tokens
    rate_limiter.try_acquire(tokens).await?;

    // Execute function
    Ok(f.await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_rate_limiter_allows_requests_within_capacity() {
        let config = RateLimiterConfig {
            capacity: 10,
            refill_rate: 100.0,
        };

        let rate_limiter = RateLimiter::new("test".to_string(), config);

        // Should allow 10 requests
        for _ in 0..10 {
            assert!(rate_limiter.try_acquire(1).await.is_ok());
        }

        // 11th should fail
        assert!(rate_limiter.try_acquire(1).await.is_err());
    }

    #[tokio::test]
    async fn test_rate_limiter_refills_tokens() {
        let config = RateLimiterConfig {
            capacity: 10,
            refill_rate: 10.0, // 10 tokens per second
        };

        let rate_limiter = RateLimiter::new("test".to_string(), config);

        // Consume all tokens
        for _ in 0..10 {
            rate_limiter.try_acquire(1).await.ok();
        }

        // Wait for refill (1 second = 10 tokens)
        tokio::time::sleep(Duration::from_millis(1100)).await;

        // Should have tokens again
        assert!(rate_limiter.try_acquire(5).await.is_ok());
    }

    #[tokio::test]
    async fn test_rate_limiter_capped_at_capacity() {
        let config = RateLimiterConfig {
            capacity: 5,
            refill_rate: 10.0,
        };

        let rate_limiter = RateLimiter::new("test".to_string(), config);

        // Wait for potential refill
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Available tokens should be capped at capacity
        let available = rate_limiter.available_tokens().await;
        assert!(available <= 5.0);
    }

    #[tokio::test]
    async fn test_rate_limiter_multi_token_acquire() {
        let config = RateLimiterConfig {
            capacity: 10,
            refill_rate: 100.0,
        };

        let rate_limiter = RateLimiter::new("test".to_string(), config);

        // Acquire 5 tokens at once
        assert!(rate_limiter.try_acquire(5).await.is_ok());

        // Should have 5 left
        let available = rate_limiter.available_tokens().await;
        assert!((available - 5.0).abs() < 0.1);

        // Try to acquire 6 (should fail)
        assert!(rate_limiter.try_acquire(6).await.is_err());

        // Try to acquire 5 (should succeed)
        assert!(rate_limiter.try_acquire(5).await.is_ok());
    }

    #[tokio::test]
    async fn test_with_rate_limit_success() {
        let config = RateLimiterConfig {
            capacity: 10,
            refill_rate: 100.0,
        };

        let rate_limiter = RateLimiter::new("test".to_string(), config);

        let result = with_rate_limit(&rate_limiter, 1, async { "success" }).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
    }

    #[tokio::test]
    async fn test_with_rate_limit_rate_exceeded() {
        let config = RateLimiterConfig {
            capacity: 1,
            refill_rate: 0.1, // Very slow refill
        };

        let rate_limiter = RateLimiter::new("test".to_string(), config);

        // First request should succeed
        assert!(with_rate_limit(&rate_limiter, 1, async { "ok" }).await.is_ok());

        // Second request should fail (no tokens)
        let result = with_rate_limit(&rate_limiter, 1, async { "should not execute" }).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_rate_limiter_getters() {
        let config = RateLimiterConfig {
            capacity: 50,
            refill_rate: 5.0,
        };

        let rate_limiter = RateLimiter::new("test".to_string(), config);

        assert_eq!(rate_limiter.name(), "test");
        assert_eq!(rate_limiter.capacity(), 50);
        assert_eq!(rate_limiter.refill_rate(), 5.0);
    }
}
