//! Reducer logic for the Todo aggregate.
//!
//! This demonstrates the simplest possible reducer: validate commands,
//! produce events, and apply events to update state.

use crate::types::{TodoAction, TodoId, TodoItem, TodoState};
use composable_rust_core::{
    effect::Effect, environment::Clock, reducer::Reducer, SmallVec,
};

/// Environment dependencies for the Todo reducer
#[derive(Clone)]
pub struct TodoEnvironment {
    /// Clock for generating timestamps
    pub clock: std::sync::Arc<dyn Clock>,
}

impl TodoEnvironment {
    /// Creates a new `TodoEnvironment`
    #[must_use]
    pub fn new(clock: std::sync::Arc<dyn Clock>) -> Self {
        Self { clock }
    }
}

/// Reducer for the Todo aggregate
#[derive(Clone, Debug)]
pub struct TodoReducer;

impl TodoReducer {
    /// Creates a new `TodoReducer`
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Validates a `CreateTodo` command
    fn validate_create_todo(state: &TodoState, id: &TodoId, title: &str) -> Result<(), String> {
        if state.exists(id) {
            return Err(format!("Todo with ID {id} already exists"));
        }

        if title.trim().is_empty() {
            return Err("Todo title cannot be empty".to_string());
        }

        if title.len() > 500 {
            return Err("Todo title too long (max 500 characters)".to_string());
        }

        Ok(())
    }

    /// Validates a `CompleteTodo` command
    fn validate_complete_todo(state: &TodoState, id: &TodoId) -> Result<(), String> {
        let Some(todo) = state.get(id) else {
            return Err(format!("Todo with ID {id} not found"));
        };

        if todo.completed {
            return Err(format!("Todo {id} is already completed"));
        }

        Ok(())
    }

    /// Validates a `DeleteTodo` command
    fn validate_delete_todo(state: &TodoState, id: &TodoId) -> Result<(), String> {
        if !state.exists(id) {
            return Err(format!("Todo with ID {id} not found"));
        }

        Ok(())
    }

    /// Applies an event to state
    fn apply_event(state: &mut TodoState, action: &TodoAction) {
        match action {
            TodoAction::TodoCreated {
                id,
                title,
                created_at,
            } => {
                let item = TodoItem::new(id.clone(), title.clone(), *created_at);
                state.todos.insert(id.clone(), item);
                state.last_error = None;
            }
            TodoAction::TodoCompleted { id, completed_at } => {
                if let Some(todo) = state.todos.get_mut(id) {
                    todo.complete(*completed_at);
                }
                state.last_error = None;
            }
            TodoAction::TodoDeleted { id } => {
                state.todos.remove(id);
                state.last_error = None;
            }
            TodoAction::ValidationFailed { error } => {
                state.last_error = Some(error.clone());
            }
            // Commands are not applied to state
            TodoAction::CreateTodo { .. }
            | TodoAction::CompleteTodo { .. }
            | TodoAction::DeleteTodo { .. } => {}
        }
    }
}

impl Default for TodoReducer {
    fn default() -> Self {
        Self::new()
    }
}

impl Reducer for TodoReducer {
    type State = TodoState;
    type Action = TodoAction;
    type Environment = TodoEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ========== Commands ==========
            TodoAction::CreateTodo { id, title } => {
                // Validate command
                if let Err(error) = Self::validate_create_todo(state, &id, &title) {
                    Self::apply_event(
                        state,
                        &TodoAction::ValidationFailed {
                            error: error.clone(),
                        },
                    );
                    return SmallVec::new();
                }

                // Create event
                let event = TodoAction::TodoCreated {
                    id,
                    title,
                    created_at: env.clock.now(),
                };

                // Apply event to state
                Self::apply_event(state, &event);

                SmallVec::new()
            }

            TodoAction::CompleteTodo { id } => {
                // Validate command
                if let Err(error) = Self::validate_complete_todo(state, &id) {
                    Self::apply_event(
                        state,
                        &TodoAction::ValidationFailed {
                            error: error.clone(),
                        },
                    );
                    return SmallVec::new();
                }

                // Create event
                let event = TodoAction::TodoCompleted {
                    id,
                    completed_at: env.clock.now(),
                };

                // Apply event to state
                Self::apply_event(state, &event);

                SmallVec::new()
            }

            TodoAction::DeleteTodo { id } => {
                // Validate command
                if let Err(error) = Self::validate_delete_todo(state, &id) {
                    Self::apply_event(
                        state,
                        &TodoAction::ValidationFailed {
                            error: error.clone(),
                        },
                    );
                    return SmallVec::new();
                }

                // Create event
                let event = TodoAction::TodoDeleted { id };

                // Apply event to state
                Self::apply_event(state, &event);

                SmallVec::new()
            }

            // ========== Events ==========
            TodoAction::TodoCreated { .. }
            | TodoAction::TodoCompleted { .. }
            | TodoAction::TodoDeleted { .. }
            | TodoAction::ValidationFailed { .. } => {
                // Events have already been applied during command processing
                // or are being replayed from event store
                Self::apply_event(state, &action);
                SmallVec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use composable_rust_core::environment::SystemClock;
    use composable_rust_testing::{assertions, ReducerTest};
    use std::sync::Arc;

    fn create_test_env() -> TodoEnvironment {
        TodoEnvironment::new(Arc::new(SystemClock))
    }

    #[test]
    fn test_create_todo_success() {
        let id = TodoId::new();

        ReducerTest::new(TodoReducer::new())
            .with_env(create_test_env())
            .given_state(TodoState::new())
            .when_action(TodoAction::CreateTodo {
                id: id.clone(),
                title: "Buy milk".to_string(),
            })
            .then_state(move |state| {
                assert_eq!(state.count(), 1);
                assert!(state.exists(&id));
                let todo = state.get(&id).unwrap();
                assert_eq!(todo.title, "Buy milk");
                assert!(!todo.completed);
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_create_todo_duplicate_id() {
        let id = TodoId::new();

        ReducerTest::new(TodoReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = TodoState::new();
                let item = TodoItem::new(id.clone(), "Existing".to_string(), Utc::now());
                state.todos.insert(id.clone(), item);
                state
            })
            .when_action(TodoAction::CreateTodo {
                id: id.clone(),
                title: "Duplicate".to_string(),
            })
            .then_state(|state| {
                assert_eq!(state.count(), 1); // Still only one todo
                assert!(state.last_error.is_some());
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("already exists"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_create_todo_empty_title() {
        ReducerTest::new(TodoReducer::new())
            .with_env(create_test_env())
            .given_state(TodoState::new())
            .when_action(TodoAction::CreateTodo {
                id: TodoId::new(),
                title: "   ".to_string(), // Empty after trim
            })
            .then_state(|state| {
                assert_eq!(state.count(), 0);
                assert!(state.last_error.is_some());
                assert!(state.last_error.as_ref().unwrap().contains("cannot be empty"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_complete_todo_success() {
        let id = TodoId::new();

        ReducerTest::new(TodoReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = TodoState::new();
                let item = TodoItem::new(id.clone(), "Buy milk".to_string(), Utc::now());
                state.todos.insert(id.clone(), item);
                state
            })
            .when_action(TodoAction::CompleteTodo { id: id.clone() })
            .then_state(move |state| {
                let todo = state.get(&id).unwrap();
                assert!(todo.completed);
                assert!(todo.completed_at.is_some());
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_complete_todo_not_found() {
        let id = TodoId::new();

        ReducerTest::new(TodoReducer::new())
            .with_env(create_test_env())
            .given_state(TodoState::new())
            .when_action(TodoAction::CompleteTodo { id })
            .then_state(|state| {
                assert!(state.last_error.is_some());
                assert!(state.last_error.as_ref().unwrap().contains("not found"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_delete_todo_success() {
        let id = TodoId::new();

        ReducerTest::new(TodoReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = TodoState::new();
                let item = TodoItem::new(id.clone(), "Buy milk".to_string(), Utc::now());
                state.todos.insert(id.clone(), item);
                state
            })
            .when_action(TodoAction::DeleteTodo { id: id.clone() })
            .then_state(move |state| {
                assert_eq!(state.count(), 0);
                assert!(!state.exists(&id));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_event_application() {
        let id = TodoId::new();
        let now = Utc::now();

        ReducerTest::new(TodoReducer::new())
            .with_env(create_test_env())
            .given_state(TodoState::new())
            .when_action(TodoAction::TodoCreated {
                id: id.clone(),
                title: "Test".to_string(),
                created_at: now,
            })
            .then_state(move |state| {
                assert_eq!(state.count(), 1);
                let todo = state.get(&id).unwrap();
                assert_eq!(todo.title, "Test");
                assert_eq!(todo.created_at, now);
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }
}
