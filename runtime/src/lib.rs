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
    /// - max_attempts: 5
    /// - initial_delay: 1 second
    /// - max_delay: 32 seconds
    /// - backoff_multiplier: 2.0 (exponential)
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
    /// delay = min(initial_delay * multiplier^attempt, max_delay) * (0.5 + random(0.5))
    ///
    /// Jitter prevents thundering herd problem.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        use rand::Rng;

        // Calculate exponential backoff: initial * multiplier^attempt
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

pub use error::StoreError;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, Weak};
use std::time::Duration;
use tokio::sync::watch;

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
        Arc, AtomicUsize, DecrementGuard, Effect, EffectHandle, EffectTracking, Ordering, Reducer,
        RwLock, TrackingMode,
    };
    use tokio::sync::watch;

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
    }

    impl<S, A, E, R> Store<S, A, E, R>
    where
        R: Reducer<State = S, Action = A, Environment = E> + Send + Sync + 'static,
        A: Send + 'static,
        S: Send + Sync + 'static,
        E: Send + Sync + 'static,
    {
        /// Create a new store with initial state, reducer, and environment
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
            Self {
                state: Arc::new(RwLock::new(initial_state)),
                reducer,
                environment,
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
        /// # Error Handling
        ///
        /// If the reducer panics, the panic will propagate and halt the store.
        /// Reducers should be pure functions that do not panic.
        ///
        /// # Example
        ///
        /// ```ignore
        /// let handle = store.send(CounterAction::Increment).await;
        /// handle.wait().await;
        /// ```
        #[must_use]
        #[tracing::instrument(skip(self, action), name = "store_send")]
        pub async fn send(&self, action: A) -> EffectHandle
        where
            R: Clone,
            E: Clone,
            A: Clone,
        {
            self.send_internal(action, TrackingMode::Direct).await
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
        #[allow(clippy::cognitive_complexity)] // TODO: Refactor in Phase 4
        #[tracing::instrument(skip(self, action, tracking_mode), name = "store_send_internal")]
        async fn send_internal(&self, action: A, tracking_mode: TrackingMode) -> EffectHandle
        where
            R: Clone,
            E: Clone,
            A: Clone,
        {
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
                metrics::histogram!("store.effects.count").record(effects.len() as f64);

                effects
            };

            // Execute effects with tracking
            tracing::trace!("Executing {} effects", effects.len());
            for effect in effects {
                self.execute_effect_internal(effect, tracking.clone());
            }
            tracing::debug!("Action processing completed, returning handle");

            handle
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
                    let tracking_clone = tracking.clone();
                    let store = self.clone();

                    tokio::spawn(async move {
                        let _guard = DecrementGuard(tracking_clone.clone());
                        if let Some(action) = fut.await {
                            tracing::trace!("Effect::Future produced an action, sending to store");
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
                    let tracking_clone = tracking.clone();
                    let store = self.clone();

                    tokio::spawn(async move {
                        let _guard = DecrementGuard(tracking_clone.clone());
                        tokio::time::sleep(duration).await;
                        tracing::trace!("Effect::Delay completed, sending action");
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
                    let tracking_clone = tracking.clone();
                    let store = self.clone();

                    tokio::spawn(async move {
                        let _guard = DecrementGuard(tracking_clone.clone());

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
                    let tracking_clone = tracking.clone();
                    let store = self.clone();

                    tokio::spawn(async move {
                        let _guard = DecrementGuard(tracking_clone.clone());

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
                                match event_store
                                    .append_events(stream_id, expected_version, events)
                                    .await
                                {
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
                                match event_store.load_events(stream_id, from_version).await {
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
                                match event_store.save_snapshot(stream_id, version, state).await {
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
                                match event_store.load_snapshot(stream_id).await {
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
                                match event_bus.publish(&topic, &event).await {
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
    use composable_rust_core::{effect::Effect, reducer::Reducer};
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
        ) -> Vec<Effect<Self::Action>> {
            match action {
                TestAction::Increment => {
                    state.value += 1;
                    vec![Effect::None]
                },
                TestAction::Decrement => {
                    state.value -= 1;
                    vec![Effect::None]
                },
                TestAction::NoOp => vec![Effect::None],
                TestAction::ProduceEffect => {
                    // Return an effect that produces another action
                    vec![Effect::Future(Box::pin(async {
                        Some(TestAction::Increment)
                    }))]
                },
                TestAction::ProduceDelayedAction => {
                    // Return a delayed effect
                    vec![Effect::Delay {
                        duration: Duration::from_millis(10),
                        action: Box::new(TestAction::Increment),
                    }]
                },
                TestAction::ProduceParallelEffects => {
                    // Return parallel effects that each produce an increment
                    vec![Effect::Parallel(vec![
                        Effect::Future(Box::pin(async { Some(TestAction::Increment) })),
                        Effect::Future(Box::pin(async { Some(TestAction::Increment) })),
                        Effect::Future(Box::pin(async { Some(TestAction::Increment) })),
                    ])]
                },
                TestAction::ProduceSequentialEffects => {
                    // Return sequential effects: increment, increment, decrement
                    vec![Effect::Sequential(vec![
                        Effect::Future(Box::pin(async { Some(TestAction::Increment) })),
                        Effect::Future(Box::pin(async { Some(TestAction::Increment) })),
                        Effect::Future(Box::pin(async { Some(TestAction::Decrement) })),
                    ])]
                },
                TestAction::ProducePanickingEffect => {
                    // Return an effect that will panic when executed
                    #[allow(clippy::panic)] // Intentional panic for testing error handling
                    {
                        vec![Effect::Future(Box::pin(async {
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
    async fn test_effect_panic_isolation() {
        // Test that a panic in an effect doesn't crash the Store
        // This verifies our error handling strategy: effects fail gracefully
        let state = TestState { value: 0 };
        let store = Store::new(state, TestReducer, TestEnv);

        // This action produces an effect that will panic
        let mut handle = store.send(TestAction::ProducePanickingEffect).await;

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
    }

    // EventStore effect tests
    mod event_store_tests {
        use super::*;
        use composable_rust_core::effect::{Effect, EventStoreOperation};
        use composable_rust_core::event::SerializedEvent;
        use composable_rust_core::event_store::EventStore;
        use composable_rust_core::stream::{StreamId, Version};
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
            ) -> Vec<Effect<Self::Action>> {
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

                        vec![Effect::EventStore(EventStoreOperation::AppendEvents {
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
                        vec![Effect::None]
                    },
                    EventStoreAction::AppendFailed { error }
                    | EventStoreAction::LoadFailed { error } => {
                        state.error = Some(error);
                        vec![Effect::None]
                    },
                    EventStoreAction::LoadEvents { stream_id } => {
                        vec![Effect::EventStore(EventStoreOperation::LoadEvents {
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
                        vec![Effect::None]
                    },
                    EventStoreAction::SaveSnapshot { stream_id, version } => {
                        let state_bytes = vec![1, 2, 3, 4]; // Mock state data
                        vec![Effect::EventStore(EventStoreOperation::SaveSnapshot {
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
                        vec![Effect::None]
                    },
                    EventStoreAction::LoadSnapshot { stream_id } => {
                        vec![Effect::EventStore(EventStoreOperation::LoadSnapshot {
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
                        vec![Effect::None]
                    },
                }
            }
        }

        #[tokio::test]
        async fn test_eventstore_append_success() {
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
                .await;

            handle.wait().await;

            // Check state was updated with version
            let last_version = store.state(|s| s.last_version).await;
            assert_eq!(last_version, Some(1)); // 2 events, version 0-1, returns last = 1
        }

        #[tokio::test]
        async fn test_eventstore_append_concurrency_conflict() {
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
                .await;

            handle.wait().await;

            // Check error was captured
            let error = store.state(|s| s.error.clone()).await;
            assert!(error.is_some());
            #[allow(clippy::unwrap_used)] // Panics: Test verified error is Some above
            {
                assert!(error.unwrap().contains("Concurrency"));
            }
        }

        #[tokio::test]
        async fn test_eventstore_load_events() {
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
                .await;

            handle.wait().await;

            // Check count was updated
            let count = store.state(|s| s.event_count).await;
            assert_eq!(count, 3);
        }

        #[tokio::test]
        async fn test_eventstore_snapshot_roundtrip() {
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
                .await;

            handle.wait().await;

            // Check snapshot was saved
            let saved = store.state(|s| s.snapshot_saved).await;
            assert!(saved);

            // Load snapshot
            let mut handle = store
                .send(EventStoreAction::LoadSnapshot {
                    stream_id: "test-stream".to_string(),
                })
                .await;

            handle.wait().await;

            // Check snapshot was loaded
            let loaded = store.state(|s| s.snapshot_loaded).await;
            assert!(loaded);
        }

        #[tokio::test]
        async fn test_eventstore_parallel_operations() {
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
                .await;

            // Append to stream2 (different stream, can run concurrently)
            let mut h2 = store
                .send(EventStoreAction::AppendEvents {
                    stream_id: "stream-2".to_string(),
                    events: vec!["event2".to_string()],
                })
                .await;

            // Wait for both
            h1.wait().await;
            h2.wait().await;

            // Both should have succeeded (last_version reflects last operation)
            let last_version = store.state(|s| s.last_version).await;
            assert!(last_version.is_some());
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
            ) -> Vec<Effect<Self::Action>> {
                // Create parallel effects on each call
                vec![Effect::Parallel(vec![
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
        async fn test_eventstore_effect_in_parallel_composition() {
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
                .await;

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
        }
    }

    /// Tests for RetryPolicy
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
}
