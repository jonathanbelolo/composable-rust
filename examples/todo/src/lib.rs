//! Todo example demonstrating the simplest possible event-sourced aggregate.
//!
//! This example shows how to build a basic todo list application using
//! Composable Rust. It demonstrates:
//!
//! - Simple domain model (create, complete, delete todos)
//! - Command validation
//! - Event application
//! - Section 3 derive macros (`#[derive(Action)]`, `#[derive(State)]`)
//! - Testing with `ReducerTest`
//!
//! # Quick Start
//!
//! ```no_run
//! use todo::{TodoAction, TodoEnvironment, TodoId, TodoReducer, TodoState};
//! use composable_rust_core::environment::SystemClock;
//! use composable_rust_runtime::Store;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create environment and store
//! let env = TodoEnvironment::new(Arc::new(SystemClock));
//! let store = Store::new(TodoState::new(), TodoReducer::new(), env);
//!
//! // Create a todo
//! let id = TodoId::new();
//! store.send(TodoAction::CreateTodo {
//!     id: id.clone(),
//!     title: "Buy milk".to_string(),
//! }).await?;
//!
//! // Complete the todo
//! store.send(TodoAction::CompleteTodo { id }).await?;
//!
//! // Read state
//! let state = store.state(|s| s.clone()).await;
//! println!("Total todos: {}", state.count());
//! println!("Completed: {}", state.completed_count());
//! # Ok(())
//! # }
//! ```

pub mod reducer;
pub mod types;

// Re-export commonly used types
pub use reducer::{TodoEnvironment, TodoReducer};
pub use types::{TodoAction, TodoId, TodoItem, TodoState};
