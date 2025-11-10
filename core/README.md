# composable-rust-core

**Core traits and types for the Composable Rust functional architecture framework.**

## Overview

`composable-rust-core` provides the fundamental abstractions for building event-driven, functional backend systems using the Reducer pattern with CQRS and Event Sourcing. This crate contains **only trait definitions and pure types**—no I/O, no side effects, just the contract that all implementations must follow.

## Installation

```toml
[dependencies]
composable-rust-core = { path = "../core" }
```

## The Five Fundamental Types

Every Composable Rust application is built on these five types:

| Type | Purpose | Example |
|------|---------|---------|
| **State** | Domain data we track | `CounterState { count: i64 }` |
| **Action** | Events that happen | `CounterAction::Increment` |
| **Reducer** | Business logic | `(State, Action, Env) → (State, Effects)` |
| **Effect** | Side effect descriptions | `Effect::Database(SaveOrder)` |
| **Environment** | Injected dependencies | `Clock`, `EventStore`, `EventBus` |

These types compose together to create unidirectional data flow:

```
Action → Reducer → (State, Effects) → Effect Execution → More Actions
         ↑_________________________________________________|
                        Feedback Loop
```

## Core Modules

### `reducer` - The Reducer Trait

The heart of the architecture. Pure functions that transform state based on actions.

```rust
use composable_rust_core::reducer::Reducer;
use composable_rust_core::effect::Effect;
use composable_rust_core::SmallVec;

pub trait Reducer: Clone + Send + Sync {
    type State: Clone + Send + Sync;
    type Action: Clone + Send + Sync;
    type Environment: Clone + Send + Sync;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]>;
}
```

**Key characteristics**:
- **Pure function**: Same inputs always produce same outputs
- **No I/O**: Database calls, HTTP requests, etc. are returned as effects
- **Fast**: Typically < 1μs execution time
- **Testable**: No mocks needed (test the return value)

**Example**:

```rust
#[derive(Clone)]
pub struct CounterReducer;

impl Reducer for CounterReducer {
    type State = CounterState;
    type Action = CounterAction;
    type Environment = CounterEnvironment;

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
            }
            CounterAction::Decrement => {
                state.count -= 1;
                smallvec![Effect::None]
            }
        }
    }
}
```

### `effect` - The Effect Type

Effects are **descriptions** of side effects, not their execution. The Store executes them after the reducer returns.

```rust
pub enum Effect<A> {
    /// No side effect
    None,

    /// Execute an async operation
    Future(Pin<Box<dyn Future<Output = Option<A>> + Send + 'static>>),

    /// Delay an action
    Delay {
        duration: Duration,
        action: Box<A>,
    },

    /// Execute effects in parallel
    Parallel(Vec<Effect<A>>),

    /// Execute effects sequentially
    Sequential(Vec<Effect<A>>),

    /// Publish an event to the event bus
    PublishEvent {
        topic: String,
        event: Vec<u8>,
    },

    /// Append events to the event store
    AppendEvents {
        stream_id: StreamId,
        events: Vec<Vec<u8>>,
        expected_version: Option<Version>,
    },

    /// Load events from the event store
    LoadEvents {
        stream_id: StreamId,
        from_version: Option<Version>,
    },

    /// Update a projection
    UpdateProjection {
        projection_id: String,
        data: Vec<u8>,
    },
}
```

**Why effects as values?**
- **Testing**: Assert on effect descriptions without execution
- **Cancellation**: Effects haven't executed yet
- **Composition**: Combine/transform effects easily
- **Time-travel debugging**: Replay without re-execution

**Example**:

```rust
use composable_rust_core::{effect::Effect, smallvec};

// Return multiple effects
fn handle_order_placed(state: &mut OrderState, order_id: String) -> SmallVec<[Effect<OrderAction>; 4]> {
    smallvec![
        Effect::AppendEvents {
            stream_id: StreamId::new(format!("order-{}", order_id)),
            events: vec![serialize(&OrderPlacedEvent { order_id: order_id.clone() })],
            expected_version: None,
        },
        Effect::PublishEvent {
            topic: "order-events".to_string(),
            event: serialize(&OrderPlacedEvent { order_id }),
        },
    ]
}
```

### `environment` - Dependency Injection

Environment provides dependencies via traits. Use static dispatch for zero-cost abstractions.

```rust
use composable_rust_core::environment::Clock;
use chrono::{DateTime, Utc};

/// Clock trait for time-based operations
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}
```

**Pattern**: Three implementations per dependency
1. **Production**: Real implementation (`SystemClock`, `PostgresEventStore`)
2. **Test**: Fast mocks (`FixedClock`, `InMemoryEventStore`)
3. **Development**: Instrumented versions (`LoggingDatabase`)

**Example**:

```rust
struct OrderEnvironment<C, ES>
where
    C: Clock,
    ES: EventStore,
{
    clock: C,
    event_store: ES,
}

// Production
let prod_env = OrderEnvironment {
    clock: SystemClock::new(),
    event_store: PostgresEventStore::new(pool),
};

// Tests
let test_env = OrderEnvironment {
    clock: FixedClock::new(test_time()),
    event_store: InMemoryEventStore::new(),
};
```

### `event` - Event Types

Serializable events for event sourcing.

```rust
use composable_rust_core::event::{Event, SerializedEvent};
use serde::{Serialize, Deserialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct OrderPlacedEvent {
    pub order_id: String,
    pub customer_id: String,
    pub items: Vec<LineItem>,
    pub total: Money,
    pub timestamp: DateTime<Utc>,
}

impl Event for OrderPlacedEvent {
    fn event_type(&self) -> &str {
        "OrderPlaced.v1"
    }
}
```

**Key principles**:
- **Fat events**: Include all data downstream needs
- **Immutable**: Never modify events after creation
- **Versioned**: Include version in event type (`OrderPlaced.v1`)
- **Complete**: Self-contained for independent processing

### `event_store` - EventStore Trait

Abstraction for event persistence (append-only log).

```rust
use composable_rust_core::event_store::EventStore;
use composable_rust_core::stream::{StreamId, Version};

pub trait EventStore: Send + Sync {
    /// Append events to a stream
    async fn append_events(
        &self,
        stream_id: &StreamId,
        events: Vec<SerializedEvent>,
        expected_version: Option<Version>,
    ) -> Result<Version>;

    /// Load events from a stream
    async fn load_events(
        &self,
        stream_id: &StreamId,
        from_version: Option<Version>,
    ) -> Result<Vec<SerializedEvent>>;

    /// Batch append for efficiency
    async fn append_batch(
        &self,
        batches: Vec<EventBatch>,
    ) -> Result<Vec<Version>>;
}
```

**Implementations**:
- `PostgresEventStore` (in `composable-rust-postgres` crate)
- `InMemoryEventStore` (in `composable-rust-testing` crate)

### `event_bus` - EventBus Trait

Abstraction for cross-aggregate communication (pub/sub).

```rust
use composable_rust_core::event_bus::EventBus;

pub trait EventBus: Send + Sync {
    /// Publish an event to a topic
    async fn publish(
        &self,
        topic: &str,
        event: SerializedEvent,
    ) -> Result<()>;

    /// Subscribe to a topic
    async fn subscribe(
        &self,
        topic: &str,
        handler: Box<dyn Fn(SerializedEvent) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync>,
    ) -> Result<()>;
}
```

**Implementations**:
- `RedpandaEventBus` (in `composable-rust-redpanda` crate)
- `InMemoryEventBus` (in `composable-rust-testing` crate)

### `composition` - Reducer Composition

Utilities for composing multiple reducers.

```rust
use composable_rust_core::composition::{combine_reducers, scope_reducer};

// Combine multiple reducers
let combined = combine_reducers(vec![
    order_reducer,
    payment_reducer,
    inventory_reducer,
]);

// Scope a reducer to a subtree of state
let scoped = scope_reducer(
    order_reducer,
    |state: &mut AppState| &mut state.orders,  // Get
    |action| match action {
        AppAction::Order(a) => Some(a),
        _ => None,
    },  // Extract
    AppAction::Order,  // Lift
);
```

### `projection` - Projection System

Read model abstractions for the query side of CQRS.

```rust
use composable_rust_core::projection::Projection;

pub trait Projection: Send + Sync {
    type Event;

    /// Handle an event and update the projection
    async fn handle_event(&mut self, event: Self::Event) -> Result<()>;

    /// Reset the projection
    async fn reset(&mut self) -> Result<()>;
}
```

**When to use projections**:
- ✅ UI display (eventual consistency OK)
- ✅ Search, analytics, reports
- ❌ **NEVER** in sagas or workflows (race conditions!)

See [Consistency Patterns](../docs/consistency-patterns.md) for critical guidance.

### `stream` - Stream ID and Version

Types for event stream identification and versioning.

```rust
use composable_rust_core::stream::{StreamId, Version};

// Create stream ID
let stream_id = StreamId::new("order-12345");

// Create version
let version = Version::new(5);

// Optimistic concurrency control
event_store.append_events(
    &stream_id,
    events,
    Some(Version::new(4)),  // Expect version 4, fail if changed
).await?;
```

### `effect_macros` - Effect Helper Macros

Ergonomic macros for constructing effects.

```rust
use composable_rust_core::{append_events, publish_event, load_events};

// Append events with less boilerplate
append_events! {
    store: event_store,
    stream: "order-123",
    events: vec![event],
    expected_version: Some(Version::new(5)),
    on_success: |version| Some(OrderAction::EventsAppended { version }),
    on_error: |err| Some(OrderAction::AppendFailed { error: err.to_string() })
}

// Publish event
publish_event! {
    bus: event_bus,
    topic: "order-events",
    event: serialized_event,
    on_success: || Some(OrderAction::EventPublished),
    on_error: |err| Some(OrderAction::PublishFailed { error: err.to_string() })
}

// Load events
load_events! {
    store: event_store,
    stream: "order-123",
    from_version: None,
    on_success: |events| Some(OrderAction::EventsLoaded { events }),
    on_error: |err| Some(OrderAction::LoadFailed { error: err.to_string() })
}
```

## Re-Exported Types

For convenience, commonly used types are re-exported:

```rust
pub use chrono::{DateTime, Utc};
pub use serde::{Deserialize, Serialize};
pub use smallvec::{smallvec, SmallVec};
```

## Design Principles

### 1. Pure Abstractions

This crate contains **zero I/O**. All I/O is delegated to:
- `composable-rust-runtime` (effect execution)
- `composable-rust-postgres` (PostgreSQL implementation)
- `composable-rust-redpanda` (Kafka implementation)

### 2. Static Dispatch

All traits use static dispatch (generic types), not dynamic dispatch (`dyn Trait`). This enables:
- **Zero-cost abstractions**: No vtable overhead
- **Compiler optimizations**: Inlining, dead code elimination
- **Type safety**: Catch errors at compile time

### 3. Functional Core

Reducers are pure functions:
- **Deterministic**: Same inputs → same outputs
- **Fast**: < 1μs typical execution
- **Testable**: No mocks, no I/O, just assertions

### 4. Explicit Effects

All side effects are **visible and explicit**:

```rust
// ❌ Hidden side effect (BAD)
fn reduce(...) {
    database.save(state).await;  // Hidden I/O!
}

// ✅ Explicit effect (GOOD)
fn reduce(...) -> SmallVec<[Effect; 4]> {
    smallvec![Effect::AppendEvents { ... }]  // Visible effect
}
```

### 5. Dependency Injection

Dependencies are injected via Environment:

```rust
// ❌ Hard-coded dependency (BAD)
fn reduce(...) {
    let db = PostgresDatabase::new(pool);  // Tightly coupled!
}

// ✅ Injected dependency (GOOD)
fn reduce(..., env: &Environment) {
    env.database.save(...);  // Loosely coupled via trait
}
```

## Usage Examples

### Basic Counter

```rust
use composable_rust_core::{reducer::Reducer, effect::Effect, smallvec, SmallVec};

#[derive(Clone, Debug)]
struct CounterState {
    count: i64,
}

#[derive(Clone, Debug)]
enum CounterAction {
    Increment,
    Decrement,
}

#[derive(Clone)]
struct CounterEnvironment;

#[derive(Clone)]
struct CounterReducer;

impl Reducer for CounterReducer {
    type State = CounterState;
    type Action = CounterAction;
    type Environment = CounterEnvironment;

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
            }
            CounterAction::Decrement => {
                state.count -= 1;
                smallvec![Effect::None]
            }
        }
    }
}
```

### Event-Sourced Aggregate

```rust
use composable_rust_core::{
    reducer::Reducer,
    effect::Effect,
    event_store::EventStore,
    stream::StreamId,
    smallvec, SmallVec,
};

#[derive(Clone)]
struct OrderReducer;

impl<ES> Reducer for OrderReducer
where
    ES: EventStore,
{
    type State = OrderState;
    type Action = OrderAction;
    type Environment = OrderEnvironment<ES>;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            OrderAction::PlaceOrder { customer_id, items } => {
                let order_id = generate_order_id();
                let event = OrderPlacedEvent {
                    order_id: order_id.clone(),
                    customer_id,
                    items,
                    timestamp: env.clock.now(),
                };

                state.orders.insert(order_id.clone(), Order::new(event.clone()));

                smallvec![
                    Effect::AppendEvents {
                        stream_id: StreamId::new(format!("order-{}", order_id)),
                        events: vec![serialize(&event)],
                        expected_version: None,
                    },
                    Effect::PublishEvent {
                        topic: "order-events".to_string(),
                        event: serialize(&event),
                    },
                ]
            }
            // ... other actions
        }
    }
}
```

## Testing

Since this crate contains only pure types and traits, testing is straightforward:

```rust
#[test]
fn test_counter_increment() {
    let mut state = CounterState { count: 0 };
    let reducer = CounterReducer;
    let env = CounterEnvironment;

    let effects = reducer.reduce(&mut state, CounterAction::Increment, &env);

    assert_eq!(state.count, 1);
    assert!(matches!(effects[0], Effect::None));
}
```

## Integration with Other Crates

### Runtime

```toml
[dependencies]
composable-rust-core = { path = "../core" }
composable-rust-runtime = { path = "../runtime" }
```

```rust
use composable_rust_core::{reducer::Reducer, effect::Effect};
use composable_rust_runtime::Store;

let store = Store::new(
    CounterState::default(),
    CounterReducer,
    CounterEnvironment,
);
```

### Event Store

```toml
[dependencies]
composable-rust-core = { path = "../core" }
composable-rust-postgres = { path = "../postgres" }
```

```rust
use composable_rust_core::event_store::EventStore;
use composable_rust_postgres::PostgresEventStore;

let event_store = PostgresEventStore::new(pool).await?;
```

### Event Bus

```toml
[dependencies]
composable-rust-core = { path = "../core" }
composable-rust-redpanda = { path = "../redpanda" }
```

```rust
use composable_rust_core::event_bus::EventBus;
use composable_rust_redpanda::RedpandaEventBus;

let event_bus = RedpandaEventBus::builder()
    .broker("localhost:9092")
    .build()
    .await?;
```

## Further Reading

- [Getting Started Guide](../docs/getting-started.md) - Tutorial walkthrough
- [Core Concepts](../docs/concepts.md) - Deep dive into architecture
- [API Reference](../docs/api-reference.md) - Complete API documentation
- [Consistency Patterns](../docs/consistency-patterns.md) - ⚠️ **Critical for sagas**
- [Examples](../examples/) - Working code examples

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
