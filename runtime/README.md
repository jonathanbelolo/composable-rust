# composable-rust-runtime

**Production-ready Store runtime and effect execution for Composable Rust.**

## Overview

`composable-rust-runtime` provides the **imperative shell** that executes the effects returned by the **functional core** (reducers). This is where I/O happens: database calls, HTTP requests, event publishing, delays, and more.

## Installation

```toml
[dependencies]
composable-rust-core = { path = "../core" }
composable-rust-runtime = { path = "../runtime" }
tokio = { version = "1.43", features = ["full"] }
```

## Core Component: The Store

The `Store` is the runtime that coordinates everything:

```rust
use composable_rust_runtime::Store;
use composable_rust_core::reducer::Reducer;

let store = Store::new(
    initial_state,  // Initial state
    my_reducer,     // Reducer (business logic)
    environment,    // Environment (dependencies)
);
```

### Store API

#### `new()` - Create a Store

```rust
pub fn new(state: S, reducer: R, environment: E) -> Arc<Self>
where
    S: Clone + Send + Sync + 'static,
    R: Reducer<State = S, Action = A, Environment = E> + Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
```

Creates a new Store with initial state, reducer, and environment.

**Example**:
```rust
let store = Store::new(
    CounterState::default(),
    CounterReducer,
    CounterEnvironment::new(SystemClock::new()),
);
```

#### `send()` - Dispatch an Action

```rust
pub async fn send(&self, action: A) -> Result<(), StoreError>
```

Dispatches an action to the reducer. The reducer processes it and returns effects, which the Store executes.

**Example**:
```rust
store.send(CounterAction::Increment).await?;
```

**What happens**:
1. Action is sent to the Store
2. Store locks state (exclusive)
3. Reducer processes: `(state, action, env) → (new_state, effects)`
4. State is updated
5. Lock is released
6. Effects are executed concurrently
7. Any actions returned by effects feed back into step 1

#### `state()` - Read State

```rust
pub async fn state<F, T>(&self, f: F) -> T
where
    F: FnOnce(&S) -> T,
```

Reads state via a closure. The closure receives a reference to the current state.

**Example**:
```rust
let count = store.state(|s| s.count).await;
println!("Count: {count}");
```

**Performance**: Uses RwLock for concurrent reads. Multiple readers can access state simultaneously.

#### `subscribe_actions()` - Subscribe to Actions

```rust
pub fn subscribe_actions(&self) -> tokio::sync::broadcast::Receiver<A>
```

Creates a broadcast receiver for all actions dispatched to the Store.

**Example**:
```rust
let mut rx = store.subscribe_actions();

tokio::spawn(async move {
    while let Ok(action) = rx.recv().await {
        println!("Action: {:?}", action);
    }
});
```

**Use cases**:
- WebSocket broadcasting (all clients receive events)
- Logging/audit trails
- Metrics collection
- Real-time UI updates

#### `send_and_wait_for()` - Wait for Specific Action

```rust
pub async fn send_and_wait_for<F>(
    &self,
    action: A,
    predicate: F,
    timeout: Duration,
) -> Result<A, StoreError>
where
    F: Fn(&A) -> bool,
```

Sends an action and waits for a matching action (based on predicate).

**Example**:
```rust
// Send command, wait for confirmation event
let result = store
    .send_and_wait_for(
        OrderAction::PlaceOrder { customer_id, items },
        |a| matches!(a, OrderAction::OrderPlaced { .. }),
        Duration::from_secs(5),
    )
    .await?;

match result {
    OrderAction::OrderPlaced { order_id } => println!("Order placed: {order_id}"),
    _ => unreachable!(),
}
```

**Use cases**:
- Synchronous command-query workflows
- Testing (wait for specific events)
- Request-response patterns

#### `health()` - Check Store Health

```rust
pub fn health(&self) -> HealthStatus
```

Returns health status based on Dead Letter Queue size and other metrics.

**Health levels**:
- `Healthy`: DLQ < 10 items, no issues
- `Degraded`: DLQ 10-100 items, elevated error rate
- `Unhealthy`: DLQ > 100 items, high failure rate

**Example**:
```rust
let status = store.health();
if status.is_unhealthy() {
    eprintln!("Store is unhealthy! DLQ size: {:?}", store.dlq_size());
}
```

#### `shutdown()` - Graceful Shutdown

```rust
pub async fn shutdown(&self, timeout: Duration) -> Result<(), StoreError>
```

Initiates graceful shutdown, waiting for in-flight effects to complete.

**Example**:
```rust
// Shutdown with 30s timeout
store.shutdown(Duration::from_secs(30)).await?;
```

**What happens**:
1. Stop accepting new actions
2. Wait for in-flight effects to complete (up to timeout)
3. Close action broadcast channel
4. Return `Ok(())` or `StoreError::ShutdownTimeout`

## Effect Execution

The Store executes effects returned by reducers. Effects are descriptions of side effects, executed by the Store's effect executor.

### Effect Types

| Effect | Description | Execution |
|--------|-------------|-----------|
| `None` | No-op | Skipped |
| `Future` | Async operation | Spawned as Tokio task |
| `Delay` | Delayed action | `tokio::time::sleep()` → dispatch action |
| `Parallel` | Concurrent effects | All effects spawn concurrently |
| `Sequential` | Sequential effects | Effects execute in order |
| `PublishEvent` | Event bus publish | Calls `EventBus::publish()` |
| `AppendEvents` | Event store append | Calls `EventStore::append_events()` |
| `LoadEvents` | Event store load | Calls `EventStore::load_events()` |
| `UpdateProjection` | Projection update | Calls `Projection::handle_event()` |

### Effect Execution Flow

```
Reducer returns effects
         ↓
Store spawns executor task (non-blocking)
         ↓
For each effect:
  - None → Skip
  - Future → Spawn Tokio task
  - Delay → sleep() → send(action)
  - Parallel → Spawn all concurrently
  - Sequential → Execute in order
  - Publish → event_bus.publish()
  - Append → event_store.append()
  - Load → event_store.load()
  - UpdateProjection → projection.handle_event()
         ↓
Actions from effects → send() → Reducer
         ↓
Feedback loop continues
```

## Production Features

### Retry Policies

Automatic retry with exponential backoff for transient failures.

```rust
use composable_rust_runtime::retry::{RetryPolicy, ExponentialBackoff};

let retry_policy = RetryPolicy::new(ExponentialBackoff {
    initial_interval: Duration::from_millis(100),
    max_interval: Duration::from_secs(30),
    multiplier: 2.0,
    max_retries: 5,
});

// Use in environment
struct MyEnvironment {
    retry_policy: RetryPolicy,
    // ... other deps
}
```

**Features**:
- Exponential backoff (configurable)
- Max retries (prevent infinite loops)
- Jitter (prevent thundering herd)
- Retry budget (circuit breaker integration)

### Circuit Breakers

Prevent cascading failures by failing fast when error rate is high.

```rust
use composable_rust_runtime::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};

let circuit_breaker = CircuitBreaker::new(CircuitBreakerConfig {
    failure_threshold: 5,        // Open after 5 failures
    success_threshold: 2,        // Close after 2 successes
    timeout: Duration::from_secs(60),  // Half-open after 60s
});

// Use in effect execution
if circuit_breaker.is_open() {
    return Err("Circuit breaker open".into());
}

match perform_operation().await {
    Ok(result) => {
        circuit_breaker.record_success();
        Ok(result)
    }
    Err(e) => {
        circuit_breaker.record_failure();
        Err(e)
    }
}
```

**States**:
- **Closed**: Normal operation, requests go through
- **Open**: Too many failures, requests fail immediately
- **Half-Open**: Testing if service recovered

### Dead Letter Queue (DLQ)

Failed effects are moved to a Dead Letter Queue for later analysis.

```rust
// Check DLQ size
let dlq_size = store.dlq_size();
if dlq_size > 100 {
    eprintln!("Warning: DLQ has {dlq_size} items");
}

// Inspect DLQ
let dlq_items = store.dlq_items();
for item in dlq_items {
    eprintln!("Failed effect: {:?}, retries: {}", item.effect, item.retry_count);
}

// Retry DLQ items
store.retry_dlq().await?;
```

**DLQ triggers**:
- Effect failed after max retries
- Circuit breaker open
- Serialization errors
- Network timeouts

### Metrics & Observability

Prometheus metrics for monitoring Store health.

```rust
use composable_rust_runtime::metrics::StoreMetrics;

let metrics = store.metrics();

println!("Actions dispatched: {}", metrics.actions_dispatched);
println!("Effects executed: {}", metrics.effects_executed);
println!("Effect failures: {}", metrics.effect_failures);
println!("DLQ size: {}", metrics.dlq_size);
println!("Avg reducer time: {:?}", metrics.avg_reducer_duration);
println!("Avg effect time: {:?}", metrics.avg_effect_duration);
```

**Metrics collected**:
- `composable_rust_actions_total` - Total actions dispatched
- `composable_rust_effects_total` - Total effects executed
- `composable_rust_effect_failures_total` - Effect failures
- `composable_rust_dlq_size` - Dead Letter Queue size
- `composable_rust_reducer_duration_seconds` - Reducer execution time (histogram)
- `composable_rust_effect_duration_seconds` - Effect execution time (histogram)

### Tracing

Distributed tracing with OpenTelemetry integration.

```rust
use tracing::{info_span, instrument};

#[instrument(skip(store))]
async fn place_order(store: Arc<Store<...>>, order: Order) -> Result<OrderId> {
    let span = info_span!("place_order", order_id = %order.id);
    let _guard = span.enter();

    store.send(OrderAction::PlaceOrder { order }).await?;

    Ok(order.id)
}
```

**Trace spans**:
- `store.send` - Action dispatch
- `reducer.reduce` - Reducer execution
- `effect.execute` - Effect execution
- `event_store.append` - Event persistence
- `event_bus.publish` - Event publishing

## Testing with Store

### Unit Testing (No Store)

Test reducers directly without the Store runtime:

```rust
#[test]
fn test_increment() {
    let mut state = CounterState::default();
    let reducer = CounterReducer;
    let env = CounterEnvironment::new(FixedClock::new(test_time()));

    let effects = reducer.reduce(&mut state, CounterAction::Increment, &env);

    assert_eq!(state.count, 1);
    assert!(matches!(effects[0], Effect::None));
}
```

### Integration Testing (With Store)

Test full workflows with the Store:

```rust
#[tokio::test]
async fn test_counter_with_store() {
    let store = Store::new(
        CounterState::default(),
        CounterReducer,
        CounterEnvironment::new(SystemClock::new()),
    );

    store.send(CounterAction::Increment).await.unwrap();
    store.send(CounterAction::Increment).await.unwrap();

    let count = store.state(|s| s.count).await;
    assert_eq!(count, 2);
}
```

### Event-Driven Testing

Test event sequences with `send_and_wait_for`:

```rust
#[tokio::test]
async fn test_order_placement() {
    let store = create_test_store();

    let result = store
        .send_and_wait_for(
            OrderAction::PlaceOrder {
                customer_id: "cust-1".into(),
                items: vec![/* ... */],
            },
            |a| matches!(a, OrderAction::OrderPlaced { .. }),
            Duration::from_secs(5),
        )
        .await
        .unwrap();

    match result {
        OrderAction::OrderPlaced { order_id, .. } => {
            assert!(!order_id.is_empty());
        }
        _ => panic!("Expected OrderPlaced"),
    }
}
```

## Usage Examples

### Basic Counter

```rust
use composable_rust_runtime::Store;
use composable_rust_core::{reducer::Reducer, effect::Effect, smallvec, SmallVec};

// State, Action, Environment, Reducer definitions...

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = Store::new(
        CounterState::default(),
        CounterReducer,
        CounterEnvironment::new(SystemClock::new()),
    );

    store.send(CounterAction::Increment).await?;
    store.send(CounterAction::Increment).await?;

    let count = store.state(|s| s.count).await;
    println!("Count: {count}");

    store.shutdown(Duration::from_secs(5)).await?;
    Ok(())
}
```

### Event-Sourced Aggregate

```rust
use composable_rust_postgres::PostgresEventStore;
use composable_rust_runtime::Store;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = create_db_pool().await?;
    let event_store = PostgresEventStore::new(pool).await?;

    let environment = OrderEnvironment {
        event_store,
        clock: SystemClock::new(),
    };

    let store = Store::new(
        OrderState::default(),
        OrderReducer,
        environment,
    );

    // Place order
    store.send(OrderAction::PlaceOrder {
        customer_id: "cust-123".into(),
        items: vec![/* ... */],
    }).await?;

    // Events are automatically persisted via Effect::AppendEvents

    Ok(())
}
```

### WebSocket Broadcasting

```rust
use composable_rust_web::handlers::websocket;
use axum::{Router, routing::get};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = Store::new(/* ... */);

    // All actions are automatically broadcast to WebSocket clients
    let app = Router::new()
        .route("/ws", get(websocket::handle::<OrderState, OrderAction, _, _>))
        .with_state(store.clone());

    // Start server
    axum::Server::bind(&"0.0.0.0:3000".parse()?)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
```

## Performance

### Reducer Execution

- **Target**: < 1μs per action
- **Typical**: 100-500ns for simple state machines
- **Measured**: See `runtime/benches/` for benchmarks

### Effect Execution

- **Non-blocking**: Effects spawn as Tokio tasks
- **Concurrent**: Multiple effects execute in parallel
- **Backpressure**: DLQ prevents unbounded memory growth

### State Access

- **Reads**: Concurrent (RwLock allows multiple readers)
- **Writes**: Exclusive (RwLock requires exclusive lock for mutations)
- **Typical read latency**: < 10μs

## Error Handling

The Store uses a three-tier error model:

### 1. Reducer Panics → Halt Store

If a reducer panics, the Store's RwLock is poisoned and the Store halts.

```rust
// ❌ DON'T: Panic in reducer
fn reduce(...) {
    panic!("This will poison the Store!");
}
```

**Fix**: Return domain errors as actions.

### 2. Effect Failures → Log and Continue

Effect failures are logged, retried (if configured), and moved to DLQ if max retries exceeded.

```rust
// ✅ Effect failures don't halt the Store
Effect::Future(Box::pin(async {
    match risky_operation().await {
        Ok(result) => Some(Action::Success(result)),
        Err(e) => Some(Action::Failed(e.to_string())),
    }
}))
```

### 3. Domain Errors → Model as Actions

Business logic errors are modeled as actions.

```rust
match validate_order(&order) {
    Ok(()) => vec![Effect::PublishEvent(OrderPlaced { order })],
    Err(e) => vec![Effect::None, Action::OrderValidationFailed { error: e }],
}
```

## Further Reading

- [Getting Started Guide](../docs/getting-started.md) - Tutorial walkthrough
- [Error Handling](../docs/error-handling.md) - Complete error handling guide
- [Observability](../docs/observability.md) - Tracing, metrics, monitoring
- [Core Crate](../core/README.md) - Reducer, Effect, Environment traits
- [Examples](../examples/) - Working code examples

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
