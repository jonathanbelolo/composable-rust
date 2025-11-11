//! Circuit Breaker Pattern (Phase 8.4 Part 3.1)
//!
//! Prevents cascading failures by detecting failures and stopping requests
//! to failing services temporarily.
//!
//! ## States
//!
//! ```text
//! Closed (normal) ──[failures >= threshold]──> Open (failing)
//!                                                     │
//!                                                     │ [timeout elapsed]
//!                                                     ▼
//!                                              HalfOpen (testing)
//!                                                     │
//!                      ┌──────────────────────────────┴───────────────┐
//!                      │                                              │
//!           [success >= threshold]                          [any failure]
//!                      │                                              │
//!                      ▼                                              ▼
//!                   Closed                                          Open
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! use agent_patterns::resilience::circuit_breaker::*;
//!
//! let config = CircuitBreakerConfig {
//!     failure_threshold: 5,
//!     success_threshold: 2,
//!     timeout: Duration::from_secs(30),
//! };
//!
//! let circuit_breaker = CircuitBreaker::new("llm_api".into(), config);
//!
//! // Check if request allowed
//! circuit_breaker.allow_request().await?;
//!
//! match risky_operation().await {
//!     Ok(result) => {
//!         circuit_breaker.record_success().await;
//!         Ok(result)
//!     }
//!     Err(e) => {
//!         circuit_breaker.record_failure().await;
//!         Err(e)
//!     }
//! }
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Circuit breaker state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation - requests allowed
    Closed,
    /// Failing - requests rejected
    Open,
    /// Testing recovery - limited requests allowed
    HalfOpen,
}

/// Circuit breaker configuration
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before opening circuit
    pub failure_threshold: usize,
    /// Number of consecutive successes in HalfOpen to close circuit
    pub success_threshold: usize,
    /// How long to wait in Open state before trying HalfOpen
    pub timeout: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 2,
            timeout: Duration::from_secs(30),
        }
    }
}

/// Internal state tracking
struct CircuitBreakerState {
    state: CircuitState,
    failure_count: usize,
    success_count: usize,
    last_failure_time: Option<Instant>,
}

/// Circuit breaker for protecting against cascading failures
///
/// Automatically opens when failures exceed threshold, preventing
/// further requests until timeout expires. Then allows test requests
/// to check if service has recovered.
pub struct CircuitBreaker {
    name: String,
    config: CircuitBreakerConfig,
    state: Arc<RwLock<CircuitBreakerState>>,
}

impl CircuitBreaker {
    /// Create new circuit breaker
    ///
    /// # Arguments
    ///
    /// * `name` - Name for logging (e.g., "llm_api", "database")
    /// * `config` - Circuit breaker configuration
    #[must_use]
    pub fn new(name: String, config: CircuitBreakerConfig) -> Self {
        Self {
            name,
            config,
            state: Arc::new(RwLock::new(CircuitBreakerState {
                state: CircuitState::Closed,
                failure_count: 0,
                success_count: 0,
                last_failure_time: None,
            })),
        }
    }

    /// Check if request should be allowed
    ///
    /// # Errors
    ///
    /// Returns error if circuit is Open (not ready to accept requests)
    pub async fn allow_request(&self) -> Result<(), String> {
        let mut state = self.state.write().await;

        match state.state {
            CircuitState::Closed => Ok(()),
            CircuitState::Open => {
                // Check if timeout has elapsed
                if let Some(last_failure) = state.last_failure_time {
                    if last_failure.elapsed() >= self.config.timeout {
                        info!(
                            "Circuit breaker {} transitioning: Open → HalfOpen",
                            self.name
                        );
                        state.state = CircuitState::HalfOpen;
                        state.success_count = 0;
                        Ok(())
                    } else {
                        Err(format!("Circuit breaker {} is OPEN", self.name))
                    }
                } else {
                    Err(format!("Circuit breaker {} is OPEN", self.name))
                }
            }
            CircuitState::HalfOpen => Ok(()),
        }
    }

    /// Record successful execution
    pub async fn record_success(&self) {
        let mut state = self.state.write().await;

        match state.state {
            CircuitState::Closed => {
                // Reset failure count on success
                state.failure_count = 0;
            }
            CircuitState::HalfOpen => {
                state.success_count += 1;
                if state.success_count >= self.config.success_threshold {
                    info!(
                        "Circuit breaker {} transitioning: HalfOpen → Closed (recovered)",
                        self.name
                    );
                    state.state = CircuitState::Closed;
                    state.failure_count = 0;
                    state.success_count = 0;
                    state.last_failure_time = None;
                }
            }
            CircuitState::Open => {
                // Shouldn't happen (request should be rejected), but handle gracefully
            }
        }
    }

    /// Record failed execution
    pub async fn record_failure(&self) {
        let mut state = self.state.write().await;

        match state.state {
            CircuitState::Closed => {
                state.failure_count += 1;
                if state.failure_count >= self.config.failure_threshold {
                    warn!(
                        "Circuit breaker {} transitioning: Closed → Open (failures: {})",
                        self.name, state.failure_count
                    );
                    state.state = CircuitState::Open;
                    state.last_failure_time = Some(Instant::now());
                }
            }
            CircuitState::HalfOpen => {
                warn!(
                    "Circuit breaker {} transitioning: HalfOpen → Open (recovery failed)",
                    self.name
                );
                state.state = CircuitState::Open;
                state.last_failure_time = Some(Instant::now());
                state.success_count = 0;
            }
            CircuitState::Open => {
                // Update last failure time
                state.last_failure_time = Some(Instant::now());
            }
        }
    }

    /// Get current circuit state
    pub async fn get_state(&self) -> CircuitState {
        self.state.read().await.state
    }

    /// Get circuit breaker name
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get current failure count
    pub async fn failure_count(&self) -> usize {
        self.state.read().await.failure_count
    }

    /// Get current success count (in HalfOpen state)
    pub async fn success_count(&self) -> usize {
        self.state.read().await.success_count
    }
}

/// Execute function with circuit breaker protection
///
/// Convenience function that checks circuit, executes function,
/// and records success/failure.
///
/// # Example
///
/// ```ignore
/// let result = with_circuit_breaker(&circuit_breaker, async {
///     external_api_call().await
/// }).await?;
/// ```
pub async fn with_circuit_breaker<F, T, E>(
    circuit_breaker: &CircuitBreaker,
    f: F,
) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    // Check if request is allowed
    circuit_breaker.allow_request().await?;

    // Execute function
    match f.await {
        Ok(result) => {
            circuit_breaker.record_success().await;
            Ok(result)
        }
        Err(e) => {
            circuit_breaker.record_failure().await;
            Err(e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_circuit_breaker_starts_closed() {
        let cb = CircuitBreaker::new(
            "test".to_string(),
            CircuitBreakerConfig::default(),
        );

        assert_eq!(cb.get_state().await, CircuitState::Closed);
        assert!(cb.allow_request().await.is_ok());
    }

    #[tokio::test]
    async fn test_circuit_breaker_opens_on_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            success_threshold: 2,
            timeout: Duration::from_secs(30),
        };

        let cb = CircuitBreaker::new("test".to_string(), config);

        // Record failures
        for _ in 0..3 {
            cb.record_failure().await;
        }

        // Should be open now
        assert_eq!(cb.get_state().await, CircuitState::Open);
        assert!(cb.allow_request().await.is_err());
    }

    #[tokio::test]
    async fn test_circuit_breaker_half_open_after_timeout() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 2,
            timeout: Duration::from_millis(100),
        };

        let cb = CircuitBreaker::new("test".to_string(), config);

        // Open circuit
        cb.record_failure().await;
        cb.record_failure().await;
        assert_eq!(cb.get_state().await, CircuitState::Open);

        // Wait for timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Should transition to HalfOpen on next request
        assert!(cb.allow_request().await.is_ok());
        assert_eq!(cb.get_state().await, CircuitState::HalfOpen);
    }

    #[tokio::test]
    async fn test_circuit_breaker_closes_after_success_threshold() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 2,
            timeout: Duration::from_millis(100),
        };

        let cb = CircuitBreaker::new("test".to_string(), config);

        // Open circuit
        cb.record_failure().await;
        cb.record_failure().await;

        // Wait for timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Transition to HalfOpen
        cb.allow_request().await.ok();

        // Record successes
        cb.record_success().await;
        cb.record_success().await;

        // Should be closed now
        assert_eq!(cb.get_state().await, CircuitState::Closed);
    }

    #[tokio::test]
    async fn test_circuit_breaker_reopens_on_halfopen_failure() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 2,
            timeout: Duration::from_millis(100),
        };

        let cb = CircuitBreaker::new("test".to_string(), config);

        // Open circuit
        cb.record_failure().await;
        cb.record_failure().await;

        // Wait for timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Transition to HalfOpen
        cb.allow_request().await.ok();
        assert_eq!(cb.get_state().await, CircuitState::HalfOpen);

        // Failure in HalfOpen should reopen circuit
        cb.record_failure().await;
        assert_eq!(cb.get_state().await, CircuitState::Open);
    }

    #[tokio::test]
    async fn test_circuit_breaker_success_resets_failure_count() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            success_threshold: 2,
            timeout: Duration::from_secs(30),
        };

        let cb = CircuitBreaker::new("test".to_string(), config);

        // Record some failures
        cb.record_failure().await;
        cb.record_failure().await;
        assert_eq!(cb.failure_count().await, 2);

        // Success should reset
        cb.record_success().await;
        assert_eq!(cb.failure_count().await, 0);
    }

    #[tokio::test]
    async fn test_with_circuit_breaker_success() {
        let cb = CircuitBreaker::new(
            "test".to_string(),
            CircuitBreakerConfig::default(),
        );

        let result = with_circuit_breaker(&cb, async {
            Ok::<_, String>("success")
        }).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
    }

    #[tokio::test]
    async fn test_with_circuit_breaker_failure() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            success_threshold: 2,
            timeout: Duration::from_secs(30),
        };

        let cb = CircuitBreaker::new("test".to_string(), config);

        let result = with_circuit_breaker(&cb, async {
            Err::<String, _>("error")
        }).await;

        assert!(result.is_err());
        assert_eq!(cb.get_state().await, CircuitState::Open);
    }

    #[tokio::test]
    async fn test_with_circuit_breaker_rejects_when_open() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            success_threshold: 2,
            timeout: Duration::from_secs(30),
        };

        let cb = CircuitBreaker::new("test".to_string(), config);

        // Open circuit
        cb.record_failure().await;

        // Should reject request
        let result = with_circuit_breaker(&cb, async {
            Ok::<_, String>("should not execute")
        }).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("OPEN"));
    }
}
