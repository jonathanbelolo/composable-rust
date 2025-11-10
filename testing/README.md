# composable-rust-testing

**Test utilities and mocks for deterministic, fast testing of Composable Rust applications.**

## Overview

`composable-rust-testing` provides mock implementations of all infrastructure dependencies, enabling **memory-speed tests** with **zero I/O**. Tests run in microseconds instead of milliseconds.

## Installation

```toml
[dev-dependencies]
composable-rust-testing = { path = "../testing" }
tokio = { version = "1.43", features = ["test-util"] }
```

## Test Utilities

### FixedClock - Deterministic Time

Mock clock for testing time-dependent logic.

```rust
use composable_rust_testing::FixedClock;
use chrono::{Duration, Utc};

// Create clock at specific time
let clock = FixedClock::new(Utc::now());

// Time is fixed
let t1 = clock.now();
let t2 = clock.now();
assert_eq!(t1, t2);  // Always same!

// Advance time manually
clock.advance(Duration::hours(1));
let t3 = clock.now();
assert_eq!(t3, t1 + Duration::hours(1));
```

**Use cases**:
- Testing expiration logic
- Testing time-based workflows
- Deterministic timeouts
- Saga timeouts

**Helper**:
```rust
use composable_rust_testing::test_clock;

// Quick helper for tests
let clock = test_clock();  // Returns FixedClock with consistent test time
```

### InMemoryEventStore - Fast Event Persistence

In-memory event store for testing event sourcing without PostgreSQL.

```rust
use composable_rust_testing::InMemoryEventStore;
use composable_rust_core::{event_store::EventStore, stream::StreamId};

#[tokio::test]
async fn test_event_sourcing() {
    let event_store = InMemoryEventStore::new();

    // Append events
    event_store.append_events(
        &StreamId::new("order-123"),
        vec![serialize(&OrderPlacedEvent { /* ... */ })],
        None,  // No version check
    ).await?;

    // Load events
    let events = event_store.load_events(
        &StreamId::new("order-123"),
        None,
    ).await?;

    assert_eq!(events.len(), 1);
}
```

**Features**:
- **Memory-only**: No database required
- **Fast**: Microsecond latency
- **Optimistic concurrency**: Supports version checks
- **Thread-safe**: Uses RwLock for concurrent access

### InMemoryEventBus - Fast Pub/Sub

In-memory event bus for testing sagas without Redpanda/Kafka.

```rust
use composable_rust_testing::InMemoryEventBus;
use composable_rust_core::event_bus::EventBus;

#[tokio::test]
async fn test_saga_communication() {
    let event_bus = InMemoryEventBus::new();

    // Subscribe
    let mut rx = event_bus.subscribe("order-events").await?;

    // Publish
    event_bus.publish(
        "order-events",
        serialize(&OrderPlacedEvent { /* ... */ }),
    ).await?;

    // Receive
    let event = rx.recv().await.unwrap();
    assert_eq!(deserialize::<OrderPlacedEvent>(&event)?.order_id, "order-123");
}
```

**Features**:
- **Synchronous**: Events delivered immediately (no network delay)
- **Deterministic**: No timing issues in tests
- **Fast**: Memory-only, no serialization overhead
- **Multiple subscribers**: Supports broadcast

### InMemoryProjectionStore - Mock Projection Storage

Mock projection storage for testing read models.

```rust
use composable_rust_testing::InMemoryProjectionStore;

#[tokio::test]
async fn test_projection_updates() {
    let store = InMemoryProjectionStore::new();

    // Update projection
    store.upsert(
        "customer:cust-123",
        serde_json::json!({
            "name": "John Doe",
            "orders": 5
        }),
    ).await?;

    // Query projection
    let data = store.get("customer:cust-123").await?;
    assert_eq!(data["orders"], 5);
}
```

**Features**:
- **Key-value storage**: Simple get/upsert API
- **JSON support**: Stores `serde_json::Value`
- **Fast**: HashMap-based storage
- **Thread-safe**: Arc<RwLock<HashMap>>

### ReducerTest - Given-When-Then Testing

Builder for readable reducer tests.

```rust
use composable_rust_testing::ReducerTest;

#[test]
fn test_increment() {
    ReducerTest::new(CounterReducer::new())
        .with_env(CounterEnvironment::new(test_clock()))
        .given_state(CounterState { count: 0 })
        .when_action(CounterAction::Increment)
        .then_state(|state| {
            assert_eq!(state.count, 1);
        })
        .then_effects(|effects| {
            assert_eq!(effects.len(), 1);
            assert!(matches!(effects[0], Effect::None));
        })
        .run();
}
```

**Benefits**:
- **Readable**: Given-When-Then structure
- **Self-documenting**: Clear test intent
- **Reusable**: Chain multiple assertions
- **Type-safe**: Compile-time checks

## Testing Patterns

### Pattern 1: Pure Reducer Tests (Fastest)

Test business logic without any I/O.

```rust
#[test]
fn test_order_placement() {
    let mut state = OrderState::default();
    let reducer = OrderReducer;
    let env = OrderEnvironment {
        clock: test_clock(),
        event_store: InMemoryEventStore::new(),
    };

    let effects = reducer.reduce(
        &mut state,
        OrderAction::PlaceOrder {
            customer_id: "cust-1".into(),
            items: vec![/* ... */],
        },
        &env,
    );

    assert_eq!(state.orders.len(), 1);
    assert!(matches!(effects[0], Effect::AppendEvents { .. }));
}
```

**Speed**: < 1μs per test
**When**: Unit tests for business logic

### Pattern 2: Store Integration Tests (Fast)

Test with Store but mock infrastructure.

```rust
#[tokio::test]
async fn test_order_workflow() {
    let env = OrderEnvironment {
        clock: test_clock(),
        event_store: InMemoryEventStore::new(),
        event_bus: InMemoryEventBus::new(),
    };

    let store = Store::new(
        OrderState::default(),
        OrderReducer,
        env,
    );

    store.send(OrderAction::PlaceOrder { /* ... */ }).await?;

    let orders = store.state(|s| s.orders.len()).await;
    assert_eq!(orders, 1);
}
```

**Speed**: < 1ms per test
**When**: Integration tests for workflows

### Pattern 3: End-to-End Tests (Real DB)

Test with real PostgreSQL (slower but realistic).

```rust
#[tokio::test]
async fn test_with_real_database() {
    let pool = create_test_db_pool().await?;
    let event_store = PostgresEventStore::new(pool).await?;

    let env = OrderEnvironment {
        clock: test_clock(),
        event_store,  // Real PostgreSQL
        event_bus: InMemoryEventBus::new(),
    };

    let store = Store::new(OrderState::default(), OrderReducer, env);

    store.send(OrderAction::PlaceOrder { /* ... */ }).await?;

    // Events persisted to real DB
    let events = env.event_store.load_events(&stream_id, None).await?;
    assert_eq!(events.len(), 1);
}
```

**Speed**: 10-100ms per test
**When**: End-to-end tests, CI/CD

## Test Pyramid

```
             ▲
            ╱ ╲
           ╱   ╲         E2E Tests (Few)
          ╱     ╲        - Real PostgreSQL
         ╱───────╲       - Real Redpanda
        ╱         ╲      - Slow (10-100ms)
       ╱───────────╲
      ╱             ╲    Integration Tests (Some)
     ╱  Store Tests  ╲   - InMemory mocks
    ╱  (Fast ~1ms)    ╲  - Store runtime
   ╱─────────────────╲
  ╱                   ╲
 ╱  Reducer Tests      ╲  Unit Tests (Many)
╱  (Fastest <1μs)       ╲ - No Store
───────────────────────── - Pure functions
```

**Recommended split**:
- 80% Unit tests (reducer tests)
- 15% Integration tests (Store + mocks)
- 5% E2E tests (real infrastructure)

## Example: Complete Test Suite

```rust
mod tests {
    use super::*;
    use composable_rust_testing::*;

    // Unit test - fastest
    #[test]
    fn test_increment_logic() {
        let mut state = CounterState { count: 0 };
        let effects = CounterReducer.reduce(&mut state, CounterAction::Increment, &test_env());
        assert_eq!(state.count, 1);
    }

    // Integration test - fast
    #[tokio::test]
    async fn test_counter_workflow() {
        let store = Store::new(
            CounterState::default(),
            CounterReducer,
            CounterEnvironment::new(test_clock()),
        );

        for _ in 0..10 {
            store.send(CounterAction::Increment).await.unwrap();
        }

        let count = store.state(|s| s.count).await;
        assert_eq!(count, 10);
    }

    // E2E test - slower
    #[tokio::test]
    #[ignore]  // Run with --ignored flag
    async fn test_with_real_infrastructure() {
        let pool = create_test_db_pool().await.unwrap();
        let event_store = PostgresEventStore::new(pool).await.unwrap();

        // ... test with real DB
    }
}
```

## Helper Functions

### `test_clock()` - Quick FixedClock

```rust
pub fn test_clock() -> FixedClock {
    FixedClock::new(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap())
}
```

### `test_env()` - Quick Test Environment

Create a test environment with common mocks:

```rust
fn create_test_env() -> MyEnvironment {
    MyEnvironment {
        clock: test_clock(),
        event_store: InMemoryEventStore::new(),
        event_bus: InMemoryEventBus::new(),
    }
}
```

## Performance Benchmarks

| Test Type | Latency | Throughput | Use Case |
|-----------|---------|------------|----------|
| Reducer test | < 1μs | 1M+ tests/sec | Unit tests |
| Store + InMemory | ~1ms | 1K tests/sec | Integration |
| Store + PostgreSQL | 10-100ms | 10-100 tests/sec | E2E |

## Best Practices

### ✅ Do

- Use `FixedClock` for all time-dependent tests
- Test reducers directly without Store (fastest)
- Use `InMemoryEventStore` for event sourcing tests
- Use `InMemoryEventBus` for saga tests
- Run E2E tests in CI/CD only

### ❌ Don't

- Don't use `SystemClock` in tests (non-deterministic)
- Don't use real PostgreSQL in unit tests (slow)
- Don't use real Redpanda in unit tests (slow, requires infrastructure)
- Don't sleep in tests (use `FixedClock::advance()`)

## Integration with Core & Runtime

```toml
[dev-dependencies]
composable-rust-core = { path = "../core" }
composable-rust-runtime = { path = "../runtime" }
composable-rust-testing = { path = "../testing" }
```

```rust
use composable_rust_core::{reducer::Reducer, effect::Effect};
use composable_rust_runtime::Store;
use composable_rust_testing::{test_clock, InMemoryEventStore};

// Test environment
struct TestEnvironment {
    clock: FixedClock,
    event_store: InMemoryEventStore,
}

impl TestEnvironment {
    fn new() -> Self {
        Self {
            clock: test_clock(),
            event_store: InMemoryEventStore::new(),
        }
    }
}
```

## Further Reading

- [Getting Started Guide](../docs/getting-started.md) - See "Testing" section
- [Core Crate](../core/README.md) - Reducer, Effect, Environment traits
- [Runtime Crate](../runtime/README.md) - Store API
- [Examples](../examples/) - Working test examples

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
