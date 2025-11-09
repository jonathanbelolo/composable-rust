//! # Composable Rust Runtime
//!
//! Runtime implementation for the Composable Rust architecture.
//!
//! This crate provides the Store runtime that coordinates reducer execution
//! and effect handling.
//!
//! ## Core Components
//!
//! - **Store**: The runtime that manages state and executes effects
//! - **Effect Executor**: Executes effect descriptions and feeds actions back to reducers
//! - **Event Loop**: Manages the action → reducer → effects → action feedback loop
//!
//! ## Example
//!
//! ```ignore
//! use composable_rust_runtime::Store;
//! use composable_rust_core::Reducer;
//!
//! let store = Store::new(
//!     initial_state,
//!     my_reducer,
//!     environment,
//! );
//!
//! // Send an action
//! store.send(Action::DoSomething).await;
//!
//! // Read state
//! let value = store.state(|s| s.some_field).await;
//! ```

use composable_rust_core::{effect::Effect, reducer::Reducer};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Retry logic with exponential backoff
pub mod retry;

/// Circuit breaker pattern for preventing cascading failures
pub mod circuit_breaker;

/// Prometheus metrics for observability
pub mod metrics;

/// Error types for the Store runtime
pub mod error {
    use thiserror::Error;

    /// Errors that can occur during Store operations
    ///
    /// # Phase 1 Implementation
    ///
    /// Currently includes basic effect execution errors.
    /// Future phases will add more error types as needed.
    #[derive(Error, Debug)]
    pub enum StoreError {
        /// An effect execution failed
        ///
        /// This error is logged but does not halt the store.
        /// Effects are fire-and-forget operations.
        #[error("Effect execution failed: {0}")]
        EffectFailed(String),

        /// A task join error occurred during parallel effect execution
        ///
        /// This typically means a spawned task panicked.
        #[error("Task failed during parallel execution: {0}")]
        TaskJoinError(#[from] tokio::task::JoinError),

        /// Store is shutting down and not accepting new actions
        ///
        /// This error is returned when `send()` is called after shutdown initiated.
        #[error("Store is shutting down")]
        ShutdownInProgress,

        /// Shutdown timed out waiting for effects to complete
        ///
        /// Some effects were still running when the timeout elapsed.
        #[error("Shutdown timed out with {0} effects still running")]
        ShutdownTimeout(usize),

        /// Timeout waiting for terminal action
        ///
        /// Returned by `send_and_wait_for` when the timeout expires before
        /// a matching action is received.
        #[error("Timeout waiting for action")]
        Timeout,

        /// Action broadcast channel closed
        ///
        /// The action broadcast channel was closed, typically because the
        /// store is shutting down.
        #[error("Action broadcast channel closed")]
        ChannelClosed,
    }
}

/// Health check status levels
///
/// Indicates the current health state of a component or system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HealthStatus {
    /// Component is fully operational
    Healthy,

    /// Component is operational but experiencing issues (e.g., high DLQ size)
    Degraded,

    /// Component is not operational
    Unhealthy,
}

impl HealthStatus {
    /// Check if status is healthy
    #[must_use]
    pub const fn is_healthy(self) -> bool {
        matches!(self, Self::Healthy)
    }

    /// Check if status is degraded
    #[must_use]
    pub const fn is_degraded(self) -> bool {
        matches!(self, Self::Degraded)
    }

    /// Check if status is unhealthy
    #[must_use]
    pub const fn is_unhealthy(self) -> bool {
        matches!(self, Self::Unhealthy)
    }

    /// Get the worst status between two statuses
    #[must_use]
    pub const fn worst(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unhealthy, _) | (_, Self::Unhealthy) => Self::Unhealthy,
            (Self::Degraded, _) | (_, Self::Degraded) => Self::Degraded,
            _ => Self::Healthy,
        }
    }
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Degraded => write!(f, "degraded"),
            Self::Unhealthy => write!(f, "unhealthy"),
        }
    }
}

/// Health check result for a component
#[derive(Debug, Clone)]
pub struct HealthCheck {
    /// Name of the component being checked
    pub component: String,

    /// Current health status
    pub status: HealthStatus,

    /// Optional message providing details
    pub message: Option<String>,

    /// Optional metadata (e.g., metrics, error counts)
    pub metadata: Vec<(String, String)>,
}

impl HealthCheck {
    /// Create a healthy check result
    #[must_use]
    pub fn healthy(component: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: HealthStatus::Healthy,
            message: None,
            metadata: Vec::new(),
        }
    }

    /// Create a degraded check result
    #[must_use]
    pub fn degraded(component: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: HealthStatus::Degraded,
            message: Some(message.into()),
            metadata: Vec::new(),
        }
    }

    /// Create an unhealthy check result
    #[must_use]
    pub fn unhealthy(component: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: HealthStatus::Unhealthy,
            message: Some(message.into()),
            metadata: Vec::new(),
        }
    }

    /// Add metadata to the health check
    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.push((key.into(), value.into()));
        self
    }
}

/// Aggregated health report
///
/// Combines multiple health checks into an overall system status.
#[derive(Debug, Clone)]
pub struct HealthReport {
    /// Overall system status (worst of all checks)
    pub status: HealthStatus,

    /// Individual component checks
    pub checks: Vec<HealthCheck>,

    /// Timestamp when report was generated
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl HealthReport {
    /// Create a new health report from checks
    #[must_use]
    pub fn new(checks: Vec<HealthCheck>) -> Self {
        let status = checks
            .iter()
            .map(|c| c.status)
            .fold(HealthStatus::Healthy, HealthStatus::worst);

        Self {
            status,
            checks,
            timestamp: chrono::Utc::now(),
        }
    }

    /// Check if overall system is healthy
    #[must_use]
    pub const fn is_healthy(&self) -> bool {
        self.status.is_healthy()
    }

    /// Check if overall system is degraded
    #[must_use]
    pub const fn is_degraded(&self) -> bool {
        self.status.is_degraded()
    }

    /// Check if overall system is unhealthy
    #[must_use]
    pub const fn is_unhealthy(&self) -> bool {
        self.status.is_unhealthy()
    }
}

/// Retry policy for handling transient failures
///
/// Implements exponential backoff with jitter to handle transient failures
/// gracefully without overwhelming downstream services.
///
/// # Example
///
/// ```ignore
/// use composable_rust_runtime::RetryPolicy;
/// use std::time::Duration;
///
/// let policy = RetryPolicy::default();
/// // Or customize:
/// let policy = RetryPolicy::new()
///     .with_max_attempts(10)
///     .with_initial_delay(Duration::from_millis(500));
/// ```
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts (including initial attempt)
    max_attempts: u32,

    /// Initial delay before first retry
    initial_delay: Duration,

    /// Maximum delay between retries (caps exponential backoff)
    max_delay: Duration,

    /// Multiplier for exponential backoff (2.0 = double each time)
    backoff_multiplier: f64,
}

impl RetryPolicy {
    /// Create a new retry policy with default settings
    ///
    /// Defaults:
    /// - `max_attempts`: 5
    /// - `initial_delay`: 1 second
    /// - `max_delay`: 32 seconds
    /// - `backoff_multiplier`: 2.0 (exponential)
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(32),
            backoff_multiplier: 2.0,
        }
    }

    /// Set maximum retry attempts
    #[must_use]
    pub const fn with_max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = attempts;
        self
    }

    /// Set initial delay before first retry
    #[must_use]
    pub const fn with_initial_delay(mut self, delay: Duration) -> Self {
        self.initial_delay = delay;
        self
    }

    /// Set maximum delay between retries
    #[must_use]
    pub const fn with_max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    /// Set backoff multiplier for exponential backoff
    #[must_use]
    pub const fn with_backoff_multiplier(mut self, multiplier: f64) -> Self {
        self.backoff_multiplier = multiplier;
        self
    }

    /// Calculate delay for a given attempt number (0-indexed)
    ///
    /// Uses exponential backoff with jitter:
    /// `delay = min(initial_delay * multiplier^attempt, max_delay) * (0.5 + random(0.5))`
    ///
    /// Jitter prevents thundering herd problem.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        use rand::Rng;

        // Calculate exponential backoff: initial * multiplier^attempt
        // Note: Cast is safe since max_attempts defaults to 5 (well within i32 range)
        #[allow(clippy::cast_possible_wrap)]
        let base_delay_secs = self.initial_delay.as_secs_f64()
            * self.backoff_multiplier.powi(attempt as i32);

        // Cap at max_delay
        let capped_secs = base_delay_secs.min(self.max_delay.as_secs_f64());

        // Add jitter: multiply by random value between 0.5 and 1.0
        // This spreads out retries to prevent thundering herd
        let jitter = rand::thread_rng().gen_range(0.5..=1.0);
        let final_secs = capped_secs * jitter;

        Duration::from_secs_f64(final_secs)
    }

    /// Get maximum number of attempts
    #[must_use]
    pub const fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// Check if we should retry based on attempt number
    #[must_use]
    pub const fn should_retry(&self, attempt: u32) -> bool {
        attempt < self.max_attempts
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::new()
    }
}

/// Circuit breaker state
///
/// A circuit breaker prevents cascading failures by "opening" after
/// a threshold of failures, rejecting requests immediately rather than
/// attempting them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Circuit is closed - normal operation, requests pass through
    Closed,

    /// Circuit is open - failing fast, rejecting requests immediately
    Open,

    /// Circuit is half-open - testing if service recovered
    HalfOpen,
}

/// Circuit breaker error
#[derive(Debug, Clone, thiserror::Error)]
pub enum CircuitBreakerError {
    /// Circuit is open, rejecting requests
    #[error("Circuit breaker is open")]
    Open,
}

/// Circuit breaker for preventing cascading failures
///
/// The circuit breaker pattern prevents cascading failures by tracking
/// failure rates and "opening" the circuit after a threshold is reached.
/// When open, requests fail fast rather than attempting the operation.
///
/// # States
///
/// - **Closed**: Normal operation, all requests go through
/// - **Open**: Failing fast, rejecting all requests immediately
/// - **`HalfOpen`**: Testing recovery with limited requests
///
/// # State Transitions
///
/// - `Closed` → `Open`: After `failure_threshold` consecutive failures
/// - `Open` → `HalfOpen`: After `timeout` duration
/// - `HalfOpen` → `Closed`: After `success_threshold` consecutive successes
/// - `HalfOpen` → `Open`: On any failure
///
/// # Example
///
/// ```ignore
/// use composable_rust_runtime::CircuitBreaker;
/// use std::time::Duration;
///
/// let breaker = CircuitBreaker::new()
///     .with_failure_threshold(5)
///     .with_timeout(Duration::from_secs(30));
///
/// // Check before operation
/// breaker.check_and_call(|| async {
///     database.query(...).await
/// }).await?;
/// ```
#[derive(Debug)]
pub struct CircuitBreaker {
    /// Current state
    state: Arc<AtomicU8>,

    /// Consecutive failure count
    failure_count: Arc<AtomicUsize>,

    /// Success count in `HalfOpen` state
    success_count: Arc<AtomicUsize>,

    /// Timestamp when circuit was opened (nanoseconds since epoch)
    opened_at: Arc<AtomicU64>,

    /// Number of consecutive failures before opening circuit
    failure_threshold: usize,

    /// Duration to wait before attempting `HalfOpen`
    timeout: Duration,

    /// Number of consecutive successes in `HalfOpen` to close circuit
    success_threshold: usize,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with default settings
    ///
    /// Defaults:
    /// - `failure_threshold`: 5
    /// - `timeout`: 60 seconds
    /// - `success_threshold`: 2
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(CircuitState::Closed as u8)),
            failure_count: Arc::new(AtomicUsize::new(0)),
            success_count: Arc::new(AtomicUsize::new(0)),
            opened_at: Arc::new(AtomicU64::new(0)),
            failure_threshold: 5,
            timeout: Duration::from_secs(60),
            success_threshold: 2,
        }
    }

    /// Set the failure threshold
    ///
    /// Number of consecutive failures before opening the circuit.
    #[must_use]
    pub const fn with_failure_threshold(mut self, threshold: usize) -> Self {
        self.failure_threshold = threshold;
        self
    }

    /// Set the timeout duration
    ///
    /// Duration to wait before attempting `HalfOpen` state.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the success threshold
    ///
    /// Number of consecutive successes in `HalfOpen` to close circuit.
    #[must_use]
    pub const fn with_success_threshold(mut self, threshold: usize) -> Self {
        self.success_threshold = threshold;
        self
    }

    /// Get current circuit state
    #[must_use]
    pub fn state(&self) -> CircuitState {
        match self.state.load(Ordering::Acquire) {
            1 => CircuitState::Open,
            2 => CircuitState::HalfOpen,
            _ => CircuitState::Closed, // Includes 0 and any unexpected values
        }
    }

    /// Check if we should allow a request through
    ///
    /// Returns `Ok(())` if request should proceed, `Err` if circuit is open.
    fn check(&self) -> Result<(), CircuitBreakerError> {
        let current_state = self.state();

        match current_state {
            CircuitState::Open => {
                // Check if timeout has elapsed
                let opened_at_nanos = self.opened_at.load(Ordering::Acquire);
                // Note: Truncation acceptable for nanosecond timestamps (wraps every ~584 years)
                #[allow(clippy::cast_possible_truncation)]
                let now_nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO)
                    .as_nanos() as u64;

                let elapsed = Duration::from_nanos(now_nanos.saturating_sub(opened_at_nanos));

                if elapsed >= self.timeout {
                    // Transition to HalfOpen
                    self.state.store(CircuitState::HalfOpen as u8, Ordering::Release);
                    self.success_count.store(0, Ordering::Release);

                    metrics::counter!("circuit_breaker.state_change", "from" => "open", "to" => "half_open")
                        .increment(1);
                    tracing::info!("Circuit breaker transitioning from Open to HalfOpen");

                    Ok(())
                } else {
                    Err(CircuitBreakerError::Open)
                }
            },
            CircuitState::Closed | CircuitState::HalfOpen => Ok(()),
        }
    }

    /// Record a successful operation
    pub fn record_success(&self) {
        let current_state = self.state();

        match current_state {
            CircuitState::Closed => {
                // Reset failure count
                self.failure_count.store(0, Ordering::Release);
            },
            CircuitState::HalfOpen => {
                let successes = self.success_count.fetch_add(1, Ordering::AcqRel) + 1;

                if successes >= self.success_threshold {
                    // Close the circuit
                    self.state.store(CircuitState::Closed as u8, Ordering::Release);
                    self.failure_count.store(0, Ordering::Release);
                    self.success_count.store(0, Ordering::Release);

                    metrics::counter!("circuit_breaker.state_change", "from" => "half_open", "to" => "closed")
                        .increment(1);
                    tracing::info!("Circuit breaker transitioning from HalfOpen to Closed");
                }
            },
            CircuitState::Open => {
                // Should not happen, but ignore
            },
        }
    }

    /// Record a failed operation
    pub fn record_failure(&self) {
        let current_state = self.state();

        match current_state {
            CircuitState::Closed => {
                let failures = self.failure_count.fetch_add(1, Ordering::AcqRel) + 1;

                if failures >= self.failure_threshold {
                    // Open the circuit
                    self.state.store(CircuitState::Open as u8, Ordering::Release);

                    // Note: Truncation acceptable for nanosecond timestamps (wraps every ~584 years)
                    #[allow(clippy::cast_possible_truncation)]
                    let now_nanos = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or(Duration::ZERO)
                        .as_nanos() as u64;
                    self.opened_at.store(now_nanos, Ordering::Release);

                    metrics::counter!("circuit_breaker.state_change", "from" => "closed", "to" => "open")
                        .increment(1);
                    tracing::warn!(
                        failures = failures,
                        threshold = self.failure_threshold,
                        "Circuit breaker opening due to failures"
                    );
                }
            },
            CircuitState::HalfOpen => {
                // Any failure in HalfOpen opens circuit immediately
                self.state.store(CircuitState::Open as u8, Ordering::Release);
                self.success_count.store(0, Ordering::Release);

                // Note: Truncation acceptable for nanosecond timestamps (wraps every ~584 years)
                #[allow(clippy::cast_possible_truncation)]
                let now_nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO)
                    .as_nanos() as u64;
                self.opened_at.store(now_nanos, Ordering::Release);

                metrics::counter!("circuit_breaker.state_change", "from" => "half_open", "to" => "open")
                    .increment(1);
                tracing::warn!("Circuit breaker opening from HalfOpen due to failure");
            },
            CircuitState::Open => {
                // Already open, nothing to do
            },
        }
    }

    /// Execute an operation with circuit breaker protection
    ///
    /// # Errors
    ///
    /// Returns `CircuitBreakerError::Open` if circuit is open.
    /// Returns any error from the operation itself.
    pub async fn call<F, Fut, T, E>(&self, f: F) -> Result<T, Either<CircuitBreakerError, E>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        self.check().map_err(Either::Left)?;

        match f().await {
            Ok(result) => {
                self.record_success();
                Ok(result)
            },
            Err(error) => {
                self.record_failure();
                Err(Either::Right(error))
            },
        }
    }
}

impl Clone for CircuitBreaker {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            failure_count: Arc::clone(&self.failure_count),
            success_count: Arc::clone(&self.success_count),
            opened_at: Arc::clone(&self.opened_at),
            failure_threshold: self.failure_threshold,
            timeout: self.timeout,
            success_threshold: self.success_threshold,
        }
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

/// Either type for circuit breaker results
#[derive(Debug)]
pub enum Either<L, R> {
    /// Left variant (circuit breaker error)
    Left(L),
    /// Right variant (operation error)
    Right(R),
}

impl<L: std::fmt::Display, R: std::fmt::Display> std::fmt::Display for Either<L, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Either::Left(l) => write!(f, "{l}"),
            Either::Right(r) => write!(f, "{r}"),
        }
    }
}

impl<L: std::error::Error, R: std::error::Error> std::error::Error for Either<L, R> {}

/// Dead letter queue entry
///
/// Represents a failed operation with metadata about the failure.
#[derive(Debug, Clone)]
pub struct DeadLetter<T> {
    /// The failed operation payload
    pub payload: T,

    /// Number of times this operation was retried
    pub retry_count: usize,

    /// The error message from the last failure
    pub error_message: String,

    /// Timestamp when first failed (nanoseconds since epoch)
    pub first_failed_at: u64,

    /// Timestamp when last failed (nanoseconds since epoch)
    pub last_failed_at: u64,
}

impl<T> DeadLetter<T> {
    /// Create a new dead letter entry
    fn new(payload: T, error_message: String, retry_count: usize) -> Self {
        // Note: Truncation acceptable for nanosecond timestamps (wraps every ~584 years)
        #[allow(clippy::cast_possible_truncation)]
        let now_nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos() as u64;

        Self {
            payload,
            retry_count,
            error_message,
            first_failed_at: now_nanos,
            last_failed_at: now_nanos,
        }
    }

}

/// Dead Letter Queue for storing failed operations
///
/// The DLQ stores operations that failed after exhausting retries.
/// These can be inspected, monitored, and potentially retried manually.
///
/// # Features
///
/// - Bounded queue with configurable max size
/// - FIFO ordering (oldest entries dropped when full)
/// - Thread-safe for concurrent access
/// - Metrics tracking for queue size and operations
///
/// # Example
///
/// ```ignore
/// use composable_rust_runtime::DeadLetterQueue;
///
/// let dlq = DeadLetterQueue::new(1000);
///
/// // Add a failed operation
/// dlq.push("operation_data".to_string(), "Connection timeout".to_string(), 5);
///
/// // Check queue size
/// println!("Failed operations: {}", dlq.len());
///
/// // Drain and retry
/// for entry in dlq.drain() {
///     println!("Retry: {:?}", entry);
/// }
/// ```
#[derive(Debug)]
pub struct DeadLetterQueue<T> {
    /// The queue storage
    queue: Arc<Mutex<VecDeque<DeadLetter<T>>>>,

    /// Maximum queue size
    max_size: usize,
}

impl<T> DeadLetterQueue<T> {
    /// Create a new dead letter queue with the given max size
    ///
    /// # Arguments
    ///
    /// - `max_size`: Maximum number of entries to store
    ///
    /// # Returns
    ///
    /// A new empty `DeadLetterQueue`
    #[must_use]
    pub fn new(max_size: usize) -> Self {
        Self {
            queue: Arc::new(Mutex::new(VecDeque::new())),
            max_size,
        }
    }

    /// Push a failed operation onto the queue
    ///
    /// If the queue is full, the oldest entry is dropped.
    ///
    /// # Arguments
    ///
    /// - `payload`: The operation data
    /// - `error_message`: Description of the failure
    /// - `retry_count`: Number of times operation was retried
    pub fn push(&self, payload: T, error_message: String, retry_count: usize) {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Drop oldest if at capacity
        if queue.len() >= self.max_size {
            queue.pop_front();
            metrics::counter!("dlq.dropped").increment(1);
            tracing::warn!(
                max_size = self.max_size,
                "DLQ at capacity, dropping oldest entry"
            );
        }

        let entry = DeadLetter::new(payload, error_message, retry_count);
        queue.push_back(entry);

        // Intentional cast for metrics - queue size limited by max_size (usize) and f64 can
        // represent all practical queue sizes (up to 2^53 exactly)
        #[allow(clippy::cast_precision_loss)]
        metrics::gauge!("dlq.size").set(queue.len() as f64);
        metrics::counter!("dlq.pushed").increment(1);

        tracing::warn!(
            retry_count = retry_count,
            queue_size = queue.len(),
            "Operation added to dead letter queue"
        );
    }

    /// Get the current queue size
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Check if the queue is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drain all entries from the queue
    ///
    /// Returns all entries and empties the queue.
    pub fn drain(&self) -> Vec<DeadLetter<T>> {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entries: Vec<_> = queue.drain(..).collect();

        metrics::gauge!("dlq.size").set(0.0);
        metrics::counter!("dlq.drained").increment(entries.len() as u64);

        tracing::info!(count = entries.len(), "Drained dead letter queue");

        entries
    }

    /// Peek at the oldest entry without removing it
    #[must_use]
    pub fn peek(&self) -> Option<DeadLetter<T>>
    where
        T: Clone,
    {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .front()
            .cloned()
    }

    /// Get the maximum queue size
    #[must_use]
    pub const fn max_size(&self) -> usize {
        self.max_size
    }
}

impl<T> Clone for DeadLetterQueue<T> {
    fn clone(&self) -> Self {
        Self {
            queue: Arc::clone(&self.queue),
            max_size: self.max_size,
        }
    }
}

impl<T> Default for DeadLetterQueue<T> {
    fn default() -> Self {
        Self::new(1000)
    }
}

pub use error::StoreError;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Mutex, Weak};
use std::time::Duration;
use tokio::sync::watch;

/// Configuration for Store instances
///
/// Provides configurable parameters for DLQ size, retry policy, and other runtime settings.
///
/// # Example
///
/// ```ignore
/// let config = StoreConfig::default()
///     .with_dlq_max_size(5000)
///     .with_retry_policy(
///         RetryPolicy::new()
///             .with_max_attempts(5)
///             .with_initial_delay(Duration::from_millis(200))
///     );
///
/// let store = Store::with_config(state, reducer, env, config);
/// ```
#[derive(Debug, Clone)]
pub struct StoreConfig {
    /// Maximum size of the dead letter queue
    pub dlq_max_size: usize,
    /// Retry policy for failed effects
    pub retry_policy: RetryPolicy,
    /// Default timeout for graceful shutdown
    pub default_shutdown_timeout: Duration,
}

impl StoreConfig {
    /// Create a new configuration with custom values
    ///
    /// # Arguments
    ///
    /// - `dlq_max_size`: Maximum number of items in the DLQ
    /// - `retry_policy`: Policy for retrying failed effects
    /// - `default_shutdown_timeout`: Default timeout for shutdown operations
    #[must_use]
    pub const fn new(
        dlq_max_size: usize,
        retry_policy: RetryPolicy,
        default_shutdown_timeout: Duration,
    ) -> Self {
        Self {
            dlq_max_size,
            retry_policy,
            default_shutdown_timeout,
        }
    }

    /// Set the DLQ maximum size
    #[must_use]
    pub const fn with_dlq_max_size(mut self, max_size: usize) -> Self {
        self.dlq_max_size = max_size;
        self
    }

    /// Set the retry policy
    #[must_use]
    pub const fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Set the default shutdown timeout
    #[must_use]
    pub const fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.default_shutdown_timeout = timeout;
        self
    }
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            dlq_max_size: 1000,
            retry_policy: RetryPolicy::default(),
            default_shutdown_timeout: Duration::from_secs(30),
        }
    }
}

/// Effect tracking mode - controls how effects are tracked for completion
///
/// # Modes
///
/// - **Direct**: Tracks only immediate effects (default)
/// - **Cascading**: Tracks effects transitively, following the entire effect tree
#[derive(Debug, Clone)]
pub enum TrackingMode {
    /// Track only immediate effects spawned by this action
    Direct,

    /// Track effects transitively - any effects produced by feedback actions
    /// are also tracked as children
    Cascading {
        /// Child effect handles that need to complete before this handle is done
        children: Arc<Mutex<Vec<EffectHandle>>>,
    },
}

/// Handle for tracking effect completion
///
/// Returned by [`Store::send()`] to allow waiting for effects to complete.
/// Inspired by JavaScript promises - each action gets a handle that can be
/// awaited to know when its effects (and optionally cascading effects) are done.
///
/// # Example
///
/// ```ignore
/// let handle = store.send(Action::Start).await;
/// handle.wait_with_timeout(Duration::from_secs(5)).await?;
/// // All effects from Action::Start are now complete
/// ```
#[derive(Clone)]
pub struct EffectHandle {
    mode: TrackingMode,
    effects: Arc<AtomicUsize>,
    completion: watch::Receiver<()>,
}

impl EffectHandle {
    /// Create a new effect handle with the given tracking mode
    ///
    /// # Arguments
    ///
    /// - `mode`: Whether to track effects directly or cascading
    ///
    /// # Returns
    ///
    /// A tuple of `(EffectHandle, EffectTracking)` where:
    /// - `EffectHandle` is returned to the caller for waiting
    /// - `EffectTracking` is used internally for effect execution
    fn new<A>(mode: TrackingMode) -> (Self, EffectTracking<A>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = watch::channel(());

        let handle = Self {
            mode: mode.clone(),
            effects: Arc::clone(&counter),
            completion: rx,
        };

        let tracking = EffectTracking {
            mode,
            counter,
            notifier: tx,
            feedback_dest: FeedbackDestination::Auto(Weak::new()),
        };

        (handle, tracking)
    }

    /// Create a handle that's already complete
    ///
    /// Useful for initialization in loops where you need a `last_handle`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut last_handle = EffectHandle::completed();
    /// for action in actions {
    ///     last_handle = store.send(action).await;
    /// }
    /// last_handle.wait().await?;
    /// ```
    #[must_use]
    pub fn completed() -> Self {
        let (tx, rx) = watch::channel(());
        let _ = tx.send(());

        Self {
            mode: TrackingMode::Direct,
            effects: Arc::new(AtomicUsize::new(0)),
            completion: rx,
        }
    }

    /// Wait for all effects to complete
    ///
    /// Blocks until the effect counter reaches zero.
    ///
    /// # Panics
    ///
    /// Panics if the mutex protecting cascading children is poisoned.
    /// This should only happen if a thread panicked while holding the lock.
    ///
    /// # Returns
    ///
    /// Returns when all effects are complete
    #[allow(clippy::unwrap_used)] // Mutex poison is unrecoverable
    pub async fn wait(&mut self) {
        // Wait for counter to reach zero
        while self.effects.load(Ordering::SeqCst) > 0 {
            let _ = self.completion.changed().await;
        }

        // If cascading, recursively wait for all children
        if let TrackingMode::Cascading { children } = &self.mode {
            loop {
                let handles = {
                    // Panic on mutex poison is acceptable - it's unrecoverable
                    #[allow(clippy::unwrap_used)]
                    let mut guard = children.lock().unwrap();
                    if guard.is_empty() {
                        break;
                    }
                    guard.drain(..).collect::<Vec<_>>()
                };

                for mut handle in handles {
                    Box::pin(handle.wait()).await;
                }
            }
        }
    }

    /// Wait for all effects to complete with a timeout
    ///
    /// # Arguments
    ///
    /// - `timeout`: Maximum duration to wait
    ///
    /// # Returns
    ///
    /// - `Ok(())` if all effects completed in time
    /// - `Err(())` if timeout was reached
    ///
    /// # Errors
    ///
    /// Returns `Err(())` if the timeout expires before all effects complete.
    ///
    /// # Panics
    ///
    /// Panics if the mutex protecting cascading children is poisoned (via `wait()`).
    ///
    /// # Example
    ///
    /// ```ignore
    /// handle.wait_with_timeout(Duration::from_secs(5)).await?;
    /// ```
    pub async fn wait_with_timeout(&mut self, timeout: Duration) -> Result<(), ()> {
        tokio::time::timeout(timeout, self.wait())
            .await
            .map_err(|_| ())
    }
}

impl std::fmt::Debug for EffectHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectHandle")
            .field("mode", &self.mode)
            .field("pending_effects", &self.effects.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

/// Internal: Effect tracking context passed through effect execution
///
/// This type is internal to the runtime and not exposed to users.
/// It carries the tracking state through effect execution.
struct EffectTracking<A> {
    mode: TrackingMode,
    counter: Arc<AtomicUsize>,
    notifier: watch::Sender<()>,
    feedback_dest: FeedbackDestination<A>,
}

impl<A> EffectTracking<A> {
    /// Increment the effect counter (effect started)
    fn increment(&self) {
        self.counter.fetch_add(1, Ordering::SeqCst);
    }

    /// Decrement the effect counter (effect completed)
    fn decrement(&self) {
        if self.counter.fetch_sub(1, Ordering::SeqCst) == 1 {
            // Counter reached zero, notify waiters
            let _ = self.notifier.send(());
        }
    }
}

impl<A> Clone for EffectTracking<A> {
    fn clone(&self) -> Self {
        Self {
            mode: self.mode.clone(),
            counter: Arc::clone(&self.counter),
            notifier: self.notifier.clone(),
            feedback_dest: self.feedback_dest.clone(),
        }
    }
}

/// Internal: Destination for actions produced by effects
///
/// Controls where actions go when effects produce them:
/// - Auto: Send back to the Store automatically (production)
/// - Queued: Push to a queue for manual processing (testing)
enum FeedbackDestination<A> {
    /// Auto-feedback to store (production mode)
    Auto(Weak<()>), // Will hold Weak<Store> in full implementation

    /// Queue for manual processing (test mode)
    Queued(Arc<Mutex<VecDeque<A>>>),
}

impl<A> Clone for FeedbackDestination<A> {
    fn clone(&self) -> Self {
        match self {
            Self::Auto(weak) => Self::Auto(Weak::clone(weak)),
            Self::Queued(queue) => Self::Queued(Arc::clone(queue)),
        }
    }
}

/// Internal: RAII guard that decrements effect counter on drop
///
/// Ensures the effect counter is always decremented, even if the effect panics.
struct DecrementGuard<A>(EffectTracking<A>);

impl<A> Drop for DecrementGuard<A> {
    fn drop(&mut self) {
        self.0.decrement();
    }
}

/// Guard that decrements an atomic counter on drop (for shutdown tracking)
struct AtomicCounterGuard(Arc<AtomicUsize>);

impl Drop for AtomicCounterGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Store module - The runtime for reducers
///
/// # Phase 1 Implementation
///
/// This module will contain:
/// - Store struct with full implementation
/// - Effect execution logic
/// - Action feedback loop
/// - Concurrency management
///
/// Store runtime for coordinating reducer execution and effect handling.
pub mod store {
    use super::{
        Arc, AtomicBool, AtomicCounterGuard, AtomicUsize, DeadLetterQueue, DecrementGuard,
        Duration, Effect, EffectHandle, EffectTracking, HealthCheck, Ordering, Reducer,
        RetryPolicy, RwLock, StoreConfig, StoreError, TrackingMode,
    };
    use tokio::sync::{broadcast, watch};

    /// The Store - runtime coordinator for a reducer
    ///
    /// The Store manages:
    /// 1. State (behind `RwLock` for concurrent access)
    /// 2. Reducer (business logic)
    /// 3. Environment (injected dependencies)
    /// 4. Effect execution (with feedback loop)
    ///
    /// # Type Parameters
    ///
    /// - `S`: State type
    /// - `A`: Action type
    /// - `E`: Environment type
    /// - `R`: Reducer implementation
    ///
    /// # Example
    ///
    /// ```ignore
    /// let store = Store::new(
    ///     OrderState::default(),
    ///     OrderReducer,
    ///     production_environment(),
    /// );
    ///
    /// store.send(OrderAction::PlaceOrder {
    ///     customer_id: CustomerId::new(1),
    ///     items: vec![],
    /// }).await;
    /// ```
    pub struct Store<S, A, E, R>
    where
        R: Reducer<State = S, Action = A, Environment = E>,
    {
        state: Arc<RwLock<S>>,
        reducer: R,
        environment: E,
        retry_policy: RetryPolicy,
        dlq: DeadLetterQueue<String>,
        shutdown: Arc<AtomicBool>,
        pending_effects: Arc<AtomicUsize>,
        /// Action broadcast channel for observing actions produced by effects.
        ///
        /// All actions produced by effects (e.g., from `Effect::Future`) are
        /// broadcast to observers. This enables HTTP request-response patterns
        /// and real-time event streaming via `WebSockets`.
        action_broadcast: broadcast::Sender<A>,
    }

    impl<S, A, E, R> Store<S, A, E, R>
    where
        R: Reducer<State = S, Action = A, Environment = E> + Send + Sync + 'static,
        A: Send + Clone + 'static,
        S: Send + Sync + 'static,
        E: Send + Sync + 'static,
    {
        /// Create a new store with initial state, reducer, and environment
        ///
        /// Creates a Store with default configuration:
        /// - Action broadcast capacity: 16 (increase with `with_broadcast_capacity`)
        /// - Retry policy: Default (exponential backoff)
        /// - DLQ max size: 100
        ///
        /// # Arguments
        ///
        /// - `initial_state`: The starting state for the store
        /// - `reducer`: The reducer implementation (business logic)
        /// - `environment`: Injected dependencies
        ///
        /// # Returns
        ///
        /// A new Store instance ready to process actions
        #[must_use]
        pub fn new(initial_state: S, reducer: R, environment: E) -> Self {
            let (action_broadcast, _) = broadcast::channel(16);

            Self {
                state: Arc::new(RwLock::new(initial_state)),
                reducer,
                environment,
                retry_policy: RetryPolicy::default(),
                dlq: DeadLetterQueue::default(),
                shutdown: Arc::new(AtomicBool::new(false)),
                pending_effects: Arc::new(AtomicUsize::new(0)),
                action_broadcast,
            }
        }

        /// Create a new Store with a custom retry policy
        ///
        /// # Arguments
        ///
        /// - `initial_state`: Initial state value
        /// - `reducer`: The reducer function
        /// - `environment`: Dependencies injected into the reducer
        /// - `retry_policy`: Policy for retrying failed effects
        pub fn with_retry_policy(
            initial_state: S,
            reducer: R,
            environment: E,
            retry_policy: RetryPolicy,
        ) -> Self {
            let (action_broadcast, _) = broadcast::channel(16);

            Self {
                state: Arc::new(RwLock::new(initial_state)),
                reducer,
                environment,
                retry_policy,
                dlq: DeadLetterQueue::default(),
                shutdown: Arc::new(AtomicBool::new(false)),
                pending_effects: Arc::new(AtomicUsize::new(0)),
                action_broadcast,
            }
        }

        /// Create a new Store with custom configuration
        ///
        /// # Arguments
        ///
        /// - `initial_state`: Initial state value
        /// - `reducer`: The reducer function
        /// - `environment`: Dependencies injected into the reducer
        /// - `config`: Configuration for DLQ, retry policy, and shutdown behavior
        ///
        /// # Example
        ///
        /// ```ignore
        /// let config = StoreConfig::default()
        ///     .with_dlq_max_size(500)
        ///     .with_shutdown_timeout(Duration::from_secs(60));
        ///
        /// let store = Store::with_config(
        ///     MyState::default(),
        ///     MyReducer,
        ///     my_environment,
        ///     config,
        /// );
        /// ```
        #[must_use]
        pub fn with_config(
            initial_state: S,
            reducer: R,
            environment: E,
            config: StoreConfig,
        ) -> Self {
            let (action_broadcast, _) = broadcast::channel(16);

            Self {
                state: Arc::new(RwLock::new(initial_state)),
                reducer,
                environment,
                retry_policy: config.retry_policy,
                dlq: DeadLetterQueue::new(config.dlq_max_size),
                shutdown: Arc::new(AtomicBool::new(false)),
                pending_effects: Arc::new(AtomicUsize::new(0)),
                action_broadcast,
            }
        }

        /// Create a new Store with custom action broadcast capacity
        ///
        /// Use this constructor when you need to handle high-throughput
        /// scenarios with many slow observers (e.g., multiple WebSocket clients).
        ///
        /// Default capacity is 16. Increase if observers frequently lag.
        ///
        /// # Arguments
        ///
        /// - `initial_state`: The starting state for the store
        /// - `reducer`: The reducer implementation (business logic)
        /// - `environment`: Injected dependencies
        /// - `capacity`: Action broadcast channel capacity (number of actions buffered)
        ///
        /// # Example
        ///
        /// ```ignore
        /// // High throughput: 256 actions buffered
        /// let store = Store::with_broadcast_capacity(
        ///     MyState::default(),
        ///     MyReducer,
        ///     my_environment,
        ///     256,
        /// );
        /// ```
        #[must_use]
        pub fn with_broadcast_capacity(
            initial_state: S,
            reducer: R,
            environment: E,
            capacity: usize,
        ) -> Self {
            let (action_broadcast, _) = broadcast::channel(capacity);

            Self {
                state: Arc::new(RwLock::new(initial_state)),
                reducer,
                environment,
                retry_policy: RetryPolicy::default(),
                dlq: DeadLetterQueue::default(),
                shutdown: Arc::new(AtomicBool::new(false)),
                pending_effects: Arc::new(AtomicUsize::new(0)),
                action_broadcast,
            }
        }

        /// Get access to the dead letter queue
        ///
        /// Returns a clone of the DLQ for inspecting failed operations.
        #[must_use]
        pub fn dlq(&self) -> DeadLetterQueue<String> {
            self.dlq.clone()
        }

        /// Perform a health check on the Store
        ///
        /// Checks:
        /// - Dead letter queue size (degraded if > 50% capacity, unhealthy if full)
        /// - Store is operational
        ///
        /// Returns a `HealthCheck` with current status and metadata.
        #[must_use]
        pub fn health(&self) -> HealthCheck {
            let dlq_size = self.dlq.len();
            let dlq_capacity = self.dlq.max_size();
            // Note: Precision loss acceptable for health check percentage (queue sizes < 2^52)
            #[allow(clippy::cast_precision_loss)]
            let dlq_usage = (dlq_size as f64 / dlq_capacity as f64) * 100.0;

            let mut check = if dlq_size >= dlq_capacity {
                HealthCheck::unhealthy("store", "Dead letter queue is full")
            } else if dlq_usage > 50.0 {
                // Note: Truncation intentional for display percentage
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let usage_pct = dlq_usage as u32;
                HealthCheck::degraded(
                    "store",
                    format!("Dead letter queue is {usage_pct}% full"),
                )
            } else {
                HealthCheck::healthy("store")
            };

            check = check
                .with_metadata("dlq_size", dlq_size.to_string())
                .with_metadata("dlq_capacity", dlq_capacity.to_string())
                .with_metadata("dlq_usage_pct", format!("{dlq_usage:.1}"));

            check
        }

        /// Initiate graceful shutdown of the store
        ///
        /// This method:
        /// 1. Sets the shutdown flag (rejecting new actions)
        /// 2. Waits for pending effects to complete (with timeout)
        /// 3. Returns when all effects finish or timeout expires
        ///
        /// # Arguments
        ///
        /// - `timeout`: Maximum time to wait for effects to complete
        ///
        /// # Returns
        ///
        /// - `Ok(())` if all effects completed within timeout
        /// - `Err(StoreError::ShutdownTimeout)` if timeout expired with effects still running
        ///
        /// # Errors
        ///
        /// Returns [`StoreError::ShutdownTimeout`] if the timeout expires before all
        /// pending effects complete.
        ///
        /// # Example
        ///
        /// ```ignore
        /// // Graceful shutdown with 30 second timeout
        /// store.shutdown(Duration::from_secs(30)).await?;
        /// ```
        #[allow(clippy::cognitive_complexity)] // TODO: Refactor in Phase 5
        pub async fn shutdown(&self, timeout: Duration) -> Result<(), StoreError> {
            tracing::info!("Initiating graceful shutdown");
            metrics::counter!("store.shutdown.initiated").increment(1);

            // Set shutdown flag to reject new actions
            self.shutdown.store(true, Ordering::Release);

            // Wait for pending effects with timeout
            let start = std::time::Instant::now();
            let poll_interval = Duration::from_millis(100);

            loop {
                let pending = self.pending_effects.load(Ordering::Acquire);

                if pending == 0 {
                    tracing::info!("All effects completed, shutdown successful");
                    metrics::counter!("store.shutdown.completed").increment(1);
                    return Ok(());
                }

                if start.elapsed() >= timeout {
                    tracing::error!(
                        pending_effects = pending,
                        "Shutdown timeout: {} effects still running", pending
                    );
                    metrics::counter!("store.shutdown.timeout").increment(1);
                    return Err(StoreError::ShutdownTimeout(pending));
                }

                tracing::debug!(
                    pending_effects = pending,
                    elapsed_ms = start.elapsed().as_millis(),
                    "Waiting for effects to complete"
                );

                tokio::time::sleep(poll_interval).await;
            }
        }

        /// Send an action to the store
        ///
        /// This is the primary way to interact with the store:
        /// 1. Acquires write lock on state
        /// 2. Calls reducer with (state, action, environment)
        /// 3. Executes returned effects asynchronously
        /// 4. Effects may produce more actions (feedback loop)
        ///
        /// # Arguments
        ///
        /// - `action`: The action to process
        ///
        /// # Returns
        ///
        /// An [`EffectHandle`] that can be used to wait for effect completion.
        ///
        /// # Concurrency and Effect Execution
        ///
        /// - The reducer executes synchronously while holding a write lock
        /// - Effects execute asynchronously in spawned tasks
        /// - `send()` returns after starting effect execution, not completion
        /// - Multiple concurrent `send()` calls serialize at the reducer level
        /// - Effects may complete in non-deterministic order
        ///
        /// # Effect Timing
        ///
        /// ```ignore
        /// let handle = store.send(Action::TriggerEffect).await;
        /// // send() returned, but effect may still be running!
        ///
        /// // To wait for effects:
        /// handle.wait_with_timeout(Duration::from_secs(5)).await?;
        /// ```
        ///
        /// # Errors
        ///
        /// Returns [`StoreError::ShutdownInProgress`] if the store is shutting down.
        ///
        /// # Panics
        ///
        /// If the reducer panics, the panic will propagate and halt the store.
        /// Reducers should be pure functions that do not panic.
        ///
        /// # Example
        ///
        /// ```ignore
        /// let handle = store.send(CounterAction::Increment).await?;
        /// handle.wait().await;
        /// ```
        #[tracing::instrument(skip(self, action), name = "store_send")]
        pub async fn send(&self, action: A) -> Result<EffectHandle, StoreError>
        where
            R: Clone,
            E: Clone,
            A: Clone,
        {
            self.send_internal(action, TrackingMode::Direct).await
        }

        /// Send an action and wait for a matching result action
        ///
        /// This method is designed for request-response patterns (HTTP, RPC).
        /// It subscribes to the action broadcast, sends the initial action,
        /// then waits for an action matching the predicate.
        ///
        /// # How It Works
        ///
        /// 1. Subscribe to action broadcast BEFORE sending (avoids race conditions)
        /// 2. Send the initial action through the store
        /// 3. Wait for actions produced by effects
        /// 4. Return the first action matching the predicate
        ///
        /// # Arguments
        ///
        /// - `action`: The initial action to send
        /// - `predicate`: Function to test if an action is the terminal result
        /// - `timeout`: Maximum time to wait for matching action
        ///
        /// # Returns
        ///
        /// The first action matching the predicate, or timeout error.
        ///
        /// # Errors
        ///
        /// - [`StoreError::Timeout`]: Timeout expired before matching action received
        /// - [`StoreError::ChannelClosed`]: Action broadcast channel closed (store shutting down)
        /// - [`StoreError::ShutdownInProgress`]: Store is shutting down
        ///
        /// # Example
        ///
        /// ```ignore
        /// use std::time::Duration;
        ///
        /// let result = store.send_and_wait_for(
        ///     OrderAction::PlaceOrder {
        ///         correlation_id: Uuid::new_v4(),
        ///         customer_id,
        ///         items,
        ///     },
        ///     |a| matches!(a,
        ///         OrderAction::OrderPlaced { .. } |
        ///         OrderAction::OrderFailed { .. }
        ///     ),
        ///     Duration::from_secs(10),
        /// ).await?;
        ///
        /// match result {
        ///     OrderAction::OrderPlaced { order_id, .. } => {
        ///         println!("Order placed: {}", order_id);
        ///     }
        ///     OrderAction::OrderFailed { reason, .. } => {
        ///         eprintln!("Order failed: {}", reason);
        ///     }
        ///     _ => unreachable!("Predicate ensures only terminal actions"),
        /// }
        /// ```
        ///
        /// # Notes
        ///
        /// - Only actions produced by effects are broadcast (not the initial action)
        /// - If the channel lags and drops actions, continues waiting (timeout catches it)
        /// - Use correlation IDs to distinguish concurrent requests
        pub async fn send_and_wait_for<F>(
            &self,
            action: A,
            predicate: F,
            timeout: Duration,
        ) -> Result<A, StoreError>
        where
            R: Clone,
            E: Clone,
            A: Clone,
            F: Fn(&A) -> bool,
        {
            // Subscribe BEFORE sending to avoid race condition
            let mut rx = self.action_broadcast.subscribe();

            // Send the initial action
            self.send(action).await?;

            // Wait for matching action with timeout
            tokio::time::timeout(timeout, async {
                loop {
                    match rx.recv().await {
                        Ok(action) if predicate(&action) => return Ok(action),
                        Ok(_) => {} // Not the action we want, keep waiting
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            // Slow consumer, some actions were dropped
                            // Continue waiting - if terminal action was dropped, timeout will catch it
                            tracing::warn!(
                                skipped,
                                "Action observer lagged, {} actions skipped",
                                skipped
                            );
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            return Err(StoreError::ChannelClosed);
                        }
                    }
                }
            })
            .await
            .map_err(|_| StoreError::Timeout)?
        }

        /// Subscribe to all actions from this store
        ///
        /// This method is designed for event streaming (`WebSockets`, SSE).
        /// Returns a receiver that gets a clone of every action produced by effects.
        ///
        /// # Returns
        ///
        /// A broadcast receiver that receives all actions produced by effects.
        ///
        /// # Notes
        ///
        /// - Only actions produced by effects are broadcast (not initial actions sent via `send`)
        /// - If the receiver lags, it will skip old actions and receive [`RecvError::Lagged`]
        /// - The receiver must be consumed in a loop or it will block the channel
        ///
        /// # Example
        ///
        /// ```ignore
        /// let mut rx = store.subscribe_actions();
        ///
        /// // Stream to WebSocket client
        /// while let Ok(action) = rx.recv().await {
        ///     let json = serde_json::to_string(&action)?;
        ///     ws.send(json).await?;
        /// }
        /// ```
        ///
        /// # Filtered Streaming
        ///
        /// You can filter actions for specific users or criteria:
        ///
        /// ```ignore
        /// let mut rx = store.subscribe_actions();
        /// let user_id = session.user_id;
        ///
        /// while let Ok(action) = rx.recv().await {
        ///     // Only stream actions for this user
        ///     if action.user_id() == Some(&user_id) {
        ///         ws.send(serde_json::to_string(&action)?).await?;
        ///     }
        /// }
        /// ```
        #[must_use]
        pub fn subscribe_actions(&self) -> broadcast::Receiver<A> {
            self.action_broadcast.subscribe()
        }

        /// Internal send implementation with tracking control
        ///
        /// This method is used by both production `send()` and test `TestStore::send()`.
        ///
        /// # Arguments
        ///
        /// - `action`: The action to process
        /// - `tracking_mode`: Whether to track effects directly or cascading
        ///
        /// # Returns
        ///
        /// An [`EffectHandle`] for waiting on effect completion
        ///
        /// # Errors
        ///
        /// Returns [`StoreError::ShutdownInProgress`] if the store is shutting down.
        #[allow(clippy::cognitive_complexity)] // TODO: Refactor in Phase 4
        #[tracing::instrument(skip(self, action, tracking_mode), name = "store_send_internal")]
        async fn send_internal(&self, action: A, tracking_mode: TrackingMode) -> Result<EffectHandle, StoreError>
        where
            R: Clone,
            E: Clone,
            A: Clone,
        {
            // Check if store is shutting down
            if self.shutdown.load(Ordering::Acquire) {
                tracing::warn!("Rejected action: store is shutting down");
                metrics::counter!("store.shutdown.rejected_actions").increment(1);
                return Err(StoreError::ShutdownInProgress);
            }

            tracing::debug!("Processing action");

            // Metrics: Increment command counter
            metrics::counter!("store.commands.total").increment(1);

            // Create tracking for this action
            let (handle, tracking) = EffectHandle::new::<A>(tracking_mode);

            let effects = {
                let mut state = self.state.write().await;
                tracing::trace!("Acquired write lock on state");

                // Create span for reducer execution
                let span = tracing::debug_span!("reducer_execution");
                let _enter = span.enter();

                // Metrics: Time reducer execution
                let start = std::time::Instant::now();
                let effects = self.reducer.reduce(&mut *state, action, &self.environment);
                let duration = start.elapsed();
                metrics::histogram!("store.reducer.duration_seconds")
                    .record(duration.as_secs_f64());

                tracing::trace!("Reducer completed, returned {} effects", effects.len());

                // Metrics: Record number of effects produced
                // Note: Precision loss acceptable for metrics (effect counts < 2^52)
                #[allow(clippy::cast_precision_loss)]
                metrics::histogram!("store.effects.count").record(effects.len() as f64);

                effects
            };

            // Execute effects with tracking
            tracing::trace!("Executing {} effects", effects.len());
            for effect in effects {
                self.execute_effect_internal(effect, tracking.clone());
            }
            tracing::debug!("Action processing completed, returning handle");

            Ok(handle)
        }

        /// Read current state via a closure
        ///
        /// Access state through a closure to ensure the lock is released promptly:
        ///
        /// ```ignore
        /// let order_count = store.state(|s| s.orders.len()).await;
        /// ```
        ///
        /// # Arguments
        ///
        /// - `f`: Closure that receives a reference to state and returns a value
        ///
        /// # Returns
        ///
        /// The value returned by the closure
        pub async fn state<F, T>(&self, f: F) -> T
        where
            F: FnOnce(&S) -> T,
        {
            let state = self.state.read().await;
            f(&*state)
        }

        /// Retry an async operation according to the retry policy
        ///
        /// This wraps an async operation with exponential backoff retry logic.
        /// Metrics are recorded for retry attempts.
        ///
        /// # Arguments
        ///
        /// - `operation_name`: Name for logging/metrics (e.g., "`append_events`")
        /// - `f`: Async function to execute (will be called multiple times on failure)
        ///
        /// # Returns
        ///
        /// Result from the operation, or the last error if all retries exhausted
        async fn retry_operation<F, Fut, T, Err>(&self, operation_name: &str, mut f: F) -> Result<T, Err>
        where
            F: FnMut() -> Fut,
            Fut: std::future::Future<Output = Result<T, Err>>,
            Err: std::fmt::Display,
        {
            let mut attempt = 0;

            loop {
                match f().await {
                    Ok(result) => {
                        // Success! Record metrics if this was a retry
                        if attempt > 0 {
                            metrics::counter!(
                                "store.retry.success",
                                "operation" => operation_name.to_string(),
                                "attempts" => attempt.to_string()
                            )
                            .increment(1);
                            tracing::info!(
                                operation = operation_name,
                                attempt = attempt,
                                "Operation succeeded after retry"
                            );
                        }
                        return Ok(result);
                    }
                    Err(error) => {
                        // Check if we should retry
                        if !self.retry_policy.should_retry(attempt + 1) {
                            // Exhausted retries - push to DLQ
                            let error_msg = format!("{error}");
                            self.dlq.push(
                                operation_name.to_string(),
                                error_msg.clone(),
                                (attempt + 1) as usize,
                            );

                            metrics::counter!(
                                "store.retry.exhausted",
                                "operation" => operation_name.to_string(),
                                "attempts" => attempt.to_string()
                            )
                            .increment(1);
                            tracing::error!(
                                operation = operation_name,
                                attempt = attempt,
                                error = %error,
                                "Operation failed after exhausting retries, added to DLQ"
                            );
                            return Err(error);
                        }

                        // Calculate delay and retry
                        let delay = self.retry_policy.delay_for_attempt(attempt);
                        metrics::counter!(
                            "store.retry.attempt",
                            "operation" => operation_name.to_string(),
                            "attempt" => attempt.to_string()
                        )
                        .increment(1);
                        tracing::warn!(
                            operation = operation_name,
                            attempt = attempt,
                            delay_ms = delay.as_millis(),
                            error = %error,
                            "Operation failed, retrying after delay"
                        );

                        tokio::time::sleep(delay).await;
                        attempt += 1;
                    }
                }
            }
        }

        /// Execute an effect with tracking
        ///
        /// Internal method that executes effects with completion tracking.
        /// Uses [`DecrementGuard`] to ensure the effect counter is always
        /// decremented, even if the effect panics.
        ///
        /// # Effect Types
        ///
        /// - `None`: No-op
        /// - `Future`: Executes async computation, sends resulting action if `Some`
        /// - `Delay`: Waits for duration, then sends action
        /// - `Parallel`: Executes effects concurrently
        /// - `Sequential`: Executes effects in order, waiting for each to complete
        ///
        /// # Error Handling Strategy
        ///
        /// **Reducer panics**: Propagate (fail fast). Reducers should be pure functions
        /// that do not panic. If a reducer panics, the store will halt.
        ///
        /// **Effect execution failures**: Log and continue. Effects are fire-and-forget
        /// operations. If an effect task panics during parallel execution, it's logged
        /// but other effects continue. The [`DecrementGuard`] ensures the counter is
        /// always updated even on panic.
        ///
        /// # Arguments
        ///
        /// - `effect`: The effect to execute
        /// - `tracking`: The tracking context for this effect (passed by value to enable cloning)
        #[allow(clippy::needless_pass_by_value)] // tracking is cloned, so pass by value is intentional
        #[allow(clippy::cognitive_complexity)] // TODO: Refactor in Phase 4
        #[allow(clippy::too_many_lines)] // TODO: Refactor in Phase 4
        #[tracing::instrument(skip(self, effect, tracking), name = "execute_effect")]
        fn execute_effect_internal(&self, effect: Effect<A>, tracking: EffectTracking<A>)
        where
            R: Clone,
            E: Clone,
            A: Clone + Send + 'static,
        {
            match effect {
                Effect::None => {
                    tracing::trace!("Executing Effect::None (no-op)");
                    metrics::counter!("store.effects.executed", "type" => "none").increment(1);
                },
                Effect::Future(fut) => {
                    tracing::trace!("Executing Effect::Future");
                    metrics::counter!("store.effects.executed", "type" => "future").increment(1);
                    tracking.increment();

                    // Track global pending effects for shutdown
                    self.pending_effects.fetch_add(1, Ordering::SeqCst);
                    let pending_guard = AtomicCounterGuard(Arc::clone(&self.pending_effects));

                    let tracking_clone = tracking.clone();
                    let store = self.clone();

                    tokio::spawn(async move {
                        let _guard = DecrementGuard(tracking_clone.clone());
                        let _pending_guard = pending_guard; // Decrement on drop

                        if let Some(action) = fut.await {
                            tracing::trace!("Effect::Future produced an action, sending to store");

                            // Broadcast to observers (HTTP handlers, WebSockets, metrics)
                            let _ = store.action_broadcast.send(action.clone());

                            // Send action back to store (auto-feedback)
                            let _ = store.send(action).await;
                        } else {
                            tracing::trace!("Effect::Future completed with no action");
                        }
                    });
                },
                Effect::Delay { duration, action } => {
                    tracing::trace!("Executing Effect::Delay (duration: {:?})", duration);
                    metrics::counter!("store.effects.executed", "type" => "delay").increment(1);
                    tracking.increment();

                    // Track global pending effects for shutdown
                    self.pending_effects.fetch_add(1, Ordering::SeqCst);
                    let pending_guard = AtomicCounterGuard(Arc::clone(&self.pending_effects));

                    let tracking_clone = tracking.clone();
                    let store = self.clone();

                    tokio::spawn(async move {
                        let _guard = DecrementGuard(tracking_clone.clone());
                        let _pending_guard = pending_guard; // Decrement on drop

                        tokio::time::sleep(duration).await;
                        tracing::trace!("Effect::Delay completed, sending action");

                        // Broadcast to observers
                        let _ = store.action_broadcast.send((*action).clone());

                        let _ = store.send(*action).await;
                    });
                },
                Effect::Parallel(effects) => {
                    let effect_count = effects.len();
                    tracing::trace!("Executing Effect::Parallel with {} effects", effect_count);
                    metrics::counter!("store.effects.executed", "type" => "parallel").increment(1);

                    // Execute all effects concurrently, each with the same tracking
                    let store = self.clone();
                    for effect in effects {
                        store.execute_effect_internal(effect, tracking.clone());
                    }
                },
                Effect::Sequential(effects) => {
                    let effect_count = effects.len();
                    tracing::trace!("Executing Effect::Sequential with {} effects", effect_count);
                    metrics::counter!("store.effects.executed", "type" => "sequential").increment(1);

                    tracking.increment();

                    // Track global pending effects for shutdown
                    self.pending_effects.fetch_add(1, Ordering::SeqCst);
                    let pending_guard = AtomicCounterGuard(Arc::clone(&self.pending_effects));

                    let tracking_clone = tracking.clone();
                    let store = self.clone();

                    tokio::spawn(async move {
                        let _guard = DecrementGuard(tracking_clone.clone());
                        let _pending_guard = pending_guard; // Decrement on drop

                        // Execute effects one by one, waiting for each to complete
                        for (idx, effect) in effects.into_iter().enumerate() {
                            tracing::trace!(
                                "Executing sequential effect {} of {}",
                                idx + 1,
                                effect_count
                            );

                            // Create sub-tracking for this effect
                            let (sub_tx, mut sub_rx) = watch::channel(());
                            let sub_tracking = EffectTracking {
                                mode: TrackingMode::Direct,
                                counter: Arc::new(AtomicUsize::new(0)),
                                notifier: sub_tx,
                                feedback_dest: tracking_clone.feedback_dest.clone(),
                            };

                            // Execute the effect
                            store.execute_effect_internal(effect, sub_tracking.clone());

                            // Wait for this effect to complete before continuing
                            if sub_tracking.counter.load(Ordering::SeqCst) > 0 {
                                let _ = sub_rx.changed().await;
                            }
                        }
                        tracing::trace!("Effect::Sequential completed");
                    });
                },
                Effect::EventStore(op) => {
                    use composable_rust_core::effect::EventStoreOperation;

                    tracing::trace!("Executing Effect::EventStore");
                    metrics::counter!("store.effects.executed", "type" => "event_store").increment(1);
                    tracking.increment();

                    // Track global pending effects for shutdown
                    self.pending_effects.fetch_add(1, Ordering::SeqCst);
                    let pending_guard = AtomicCounterGuard(Arc::clone(&self.pending_effects));

                    let tracking_clone = tracking.clone();
                    let store = self.clone();

                    tokio::spawn(async move {
                        let _guard = DecrementGuard(tracking_clone.clone());
                        let _pending_guard = pending_guard; // Decrement on drop

                        let action = match op {
                            EventStoreOperation::AppendEvents {
                                event_store,
                                stream_id,
                                expected_version,
                                events,
                                on_success,
                                on_error,
                            } => {
                                tracing::debug!(
                                    stream_id = %stream_id,
                                    expected_version = ?expected_version,
                                    event_count = events.len(),
                                    "Executing append_events"
                                );

                                // Wrap with retry logic
                                let stream_id_clone = stream_id.clone();
                                let result = store.retry_operation("append_events", || {
                                    let event_store_clone = event_store.clone();
                                    let stream_id_clone = stream_id_clone.clone();
                                    let events_clone = events.clone();
                                    async move {
                                        event_store_clone
                                            .append_events(stream_id_clone, expected_version, events_clone)
                                            .await
                                    }
                                }).await;

                                match result {
                                    Ok(version) => {
                                        tracing::debug!(new_version = ?version, "append_events succeeded");
                                        on_success(version)
                                    },
                                    Err(error) => {
                                        tracing::warn!(error = %error, "append_events failed");
                                        on_error(error)
                                    },
                                }
                            },
                            EventStoreOperation::LoadEvents {
                                event_store,
                                stream_id,
                                from_version,
                                on_success,
                                on_error,
                            } => {
                                tracing::debug!(
                                    stream_id = %stream_id,
                                    from_version = ?from_version,
                                    "Executing load_events"
                                );

                                // Wrap with retry logic
                                let stream_id_clone = stream_id.clone();
                                let result = store.retry_operation("load_events", || {
                                    let event_store_clone = event_store.clone();
                                    let stream_id_clone = stream_id_clone.clone();
                                    async move {
                                        event_store_clone.load_events(stream_id_clone, from_version).await
                                    }
                                }).await;

                                match result {
                                    Ok(events) => {
                                        tracing::debug!(
                                            event_count = events.len(),
                                            "load_events succeeded"
                                        );
                                        on_success(events)
                                    },
                                    Err(error) => {
                                        tracing::warn!(error = %error, "load_events failed");
                                        on_error(error)
                                    },
                                }
                            },
                            EventStoreOperation::SaveSnapshot {
                                event_store,
                                stream_id,
                                version,
                                state,
                                on_success,
                                on_error,
                            } => {
                                tracing::debug!(
                                    stream_id = %stream_id,
                                    version = ?version,
                                    state_size = state.len(),
                                    "Executing save_snapshot"
                                );

                                // Wrap with retry logic
                                let stream_id_clone = stream_id.clone();
                                let state_clone = state.clone();
                                let result = store.retry_operation("save_snapshot", || {
                                    let event_store_clone = event_store.clone();
                                    let stream_id_clone = stream_id_clone.clone();
                                    let state_clone = state_clone.clone();
                                    async move {
                                        event_store_clone
                                            .save_snapshot(stream_id_clone, version, state_clone)
                                            .await
                                    }
                                }).await;

                                match result {
                                    Ok(()) => {
                                        tracing::debug!("save_snapshot succeeded");
                                        on_success(())
                                    },
                                    Err(error) => {
                                        tracing::warn!(error = %error, "save_snapshot failed");
                                        on_error(error)
                                    },
                                }
                            },
                            EventStoreOperation::LoadSnapshot {
                                event_store,
                                stream_id,
                                on_success,
                                on_error,
                            } => {
                                tracing::debug!(stream_id = %stream_id, "Executing load_snapshot");

                                // Wrap with retry logic
                                let stream_id_clone = stream_id.clone();
                                let result = store.retry_operation("load_snapshot", || {
                                    let event_store_clone = event_store.clone();
                                    let stream_id_clone = stream_id_clone.clone();
                                    async move {
                                        event_store_clone.load_snapshot(stream_id_clone).await
                                    }
                                }).await;

                                match result {
                                    Ok(snapshot) => {
                                        tracing::debug!(
                                            has_snapshot = snapshot.is_some(),
                                            "load_snapshot succeeded"
                                        );
                                        on_success(snapshot)
                                    },
                                    Err(error) => {
                                        tracing::warn!(error = %error, "load_snapshot failed");
                                        on_error(error)
                                    },
                                }
                            },
                        };

                        // Send action back to store if callback produced one
                        if let Some(action) = action {
                            tracing::trace!(
                                "EventStore operation produced an action, sending to store"
                            );
                            let _ = store.send(action).await;
                        } else {
                            tracing::trace!("EventStore operation completed with no action");
                        }
                    });
                },
                Effect::PublishEvent(op) => {
                    use composable_rust_core::effect::EventBusOperation;

                    tracing::trace!("Executing Effect::PublishEvent");
                    metrics::counter!("store.effects.executed", "type" => "publish_event").increment(1);
                    tracking.increment();
                    let tracking_clone = tracking.clone();
                    let store = self.clone();

                    tokio::spawn(async move {
                        let _guard = DecrementGuard(tracking_clone.clone());

                        let action = match op {
                            EventBusOperation::Publish {
                                event_bus,
                                topic,
                                event,
                                on_success,
                                on_error,
                            } => {
                                tracing::debug!(
                                    topic = %topic,
                                    event_type = %event.event_type,
                                    "Executing publish"
                                );

                                // Wrap with retry logic
                                let topic_clone = topic.clone();
                                let event_clone = event.clone();
                                let result = store.retry_operation("publish", || {
                                    let event_bus_clone = event_bus.clone();
                                    let topic_clone = topic_clone.clone();
                                    let event_clone = event_clone.clone();
                                    async move {
                                        event_bus_clone.publish(&topic_clone, &event_clone).await
                                    }
                                }).await;

                                match result {
                                    Ok(()) => {
                                        tracing::debug!(topic = %topic, "publish succeeded");
                                        on_success(())
                                    },
                                    Err(error) => {
                                        tracing::warn!(
                                            topic = %topic,
                                            error = %error,
                                            "publish failed"
                                        );
                                        on_error(error)
                                    },
                                }
                            },
                        };

                        // Send action back to store if callback produced one
                        if let Some(action) = action {
                            tracing::trace!(
                                "PublishEvent operation produced an action, sending to store"
                            );
                            let _ = store.send(action).await;
                        } else {
                            tracing::trace!("PublishEvent operation completed with no action");
                        }
                    });
                },
            }
        }
    }

    impl<S, A, E, R> Clone for Store<S, A, E, R>
    where
        R: Reducer<State = S, Action = A, Environment = E> + Clone,
        E: Clone,
    {
        fn clone(&self) -> Self {
            Self {
                state: Arc::clone(&self.state),
                reducer: self.reducer.clone(),
                environment: self.environment.clone(),
                retry_policy: self.retry_policy.clone(),
                dlq: self.dlq.clone(),
                shutdown: Arc::clone(&self.shutdown),
                pending_effects: Arc::clone(&self.pending_effects),
                action_broadcast: self.action_broadcast.clone(),
            }
        }
    }
}

// Re-export for convenience
pub use store::Store;

// Test module
#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_core::{effect::Effect, reducer::Reducer, smallvec, SmallVec};
    use std::time::Duration;

    // Test state
    #[derive(Debug, Clone)]
    struct TestState {
        value: i32,
    }

    // Test action
    #[derive(Debug, Clone)]
    enum TestAction {
        Increment,
        Decrement,
        NoOp,
        ProduceEffect,
        ProduceDelayedAction,
        ProduceParallelEffects,
        ProduceSequentialEffects,
        ProducePanickingEffect,
    }

    // Test environment
    #[derive(Debug, Clone)]
    struct TestEnv;

    // Test reducer
    #[derive(Debug, Clone)]
    struct TestReducer;

    impl Reducer for TestReducer {
        type State = TestState;
        type Action = TestAction;
        type Environment = TestEnv;

        fn reduce(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            _env: &Self::Environment,
        ) -> SmallVec<[Effect<Self::Action>; 4]> {
            match action {
                TestAction::Increment => {
                    state.value += 1;
                    smallvec![Effect::None]
                },
                TestAction::Decrement => {
                    state.value -= 1;
                    smallvec![Effect::None]
                },
                TestAction::NoOp => smallvec![Effect::None],
                TestAction::ProduceEffect => {
                    // Return an effect that produces another action
                    smallvec![Effect::Future(Box::pin(async {
                        Some(TestAction::Increment)
                    }))]
                },
                TestAction::ProduceDelayedAction => {
                    // Return a delayed effect
                    smallvec![Effect::Delay {
                        duration: Duration::from_millis(10),
                        action: Box::new(TestAction::Increment),
                    }]
                },
                TestAction::ProduceParallelEffects => {
                    // Return parallel effects that each produce an increment
                    smallvec![Effect::Parallel(vec![
                        Effect::Future(Box::pin(async { Some(TestAction::Increment) })),
                        Effect::Future(Box::pin(async { Some(TestAction::Increment) })),
                        Effect::Future(Box::pin(async { Some(TestAction::Increment) })),
                    ])]
                },
                TestAction::ProduceSequentialEffects => {
                    // Return sequential effects: increment, increment, decrement
                    smallvec![Effect::Sequential(vec![
                        Effect::Future(Box::pin(async { Some(TestAction::Increment) })),
                        Effect::Future(Box::pin(async { Some(TestAction::Increment) })),
                        Effect::Future(Box::pin(async { Some(TestAction::Decrement) })),
                    ])]
                },
                TestAction::ProducePanickingEffect => {
                    // Return an effect that will panic when executed
                    #[allow(clippy::panic)] // Intentional panic for testing error handling
                    {
                        smallvec![Effect::Future(Box::pin(async {
                            panic!("Intentional panic in effect for testing");
                        }))]
                    }
                },
            }
        }
    }

    #[tokio::test]
    async fn test_store_creation() {
        let state = TestState { value: 0 };
        let store = Store::new(state, TestReducer, TestEnv);

        let value = store.state(|s| s.value).await;
        assert_eq!(value, 0);
    }

    #[tokio::test]
    async fn test_send_action() {
        let state = TestState { value: 0 };
        let store = Store::new(state, TestReducer, TestEnv);

        let _ = store.send(TestAction::Increment).await;
        let value = store.state(|s| s.value).await;
        assert_eq!(value, 1);
    }

    #[tokio::test]
    async fn test_multiple_actions() {
        let state = TestState { value: 0 };
        let store = Store::new(state, TestReducer, TestEnv);

        let _ = store.send(TestAction::Increment).await;
        let _ = store.send(TestAction::Increment).await;
        let _ = store.send(TestAction::Decrement).await;

        let value = store.state(|s| s.value).await;
        assert_eq!(value, 1);
    }

    #[tokio::test]
    async fn test_effect_none() {
        let state = TestState { value: 0 };
        let store = Store::new(state, TestReducer, TestEnv);

        let _ = store.send(TestAction::NoOp).await;
        let value = store.state(|s| s.value).await;
        assert_eq!(value, 0);
    }

    #[tokio::test]
    async fn test_effect_future() {
        let state = TestState { value: 0 };
        let store = Store::new(state, TestReducer, TestEnv);

        // Send action that produces an effect
        let _ = store.send(TestAction::ProduceEffect).await;

        // Give the spawned task time to complete
        tokio::time::sleep(Duration::from_millis(50)).await;

        // The effect should have produced an Increment action
        let value = store.state(|s| s.value).await;
        assert_eq!(value, 1);
    }

    #[tokio::test]
    async fn test_effect_delay() {
        let state = TestState { value: 0 };
        let store = Store::new(state, TestReducer, TestEnv);

        // Send action that produces a delayed effect
        let _ = store.send(TestAction::ProduceDelayedAction).await;

        // Value should still be 0 immediately
        let value = store.state(|s| s.value).await;
        assert_eq!(value, 0);

        // Wait for delay to complete
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Now value should be 1
        let value = store.state(|s| s.value).await;
        assert_eq!(value, 1);
    }

    #[tokio::test]
    async fn test_effect_parallel() {
        let state = TestState { value: 0 };
        let store = Store::new(state, TestReducer, TestEnv);

        // Send action that produces parallel effects
        let _ = store.send(TestAction::ProduceParallelEffects).await;

        // Give the spawned tasks time to complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        // All three increments should have completed
        let value = store.state(|s| s.value).await;
        assert_eq!(value, 3);
    }

    #[tokio::test]
    async fn test_effect_sequential() {
        let state = TestState { value: 0 };
        let store = Store::new(state, TestReducer, TestEnv);

        // Send action that produces sequential effects
        let _ = store.send(TestAction::ProduceSequentialEffects).await;

        // Give the spawned tasks time to complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Net result: +1 +1 -1 = 1
        let value = store.state(|s| s.value).await;
        assert_eq!(value, 1);
    }

    #[tokio::test]
    #[allow(clippy::panic)] // Tests are allowed to panic on failures
    async fn test_concurrent_sends() {
        let state = TestState { value: 0 };
        let store = Store::new(state, TestReducer, TestEnv);

        // Send multiple actions concurrently
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let store = store.clone();
                tokio::spawn(async move {
                    let _ = store.send(TestAction::Increment).await;
                })
            })
            .collect();

        // Wait for all to complete
        for handle in handles {
            if let Err(e) = handle.await {
                panic!("concurrent send task panicked: {e}");
            }
        }

        // All increments should have been applied
        let value = store.state(|s| s.value).await;
        assert_eq!(value, 10);
    }

    #[tokio::test]
    async fn test_state_read_during_execution() {
        let state = TestState { value: 0 };
        let store = Store::new(state, TestReducer, TestEnv);

        let _ = store.send(TestAction::Increment).await;

        // Reading state should work while effects might be executing
        let value = store.state(|s| s.value).await;
        assert_eq!(value, 1);
    }

    #[tokio::test]
    async fn test_store_clone() {
        let state = TestState { value: 0 };
        let store1 = Store::new(state, TestReducer, TestEnv);
        let store2 = store1.clone();

        // Both stores should share the same state
        let _ = store1.send(TestAction::Increment).await;
        let value2 = store2.state(|s| s.value).await;
        assert_eq!(value2, 1);

        let _ = store2.send(TestAction::Increment).await;
        let value1 = store1.state(|s| s.value).await;
        assert_eq!(value1, 2);
    }

    #[tokio::test]
    async fn test_effect_panic_isolation() -> Result<(), StoreError> {
        // Test that a panic in an effect doesn't crash the Store
        // This verifies our error handling strategy: effects fail gracefully
        let state = TestState { value: 0 };
        let store = Store::new(state, TestReducer, TestEnv);

        // This action produces an effect that will panic
        let mut handle = store.send(TestAction::ProducePanickingEffect).await?;

        // Wait for the effect to complete (which includes the panic)
        // The effect will panic, but it's isolated in the spawned task
        handle.wait().await;

        // Small delay to ensure the panicking task has finished
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Store should still be functional after effect panic
        let _ = store.send(TestAction::Increment).await;
        let value = store.state(|s| s.value).await;
        assert_eq!(value, 1);

        // Can send multiple actions after panic
        let _ = store.send(TestAction::Increment).await;
        let value = store.state(|s| s.value).await;
        assert_eq!(value, 2);

        Ok(())
    }

    // EventStore effect tests
    mod event_store_tests {
        use super::*;
        use composable_rust_core::effect::{Effect, EventStoreOperation};
        use composable_rust_core::event::SerializedEvent;
        use composable_rust_core::event_store::EventStore;
        use composable_rust_core::stream::{StreamId, Version};
        use composable_rust_core::{smallvec, SmallVec};
        use std::sync::Arc;

        // Test action for EventStore effects
        #[derive(Debug, Clone)]
        enum EventStoreAction {
            AppendEvents {
                stream_id: String,
                events: Vec<String>,
            },
            EventsAppended {
                version: u64,
            },
            AppendFailed {
                error: String,
            },
            LoadEvents {
                stream_id: String,
            },
            EventsLoaded {
                count: usize,
            },
            LoadFailed {
                error: String,
            },
            SaveSnapshot {
                stream_id: String,
                version: u64,
            },
            SnapshotSaved,
            LoadSnapshot {
                stream_id: String,
            },
            SnapshotLoaded {
                found: bool,
            },
        }

        // Test state for EventStore
        #[derive(Debug, Clone)]
        struct EventStoreState {
            last_version: Option<u64>,
            event_count: usize,
            snapshot_saved: bool,
            snapshot_loaded: bool,
            error: Option<String>,
        }

        // Test environment with EventStore
        #[derive(Clone)]
        struct EventStoreEnv {
            event_store: Arc<dyn EventStore>,
        }

        // Test reducer for EventStore
        #[derive(Clone)]
        struct EventStoreReducer;

        impl Reducer for EventStoreReducer {
            type State = EventStoreState;
            type Action = EventStoreAction;
            type Environment = EventStoreEnv;

            fn reduce(
                &self,
                state: &mut Self::State,
                action: Self::Action,
                env: &Self::Environment,
            ) -> SmallVec<[Effect<Self::Action>; 4]> {
                match action {
                    EventStoreAction::AppendEvents { stream_id, events } => {
                        let serialized_events: Vec<SerializedEvent> = events
                            .into_iter()
                            .map(|data| {
                                SerializedEvent::new(
                                    "TestEvent.v1".to_string(),
                                    data.into_bytes(),
                                    None,
                                )
                            })
                            .collect();

                        smallvec![Effect::EventStore(EventStoreOperation::AppendEvents {
                            event_store: Arc::clone(&env.event_store),
                            stream_id: StreamId::new(&stream_id),
                            expected_version: state.last_version.map(Version::new),
                            events: serialized_events,
                            on_success: Box::new(|version| {
                                Some(EventStoreAction::EventsAppended {
                                    version: version.value(),
                                })
                            }),
                            on_error: Box::new(|error| {
                                Some(EventStoreAction::AppendFailed {
                                    error: error.to_string(),
                                })
                            }),
                        })]
                    },
                    EventStoreAction::EventsAppended { version } => {
                        state.last_version = Some(version);
                        smallvec![Effect::None]
                    },
                    EventStoreAction::AppendFailed { error }
                    | EventStoreAction::LoadFailed { error } => {
                        state.error = Some(error);
                        smallvec![Effect::None]
                    },
                    EventStoreAction::LoadEvents { stream_id } => {
                        smallvec![Effect::EventStore(EventStoreOperation::LoadEvents {
                            event_store: Arc::clone(&env.event_store),
                            stream_id: StreamId::new(&stream_id),
                            from_version: None,
                            on_success: Box::new(|events| {
                                Some(EventStoreAction::EventsLoaded {
                                    count: events.len(),
                                })
                            }),
                            on_error: Box::new(|error| {
                                Some(EventStoreAction::LoadFailed {
                                    error: error.to_string(),
                                })
                            }),
                        })]
                    },
                    EventStoreAction::EventsLoaded { count } => {
                        state.event_count = count;
                        smallvec![Effect::None]
                    },
                    EventStoreAction::SaveSnapshot { stream_id, version } => {
                        let state_bytes = vec![1, 2, 3, 4]; // Mock state data
                        smallvec![Effect::EventStore(EventStoreOperation::SaveSnapshot {
                            event_store: Arc::clone(&env.event_store),
                            stream_id: StreamId::new(&stream_id),
                            version: Version::new(version),
                            state: state_bytes,
                            on_success: Box::new(|()| Some(EventStoreAction::SnapshotSaved)),
                            on_error: Box::new(|error| {
                                Some(EventStoreAction::AppendFailed {
                                    error: error.to_string(),
                                })
                            }),
                        })]
                    },
                    EventStoreAction::SnapshotSaved => {
                        state.snapshot_saved = true;
                        smallvec![Effect::None]
                    },
                    EventStoreAction::LoadSnapshot { stream_id } => {
                        smallvec![Effect::EventStore(EventStoreOperation::LoadSnapshot {
                            event_store: Arc::clone(&env.event_store),
                            stream_id: StreamId::new(&stream_id),
                            on_success: Box::new(|snapshot| {
                                Some(EventStoreAction::SnapshotLoaded {
                                    found: snapshot.is_some(),
                                })
                            }),
                            on_error: Box::new(|error| {
                                Some(EventStoreAction::LoadFailed {
                                    error: error.to_string(),
                                })
                            }),
                        })]
                    },
                    EventStoreAction::SnapshotLoaded { found } => {
                        state.snapshot_loaded = found;
                        smallvec![Effect::None]
                    },
                }
            }
        }

        #[tokio::test]
        async fn test_eventstore_append_success() -> Result<(), StoreError> {
            use composable_rust_testing::mocks::InMemoryEventStore;

            let event_store = Arc::new(InMemoryEventStore::new()) as Arc<dyn EventStore>;
            let env = EventStoreEnv {
                event_store: Arc::clone(&event_store),
            };
            let state = EventStoreState {
                last_version: None,
                event_count: 0,
                snapshot_saved: false,
                snapshot_loaded: false,
                error: None,
            };

            let store = Store::new(state, EventStoreReducer, env);

            // Append events
            let mut handle = store
                .send(EventStoreAction::AppendEvents {
                    stream_id: "test-stream".to_string(),
                    events: vec!["event1".to_string(), "event2".to_string()],
                })
                .await?;

            handle.wait().await;

            // Check state was updated with version
            let last_version = store.state(|s| s.last_version).await;
            assert_eq!(last_version, Some(1)); // 2 events, version 0-1, returns last = 1

            Ok(())
        }

        #[tokio::test]
        async fn test_eventstore_append_concurrency_conflict() -> Result<(), StoreError> {
            use composable_rust_testing::mocks::InMemoryEventStore;

            let event_store = Arc::new(InMemoryEventStore::new()) as Arc<dyn EventStore>;
            let env = EventStoreEnv {
                event_store: Arc::clone(&event_store),
            };

            // Pre-populate event store with some events
            let stream_id = StreamId::new("test-stream");
            let events = vec![SerializedEvent::new(
                "TestEvent.v1".to_string(),
                b"data".to_vec(),
                None,
            )];
            event_store
                .append_events(stream_id.clone(), Some(Version::new(0)), events)
                .await
                .ok();

            // Create store with wrong expected version
            let state = EventStoreState {
                last_version: Some(5), // Wrong version, actual is 0
                event_count: 0,
                snapshot_saved: false,
                snapshot_loaded: false,
                error: None,
            };

            let store = Store::new(state, EventStoreReducer, env);

            // Try to append with wrong version
            let mut handle = store
                .send(EventStoreAction::AppendEvents {
                    stream_id: "test-stream".to_string(),
                    events: vec!["event".to_string()],
                })
                .await?;

            handle.wait().await;

            // Check error was captured
            let error = store.state(|s| s.error.clone()).await;
            assert!(error.is_some());
            #[allow(clippy::unwrap_used)] // Panics: Test verified error is Some above
            {
                assert!(error.unwrap().contains("Concurrency"));
            }

            Ok(())
        }

        #[tokio::test]
        async fn test_eventstore_load_events() -> Result<(), StoreError> {
            use composable_rust_testing::mocks::InMemoryEventStore;

            let event_store = Arc::new(InMemoryEventStore::new()) as Arc<dyn EventStore>;
            let env = EventStoreEnv {
                event_store: Arc::clone(&event_store),
            };

            // Pre-populate with events
            let stream_id = StreamId::new("test-stream");
            let events = vec![
                SerializedEvent::new("TestEvent.v1".to_string(), b"event1".to_vec(), None),
                SerializedEvent::new("TestEvent.v1".to_string(), b"event2".to_vec(), None),
                SerializedEvent::new("TestEvent.v1".to_string(), b"event3".to_vec(), None),
            ];
            event_store
                .append_events(stream_id, Some(Version::new(0)), events)
                .await
                .ok();

            let state = EventStoreState {
                last_version: None,
                event_count: 0,
                snapshot_saved: false,
                snapshot_loaded: false,
                error: None,
            };

            let store = Store::new(state, EventStoreReducer, env);

            // Load events
            let mut handle = store
                .send(EventStoreAction::LoadEvents {
                    stream_id: "test-stream".to_string(),
                })
                .await?;

            handle.wait().await;

            // Check count was updated
            let count = store.state(|s| s.event_count).await;
            assert_eq!(count, 3);

            Ok(())
        }

        #[tokio::test]
        async fn test_eventstore_snapshot_roundtrip() -> Result<(), StoreError> {
            use composable_rust_testing::mocks::InMemoryEventStore;

            let event_store = Arc::new(InMemoryEventStore::new()) as Arc<dyn EventStore>;
            let env = EventStoreEnv {
                event_store: Arc::clone(&event_store),
            };
            let state = EventStoreState {
                last_version: None,
                event_count: 0,
                snapshot_saved: false,
                snapshot_loaded: false,
                error: None,
            };

            let store = Store::new(state, EventStoreReducer, env);

            // Save snapshot
            let mut handle = store
                .send(EventStoreAction::SaveSnapshot {
                    stream_id: "test-stream".to_string(),
                    version: 10,
                })
                .await?;

            handle.wait().await;

            // Check snapshot was saved
            let saved = store.state(|s| s.snapshot_saved).await;
            assert!(saved);

            // Load snapshot
            let mut handle = store
                .send(EventStoreAction::LoadSnapshot {
                    stream_id: "test-stream".to_string(),
                })
                .await?;

            handle.wait().await;

            // Check snapshot was loaded
            let loaded = store.state(|s| s.snapshot_loaded).await;
            assert!(loaded);

            Ok(())
        }

        #[tokio::test]
        async fn test_eventstore_parallel_operations() -> Result<(), StoreError> {
            use composable_rust_testing::mocks::InMemoryEventStore;

            let event_store = Arc::new(InMemoryEventStore::new()) as Arc<dyn EventStore>;
            let env = EventStoreEnv {
                event_store: Arc::clone(&event_store),
            };
            let state = EventStoreState {
                last_version: None,
                event_count: 0,
                snapshot_saved: false,
                snapshot_loaded: false,
                error: None,
            };

            let store = Store::new(state, EventStoreReducer, env);

            // Append to stream1
            let mut h1 = store
                .send(EventStoreAction::AppendEvents {
                    stream_id: "stream-1".to_string(),
                    events: vec!["event1".to_string()],
                })
                .await?;

            // Append to stream2 (different stream, can run concurrently)
            let mut h2 = store
                .send(EventStoreAction::AppendEvents {
                    stream_id: "stream-2".to_string(),
                    events: vec!["event2".to_string()],
                })
                .await?;

            // Wait for both
            h1.wait().await;
            h2.wait().await;

            // Both should have succeeded (last_version reflects last operation)
            let last_version = store.state(|s| s.last_version).await;
            assert!(last_version.is_some());

            Ok(())
        }

        // Reducer for testing parallel EventStore effects
        #[derive(Clone)]
        struct ParallelTestReducer;

        impl Reducer for ParallelTestReducer {
            type State = EventStoreState;
            type Action = EventStoreAction;
            type Environment = EventStoreEnv;

            fn reduce(
                &self,
                _state: &mut Self::State,
                _action: Self::Action,
                env: &Self::Environment,
            ) -> SmallVec<[Effect<Self::Action>; 4]> {
                // Create parallel effects on each call
                smallvec![Effect::Parallel(vec![
                    Effect::EventStore(EventStoreOperation::AppendEvents {
                        event_store: Arc::clone(&env.event_store),
                        stream_id: StreamId::new("stream-1"),
                        expected_version: Some(Version::new(0)),
                        events: vec![SerializedEvent::new(
                            "Test.v1".to_string(),
                            b"data1".to_vec(),
                            None,
                        )],
                        on_success: Box::new(|_| None), // No feedback action
                        on_error: Box::new(|_| None),
                    }),
                    Effect::EventStore(EventStoreOperation::AppendEvents {
                        event_store: Arc::clone(&env.event_store),
                        stream_id: StreamId::new("stream-2"),
                        expected_version: Some(Version::new(0)),
                        events: vec![SerializedEvent::new(
                            "Test.v1".to_string(),
                            b"data2".to_vec(),
                            None,
                        )],
                        on_success: Box::new(|_| None),
                        on_error: Box::new(|_| None),
                    }),
                ])]
            }
        }

        #[tokio::test]
        async fn test_eventstore_effect_in_parallel_composition() -> Result<(), StoreError> {
            use composable_rust_testing::mocks::InMemoryEventStore;

            // Test that EventStore effects work inside Parallel effect composition
            let event_store = Arc::new(InMemoryEventStore::new()) as Arc<dyn EventStore>;
            let env = EventStoreEnv {
                event_store: Arc::clone(&event_store),
            };

            let test_store = Store::new(
                EventStoreState {
                    last_version: None,
                    event_count: 0,
                    snapshot_saved: false,
                    snapshot_loaded: false,
                    error: None,
                },
                ParallelTestReducer,
                env.clone(),
            );

            // Trigger the parallel effect
            let mut handle = test_store
                .send(EventStoreAction::AppendEvents {
                    stream_id: "dummy".to_string(),
                    events: vec![],
                })
                .await?;

            handle.wait().await;

            // Verify both streams got events
            let events1 = env
                .event_store
                .load_events(StreamId::new("stream-1"), None)
                .await
                .ok();
            let events2 = env
                .event_store
                .load_events(StreamId::new("stream-2"), None)
                .await
                .ok();

            #[allow(clippy::unwrap_used)] // Panics: Test verified events are Some above
            {
                assert!(events1.is_some());
                assert!(events2.is_some());
                assert_eq!(events1.unwrap().len(), 1);
                assert_eq!(events2.unwrap().len(), 1);
            }

            Ok(())
        }
    }

    /// Tests for `RetryPolicy`
    mod retry_policy_tests {
        use super::*;

        #[test]
        fn test_retry_policy_default() {
            let policy = RetryPolicy::default();
            assert_eq!(policy.max_attempts(), 5);
            assert!(policy.should_retry(0));
            assert!(policy.should_retry(4));
            assert!(!policy.should_retry(5));
        }

        #[test]
        fn test_retry_policy_builder() {
            let policy = RetryPolicy::new()
                .with_max_attempts(3)
                .with_initial_delay(Duration::from_millis(100))
                .with_max_delay(Duration::from_secs(10))
                .with_backoff_multiplier(3.0);

            assert_eq!(policy.max_attempts(), 3);
            assert!(policy.should_retry(2));
            assert!(!policy.should_retry(3));
        }

        #[test]
        fn test_delay_increases_exponentially() {
            let policy = RetryPolicy::new()
                .with_initial_delay(Duration::from_secs(1))
                .with_backoff_multiplier(2.0)
                .with_max_delay(Duration::from_secs(100));

            let delay0 = policy.delay_for_attempt(0);
            let delay1 = policy.delay_for_attempt(1);
            let delay2 = policy.delay_for_attempt(2);

            // Delays should generally increase (accounting for jitter 0.5-1.0)
            // delay0 ~= 1s * (0.5-1.0)
            // delay1 ~= 2s * (0.5-1.0)
            // delay2 ~= 4s * (0.5-1.0)
            assert!(delay0.as_millis() >= 500 && delay0.as_millis() <= 1000);
            assert!(delay1.as_millis() >= 1000 && delay1.as_millis() <= 2000);
            assert!(delay2.as_millis() >= 2000 && delay2.as_millis() <= 4000);
        }

        #[test]
        fn test_delay_caps_at_max() {
            let policy = RetryPolicy::new()
                .with_initial_delay(Duration::from_secs(1))
                .with_backoff_multiplier(2.0)
                .with_max_delay(Duration::from_secs(5));

            // Attempt 10 would normally be 2^10 = 1024 seconds
            // But should be capped at 5 seconds (plus jitter 0.5-1.0)
            let delay = policy.delay_for_attempt(10);
            assert!(delay.as_millis() >= 2500 && delay.as_millis() <= 5000);
        }

        #[test]
        fn test_jitter_variation() {
            let policy = RetryPolicy::new()
                .with_initial_delay(Duration::from_secs(1))
                .with_backoff_multiplier(2.0);

            // Run multiple times to ensure jitter provides variation
            let mut delays = Vec::new();
            for _ in 0..10 {
                delays.push(policy.delay_for_attempt(1));
            }

            // Should have variation due to jitter
            // Not all delays should be exactly the same
            let first = delays[0];
            let has_variation = delays.iter().any(|d| d != &first);
            assert!(has_variation, "Jitter should produce variation in delays");
        }
    }

    mod circuit_breaker_tests {
        use super::*;

        #[test]
        fn test_circuit_breaker_initial_state() {
            let breaker = CircuitBreaker::new();
            assert_eq!(breaker.state(), CircuitState::Closed);
        }

        #[test]
        fn test_circuit_breaker_opens_after_threshold() {
            let breaker = CircuitBreaker::new().with_failure_threshold(3);

            assert_eq!(breaker.state(), CircuitState::Closed);

            // Record failures
            breaker.record_failure();
            assert_eq!(breaker.state(), CircuitState::Closed);

            breaker.record_failure();
            assert_eq!(breaker.state(), CircuitState::Closed);

            breaker.record_failure();
            assert_eq!(breaker.state(), CircuitState::Open);
        }

        #[test]
        fn test_circuit_breaker_resets_on_success() {
            let breaker = CircuitBreaker::new().with_failure_threshold(3);

            // Record some failures
            breaker.record_failure();
            breaker.record_failure();
            assert_eq!(breaker.state(), CircuitState::Closed);

            // Success resets the count
            breaker.record_success();

            // Should need 3 more failures to open
            breaker.record_failure();
            breaker.record_failure();
            assert_eq!(breaker.state(), CircuitState::Closed);

            breaker.record_failure();
            assert_eq!(breaker.state(), CircuitState::Open);
        }

        #[tokio::test]
        async fn test_circuit_breaker_half_open_after_timeout() {
            let breaker = CircuitBreaker::new()
                .with_failure_threshold(2)
                .with_timeout(Duration::from_millis(100));

            // Open the circuit
            breaker.record_failure();
            breaker.record_failure();
            assert_eq!(breaker.state(), CircuitState::Open);

            // Check should fail immediately
            assert!(breaker.check().is_err());

            // Wait for timeout
            tokio::time::sleep(Duration::from_millis(150)).await;

            // Check should transition to HalfOpen
            assert!(breaker.check().is_ok());
            assert_eq!(breaker.state(), CircuitState::HalfOpen);
        }

        #[test]
        fn test_circuit_breaker_closes_after_success_threshold() {
            let breaker = CircuitBreaker::new()
                .with_failure_threshold(2)
                .with_success_threshold(2);

            // Open the circuit
            breaker.record_failure();
            breaker.record_failure();
            assert_eq!(breaker.state(), CircuitState::Open);

            // Manually transition to HalfOpen for testing
            breaker
                .state
                .store(CircuitState::HalfOpen as u8, Ordering::Release);

            // Record successes
            breaker.record_success();
            assert_eq!(breaker.state(), CircuitState::HalfOpen);

            breaker.record_success();
            assert_eq!(breaker.state(), CircuitState::Closed);
        }

        #[test]
        fn test_circuit_breaker_reopens_on_half_open_failure() {
            let breaker = CircuitBreaker::new().with_failure_threshold(2);

            // Open the circuit
            breaker.record_failure();
            breaker.record_failure();
            assert_eq!(breaker.state(), CircuitState::Open);

            // Manually transition to HalfOpen
            breaker
                .state
                .store(CircuitState::HalfOpen as u8, Ordering::Release);

            // Any failure in HalfOpen should reopen circuit
            breaker.record_failure();
            assert_eq!(breaker.state(), CircuitState::Open);
        }

        #[tokio::test]
        async fn test_circuit_breaker_call_success() {
            let breaker = CircuitBreaker::new();

            let result = breaker
                .call(|| async { Ok::<i32, String>(42) })
                .await;

            assert!(result.is_ok());
            #[allow(clippy::unwrap_used)] // Safe: just asserted is_ok()
            {
                assert_eq!(result.unwrap(), 42);
            }
            assert_eq!(breaker.state(), CircuitState::Closed);
        }

        #[tokio::test]
        async fn test_circuit_breaker_call_opens_on_failures() {
            let breaker = CircuitBreaker::new().with_failure_threshold(3);

            // Fail 3 times
            for _ in 0..3 {
                let result = breaker
                    .call(|| async { Err::<i32, String>("error".to_string()) })
                    .await;
                assert!(result.is_err());
            }

            assert_eq!(breaker.state(), CircuitState::Open);

            // Next call should fail fast
            let result = breaker
                .call(|| async { Ok::<i32, String>(42) })
                .await;

            assert!(result.is_err());
            assert!(matches!(result, Err(Either::Left(_))));
        }

        #[tokio::test]
        async fn test_circuit_breaker_concurrent_access() {
            use std::sync::Arc;

            let breaker = Arc::new(CircuitBreaker::new().with_failure_threshold(5));

            // Record failures concurrently
            let mut handles = vec![];
            for _ in 0..10 {
                let breaker_clone = Arc::clone(&breaker);
                handles.push(tokio::spawn(async move {
                    breaker_clone.record_failure();
                }));
            }

            #[allow(clippy::unwrap_used)] // Test code: tasks should not panic
            for handle in handles {
                handle.await.unwrap();
            }

            // Circuit should be open (10 failures, threshold is 5)
            assert_eq!(breaker.state(), CircuitState::Open);
        }

        #[test]
        fn test_circuit_breaker_builder() {
            let breaker = CircuitBreaker::new()
                .with_failure_threshold(10)
                .with_timeout(Duration::from_secs(30))
                .with_success_threshold(3);

            // Verify by opening the circuit
            for _ in 0..10 {
                breaker.record_failure();
            }
            assert_eq!(breaker.state(), CircuitState::Open);
        }
    }

    mod dlq_tests {
        use super::*;

        #[test]
        fn test_dlq_new() {
            let dlq: DeadLetterQueue<String> = DeadLetterQueue::new(100);
            assert_eq!(dlq.len(), 0);
            assert!(dlq.is_empty());
            assert_eq!(dlq.max_size(), 100);
        }

        #[test]
        fn test_dlq_push_and_len() {
            let dlq = DeadLetterQueue::new(10);

            dlq.push(
                "operation1".to_string(),
                "Connection timeout".to_string(),
                5,
            );
            assert_eq!(dlq.len(), 1);
            assert!(!dlq.is_empty());

            dlq.push(
                "operation2".to_string(),
                "Database error".to_string(),
                3,
            );
            assert_eq!(dlq.len(), 2);
        }

        #[test]
        fn test_dlq_peek() {
            let dlq = DeadLetterQueue::new(10);

            dlq.push(
                "first".to_string(),
                "error1".to_string(),
                1,
            );
            dlq.push(
                "second".to_string(),
                "error2".to_string(),
                2,
            );

            // Peek should return first entry without removing it
            #[allow(clippy::unwrap_used)] // Test code: just pushed entries
            let entry = dlq.peek().unwrap();
            assert_eq!(entry.payload, "first");
            assert_eq!(entry.error_message, "error1");
            assert_eq!(entry.retry_count, 1);
            assert_eq!(dlq.len(), 2); // Still has 2 entries
        }

        #[test]
        fn test_dlq_drain() {
            let dlq = DeadLetterQueue::new(10);

            dlq.push("op1".to_string(), "err1".to_string(), 1);
            dlq.push("op2".to_string(), "err2".to_string(), 2);
            dlq.push("op3".to_string(), "err3".to_string(), 3);

            assert_eq!(dlq.len(), 3);

            let entries = dlq.drain();
            assert_eq!(entries.len(), 3);
            assert_eq!(entries[0].payload, "op1");
            assert_eq!(entries[1].payload, "op2");
            assert_eq!(entries[2].payload, "op3");

            // Queue should be empty after drain
            assert_eq!(dlq.len(), 0);
            assert!(dlq.is_empty());
        }

        #[test]
        fn test_dlq_max_size_drops_oldest() {
            let dlq = DeadLetterQueue::new(3);

            dlq.push("op1".to_string(), "err".to_string(), 1);
            dlq.push("op2".to_string(), "err".to_string(), 1);
            dlq.push("op3".to_string(), "err".to_string(), 1);
            assert_eq!(dlq.len(), 3);

            // This should drop op1
            dlq.push("op4".to_string(), "err".to_string(), 1);
            assert_eq!(dlq.len(), 3);

            // Peek should show op2 (op1 was dropped)
            #[allow(clippy::unwrap_used)] // Test code: just pushed entries
            let entry = dlq.peek().unwrap();
            assert_eq!(entry.payload, "op2");
        }

        #[test]
        fn test_dlq_fifo_ordering() {
            let dlq = DeadLetterQueue::new(10);

            dlq.push("first".to_string(), "err".to_string(), 1);
            dlq.push("second".to_string(), "err".to_string(), 1);
            dlq.push("third".to_string(), "err".to_string(), 1);

            let entries = dlq.drain();
            assert_eq!(entries[0].payload, "first");
            assert_eq!(entries[1].payload, "second");
            assert_eq!(entries[2].payload, "third");
        }

        #[test]
        fn test_dlq_clone() {
            let dlq1 = DeadLetterQueue::new(10);
            dlq1.push("op1".to_string(), "err".to_string(), 1);

            let dlq2 = dlq1.clone();
            assert_eq!(dlq2.len(), 1);
            assert_eq!(dlq2.max_size(), 10);

            // Both should share the same queue
            dlq2.push("op2".to_string(), "err".to_string(), 1);
            assert_eq!(dlq1.len(), 2); // dlq1 sees the change
        }

        #[test]
        fn test_dlq_default() {
            let dlq: DeadLetterQueue<i32> = DeadLetterQueue::default();
            assert_eq!(dlq.max_size(), 1000);
            assert!(dlq.is_empty());
        }

        #[test]
        fn test_dead_letter_metadata() {
            let dlq = DeadLetterQueue::new(10);

            dlq.push(
                "operation".to_string(),
                "Connection timeout".to_string(),
                5,
            );

            #[allow(clippy::unwrap_used)] // Test code: just pushed entry
            let entry = dlq.peek().unwrap();
            assert_eq!(entry.retry_count, 5);
            assert_eq!(entry.error_message, "Connection timeout");
            assert!(entry.first_failed_at > 0);
            assert!(entry.last_failed_at > 0);
            assert_eq!(entry.first_failed_at, entry.last_failed_at); // Same for first failure
        }

        #[tokio::test]
        async fn test_dlq_concurrent_push() {
            use std::sync::Arc;

            let dlq = Arc::new(DeadLetterQueue::new(100));

            let mut handles = vec![];
            for i in 0..10 {
                let dlq_clone = Arc::clone(&dlq);
                handles.push(tokio::spawn(async move {
                    dlq_clone.push(
                        format!("op{i}"),
                        "error".to_string(),
                        1,
                    );
                }));
            }

            #[allow(clippy::unwrap_used)] // Test code: tasks should not panic
            for handle in handles {
                handle.await.unwrap();
            }

            assert_eq!(dlq.len(), 10);
        }

        #[test]
        fn test_dlq_empty_drain() {
            let dlq: DeadLetterQueue<String> = DeadLetterQueue::new(10);

            let entries = dlq.drain();
            assert_eq!(entries.len(), 0);
            assert!(dlq.is_empty());
        }
    }

    mod health_check_tests {
        use super::*;

        #[test]
        fn test_health_status_ordering() {
            assert!(HealthStatus::Healthy < HealthStatus::Degraded);
            assert!(HealthStatus::Degraded < HealthStatus::Unhealthy);
        }

        #[test]
        fn test_health_status_worst() {
            assert_eq!(
                HealthStatus::Healthy.worst(HealthStatus::Degraded),
                HealthStatus::Degraded
            );
            assert_eq!(
                HealthStatus::Degraded.worst(HealthStatus::Unhealthy),
                HealthStatus::Unhealthy
            );
            assert_eq!(
                HealthStatus::Healthy.worst(HealthStatus::Unhealthy),
                HealthStatus::Unhealthy
            );
        }

        #[test]
        fn test_health_check_healthy() {
            let check = HealthCheck::healthy("store");
            assert_eq!(check.component, "store");
            assert_eq!(check.status, HealthStatus::Healthy);
            assert!(check.message.is_none());
        }

        #[test]
        fn test_health_check_degraded() {
            let check = HealthCheck::degraded("store", "High latency");
            assert_eq!(check.component, "store");
            assert_eq!(check.status, HealthStatus::Degraded);
            assert_eq!(check.message, Some("High latency".to_string()));
        }

        #[test]
        fn test_health_check_unhealthy() {
            let check = HealthCheck::unhealthy("database", "Connection failed");
            assert_eq!(check.component, "database");
            assert_eq!(check.status, HealthStatus::Unhealthy);
            assert_eq!(check.message, Some("Connection failed".to_string()));
        }

        #[test]
        fn test_health_check_with_metadata() {
            let check = HealthCheck::healthy("store")
                .with_metadata("requests", "1000")
                .with_metadata("errors", "5");

            assert_eq!(check.metadata.len(), 2);
            assert_eq!(check.metadata[0], ("requests".to_string(), "1000".to_string()));
            assert_eq!(check.metadata[1], ("errors".to_string(), "5".to_string()));
        }

        #[test]
        fn test_health_report_all_healthy() {
            let checks = vec![
                HealthCheck::healthy("store"),
                HealthCheck::healthy("database"),
                HealthCheck::healthy("cache"),
            ];

            let report = HealthReport::new(checks);
            assert_eq!(report.status, HealthStatus::Healthy);
            assert!(report.is_healthy());
            assert!(!report.is_degraded());
            assert!(!report.is_unhealthy());
        }

        #[test]
        fn test_health_report_one_degraded() {
            let checks = vec![
                HealthCheck::healthy("store"),
                HealthCheck::degraded("database", "Slow queries"),
                HealthCheck::healthy("cache"),
            ];

            let report = HealthReport::new(checks);
            assert_eq!(report.status, HealthStatus::Degraded);
            assert!(!report.is_healthy());
            assert!(report.is_degraded());
        }

        #[test]
        fn test_health_report_one_unhealthy() {
            let checks = vec![
                HealthCheck::healthy("store"),
                HealthCheck::degraded("database", "Slow"),
                HealthCheck::unhealthy("cache", "Disconnected"),
            ];

            let report = HealthReport::new(checks);
            assert_eq!(report.status, HealthStatus::Unhealthy);
            assert!(report.is_unhealthy());
        }

        #[test]
        fn test_store_health_empty_dlq() {
            let store = Store::new(TestState { value: 0 }, TestReducer, TestEnv);

            let health = store.health();
            assert_eq!(health.status, HealthStatus::Healthy);
            assert_eq!(health.component, "store");
        }

        #[test]
        fn test_store_health_degraded_dlq() {
            let store = Store::new(TestState { value: 0 }, TestReducer, TestEnv);

            // Fill DLQ to 60% capacity (degraded threshold is 50%)
            for i in 0..600 {
                store.dlq().push(format!("op_{i}"), "error".to_string(), 5);
            }

            let health = store.health();
            assert_eq!(health.status, HealthStatus::Degraded);
            assert!(health.message.is_some());
        }

        #[test]
        fn test_store_health_unhealthy_full_dlq() {
            let store = Store::new(TestState { value: 0 }, TestReducer, TestEnv);

            // Fill DLQ to capacity
            for i in 0..1000 {
                store.dlq().push(format!("op_{i}"), "error".to_string(), 5);
            }

            let health = store.health();
            assert_eq!(health.status, HealthStatus::Unhealthy);
            assert_eq!(health.message, Some("Dead letter queue is full".to_string()));
        }

        #[test]
        fn test_health_status_display() {
            assert_eq!(format!("{}", HealthStatus::Healthy), "healthy");
            assert_eq!(format!("{}", HealthStatus::Degraded), "degraded");
            assert_eq!(format!("{}", HealthStatus::Unhealthy), "unhealthy");
        }
    }

    /// Tests for graceful shutdown
    mod shutdown_tests {
        use super::*;
        use std::time::Duration;

        #[tokio::test]
        async fn test_shutdown_with_no_pending_effects() -> Result<(), StoreError> {
            let state = TestState { value: 0 };
            let store = Store::new(state, TestReducer, TestEnv);

            // Shutdown immediately with no effects running
            let result = store.shutdown(Duration::from_secs(5)).await;
            assert!(result.is_ok());

            Ok(())
        }

        #[tokio::test]
        async fn test_shutdown_rejects_new_actions() -> Result<(), StoreError> {
            let state = TestState { value: 0 };
            let store = Store::new(state, TestReducer, TestEnv);

            // Initiate shutdown
            tokio::spawn({
                let store = store.clone();
                async move {
                    let _ = store.shutdown(Duration::from_secs(10)).await;
                }
            });

            // Give shutdown time to set the flag
            tokio::time::sleep(Duration::from_millis(50)).await;

            // Try to send action during shutdown
            let result = store.send(TestAction::Increment).await;
            assert!(matches!(result, Err(StoreError::ShutdownInProgress)));

            Ok(())
        }

        #[tokio::test]
        async fn test_shutdown_waits_for_effects() -> Result<(), StoreError> {
            let state = TestState { value: 0 };
            let store = Store::new(state, TestReducer, TestEnv);

            // Send action with delayed effect
            let _handle = store.send(TestAction::ProduceDelayedAction).await?;

            // Start shutdown in background (should wait for effect)
            let shutdown_store = store.clone();
            let shutdown_handle = tokio::spawn(async move {
                shutdown_store.shutdown(Duration::from_secs(5)).await
            });

            // Give it a moment to start shutdown
            tokio::time::sleep(Duration::from_millis(50)).await;

            // Wait for shutdown to complete
            let result = shutdown_handle.await;
            assert!(result.is_ok());
            #[allow(clippy::unwrap_used)] // Test code: just asserted is_ok()
            {
                assert!(result.unwrap().is_ok());
            }

            Ok(())
        }

        #[tokio::test]
        async fn test_shutdown_timeout() -> Result<(), StoreError> {
            // Create a custom reducer that returns a long-running effect
            #[derive(Clone)]
            struct LongRunningReducer;

            impl Reducer for LongRunningReducer {
                type State = TestState;
                type Action = TestAction;
                type Environment = TestEnv;

                fn reduce(
                    &self,
                    _state: &mut Self::State,
                    _action: Self::Action,
                    _env: &Self::Environment,
                ) -> SmallVec<[Effect<Self::Action>; 4]> {
                    smallvec![Effect::Future(Box::pin(async {
                        // Sleep for 200ms - longer than our timeout
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        Some(TestAction::Increment)
                    }))]
                }
            }

            let state = TestState { value: 0 };
            let store = Store::new(state, LongRunningReducer, TestEnv);

            // Send action that triggers long-running effect
            let _handle = store.send(TestAction::Increment).await?;

            // Give the effect time to start running
            tokio::time::sleep(Duration::from_millis(10)).await;

            // Try to shutdown with short timeout (50ms - effect won't finish in time)
            let result = store.shutdown(Duration::from_millis(50)).await;

            // Should timeout because the effect takes 200ms
            assert!(matches!(result, Err(StoreError::ShutdownTimeout(_))), "Expected ShutdownTimeout, got: {result:?}");

            if let Err(StoreError::ShutdownTimeout(pending)) = result {
                assert!(pending > 0, "Should report pending effects");
            }

            Ok(())
        }

        #[tokio::test]
        async fn test_shutdown_idempotent() -> Result<(), StoreError> {
            let state = TestState { value: 0 };
            let store = Store::new(state, TestReducer, TestEnv);

            // First shutdown
            let result1 = store.shutdown(Duration::from_secs(1)).await;
            assert!(result1.is_ok());

            // Second shutdown should also succeed (already shut down)
            let result2 = store.shutdown(Duration::from_secs(1)).await;
            assert!(result2.is_ok());

            Ok(())
        }
    }

    mod config_tests {
        use super::*;
        use std::time::Duration;

        #[test]
        fn test_store_config_default() {
            let config = StoreConfig::default();
            assert_eq!(config.dlq_max_size, 1000);
            assert_eq!(config.default_shutdown_timeout, Duration::from_secs(30));
        }

        #[test]
        fn test_store_config_builder_pattern() {
            let custom_retry = RetryPolicy::new()
                .with_max_attempts(10)
                .with_initial_delay(Duration::from_millis(100));

            let config = StoreConfig::default()
                .with_dlq_max_size(500)
                .with_retry_policy(custom_retry.clone())
                .with_shutdown_timeout(Duration::from_secs(60));

            assert_eq!(config.dlq_max_size, 500);
            assert_eq!(config.default_shutdown_timeout, Duration::from_secs(60));
        }

        #[test]
        fn test_store_config_new() {
            let retry_policy = RetryPolicy::new().with_max_attempts(5);
            let config = StoreConfig::new(
                2000,
                retry_policy.clone(),
                Duration::from_secs(45),
            );

            assert_eq!(config.dlq_max_size, 2000);
            assert_eq!(config.default_shutdown_timeout, Duration::from_secs(45));
        }

        #[tokio::test]
        async fn test_store_with_config_uses_custom_dlq_size() -> Result<(), StoreError> {
            let config = StoreConfig::default().with_dlq_max_size(50);

            let state = TestState { value: 0 };
            let store = Store::with_config(state, TestReducer, TestEnv, config);

            // Verify DLQ has correct capacity
            let dlq = store.dlq();
            assert_eq!(dlq.max_size(), 50);

            Ok(())
        }

        #[tokio::test]
        async fn test_store_with_config_uses_custom_retry_policy() -> Result<(), StoreError> {
            let custom_retry = RetryPolicy::new()
                .with_max_attempts(2)
                .with_initial_delay(Duration::from_millis(50));

            let config = StoreConfig::default().with_retry_policy(custom_retry);

            let state = TestState { value: 0 };
            let store = Store::with_config(state, TestReducer, TestEnv, config);

            // Verify store was created successfully with custom config
            // The retry policy is used internally by the store
            // This test confirms the store accepts and uses the configuration
            let _handle = store.send(TestAction::Increment).await?;

            Ok(())
        }

        #[tokio::test]
        async fn test_store_with_config_independent_instances() -> Result<(), StoreError> {
            // Create two stores with different configurations
            let config1 = StoreConfig::default().with_dlq_max_size(100);
            let config2 = StoreConfig::default().with_dlq_max_size(200);

            let store1 = Store::with_config(
                TestState { value: 0 },
                TestReducer,
                TestEnv,
                config1,
            );
            let store2 = Store::with_config(
                TestState { value: 0 },
                TestReducer,
                TestEnv,
                config2,
            );

            // Verify each has independent configuration
            assert_eq!(store1.dlq().max_size(), 100);
            assert_eq!(store2.dlq().max_size(), 200);

            Ok(())
        }

        #[test]
        fn test_store_config_clone() {
            let config1 = StoreConfig::default().with_dlq_max_size(300);
            let config2 = config1.clone();

            assert_eq!(config1.dlq_max_size, config2.dlq_max_size);
            assert_eq!(config1.default_shutdown_timeout, config2.default_shutdown_timeout);
        }

        #[test]
        fn test_store_config_chaining() {
            // Test that builder methods can be chained
            let config = StoreConfig::default()
                .with_dlq_max_size(400)
                .with_shutdown_timeout(Duration::from_secs(120))
                .with_retry_policy(RetryPolicy::new().with_max_attempts(3));

            assert_eq!(config.dlq_max_size, 400);
            assert_eq!(config.default_shutdown_timeout, Duration::from_secs(120));
        }
    }
}
