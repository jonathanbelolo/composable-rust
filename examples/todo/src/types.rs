//! Domain types for the Todo example.
//!
//! This module demonstrates the simplest possible event-sourced aggregate using
//! Composable Rust. A todo list is just a collection of todo items that can be
//! created, completed, and deleted.

use chrono::{DateTime, Utc};
use composable_rust_macros::{Action, State};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Unique identifier for a todo item
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TodoId(Uuid);

impl TodoId {
    /// Creates a new random `TodoId`
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Creates a `TodoId` from a UUID
    #[must_use]
    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// Returns the inner UUID
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for TodoId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TodoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A single todo item
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    /// Unique identifier
    pub id: TodoId,
    /// Title/description of the todo
    pub title: String,
    /// Whether the todo is completed
    pub completed: bool,
    /// When the todo was created
    pub created_at: DateTime<Utc>,
    /// When the todo was completed (if completed)
    pub completed_at: Option<DateTime<Utc>>,
}

impl TodoItem {
    /// Creates a new todo item
    #[must_use]
    pub const fn new(id: TodoId, title: String, created_at: DateTime<Utc>) -> Self {
        Self {
            id,
            title,
            completed: false,
            created_at,
            completed_at: None,
        }
    }

    /// Marks the todo as completed
    pub fn complete(&mut self, completed_at: DateTime<Utc>) {
        self.completed = true;
        self.completed_at = Some(completed_at);
    }
}

/// State of the todo list aggregate
///
/// This represents the current state of all todos, derived from replaying events.
#[derive(State, Clone, Debug, Default, Serialize, Deserialize)]
pub struct TodoState {
    /// All todos indexed by ID
    pub todos: HashMap<TodoId, TodoItem>,
    /// Last validation error (if any)
    pub last_error: Option<String>,
}

impl TodoState {
    /// Creates a new empty todo state
    #[must_use]
    pub fn new() -> Self {
        Self {
            todos: HashMap::new(),
            last_error: None,
        }
    }

    /// Returns the number of todos
    #[must_use]
    pub fn count(&self) -> usize {
        self.todos.len()
    }

    /// Returns the number of completed todos
    #[must_use]
    pub fn completed_count(&self) -> usize {
        self.todos.values().filter(|t| t.completed).count()
    }

    /// Returns a todo by ID
    #[must_use]
    pub fn get(&self, id: &TodoId) -> Option<&TodoItem> {
        self.todos.get(id)
    }

    /// Checks if a todo exists
    #[must_use]
    pub fn exists(&self, id: &TodoId) -> bool {
        self.todos.contains_key(id)
    }
}

/// Actions representing commands and events for todos
///
/// This enum combines both commands (intent to do something) and events
/// (something that happened). Commands are validated by the reducer and
/// produce events. Events are persisted and used to reconstruct state.
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum TodoAction {
    // ========== Commands ==========
    /// Command: Create a new todo
    #[command]
    CreateTodo {
        /// Todo identifier
        id: TodoId,
        /// Title of the todo
        title: String,
    },

    /// Command: Mark a todo as completed
    #[command]
    CompleteTodo {
        /// Todo to complete
        id: TodoId,
    },

    /// Command: Delete a todo
    #[command]
    DeleteTodo {
        /// Todo to delete
        id: TodoId,
    },

    // ========== Events ==========
    /// Event: Todo was created
    #[event]
    TodoCreated {
        /// Todo identifier
        id: TodoId,
        /// Title of the todo
        title: String,
        /// When the todo was created
        created_at: DateTime<Utc>,
    },

    /// Event: Todo was completed
    #[event]
    TodoCompleted {
        /// Todo identifier
        id: TodoId,
        /// When the todo was completed
        completed_at: DateTime<Utc>,
    },

    /// Event: Todo was deleted
    #[event]
    TodoDeleted {
        /// Todo identifier
        id: TodoId,
    },

    /// Event: Command validation failed
    #[event]
    ValidationFailed {
        /// Error message
        error: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todo_id_display() {
        let id = TodoId::new();
        let display = format!("{id}");
        assert!(!display.is_empty());
    }

    #[test]
    fn todo_item_new() {
        let id = TodoId::new();
        let now = Utc::now();
        let item = TodoItem::new(id.clone(), "Test todo".to_string(), now);

        assert_eq!(item.id, id);
        assert_eq!(item.title, "Test todo");
        assert!(!item.completed);
        assert_eq!(item.created_at, now);
        assert_eq!(item.completed_at, None);
    }

    #[test]
    fn todo_item_complete() {
        let id = TodoId::new();
        let created = Utc::now();
        let mut item = TodoItem::new(id, "Test".to_string(), created);

        let completed = Utc::now();
        item.complete(completed);

        assert!(item.completed);
        assert_eq!(item.completed_at, Some(completed));
    }

    #[test]
    fn todo_state_count() {
        let mut state = TodoState::new();
        assert_eq!(state.count(), 0);
        assert_eq!(state.completed_count(), 0);

        let id1 = TodoId::new();
        let item1 = TodoItem::new(id1.clone(), "Todo 1".to_string(), Utc::now());
        state.todos.insert(id1, item1);

        assert_eq!(state.count(), 1);
        assert_eq!(state.completed_count(), 0);
    }

    #[test]
    fn todo_action_is_command() {
        let action = TodoAction::CreateTodo {
            id: TodoId::new(),
            title: "Test".to_string(),
        };
        assert!(action.is_command());
        assert!(!action.is_event());
    }

    #[test]
    fn todo_action_is_event() {
        let action = TodoAction::TodoCreated {
            id: TodoId::new(),
            title: "Test".to_string(),
            created_at: Utc::now(),
        };
        assert!(action.is_event());
        assert!(!action.is_command());
    }
}
