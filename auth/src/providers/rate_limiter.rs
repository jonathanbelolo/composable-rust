//! Rate limiter trait for authentication attempts.
//!
//! # Security
//!
//! Rate limiting is essential for preventing brute force attacks on authentication endpoints.
//!
//! # Implementation
//!
//! Use Redis with sliding window algorithm for distributed rate limiting.

use crate::error::Result;
use std::time::Duration;

/// Rate limiter for authentication attempts.
///
/// Implements sliding window rate limiting to prevent brute force attacks.
///
/// # Security
///
/// **CRITICAL**: Rate limiting prevents:
/// - Brute force password attacks
/// - Credential stuffing attacks
/// - Account enumeration via timing
/// - Denial of service via repeated authentication attempts
///
/// # Example
///
/// ```no_run
/// use composable_rust_auth::providers::RateLimiter;
/// use std::time::Duration;
///
/// # async fn example(limiter: impl RateLimiter) -> Result<(), Box<dyn std::error::Error>> {
/// // Check if user can attempt authentication
/// limiter.check_rate_limit("user@example.com", Duration::from_secs(900)).await?;
///
/// // ... perform authentication ...
///
/// // Record the attempt
/// limiter.record_attempt("user@example.com").await?;
/// # Ok(())
/// # }
/// ```
pub trait RateLimiter: Send + Sync {
    /// Check if the key is rate limited.
    ///
    /// # Arguments
    ///
    /// * `key` - Rate limit key (e.g., email address, IP address)
    /// * `window` - Time window for rate limiting
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Request allowed
    /// * `Err(AuthError::TooManyAttempts)` - Rate limit exceeded
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Rate limit exceeded → `AuthError::TooManyAttempts`
    /// - Database/`Redis` error → `AuthError::InternalError`
    fn check_rate_limit(
        &self,
        key: &str,
        window: Duration,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Record an authentication attempt.
    ///
    /// Increments the counter for the given key.
    ///
    /// # Arguments
    ///
    /// * `key` - Rate limit key (e.g., email address, IP address)
    ///
    /// # Errors
    ///
    /// Returns error if database/`Redis` operation fails.
    fn record_attempt(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Check and record in one atomic operation.
    ///
    /// More efficient than calling `check_rate_limit` + `record_attempt` separately.
    ///
    /// # Arguments
    ///
    /// * `key` - Rate limit key
    /// * `max_attempts` - Maximum attempts allowed in window
    /// * `window` - Time window duration
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Request allowed and recorded
    /// * `Err(AuthError::TooManyAttempts)` - Rate limit exceeded
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Rate limit exceeded → `AuthError::TooManyAttempts`
    /// - Database/`Redis` error → `AuthError::InternalError`
    fn check_and_record(
        &self,
        key: &str,
        max_attempts: u32,
        window: Duration,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Reset rate limit for a key.
    ///
    /// Useful for:
    /// - Successful authentication (reset failed attempt counter)
    /// - Admin override
    /// - Testing
    ///
    /// # Arguments
    ///
    /// * `key` - Rate limit key to reset
    ///
    /// # Errors
    ///
    /// Returns error if database/`Redis` operation fails.
    fn reset(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Get current attempt count for a key.
    ///
    /// Useful for monitoring and analytics.
    ///
    /// # Arguments
    ///
    /// * `key` - Rate limit key
    ///
    /// # Returns
    ///
    /// Number of attempts in current window.
    ///
    /// # Errors
    ///
    /// Returns error if database/`Redis` operation fails.
    fn get_attempts(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = Result<u32>> + Send;
}
