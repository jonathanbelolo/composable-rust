# Getting Started with Composable Rust

Welcome to Composable Rust! This guide will walk you through building your first feature using the Counter example, introducing you to the core concepts along the way.

## What is Composable Rust?

Composable Rust is a functional architecture framework for building event-driven backend systems in Rust. Inspired by Swift's Composable Architecture (TCA), it brings together:

- **Functional programming patterns** (pure functions, immutable data)
- **Type-safe state management** (compile-time guarantees)
- **Explicit side effects** (effects as values, not execution)
- **Zero-cost abstractions** (static dispatch, no runtime overhead)

If you're building backend systems that need to handle complex business logic, coordinate across multiple services, or maintain audit trails of state changes, Composable Rust provides battle-tested patterns for managing that complexity.

## Prerequisites

Before starting, ensure you have:

- **Rust 1.85.0 or later** (Edition 2024 required)
- **Tokio runtime** (for async execution)
- Basic understanding of Rust async/await

## Installation

### Adding to Your Project

Add Composable Rust to your `Cargo.toml`:

```toml
[dependencies]
composable-rust-core = { path = "path/to/composable-rust/core" }
composable-rust-runtime = { path = "path/to/composable-rust/runtime" }
tokio = { version = "1.43", features = ["full"] }

[dev-dependencies]
composable-rust-testing = { path = "path/to/composable-rust/testing" }
```

> **Note**: Composable Rust is currently in development. Crates.io publication is planned for Phase 5.

### Clone the Repository

To run the examples and explore the codebase:

```bash
git clone https://github.com/yourusername/composable-rust.git
cd composable-rust

# Verify everything works
cargo test --all-features
cargo run -p counter
```

## The Five Fundamental Types

Every Composable Rust application is built on **five types**:

| Type | Purpose | Example |
|------|---------|---------|
| **State** | Domain data we track | `CounterState { count: i64 }` |
| **Action** | Events that happen | `CounterAction::Increment` |
| **Reducer** | Business logic | `(State, Action) → (New State, Effects)` |
| **Effect** | Side effect descriptions | `Effect::Database(SaveOrder)` |
| **Environment** | Injected dependencies | `Clock`, `Database`, `HttpClient` |

These five types compose together to create a **unidirectional data flow**:

```
Action → Reducer → (State, Effects) → Effect Execution → More Actions
         ↑_________________________________________________|
                        Feedback Loop
```

Let's see these concepts in action by building a counter.

## Your First Feature: Counter

The counter is the "Hello World" of Composable Rust. It demonstrates all core concepts in the simplest possible implementation.

### Step 1: Define Your State

State represents what your feature knows about the world. It must be `Clone` for time-travel debugging and snapshots.

```rust
use composable_rust_core::*;

#[derive(Clone, Debug, Default)]
pub struct CounterState {
    pub count: i64,
}
```

**Key principles**:
- **Owned data**, not references (enables cloning for snapshots)
- **All fields public** for easy testing
- **Derive `Clone` and `Debug`** (required by framework)

### Step 2: Define Your Actions

Actions are events that happen in your system. They're the unified input type for everything: user commands, system events, responses from services.

```rust
#[derive(Clone, Debug)]
pub enum CounterAction {
    Increment,
    Decrement,
    Reset,
}
```

**Key principles**:
- **Actions are values** describing what happened (not functions that do things)
- **Always derive `Clone` and `Debug`**
- **Use enum variants** to represent different event types

### Step 3: Define Your Environment

Environment provides dependencies your reducer needs. It uses traits for dependency injection.

```rust
use composable_rust_core::*;

pub struct CounterEnvironment<C: Clock> {
    clock: C,
}

impl<C: Clock> CounterEnvironment<C> {
    pub fn new(clock: C) -> Self {
        Self { clock }
    }
}
```

**Key principles**:
- **Generic over trait implementations** (enables production/test/dev versions)
- **Trait bounds** ensure compile-time verification (`C: Clock`)
- **Static dispatch** means zero runtime cost

For tests, you'll use `FixedClock`. For production, `SystemClock`.

### Step 4: Implement Your Reducer

The reducer is where business logic lives. It's a pure function: given state and action, produce new state and effects.

```rust
use composable_rust_core::{effect::Effect, reducer::Reducer};

#[derive(Clone)]
pub struct CounterReducer;

impl CounterReducer {
    pub fn new() -> Self {
        Self
    }
}

impl<C: Clock> Reducer for CounterReducer {
    type State = CounterState;
    type Action = CounterAction;
    type Environment = CounterEnvironment<C>;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            CounterAction::Increment => {
                state.count += 1;
                smallvec![Effect::None]
            },
            CounterAction::Decrement => {
                state.count -= 1;
                smallvec![Effect::None]
            },
            CounterAction::Reset => {
                state.count = 0;
                smallvec![Effect::None]
            },
        }
    }
}
```

**Key principles**:
- **Pure function**: Same inputs always produce same outputs
- **No I/O inside reducers**: Database, HTTP, etc. are returned as effects
- **`&mut State`**: Mutate for performance, but still pure from caller's perspective
- **Return `SmallVec<[Effect; 4]>`**: Describe side effects, don't execute them

`SmallVec` stores up to 4 effects inline on the stack, avoiding heap allocations for the common case (0-3 effects).

**Why is this "pure" despite mutation?** The reducer owns the state during reduction. From the caller's perspective, `reduce()` has no side effects - it's referentially transparent.

### Step 5: Create and Use the Store

The Store is the runtime that coordinates everything.

```rust
use composable_rust_runtime::Store;
use composable_rust_testing::test_clock;

#[tokio::main]
async fn main() {
    // 1. Create environment
    let env = CounterEnvironment::new(test_clock());

    // 2. Create store
    let store = Store::new(
        CounterState::default(),  // Initial state
        CounterReducer::new(),     // Business logic
        env,                       // Dependencies
    );

    // 3. Send actions
    let _ = store.send(CounterAction::Increment).await;
    let _ = store.send(CounterAction::Increment).await;

    // 4. Read state
    let count = store.state(|s| s.count).await;
    println!("Count: {count}"); // Count: 2

    // 5. Reset
    let _ = store.send(CounterAction::Reset).await;
    let count = store.state(|s| s.count).await;
    println!("Count: {count}"); // Count: 0
}
```

**What's happening**:
1. **`Store::new()`** initializes the runtime with initial state, reducer, and environment
2. **`store.send(action)`** dispatches an action through the reducer
3. **`store.state(|s| ...)`** reads current state via closure
4. **Store coordinates** the entire flow: locking, reducing, effect execution

### Step 6: Write Tests

Business logic tests run at **memory speed** because reducers have no I/O.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_testing::test_clock;
    use composable_rust_core::effect::Effect;

    #[test]
    fn test_increment() {
        // Arrange
        let mut state = CounterState::default();
        let env = CounterEnvironment::new(test_clock());
        let reducer = CounterReducer::new();

        // Act
        let effects = reducer.reduce(
            &mut state,
            CounterAction::Increment,
            &env
        );

        // Assert
        assert_eq!(state.count, 1);
        assert!(matches!(effects[0], Effect::None));
    }

    #[tokio::test]
    async fn test_with_store() {
        // Create store
        let env = CounterEnvironment::new(test_clock());
        let store = Store::new(
            CounterState::default(),
            CounterReducer::new(),
            env,
        );

        // Send action
        let _ = store.send(CounterAction::Increment).await;

        // Verify state
        let count = store.state(|s| s.count).await;
        assert_eq!(count, 1);
    }
}
```

**Two testing levels**:
1. **Reducer tests**: Pure functions, no async, instant feedback
2. **Store tests**: Full integration, async, tests coordination

Most tests should be reducer tests for maximum speed.

## Running the Example

The Counter example is available in `examples/counter/`:

```bash
# Run the example
cargo run -p counter

# Run tests
cargo test -p counter

# Run with debug logging
RUST_LOG=debug cargo run -p counter

# Run benchmarks
cargo bench -p composable-rust-runtime
```

**Expected output**:

```
=== Counter Example: Composable Rust Architecture ===

Initial count: 0

>>> Sending: Increment
Count after Increment: 1

>>> Sending: Increment
Count after Increment: 2

>>> Sending: Reset
Count after Reset: 0

=== Architecture Demonstration Complete ===
```

## Understanding Effects

The counter uses `Effect::None` because it's a pure state machine. In real applications, you'll use effects for side effects:

```rust
// Phase 2+: Effects with side effects
vec![
    Effect::Database(SaveOrder { order }),
    Effect::Http {
        url: "https://api.example.com",
        method: HttpMethod::Post,
        body: json_body,
    },
    Effect::PublishEvent(OrderPlaced { order_id }),
    Effect::Delay {
        duration: Duration::from_secs(60),
        action: Box::new(CheckOrderStatus { order_id }),
    },
]
```

**Key insight**: Effects are **values**, not execution. The Store executes them after the reducer returns. This keeps reducers pure and enables:

- **Testing without mocks** (assert on effect descriptions)
- **Effect cancellation** (effects haven't executed yet)
- **Time-travel debugging** (replay without re-execution)
- **Effect composition** (combine/transform effects)

## Core Architectural Patterns

### Pattern 1: Functional Core, Imperative Shell

- **Core (Reducer)**: Pure functions, < 1μs execution, easily tested
- **Shell (Store + Effects)**: I/O, side effects, async runtime

This separation means you can test business logic without mocking databases, HTTP clients, or time.

### Pattern 2: Effects as Values

```rust
// ❌ DON'T: Execute in reducer
fn reduce(...) {
    database.save(state).await;  // NO! Hidden side effect!
}

// ✅ DO: Return effect description
fn reduce(...) -> SmallVec<[Effect; 4]> {
    smallvec![Effect::Database(SaveState { state })]  // YES! Just data
}
```

### Pattern 3: Dependency Injection via Traits

```rust
// Production
let env = MyEnvironment {
    clock: SystemClock::new(),
    database: PostgresDatabase::new(pool),
};

// Tests
let env = MyEnvironment {
    clock: FixedClock::new(test_time()),
    database: MockDatabase::new(),
};
```

Same code, different implementations. Static dispatch means **zero runtime cost**.

### Pattern 4: Unidirectional Data Flow

```
User clicks button → Action::ButtonClicked
                      ↓
                    Reducer processes
                      ↓
            (New State, [Effect::Http {...}])
                      ↓
                Effect executes HTTP call
                      ↓
            HTTP response → Action::ResponseReceived
                      ↓
                Back to Reducer
```

Data flows one way. No callbacks, no bidirectional bindings, no event emitters. Easy to reason about.

## Common Questions

### Q: Why not just use `async fn` everywhere?

**A:** Async functions can hide side effects. By forcing effects to be explicit values, we make all I/O visible. This enables:
- Testing without execution
- Deterministic replay
- Effect cancellation
- Clear separation between logic and I/O

### Q: Why `&mut State` if reducers are pure?

**A:** Performance. Copying large state structs on every action would be wasteful. The mutation is an implementation detail - the reducer is still pure (same inputs = same outputs).

### Q: How do I handle errors?

**A:** Three ways:
1. **Reducer panics** → Halt store (lock poison) - for bugs
2. **Effect failures** → Log and continue - for expected runtime failures
3. **Domain errors** → Model as actions (`Action::OrderFailed`)

See `docs/error-handling.md` for details.

### Q: Can I use this in production?

**A:** Phase 1 provides core abstractions. Phase 2 adds persistence, Phase 3 adds event bus, Phase 4 adds production hardening. The architecture is production-ready, but the full feature set is still in development.

## Developer Experience Enhancements

Now that you understand the fundamentals, Composable Rust provides ergonomic helpers to reduce boilerplate and improve readability.

### Derive Macros

Instead of manually implementing methods, use derive macros:

#### `#[derive(Action)]`

```rust
use composable_rust_macros::Action;

#[derive(Action, Clone, Debug)]
enum OrderAction {
    #[command]
    PlaceOrder { customer_id: String, items: Vec<LineItem> },

    #[command]
    CancelOrder { order_id: String },

    #[event]
    OrderPlaced { order_id: String, timestamp: DateTime<Utc> },

    #[event]
    OrderCancelled { order_id: String, reason: String },
}

// Auto-generated methods:
let action = OrderAction::PlaceOrder { customer_id: "c1".into(), items: vec![] };
assert!(action.is_command());  // ✅ true
assert!(!action.is_event());   // ✅ false

let event = OrderAction::OrderPlaced { order_id: "o1".into(), timestamp: Utc::now() };
assert!(event.is_event());                    // ✅ true
assert_eq!(event.event_type(), "OrderPlaced.v1");  // ✅ Versioned event types
```

**Benefits**: Automatic CQRS pattern enforcement, versioned event types, less boilerplate.

#### `#[derive(State)]`

```rust
use composable_rust_macros::State;
use composable_rust_core::stream::Version;

#[derive(State, Clone, Debug)]
struct OrderState {
    pub orders: HashMap<OrderId, Order>,
    #[version]
    pub version: Option<Version>,
}

// Auto-generated version management:
let mut state = OrderState { orders: HashMap::new(), version: None };
assert_eq!(state.version(), None);

state.set_version(Version::new(5));
assert_eq!(state.version(), Some(Version::new(5)));
```

**Benefits**: Automatic version tracking for event sourcing, cleaner state definitions.

### Effect Helper Macros

For event sourcing and async operations, helper macros eliminate boilerplate:

#### `append_events!` - Event Store Operations

**Before** (18 lines):
```rust
Effect::EventStore(EventStoreOperation::AppendEvents {
    event_store: Arc::clone(&event_store),
    stream_id: StreamId::new("order-123"),
    expected_version: Some(Version::new(5)),
    events: vec![event],
    on_success: Box::new(move |version| {
        Some(OrderAction::EventsAppended { version })
    }),
    on_error: Box::new(|error| {
        Some(OrderAction::AppendFailed { error: error.to_string() })
    }),
})
```

**After** (7 lines - 60% reduction):
```rust
use composable_rust_core::append_events;

append_events! {
    store: event_store,
    stream: "order-123",
    expected_version: Some(Version::new(5)),
    events: vec![event],
    on_success: |version| Some(OrderAction::EventsAppended { version }),
    on_error: |err| Some(OrderAction::AppendFailed { error: err.to_string() })
}
```

#### Other Effect Macros

```rust
// Load events from stream
load_events! {
    store: event_store,
    stream: "order-123",
    from_version: None,
    on_success: |events| Some(OrderAction::EventsLoaded { events }),
    on_error: |err| Some(OrderAction::LoadFailed { error: err.to_string() })
}

// Publish event to bus
publish_event! {
    bus: event_bus,
    topic: "order-events",
    event: serialized_event,
    on_success: || Some(OrderAction::EventPublished),
    on_error: |err| Some(OrderAction::PublishFailed { error: err.to_string() })
}

// Async effect
async_effect! {
    let response = http_client.get("https://api.example.com").await?;
    Some(OrderAction::ResponseReceived { response })
}

// Delayed action
delay! {
    duration: Duration::from_secs(30),
    action: OrderAction::TimeoutExpired
}
```

### ReducerTest Builder

Write more readable tests with Given-When-Then syntax:

**Before**:
```rust
#[test]
fn test_increment() {
    let mut state = CounterState::default();
    let env = CounterEnvironment::new(test_clock());
    let reducer = CounterReducer::new();

    let effects = reducer.reduce(&mut state, CounterAction::Increment, &env);

    assert_eq!(state.count, 1);
    assert!(matches!(effects[0], Effect::None));
}
```

**After**:
```rust
use composable_rust_testing::{ReducerTest, assertions};

#[test]
fn test_increment() {
    ReducerTest::new(CounterReducer::new())
        .with_env(CounterEnvironment::new(test_clock()))
        .given_state(CounterState::default())
        .when_action(CounterAction::Increment)
        .then_state(|state| {
            assert_eq!(state.count, 1);
        })
        .then_effects(|effects| {
            assertions::assert_no_effects(effects);
        })
        .run();
}
```

**Benefits**:
- Self-documenting test structure
- Reusable assertion helpers
- 30-50% more readable

### When to Use Enhancements

**Use derive macros when**:
- You have commands and events in your Action enum
- You need event versioning
- Your state tracks versions for event sourcing

**Use effect macros when**:
- Working with event stores (Phase 2+)
- Working with event bus (Phase 3+)
- Verbose Arc::clone() and Box::new() hurts readability

**Use ReducerTest when**:
- You want Given-When-Then test structure
- You have many similar test cases
- Readability matters

All enhancements are **opt-in**. The fundamental patterns work perfectly well without them.

## Next Steps

### Explore the Counter

The Counter example demonstrates all core concepts:

```bash
cd examples/counter
cat README.md  # Comprehensive architecture reference
cat src/lib.rs # Implementation details
cargo test     # See tests in action
```

The Counter README is the **primary reference document** for Phase 1 architecture.

### Read Core Concepts

See `docs/concepts.md` for deeper explanations of:
- State, Action, Reducer, Effect, Environment
- Effect composition (map, chain, merge)
- TestStore for deterministic testing
- Error handling strategy

### Review API Reference

See `docs/api-reference.md` for detailed API documentation:
- `Store::new()`, `Store::send()`, `Store::state()`
- `Effect` variants and methods
- Environment traits (`Clock`, etc.)

### Study Implementation Decisions

See `docs/implementation-decisions.md` to understand **why** the architecture is designed this way:
- Why `&mut State`?
- Why effects as values?
- Why static dispatch?
- What alternatives were considered?

### Build Your Own Feature

Try implementing a simple TODO list:
1. State: `Vec<Todo>` with `id`, `text`, `completed`
2. Actions: `Add`, `Toggle`, `Remove`, `Clear`
3. Reducer: Pure state transitions
4. Tests: Verify each action works correctly

### Coming in Future Phases

- **Phase 2**: PostgreSQL event store, event sourcing ✅ COMPLETE
- **Phase 3**: Redpanda event bus, sagas for distributed transactions ✅ COMPLETE
- **Phase 4**: Observability, circuit breakers, production hardening ✅ COMPLETE
- **Phase 5**: Developer experience ✅ Section 3 COMPLETE (derive macros, effect helpers, testing utilities)

## Key Takeaways

✅ **Five types**: State, Action, Reducer, Effect, Environment
✅ **One-way flow**: Action → Reducer → (State, Effects) → More Actions
✅ **Pure core**: Reducers are pure functions (< 1μs execution)
✅ **Effects as values**: Side effects are data, not execution
✅ **Fast tests**: Business logic tests run at memory speed
✅ **Static dispatch**: Zero-cost abstractions via traits

**You now understand the foundations of Composable Rust.** Everything else builds on these five types and their interactions.

## Getting Help

- **Architecture questions**: See `specs/architecture.md` (comprehensive 2800+ line spec)
- **Implementation details**: See `docs/implementation-decisions.md`
- **API documentation**: `cargo doc --open`
- **Examples**: Browse `examples/` directory
- **Issues**: GitHub issue tracker

Welcome to functional architecture in Rust! 🦀
