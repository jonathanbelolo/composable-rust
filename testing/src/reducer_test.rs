//! Ergonomic testing utilities for reducers
//!
//! This module provides a fluent API for testing reducers with readable Given-When-Then syntax.

#![allow(clippy::module_name_repetitions)] // ReducerTest is the natural name

use composable_rust_core::{effect::Effect, reducer::Reducer};

/// Type alias for state assertion functions
type StateAssertion<S> = Box<dyn FnOnce(&S)>;

/// Type alias for effect assertion functions
type EffectAssertion<A> = Box<dyn FnOnce(&[Effect<A>])>;

/// Fluent API for testing reducers with Given-When-Then syntax
///
/// # Example
///
/// ```ignore
/// use composable_rust_testing::ReducerTest;
///
/// ReducerTest::new(CounterReducer)
///     .with_env(test_environment())
///     .given_state(CounterState { count: 0 })
///     .when_action(CounterAction::Increment)
///     .then_state(|state| {
///         assert_eq!(state.count, 1);
///     })
///     .then_effects(|effects| {
///         assert_eq!(effects.len(), 1);
///     })
///     .run();
/// ```
pub struct ReducerTest<R, S, A, E>
where
    R: Reducer<State = S, Action = A, Environment = E>,
{
    reducer: R,
    environment: Option<E>,
    initial_state: Option<S>,
    action: Option<A>,
    state_assertions: Vec<StateAssertion<S>>,
    effect_assertions: Vec<EffectAssertion<A>>,
}

impl<R, S, A, E> ReducerTest<R, S, A, E>
where
    R: Reducer<State = S, Action = A, Environment = E>,
    S: Clone,
    A: Clone,
{
    /// Create a new reducer test with the given reducer
    #[must_use]
    pub const fn new(reducer: R) -> Self {
        Self {
            reducer,
            environment: None,
            initial_state: None,
            action: None,
            state_assertions: Vec::new(),
            effect_assertions: Vec::new(),
        }
    }

    /// Set the environment for the test
    #[must_use]
    pub fn with_env(mut self, env: E) -> Self {
        self.environment = Some(env);
        self
    }

    /// Set the initial state (Given)
    #[must_use]
    pub fn given_state(mut self, state: S) -> Self {
        self.initial_state = Some(state);
        self
    }

    /// Set the action to test (When)
    #[must_use]
    pub fn when_action(mut self, action: A) -> Self {
        self.action = Some(action);
        self
    }

    /// Add an assertion about the resulting state (Then)
    #[must_use]
    pub fn then_state<F>(mut self, assertion: F) -> Self
    where
        F: FnOnce(&S) + 'static,
    {
        self.state_assertions.push(Box::new(assertion));
        self
    }

    /// Add an assertion about the resulting effects (Then)
    #[must_use]
    pub fn then_effects<F>(mut self, assertion: F) -> Self
    where
        F: FnOnce(&[Effect<A>]) + 'static,
    {
        self.effect_assertions.push(Box::new(assertion));
        self
    }

    /// Run the test and execute all assertions
    ///
    /// # Panics
    ///
    /// Panics if initial state, action, or environment is not set,
    /// or if any assertions fail.
    #[allow(clippy::panic)] // Test code can panic
    #[allow(clippy::expect_used)] // Test code can use expect
    pub fn run(self) {
        let mut state = self
            .initial_state
            .expect("Initial state must be set with given_state()");

        let action = self.action.expect("Action must be set with when_action()");

        let env = self
            .environment
            .expect("Environment must be set with with_env()");

        // Execute reducer
        let effects = self.reducer.reduce(&mut state, action, &env);

        // Run state assertions
        for assertion in self.state_assertions {
            assertion(&state);
        }

        // Run effect assertions
        for assertion in self.effect_assertions {
            assertion(&effects);
        }
    }
}

/// Helper assertions for effects
pub mod assertions {
    use composable_rust_core::effect::Effect;

    /// Assert that there are no effects
    ///
    /// # Panics
    ///
    /// Panics if effects is not empty.
    #[allow(clippy::panic)] // Test assertion
    pub fn assert_no_effects<A: std::fmt::Debug>(effects: &[Effect<A>]) {
        assert!(
            effects.is_empty() || matches!(effects, [Effect::None]),
            "Expected no effects, but found {}: {:?}",
            effects.len(),
            effects
        );
    }

    /// Assert the number of effects
    ///
    /// # Panics
    ///
    /// Panics if the number of effects doesn't match expected.
    #[allow(clippy::panic)] // Test assertion
    pub fn assert_effects_count<A>(effects: &[Effect<A>], expected: usize) {
        assert_eq!(
            effects.len(),
            expected,
            "Expected {} effects, but found {}",
            expected,
            effects.len()
        );
    }

    /// Assert that effects contain at least one Future effect
    ///
    /// # Panics
    ///
    /// Panics if no Future effect is found.
    #[allow(clippy::panic)] // Test assertion
    pub fn assert_has_future_effect<A>(effects: &[Effect<A>]) {
        assert!(
            effects.iter().any(|e| matches!(e, Effect::Future(_))),
            "Expected at least one Future effect, but none found"
        );
    }

    /// Assert that effects contain at least one `EventStore` effect
    ///
    /// # Panics
    ///
    /// Panics if no `EventStore` effect is found.
    #[allow(clippy::panic)] // Test assertion
    pub fn assert_has_event_store_effect<A>(effects: &[Effect<A>]) {
        assert!(
            effects.iter().any(|e| matches!(e, Effect::EventStore(_))),
            "Expected at least one EventStore effect, but none found"
        );
    }

    /// Assert that effects contain at least one `PublishEvent` effect
    ///
    /// # Panics
    ///
    /// Panics if no `PublishEvent` effect is found.
    #[allow(clippy::panic)] // Test assertion
    pub fn assert_has_publish_event_effect<A>(effects: &[Effect<A>]) {
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::PublishEvent(_))),
            "Expected at least one PublishEvent effect, but none found"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_core::effect::Effect;
    use composable_rust_core::reducer::Reducer;

    #[derive(Clone, Debug)]
    struct TestState {
        count: i32,
    }

    #[derive(Clone, Debug)]
    enum TestAction {
        Increment,
        Decrement,
    }

    struct TestReducer;

    struct TestEnv;

    impl Reducer for TestReducer {
        type State = TestState;
        type Action = TestAction;
        type Environment = TestEnv;

        fn reduce(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            _env: &Self::Environment,
        ) -> smallvec::SmallVec<[Effect<Self::Action>; 4]> {
            match action {
                TestAction::Increment => {
                    state.count += 1;
                    smallvec::smallvec![Effect::None]
                }
                TestAction::Decrement => {
                    state.count -= 1;
                    smallvec::smallvec![Effect::None]
                }
            }
        }
    }

    #[test]
    fn test_reducer_test_increment() {
        ReducerTest::new(TestReducer)
            .with_env(TestEnv)
            .given_state(TestState { count: 0 })
            .when_action(TestAction::Increment)
            .then_state(|state| {
                assert_eq!(state.count, 1);
            })
            .then_effects(|effects| {
                assertions::assert_no_effects(effects);
            })
            .run();
    }

    #[test]
    fn test_reducer_test_decrement() {
        ReducerTest::new(TestReducer)
            .with_env(TestEnv)
            .given_state(TestState { count: 5 })
            .when_action(TestAction::Decrement)
            .then_state(|state| {
                assert_eq!(state.count, 4);
            })
            .run();
    }

    #[test]
    fn test_assertions_no_effects() {
        assertions::assert_no_effects::<TestAction>(&[Effect::None]);
        assertions::assert_no_effects::<TestAction>(&[]);
    }

    #[test]
    fn test_assertions_effects_count() {
        assertions::assert_effects_count(&[Effect::<TestAction>::None], 1);
        assertions::assert_effects_count::<TestAction>(&[], 0);
    }
}
