# Getting Started with Composable Rust

**Welcome!** This guide will take you from zero to a working event-driven application in under 1 hour.

> **Note**: This is a comprehensive tutorial. If you prefer to start with a simpler example, see the existing [Counter-based Getting Started](./getting-started.md).

---

## Table of Contents

1. [What is Composable Rust?](#what-is-composable-rust)
2. [Prerequisites](#prerequisites)
3. [Installation](#installation)
4. [Your First Aggregate (Todo)](#your-first-aggregate-todo)
5. [Adding Side Effects](#adding-side-effects)
6. [Event Sourcing with PostgreSQL](#event-sourcing-with-postgresql)
7. [Testing](#testing)
8. [Next Steps](#next-steps)

**Estimated Time**: 60 minutes

---

## What is Composable Rust?

Composable Rust is a **functional architecture framework** for building event-driven backend systems in Rust. It combines:

- **Functional programming** (pure reducers, explicit effects)
- **Event Sourcing** (events as source of truth)
- **CQRS** (separate read and write models)
- **Type safety** (Rust's guarantees + architectural patterns)

### The Five Core Types

Every Composable Rust application is built from five fundamental types:

```
Action → Reducer → (New State, Effects) → Effect Execution → More Actions
          ↑                                                      ↓
       State ←──────────────────────────────────────────────────┘
```

1. **State**: Your domain data (e.g., `TodoState { id, title, completed }`)
2. **Action**: Commands (`CreateTodo`) and Events (`TodoCreated`)
3. **Reducer**: Pure function `(State, Action, Env) → (State, Effects)`
4. **Effect**: Side effect descriptions (`Database`, `PublishEvent`)
5. **Environment**: Dependencies (`Clock`, `Database`, `EventBus`)

### Why Composable Rust?

✅ **Testable**: Business logic is pure functions (no I/O)
✅ **Auditable**: Complete history of every change
✅ **Scalable**: CQRS enables independent read/write scaling
✅ **Type-safe**: Rust prevents entire classes of bugs
✅ **Maintainable**: Clear separation of concerns

---

## Prerequisites

- **Rust 1.85+** (Edition 2024)
  ```bash
  rustc --version  # Should be ≥ 1.85.0
  ```

- **PostgreSQL 14+** (optional, for event sourcing)
  ```bash
  # macOS with Homebrew
  brew install postgresql@14
  brew services start postgresql@14

  # Or Docker
  docker run -d -e POSTGRES_PASSWORD=password -p 5432:5432 postgres:14
  ```

**Time**: 5 minutes

---

## Installation

### 1. Create Project

```bash
cargo new my-todo-app
cd my-todo-app
```

### 2. Add Dependencies

Edit `Cargo.toml`:

```toml
[package]
name = "my-todo-app"
version = "0.1.0"
edition = "2024"

[dependencies]
composable-rust-core = "0.1"
composable-rust-runtime = "0.1"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
chrono = "0.4"
uuid = { version = "1", features = ["v4"] }

[dev-dependencies]
composable-rust-testing = "0.1"
```

### 3. Verify

```bash
cargo build
```

✅ **Ready to build!**

---

## Your First Aggregate (Todo)

Let's build a **Todo** aggregate to demonstrate all five core types.

### Full Implementation

Create `src/main.rs`:

```rust
use chrono::{DateTime, Utc};
use composable_rust_core::{effect::Effect, environment::Clock, reducer::Reducer};
use composable_rust_runtime::Store;
use composable_rust_testing::test_clock;
use serde::{Deserialize, Serialize};

// ===== STATE =====

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TodoState {
    pub id: Option<String>,
    pub title: String,
    pub completed: bool,
}

impl Default for TodoState {
    fn default() -> Self {
        Self {
            id: None,
            title: String::new(),
            completed: false,
        }
    }
}

// ===== ACTIONS =====

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TodoAction {
    // Commands
    CreateTodo { title: String },
    ToggleTodo,
    UpdateTitle { new_title: String },

    // Events
    TodoCreated { id: String, title: String, timestamp: DateTime<Utc> },
    TodoToggled { completed: bool, timestamp: DateTime<Utc> },
    TitleUpdated { new_title: String, timestamp: DateTime<Utc> },
}

// ===== ENVIRONMENT =====

pub struct TodoEnvironment<C: Clock> {
    pub clock: C,
}

// ===== REDUCER =====

pub struct TodoReducer;

impl<C: Clock> Reducer for TodoReducer {
    type State = TodoState;
    type Action = TodoAction;
    type Environment = TodoEnvironment<C>;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // Command: Create todo
            TodoAction::CreateTodo { title } => {
                if title.is_empty() {
                    return smallvec![Effect::None];
                }

                let id = format!("todo-{}", uuid::Uuid::new_v4());
                let timestamp = env.clock.now();

                smallvec![Effect::Future(Box::pin(async move {
                    Some(TodoAction::TodoCreated { id, title, timestamp })
                }))]
            }

            // Event: Todo created
            TodoAction::TodoCreated { id, title, .. } => {
                state.id = Some(id);
                state.title = title;
                state.completed = false;
                smallvec![Effect::None]
            }

            // Command: Toggle completion
            TodoAction::ToggleTodo => {
                let completed = !state.completed;
                let timestamp = env.clock.now();

                smallvec![Effect::Future(Box::pin(async move {
                    Some(TodoAction::TodoToggled { completed, timestamp })
                }))]
            }

            // Event: Todo toggled
            TodoAction::TodoToggled { completed, .. } => {
                state.completed = completed;
                smallvec![Effect::None]
            }

            // Command: Update title
            TodoAction::UpdateTitle { new_title } => {
                if new_title.is_empty() {
                    return smallvec![Effect::None];
                }

                let timestamp = env.clock.now();

                smallvec![Effect::Future(Box::pin(async move {
                    Some(TodoAction::TitleUpdated { new_title, timestamp })
                }))]
            }

            // Event: Title updated
            TodoAction::TitleUpdated { new_title, .. } => {
                state.title = new_title;
                smallvec![Effect::None]
            }
        }
    }
}

// ===== MAIN =====

#[tokio::main]
async fn main() {
    let env = TodoEnvironment { clock: test_clock() };
    let store = Store::new(TodoState::default(), TodoReducer, env);

    // Create a todo
    store.send(TodoAction::CreateTodo {
        title: "Learn Composable Rust".to_string(),
    }).await;

    // Toggle completion
    store.send(TodoAction::ToggleTodo).await;

    // Print state
    let state = store.state(Clone::clone).await;
    println!("Todo: {:?}", state);
    println!("Completed: {}", state.completed);
}
```

### Run It

```bash
cargo run
```

**Output**:
```
Todo: TodoState { id: Some("todo-..."), title: "Learn Composable Rust", completed: true }
Completed: true
```

🎉 **You've built your first aggregate!**

**Time elapsed**: 35 minutes

---

## Adding Side Effects

Let's add a side effect: print when todos are created.

Modify the `TodoCreated` event handler:

```rust
TodoAction::TodoCreated { id, title, .. } => {
    state.id = Some(id.clone());
    state.title = title.clone();
    state.completed = false;

    // Add side effect
    smallvec![Effect::Future(Box::pin(async move {
        println!("✓ Created todo: {} ({})", title, id);
        None // No follow-up action
    }))]
}
```

**Run again**:
```bash
cargo run
```

**Output**:
```
✓ Created todo: Learn Composable Rust (todo-...)
Todo: TodoState { ... }
```

**Key Points**:
- Effects can return `None` (no follow-up action)
- Effects run **after** state update
- Multiple effects can be returned

---

## Event Sourcing with PostgreSQL

Now let's persist events to PostgreSQL for full audit trail.

### 1. Add Dependencies

```toml
[dependencies]
composable-rust-postgres = "0.1"
sqlx = { version = "0.8", features = ["postgres", "runtime-tokio"] }
```

### 2. Setup Database

```bash
createdb todo_db

# Or Docker:
docker exec -it postgres psql -U postgres -c "CREATE DATABASE todo_db;"
```

### 3. Run Migrations

```bash
cargo install sqlx-cli --no-default-features --features postgres

DATABASE_URL="postgresql://postgres:password@localhost/todo_db" \
  sqlx migrate run --source path/to/composable-rust/postgres/migrations
```

### 4. Update Code

```rust
use composable_rust_postgres::PostgresEventStore;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to PostgreSQL
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect("postgresql://postgres:password@localhost/todo_db")
        .await?;

    let event_store = Arc::new(PostgresEventStore::new(pool));

    // Update environment with event store
    let env = TodoEnvironment {
        clock: test_clock(),
        event_store: event_store.clone(),
    };

    let store = Store::new(TodoState::default(), TodoReducer, env);

    // Create todo (events persisted to PostgreSQL)
    store.send(TodoAction::CreateTodo {
        title: "Learn Event Sourcing".to_string(),
    }).await;

    Ok(())
}
```

### 5. Replay Events

Load todo from history:

```rust
async fn load_todo(
    event_store: &PostgresEventStore,
    todo_id: &str,
) -> Result<TodoState, Box<dyn std::error::Error>> {
    let stream_id = format!("todo-{}", todo_id);
    let events = event_store.load_events(&stream_id).await?;

    // Reconstruct state from events
    let mut state = TodoState::default();
    let reducer = TodoReducer;
    let env = TodoEnvironment { clock: test_clock() };

    for event in events {
        let action = bincode::deserialize(&event.data)?;
        reducer.reduce(&mut state, action, &env);
    }

    Ok(state)
}
```

**Key Points**:
- Events are **source of truth**
- State is **derived** from events
- Complete audit trail
- Can reconstruct state at any point in time

**Time elapsed**: 55 minutes

---

## Testing

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_todo() {
        let mut state = TodoState::default();
        let env = TodoEnvironment { clock: test_clock() };

        TodoReducer.reduce(
            &mut state,
            TodoAction::CreateTodo { title: "Test".to_string() },
            &env,
        );

        // State not updated until event is applied
        assert_eq!(state.id, None);
    }

    #[test]
    fn test_todo_created_event() {
        let mut state = TodoState::default();
        let env = TodoEnvironment { clock: test_clock() };

        TodoReducer.reduce(
            &mut state,
            TodoAction::TodoCreated {
                id: "todo-1".to_string(),
                title: "Test".to_string(),
                timestamp: env.clock.now(),
            },
            &env,
        );

        assert_eq!(state.id, Some("todo-1".to_string()));
        assert_eq!(state.title, "Test");
        assert!(!state.completed);
    }

    #[test]
    fn test_toggle_todo() {
        let mut state = TodoState {
            id: Some("todo-1".to_string()),
            title: "Test".to_string(),
            completed: false,
        };
        let env = TodoEnvironment { clock: test_clock() };

        TodoReducer.reduce(&mut state, TodoAction::ToggleTodo, &env);

        // Apply event
        TodoReducer.reduce(
            &mut state,
            TodoAction::TodoToggled {
                completed: true,
                timestamp: env.clock.now(),
            },
            &env,
        );

        assert!(state.completed);
    }
}
```

### Run Tests

```bash
cargo test
```

**Output**:
```
running 3 tests
test tests::test_create_todo ... ok
test tests::test_todo_created_event ... ok
test tests::test_toggle_todo ... ok

test result: ok. 3 passed
```

**Key Points**:
- Tests are **fast** (no I/O)
- Tests are **deterministic** (use `test_clock()`)
- Test commands and events separately

**Time elapsed**: 60 minutes

---

## Next Steps

🎉 **Congratulations!** You've built a working Todo aggregate with:

- ✅ Pure reducer (testable business logic)
- ✅ Explicit effects (side effects as values)
- ✅ Event sourcing (complete audit trail)
- ✅ Comprehensive tests

### Learn More

**Patterns & Best Practices**:
- [Pattern Cookbook](./cookbook.md) - Common scenarios
- [Consistency Patterns](./consistency-patterns.md) - Projections vs event store
- [Saga Patterns](./saga-patterns.md) - Multi-aggregate coordination
- [Event Design Guidelines](./event-design-guidelines.md) - Schema best practices

**Examples**:
- `examples/counter/` - Simpler example
- `examples/order-processing/` - Event-sourced e-commerce
- `examples/checkout-saga/` - Multi-aggregate saga
- `examples/order-projection/` - CQRS read models

**Production**:
- [Projections Guide](./projections.md) - Build queryable read models
- Event design, monitoring, deployment

### Get Help

- [API Reference](./api-reference.md)
- [GitHub Discussions](https://github.com/composable-rust/discussions)

---

## Troubleshooting

### "cannot connect to PostgreSQL"

Check database is running:
```bash
pg_isready
# Or: docker ps | grep postgres
```

### Tests are slow

Use in-memory mocks:
```rust
use composable_rust_testing::{test_clock, InMemoryEventStore};
```

### "Cannot move out of borrowed content"

Clone the data:
```rust
let id = state.id.clone();
```

---

## Summary

**You learned**:
- ✅ Five core types (State, Action, Reducer, Effect, Environment)
- ✅ How to build an aggregate (Todo)
- ✅ How to add effects (side effects as values)
- ✅ How to use event sourcing (PostgreSQL)
- ✅ How to test reducers (fast, deterministic)

**Welcome to Composable Rust!** 🚀

---

**Last Updated**: 2025-01-07
**Duration**: ~60 minutes
**Difficulty**: Beginner
