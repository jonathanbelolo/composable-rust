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
        async fn send_internal(&self, action: A, tracking_mode: TrackingMode) -> EffectHandle
        where
            R: Clone,
            E: Clone,
            A: Clone,
        {
            tracing::debug!("Processing action");

            // Create tracking for this action
            let (handle, tracking) = EffectHandle::new::<A>(tracking_mode);

            let effects = {
                let mut state = self.state.write().await;
                tracing::trace!("Acquired write lock on state");
                let effects = self.reducer.reduce(&mut *state, action, &self.environment);
                tracing::trace!("Reducer completed, returned {} effects", effects.len());
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
        fn execute_effect_internal(&self, effect: Effect<A>, tracking: EffectTracking<A>)
        where
            R: Clone,
            E: Clone,
            A: Clone + Send + 'static,
        {
            match effect {
                Effect::None => {
                    tracing::trace!("Executing Effect::None (no-op)");
                },
                Effect::Future(fut) => {
                    tracing::trace!("Executing Effect::Future");
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

                    // Execute all effects concurrently, each with the same tracking
                    let store = self.clone();
                    for effect in effects {
                        store.execute_effect_internal(effect, tracking.clone());
                    }
                },
                Effect::Sequential(effects) => {
                    let effect_count = effects.len();
                    tracing::trace!("Executing Effect::Sequential with {} effects", effect_count);

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
                                match event_store.append_events(stream_id, expected_version, events).await {
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
                                        tracing::debug!(event_count = events.len(), "load_events succeeded");
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
                                        tracing::debug!(has_snapshot = snapshot.is_some(), "load_snapshot succeeded");
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
                            tracing::trace!("EventStore operation produced an action, sending to store");
                            let _ = store.send(action).await;
                        } else {
                            tracing::trace!("EventStore operation completed with no action");
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
}
