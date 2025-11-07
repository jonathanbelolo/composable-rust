//! Reducer composition utilities
//!
//! This module provides utilities for composing reducers in various ways:
//! - **`combine_reducers`**: Run multiple reducers on the same state/action
//! - **`scope_reducer`**: Focus a reducer on a subset of state
//!
//! # Examples
//!
//! ## Combining Reducers
//!
//! ```
//! use composable_rust_core::{Reducer, Effect};
//!
//! #[derive(Clone)]
//! struct MyState {
//!     count: i32,
//!     name: String,
//! }
//!
//! #[derive(Clone)]
//! enum MyAction {
//!     Increment,
//!     SetName(String),
//! }
//!
//! struct CounterReducer;
//! struct NameReducer;
//!
//! impl Reducer for CounterReducer {
//!     type State = MyState;
//!     type Action = MyAction;
//!     type Environment = ();
//!
//!     fn reduce(&self, state: &mut Self::State, action: Self::Action, _env: &Self::Environment) -> Vec<Effect<Self::Action>> {
//!         match action {
//!             MyAction::Increment => {
//!                 state.count += 1;
//!                 vec![Effect::None]
//!             }
//!             _ => vec![Effect::None],
//!         }
//!     }
//! }
//!
//! impl Reducer for NameReducer {
//!     type State = MyState;
//!     type Action = MyAction;
//!     type Environment = ();
//!
//!     fn reduce(&self, state: &mut Self::State, action: Self::Action, _env: &Self::Environment) -> Vec<Effect<Self::Action>> {
//!         match action {
//!             MyAction::SetName(name) => {
//!                 state.name = name;
//!                 vec![Effect::None]
//!             }
//!             _ => vec![Effect::None],
//!         }
//!     }
//! }
//!
//! // Combine both reducers
//! use composable_rust_core::composition::combine_reducers;
//! let combined = combine_reducers(vec![Box::new(CounterReducer), Box::new(NameReducer)]);
//! ```

use crate::effect::Effect;
use crate::reducer::Reducer;

/// Combines multiple reducers that operate on the same state and action types.
///
/// Each reducer is run in sequence, and all effects are collected and concatenated.
/// This is useful when you want to split reducer logic across multiple implementations.
///
/// # Type Parameters
///
/// - `S`: The state type
/// - `A`: The action type
/// - `E`: The environment type
///
/// # Examples
///
/// ```
/// use composable_rust_core::{Reducer, Effect};
/// use composable_rust_core::composition::combine_reducers;
///
/// #[derive(Clone)]
/// struct AppState {
///     counter: i32,
///     logged: bool,
/// }
///
/// #[derive(Clone)]
/// enum AppAction {
///     Increment,
///     Log,
/// }
///
/// struct CounterReducer;
/// struct LoggingReducer;
///
/// impl Reducer for CounterReducer {
///     type State = AppState;
///     type Action = AppAction;
///     type Environment = ();
///
///     fn reduce(&self, state: &mut Self::State, action: Self::Action, _env: &Self::Environment) -> Vec<Effect<Self::Action>> {
///         if matches!(action, AppAction::Increment) {
///             state.counter += 1;
///         }
///         vec![Effect::None]
///     }
/// }
///
/// impl Reducer for LoggingReducer {
///     type State = AppState;
///     type Action = AppAction;
///     type Environment = ();
///
///     fn reduce(&self, state: &mut Self::State, action: Self::Action, _env: &Self::Environment) -> Vec<Effect<Self::Action>> {
///         if matches!(action, AppAction::Log) {
///             state.logged = true;
///         }
///         vec![Effect::None]
///     }
/// }
///
/// let combined = combine_reducers(vec![Box::new(CounterReducer), Box::new(LoggingReducer)]);
///
/// let mut state = AppState { counter: 0, logged: false };
/// let effects = combined.reduce(&mut state, AppAction::Increment, &());
/// assert_eq!(state.counter, 1);
/// ```
#[must_use]
pub fn combine_reducers<S, A, E>(
    reducers: Vec<Box<dyn Reducer<State = S, Action = A, Environment = E>>>,
) -> CombinedReducer<S, A, E>
where
    S: 'static,
    A: Clone + 'static,
    E: 'static,
{
    CombinedReducer { reducers }
}

/// A combined reducer that runs multiple reducers in sequence.
///
/// Created by [`combine_reducers`].
pub struct CombinedReducer<S, A, E>
where
    S: 'static,
    A: Clone + 'static,
    E: 'static,
{
    reducers: Vec<Box<dyn Reducer<State = S, Action = A, Environment = E>>>,
}

impl<S, A, E> Reducer for CombinedReducer<S, A, E>
where
    S: 'static,
    A: Clone + 'static,
    E: 'static,
{
    type State = S;
    type Action = A;
    type Environment = E;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> smallvec::SmallVec<[Effect<Self::Action>; 4]> {
        let mut all_effects = smallvec::SmallVec::new();

        for reducer in &self.reducers {
            let effects = reducer.reduce(state, action.clone(), env);
            all_effects.extend(effects);
        }

        all_effects
    }
}

/// Scopes a reducer to operate on a subset of a larger state.
///
/// This allows you to reuse reducers designed for smaller state types
/// within a larger application state.
///
/// # Type Parameters
///
/// - `S`: The parent state type
/// - `SubS`: The child state type (subset of `S`)
/// - `A`: The action type
/// - `E`: The environment type
///
/// # Examples
///
/// ```
/// use composable_rust_core::{Reducer, Effect};
/// use composable_rust_core::composition::scope_reducer;
///
/// // Child state and reducer
/// #[derive(Clone, Default)]
/// struct CounterState {
///     count: i32,
/// }
///
/// #[derive(Clone)]
/// enum CounterAction {
///     Increment,
///     Decrement,
/// }
///
/// struct CounterReducer;
///
/// impl Reducer for CounterReducer {
///     type State = CounterState;
///     type Action = CounterAction;
///     type Environment = ();
///
///     fn reduce(&self, state: &mut Self::State, action: Self::Action, _env: &Self::Environment) -> Vec<Effect<Self::Action>> {
///         match action {
///             CounterAction::Increment => state.count += 1,
///             CounterAction::Decrement => state.count -= 1,
///         }
///         vec![Effect::None]
///     }
/// }
///
/// // Parent state
/// #[derive(Clone, Default)]
/// struct AppState {
///     counter: CounterState,
///     other_data: String,
/// }
///
/// // Scope the counter reducer to work with AppState
/// let scoped = scope_reducer(
///     CounterReducer,
///     |app_state: &AppState| &app_state.counter,
///     |app_state: &mut AppState, counter: CounterState| {
///         app_state.counter = counter;
///     },
/// );
///
/// let mut state = AppState::default();
/// let effects = scoped.reduce(&mut state, CounterAction::Increment, &());
/// assert_eq!(state.counter.count, 1);
/// ```
pub fn scope_reducer<S, SubS, A, E, R>(
    reducer: R,
    get_state: fn(&S) -> &SubS,
    set_state: fn(&mut S, SubS),
) -> ScopedReducer<S, SubS, A, E, R>
where
    S: 'static,
    SubS: Clone + 'static,
    A: 'static,
    E: 'static,
    R: Reducer<State = SubS, Action = A, Environment = E>,
{
    ScopedReducer {
        reducer,
        get_state,
        set_state,
        _phantom: std::marker::PhantomData,
    }
}

/// A scoped reducer that operates on a subset of state.
///
/// Created by [`scope_reducer`].
pub struct ScopedReducer<S, SubS, A, E, R>
where
    S: 'static,
    SubS: Clone + 'static,
    A: 'static,
    E: 'static,
    R: Reducer<State = SubS, Action = A, Environment = E>,
{
    reducer: R,
    get_state: fn(&S) -> &SubS,
    set_state: fn(&mut S, SubS),
    _phantom: std::marker::PhantomData<(A, E)>,
}

impl<S, SubS, A, E, R> Reducer for ScopedReducer<S, SubS, A, E, R>
where
    S: 'static,
    SubS: Clone + 'static,
    A: 'static,
    E: 'static,
    R: Reducer<State = SubS, Action = A, Environment = E>,
{
    type State = S;
    type Action = A;
    type Environment = E;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> smallvec::SmallVec<[Effect<Self::Action>; 4]> {
        // Extract the sub-state
        let sub_state = (self.get_state)(state).clone();

        // Create a mutable copy
        let mut mutable_sub_state = sub_state;

        // Run the reducer on the sub-state
        let effects = self.reducer.reduce(&mut mutable_sub_state, action, env);

        // Write the updated sub-state back
        (self.set_state)(state, mutable_sub_state);

        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{smallvec, SmallVec};

    #[derive(Clone, Default)]
    struct TestState {
        counter: i32,
        name: String,
    }

    #[derive(Clone)]
    enum TestAction {
        Increment,
        Decrement,
        SetName(String),
    }

    struct CounterReducer;

    impl Reducer for CounterReducer {
        type State = TestState;
        type Action = TestAction;
        type Environment = ();

        fn reduce(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            _env: &Self::Environment,
        ) -> SmallVec<[Effect<Self::Action>; 4]> {
            match action {
                TestAction::Increment => {
                    state.counter += 1;
                    smallvec![Effect::None]
                },
                TestAction::Decrement => {
                    state.counter -= 1;
                    smallvec![Effect::None]
                },
                TestAction::SetName(_) => smallvec![Effect::None],
            }
        }
    }

    struct NameReducer;

    impl Reducer for NameReducer {
        type State = TestState;
        type Action = TestAction;
        type Environment = ();

        fn reduce(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            _env: &Self::Environment,
        ) -> SmallVec<[Effect<Self::Action>; 4]> {
            if let TestAction::SetName(name) = action {
                state.name = name;
            }
            smallvec![Effect::None]
        }
    }

    #[test]
    fn test_combine_reducers() {
        let combined = combine_reducers(vec![Box::new(CounterReducer), Box::new(NameReducer)]);

        let mut state = TestState::default();

        // Test counter reducer
        let _ = combined.reduce(&mut state, TestAction::Increment, &());
        assert_eq!(state.counter, 1);

        // Test name reducer
        let _ = combined.reduce(&mut state, TestAction::SetName("Alice".to_string()), &());
        assert_eq!(state.name, "Alice");

        // Both reducers work
        let _ = combined.reduce(&mut state, TestAction::Decrement, &());
        assert_eq!(state.counter, 0);
        assert_eq!(state.name, "Alice");
    }

    // Scoped reducer tests
    #[derive(Clone, Default)]
    struct SubState {
        value: i32,
    }

    #[derive(Clone)]
    enum SubAction {
        Add(i32),
        Multiply(i32),
    }

    struct SubReducer;

    impl Reducer for SubReducer {
        type State = SubState;
        type Action = SubAction;
        type Environment = ();

        fn reduce(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            _env: &Self::Environment,
        ) -> SmallVec<[Effect<Self::Action>; 4]> {
            match action {
                SubAction::Add(n) => {
                    state.value += n;
                    smallvec![Effect::None]
                },
                SubAction::Multiply(n) => {
                    state.value *= n;
                    smallvec![Effect::None]
                },
            }
        }
    }

    #[derive(Clone, Default)]
    struct ParentState {
        sub: SubState,
        other: String,
    }

    #[test]
    fn test_scope_reducer() {
        let scoped = scope_reducer(
            SubReducer,
            |parent: &ParentState| &parent.sub,
            |parent: &mut ParentState, sub: SubState| {
                parent.sub = sub;
            },
        );

        let mut state = ParentState {
            sub: SubState { value: 5 },
            other: "test".to_string(),
        };

        // Test scoped operations
        let _ = scoped.reduce(&mut state, SubAction::Add(3), &());
        assert_eq!(state.sub.value, 8);
        assert_eq!(state.other, "test"); // Other state unchanged

        let _ = scoped.reduce(&mut state, SubAction::Multiply(2), &());
        assert_eq!(state.sub.value, 16);
        assert_eq!(state.other, "test");
    }
}
