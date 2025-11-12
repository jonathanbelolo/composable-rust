//! Simple CLI demo for the todo example.
//!
//! This demonstrates basic usage of the todo application with a command-line
//! interface.

use composable_rust_core::environment::SystemClock;
use composable_rust_runtime::Store;
use std::sync::Arc;
use todo::{TodoAction, TodoEnvironment, TodoId, TodoReducer, TodoState};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Todo Example ===\n");

    // Create environment and store
    let env = TodoEnvironment::new(Arc::new(SystemClock));
    let store = Store::new(TodoState::new(), TodoReducer::new(), env);

    // Create some todos
    let id1 = TodoId::new();
    let id2 = TodoId::new();
    let id3 = TodoId::new();

    println!("Creating todos...");
    store
        .send(TodoAction::CreateTodo {
            id: id1.clone(),
            title: "Buy milk".to_string(),
        })
        .await?;

    store
        .send(TodoAction::CreateTodo {
            id: id2.clone(),
            title: "Write documentation".to_string(),
        })
        .await?;

    store
        .send(TodoAction::CreateTodo {
            id: id3.clone(),
            title: "Deploy to production".to_string(),
        })
        .await?;

    // List todos
    let state = store.state(std::clone::Clone::clone).await;
    println!("\nTodos created: {}", state.count());
    for todo in state.todos.values() {
        let status = if todo.completed { "✓" } else { " " };
        println!("  [{}] {}", status, todo.title);
    }

    // Complete one todo
    println!("\nCompleting 'Buy milk'...");
    store
        .send(TodoAction::CompleteTodo { id: id1.clone() })
        .await?;

    // List todos again
    let state = store.state(std::clone::Clone::clone).await;
    println!("\nCurrent status:");
    for todo in state.todos.values() {
        let status = if todo.completed { "✓" } else { " " };
        println!("  [{}] {}", status, todo.title);
    }
    println!(
        "\nCompleted: {}/{}",
        state.completed_count(),
        state.count()
    );

    // Delete a todo
    println!("\nDeleting 'Deploy to production'...");
    store.send(TodoAction::DeleteTodo { id: id3 }).await?;

    // Final state
    let state = store.state(std::clone::Clone::clone).await;
    println!("\nFinal todos: {}", state.count());
    for todo in state.todos.values() {
        let status = if todo.completed { "✓" } else { " " };
        println!("  [{}] {}", status, todo.title);
    }

    println!("\n=== Demo Complete ===");
    Ok(())
}
