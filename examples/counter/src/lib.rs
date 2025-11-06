//! # Counter Example
//!
//! A simple counter demonstrating the Composable Rust architecture.
//!
//! This example showcases:
//! - Pure state machine (no side effects)
//! - Basic reducer implementation
//! - Store usage
//! - State queries
//!
//! ## Architecture
//!
//! The Counter is a **pure state machine** with NO side effects:
//! - All effects return `Effect::None`
//! - State changes are synchronous and deterministic
//! - Perfect for understanding the core abstractions
//!
//! ## Example
//!
//! ```no_run
//! use counter::{CounterState, CounterAction, CounterReducer, CounterEnvironment};
//! use composable_rust_runtime::Store;
//! use composable_rust_testing::test_clock;
//!
//! # async fn example() {
//! let env = CounterEnvironment::new(test_clock());
//! let store = Store::new(CounterState::default(), CounterReducer::new(), env);
//!
//! store.send(CounterAction::Increment).await;
//! let count = store.state(|s| s.count).await;
//! assert_eq!(count, 1);
//! # }
//! ```

use composable_rust_core::{effect::Effect, environment::Clock, reducer::Reducer, smallvec, SmallVec};

/// Counter state
///
/// The state is just a simple count. In a real application, this might
/// contain more complex domain data.
#[derive(Debug, Clone, Default)]
pub struct CounterState {
    /// Current count value
    pub count: i64,
}

/// Counter actions
///
/// These are the events that can happen to the counter.
/// Each action will be processed by the reducer.
#[derive(Debug, Clone)]
pub enum CounterAction {
    /// Increment the counter by 1
    Increment,
    /// Decrement the counter by 1
    Decrement,
    /// Reset the counter to 0
    Reset,
}

/// Counter environment
///
/// This demonstrates dependency injection. The clock is included
/// for demonstration purposes but not actually used since the
/// Counter is a pure state machine.
///
/// In more complex examples (Phase 2+), the environment would
/// include things like database connections, HTTP clients, etc.
#[derive(Debug, Clone)]
pub struct CounterEnvironment<C: Clock> {
    /// Clock for time-based operations (demonstration only)
    pub clock: C,
}

impl<C: Clock> CounterEnvironment<C> {
    /// Create a new counter environment with the given clock
    #[must_use]
    pub const fn new(clock: C) -> Self {
        Self { clock }
    }
}

/// Counter reducer
///
/// Implements the business logic for the counter.
/// This is a pure function that takes the current state and an action,
/// and returns the new state and effects.
///
/// Since Counter is a pure state machine, all effects are `Effect::None`.
///
/// Generic over the Clock type C to work with any clock implementation.
#[derive(Debug, Clone, Copy)]
pub struct CounterReducer<C> {
    _phantom: std::marker::PhantomData<C>,
}

impl<C> CounterReducer<C> {
    /// Create a new counter reducer
    #[must_use]
    pub const fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<C> Default for CounterReducer<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: Clock> Reducer for CounterReducer<C> {
    type State = CounterState;
    type Action = CounterAction;
    type Environment = CounterEnvironment<C>;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _environment: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            CounterAction::Increment => {
                state.count += 1;
            },
            CounterAction::Decrement => {
                state.count -= 1;
            },
            CounterAction::Reset => {
                state.count = 0;
            },
        }

        // Pure state machine - no side effects
        smallvec![Effect::None]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_testing::test_clock;

    #[test]
    fn test_increment() {
        let mut state = CounterState::default();
        let env = CounterEnvironment::new(test_clock());
        let reducer = CounterReducer::new();

        let effects = reducer.reduce(&mut state, CounterAction::Increment, &env);

        assert_eq!(state.count, 1);
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_decrement() {
        let mut state = CounterState { count: 5 };
        let env = CounterEnvironment::new(test_clock());
        let reducer = CounterReducer::new();

        let effects = reducer.reduce(&mut state, CounterAction::Decrement, &env);

        assert_eq!(state.count, 4);
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_reset() {
        let mut state = CounterState { count: 42 };
        let env = CounterEnvironment::new(test_clock());
        let reducer = CounterReducer::new();

        let effects = reducer.reduce(&mut state, CounterAction::Reset, &env);

        assert_eq!(state.count, 0);
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_multiple_operations() {
        let mut state = CounterState::default();
        let env = CounterEnvironment::new(test_clock());
        let reducer = CounterReducer::new();

        // Increment twice
        reducer.reduce(&mut state, CounterAction::Increment, &env);
        reducer.reduce(&mut state, CounterAction::Increment, &env);
        assert_eq!(state.count, 2);

        // Decrement once
        reducer.reduce(&mut state, CounterAction::Decrement, &env);
        assert_eq!(state.count, 1);

        // Reset
        reducer.reduce(&mut state, CounterAction::Reset, &env);
        assert_eq!(state.count, 0);
    }
}
