//! Circuit breaker pattern for preventing cascading failures.
//!
//! A circuit breaker monitors operations and "opens" (stops allowing requests) when
//! failures exceed a threshold, preventing cascading failures in distributed systems.
//!
//! # States
//!
//! - **Closed**: Normal operation. Requests pass through. Failures are counted.
//! - **Open**: Too many failures detected. Requests fail immediately for a timeout period.
//! - **HalfOpen**: After timeout, limited requests are allowed to test recovery.
//!
//! # Example
//!
//! ```rust
//! use composable_rust_runtime::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = CircuitBreakerConfig::builder()
//!     .failure_threshold(5)
//!     .timeout(Duration::from_secs(60))
//!     .success_threshold(2)
//!     .build();
//!
//! let breaker = CircuitBreaker::new(config);
//!
//! match breaker.call(|| async {
//!     // Your fallible operation
//!     Ok::<_, String>(42)
//! }).await {
//!     Ok(result) => println!("Success: {result}"),
//!     Err(e) => println!("Failed: {e}"),
//! }
//! # Ok(())
//! # }
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use thiserror::Error;

/// Circuit breaker configuration.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening the circuit
    pub failure_threshold: usize,
    /// Duration to wait before transitioning from Open to `HalfOpen`
    pub timeout: Duration,
    /// Number of successes in `HalfOpen` state before closing the circuit
    pub success_threshold: usize,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            timeout: Duration::from_secs(60),
            success_threshold: 2,
        }
    }
}

impl CircuitBreakerConfig {
    /// Create a new configuration builder.
    #[must_use]
    pub const fn builder() -> CircuitBreakerConfigBuilder {
        CircuitBreakerConfigBuilder {
            failure_threshold: Some(5),
            timeout: Some(Duration::from_secs(60)),
            success_threshold: Some(2),
        }
    }
}

/// Builder for [`CircuitBreakerConfig`].
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfigBuilder {
    failure_threshold: Option<usize>,
    timeout: Option<Duration>,
    success_threshold: Option<usize>,
}

impl CircuitBreakerConfigBuilder {
    /// Set the failure threshold.
    ///
    /// Circuit opens after this many consecutive failures.
    #[must_use]
    pub const fn failure_threshold(mut self, threshold: usize) -> Self {
        self.failure_threshold = Some(threshold);
        self
    }

    /// Set the timeout duration.
    ///
    /// How long to wait in Open state before trying `HalfOpen`.
    #[must_use]
    pub const fn timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(duration);
        self
    }

    /// Set the success threshold.
    ///
    /// Number of successes in `HalfOpen` state before closing the circuit.
    #[must_use]
    pub const fn success_threshold(mut self, threshold: usize) -> Self {
        self.success_threshold = Some(threshold);
        self
    }

    /// Build the configuration.
    #[must_use]
    pub fn build(self) -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            failure_threshold: self.failure_threshold.unwrap_or(5),
            timeout: self.timeout.unwrap_or(Duration::from_secs(60)),
            success_threshold: self.success_threshold.unwrap_or(2),
        }
    }
}

/// Circuit breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Circuit is closed, requests pass through normally
    Closed,
    /// Circuit is open, requests fail immediately
    Open,
    /// Circuit is half-open, testing if service recovered
    HalfOpen,
}

/// Errors from circuit breaker operations.
#[derive(Error, Debug)]
pub enum CircuitBreakerError<E> {
    /// Circuit is open, request rejected
    #[error("Circuit breaker is open")]
    Open,
    /// Operation failed
    #[error("Operation failed: {0}")]
    Inner(E),
}

/// Internal state of the circuit breaker.
#[derive(Debug)]
struct CircuitBreakerState {
    state: State,
    failure_count: usize,
    success_count: usize,
    last_failure_time: Option<Instant>,
}

/// Circuit breaker for preventing cascading failures.
///
/// Wraps operations and monitors their success/failure. When failures exceed
/// a threshold, the circuit "opens" and rejects requests for a timeout period.
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    config: Arc<CircuitBreakerConfig>,
    state: Arc<RwLock<CircuitBreakerState>>,
    // Metrics
    total_calls: Arc<AtomicU64>,
    total_successes: Arc<AtomicU64>,
    total_failures: Arc<AtomicU64>,
    total_rejections: Arc<AtomicU64>,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with the given configuration.
    #[must_use]
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config: Arc::new(config),
            state: Arc::new(RwLock::new(CircuitBreakerState {
                state: State::Closed,
                failure_count: 0,
                success_count: 0,
                last_failure_time: None,
            })),
            total_calls: Arc::new(AtomicU64::new(0)),
            total_successes: Arc::new(AtomicU64::new(0)),
            total_failures: Arc::new(AtomicU64::new(0)),
            total_rejections: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Get the current state of the circuit breaker.
    pub async fn state(&self) -> State {
        let state = self.state.read().await;
        state.state
    }

    /// Call an operation through the circuit breaker.
    ///
    /// # Errors
    ///
    /// Returns `CircuitBreakerError::Open` if the circuit is open.
    /// Returns `CircuitBreakerError::Inner` if the operation fails.
    pub async fn call<F, Fut, T, E>(&self, operation: F) -> Result<T, CircuitBreakerError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        self.total_calls.fetch_add(1, Ordering::Relaxed);

        // Check if we should allow this request
        if !self.can_attempt().await {
            self.total_rejections.fetch_add(1, Ordering::Relaxed);
            tracing::warn!("Circuit breaker is OPEN, rejecting request");
            return Err(CircuitBreakerError::Open);
        }

        // Execute the operation
        match operation().await {
            Ok(result) => {
                self.on_success().await;
                self.total_successes.fetch_add(1, Ordering::Relaxed);
                Ok(result)
            }
            Err(err) => {
                self.on_failure().await;
                self.total_failures.fetch_add(1, Ordering::Relaxed);
                Err(CircuitBreakerError::Inner(err))
            }
        }
    }

    /// Check if the circuit breaker should allow an attempt.
    async fn can_attempt(&self) -> bool {
        let mut state = self.state.write().await;

        match state.state {
            State::Closed | State::HalfOpen => true,
            State::Open => {
                // Check if timeout has expired
                if let Some(last_failure) = state.last_failure_time {
                    if last_failure.elapsed() >= self.config.timeout {
                        tracing::info!("Circuit breaker transitioning OPEN -> HALF_OPEN");
                        state.state = State::HalfOpen;
                        state.success_count = 0;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
        }
    }

    /// Handle successful operation.
    async fn on_success(&self) {
        let mut state = self.state.write().await;

        match state.state {
            State::Closed => {
                // Reset failure count on success
                state.failure_count = 0;
            }
            State::HalfOpen => {
                state.success_count += 1;
                if state.success_count >= self.config.success_threshold {
                    tracing::info!(
                        successes = state.success_count,
                        "Circuit breaker transitioning HALF_OPEN -> CLOSED"
                    );
                    state.state = State::Closed;
                    state.failure_count = 0;
                    state.success_count = 0;
                    state.last_failure_time = None;
                }
            }
            State::Open => {
                // Should not happen, but reset just in case
                state.failure_count = 0;
            }
        }
    }

    /// Handle failed operation.
    async fn on_failure(&self) {
        let mut state = self.state.write().await;
        state.last_failure_time = Some(Instant::now());

        match state.state {
            State::Closed => {
                state.failure_count += 1;
                if state.failure_count >= self.config.failure_threshold {
                    tracing::warn!(
                        failures = state.failure_count,
                        threshold = self.config.failure_threshold,
                        "Circuit breaker transitioning CLOSED -> OPEN"
                    );
                    state.state = State::Open;
                }
            }
            State::HalfOpen => {
                tracing::warn!("Circuit breaker transitioning HALF_OPEN -> OPEN (recovery failed)");
                state.state = State::Open;
                state.failure_count = 1;
                state.success_count = 0;
            }
            State::Open => {
                // Already open, just update failure count
                state.failure_count += 1;
            }
        }
    }

    /// Get circuit breaker metrics.
    #[must_use]
    pub fn metrics(&self) -> CircuitBreakerMetrics {
        CircuitBreakerMetrics {
            total_calls: self.total_calls.load(Ordering::Relaxed),
            total_successes: self.total_successes.load(Ordering::Relaxed),
            total_failures: self.total_failures.load(Ordering::Relaxed),
            total_rejections: self.total_rejections.load(Ordering::Relaxed),
        }
    }

    /// Reset the circuit breaker to closed state.
    ///
    /// Useful for testing or manual intervention.
    pub async fn reset(&self) {
        let mut state = self.state.write().await;
        tracing::info!("Circuit breaker manually reset to CLOSED");
        state.state = State::Closed;
        state.failure_count = 0;
        state.success_count = 0;
        state.last_failure_time = None;
    }
}

/// Metrics for circuit breaker monitoring.
#[derive(Debug, Clone, Copy)]
pub struct CircuitBreakerMetrics {
    /// Total number of calls attempted
    pub total_calls: u64,
    /// Total number of successful calls
    pub total_successes: u64,
    /// Total number of failed calls
    pub total_failures: u64,
    /// Total number of rejected calls (circuit open)
    pub total_rejections: u64,
}

impl CircuitBreakerMetrics {
    /// Calculate success rate (0.0 to 1.0).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn success_rate(&self) -> f64 {
        if self.total_calls == 0 {
            return 1.0;
        }
        self.total_successes as f64 / self.total_calls as f64
    }

    /// Calculate rejection rate (0.0 to 1.0).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn rejection_rate(&self) -> f64 {
        if self.total_calls == 0 {
            return 0.0;
        }
        self.total_rejections as f64 / self.total_calls as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_circuit_breaker_closed_on_success() {
        let config = CircuitBreakerConfig::default();
        let breaker = CircuitBreaker::new(config);

        let result = breaker.call(|| async { Ok::<_, String>(42) }).await;

        assert!(result.is_ok());
        assert_eq!(breaker.state().await, State::Closed);
    }

    #[tokio::test]
    async fn test_circuit_breaker_opens_after_threshold() {
        let config = CircuitBreakerConfig::builder()
            .failure_threshold(3)
            .build();
        let breaker = CircuitBreaker::new(config);

        // Fail 3 times
        for _ in 0..3 {
            let _ = breaker.call(|| async { Err::<i32, _>("error") }).await;
        }

        assert_eq!(breaker.state().await, State::Open);
    }

    #[tokio::test]
    async fn test_circuit_breaker_rejects_when_open() {
        let config = CircuitBreakerConfig::builder()
            .failure_threshold(2)
            .build();
        let breaker = CircuitBreaker::new(config);

        // Open the circuit
        for _ in 0..2 {
            let _ = breaker.call(|| async { Err::<i32, _>("error") }).await;
        }

        // Next call should be rejected
        let result = breaker.call(|| async { Ok::<_, String>(42) }).await;

        assert!(matches!(result, Err(CircuitBreakerError::Open)));
    }

    #[tokio::test]
    async fn test_circuit_breaker_transitions_to_half_open() {
        let config = CircuitBreakerConfig::builder()
            .failure_threshold(2)
            .timeout(Duration::from_millis(100))
            .build();
        let breaker = CircuitBreaker::new(config);

        // Open the circuit
        for _ in 0..2 {
            let _ = breaker.call(|| async { Err::<i32, _>("error") }).await;
        }

        assert_eq!(breaker.state().await, State::Open);

        // Wait for timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Next call should transition to HalfOpen
        let _ = breaker.call(|| async { Ok::<_, String>(42) }).await;

        // Should be either HalfOpen or Closed (if success threshold is 1)
        let state = breaker.state().await;
        assert!(state == State::HalfOpen || state == State::Closed);
    }

    #[tokio::test]
    async fn test_circuit_breaker_closes_after_success_threshold() {
        let config = CircuitBreakerConfig::builder()
            .failure_threshold(2)
            .timeout(Duration::from_millis(100))
            .success_threshold(2)
            .build();
        let breaker = CircuitBreaker::new(config);

        // Open the circuit
        for _ in 0..2 {
            let _ = breaker.call(|| async { Err::<i32, _>("error") }).await;
        }

        // Wait for timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Succeed twice in HalfOpen state
        for _ in 0..2 {
            let _ = breaker.call(|| async { Ok::<_, String>(42) }).await;
        }

        assert_eq!(breaker.state().await, State::Closed);
    }

    #[tokio::test]
    async fn test_circuit_breaker_reopens_on_half_open_failure() {
        let config = CircuitBreakerConfig::builder()
            .failure_threshold(2)
            .timeout(Duration::from_millis(100))
            .build();
        let breaker = CircuitBreaker::new(config);

        // Open the circuit
        for _ in 0..2 {
            let _ = breaker.call(|| async { Err::<i32, _>("error") }).await;
        }

        // Wait for timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Fail in HalfOpen state
        let _ = breaker.call(|| async { Err::<i32, _>("error") }).await;

        assert_eq!(breaker.state().await, State::Open);
    }

    #[tokio::test]
    async fn test_circuit_breaker_metrics() {
        let config = CircuitBreakerConfig::default();
        let breaker = CircuitBreaker::new(config);

        // 3 successes
        for _ in 0..3 {
            let _ = breaker.call(|| async { Ok::<_, String>(42) }).await;
        }

        // 2 failures
        for _ in 0..2 {
            let _ = breaker.call(|| async { Err::<i32, _>("error") }).await;
        }

        let metrics = breaker.metrics();
        assert_eq!(metrics.total_calls, 5);
        assert_eq!(metrics.total_successes, 3);
        assert_eq!(metrics.total_failures, 2);
        assert_eq!(metrics.success_rate(), 0.6);
    }

    #[tokio::test]
    async fn test_circuit_breaker_reset() {
        let config = CircuitBreakerConfig::builder()
            .failure_threshold(2)
            .build();
        let breaker = CircuitBreaker::new(config);

        // Open the circuit
        for _ in 0..2 {
            let _ = breaker.call(|| async { Err::<i32, _>("error") }).await;
        }

        assert_eq!(breaker.state().await, State::Open);

        // Reset
        breaker.reset().await;

        assert_eq!(breaker.state().await, State::Closed);
    }

    #[tokio::test]
    async fn test_circuit_breaker_concurrent_calls() {
        let config = CircuitBreakerConfig::default();
        let breaker = Arc::new(CircuitBreaker::new(config));

        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = vec![];

        for _ in 0..100 {
            let breaker_clone = Arc::clone(&breaker);
            let counter_clone = Arc::clone(&counter);

            let handle = tokio::spawn(async move {
                let _ = breaker_clone
                    .call(|| async {
                        counter_clone.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, String>(())
                    })
                    .await;
            });

            handles.push(handle);
        }

        for handle in handles {
            handle.await.ok();
        }

        let metrics = breaker.metrics();
        assert_eq!(metrics.total_calls, 100);
        assert_eq!(metrics.total_successes, 100);
        assert_eq!(counter.load(Ordering::SeqCst), 100);
    }
}
