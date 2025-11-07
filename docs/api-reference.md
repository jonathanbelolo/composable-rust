# API Reference

Complete API documentation for Composable Rust Phase 1. For conceptual explanations, see [Core Concepts](concepts.md).

---

## Module: `composable_rust_core`

Core traits and types. No dependencies on I/O or async runtime.

### Trait: `Reducer`

Pure function trait for business logic.

```rust
pub trait Reducer: Clone + Send + Sync + 'static {
    type State: Clone + Send + Sync + 'static;
    type Action: Send + 'static;
    type Environment: Send + Sync + 'static;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]>;
}
```

#### Associated Types

- **`State`**: Domain state for your feature
  - Must be `Clone` (for snapshots, time-travel debugging)
  - Must be `Send + Sync` (for concurrent access)

- **`Action`**: Unified input type for all events
  - Must be `Send` (can be sent across threads)
  - Typically an enum with variants for different events

- **`Environment`**: Injected dependencies
  - Must be `Send + Sync` (shared across tasks)
  - Generic over trait implementations for testing

#### Method: `reduce`

Pure function that processes actions.

**Parameters:**
- `&self` - The reducer instance
- `state: &mut Self::State` - Mutable reference to state (mutation is an optimization)
- `action: Self::Action` - The action to process
- `env: &Self::Environment` - Dependencies

**Returns:**
- `SmallVec<[Effect<Self::Action>; 4]>` - Side effects to execute (stack-allocated for ≤4 effects)

**Guarantees:**
- **Pure**: Same inputs always produce same outputs
- **Fast**: Should complete in < 1μs
- **No I/O**: All side effects returned as `Effect` values

#### Example

```rust
use composable_rust_core::{reducer::Reducer, effect::Effect};

#[derive(Clone)]
struct CounterReducer;

impl Reducer for CounterReducer {
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

---

### Enum: `Effect<Action>`

Description of a side effect (value, not execution).

```rust
pub enum Effect<Action> {
    None,
    Future(Pin<Box<dyn Future<Output = Option<Action>> + Send>>),
    Delay { duration: Duration, action: Box<Action> },
    Parallel(SmallVec<[Effect<Action>; 4]>),
    Sequential(SmallVec<[Effect<Action>; 4]>),
}
```

#### Variants

##### `Effect::None`

No side effect. Used for pure state machines.

```rust
Effect::None
```

##### `Effect::Future`

Arbitrary async computation that may produce an action.

```rust
Effect::Future(Box::pin(async move {
    let result = some_async_work().await;
    Some(Action::Completed(result))
}))
```

**Returns:**
- `Some(action)` - Feeds action back to reducer
- `None` - No follow-up action

##### `Effect::Delay`

Delayed action dispatch (like JavaScript's `setTimeout`).

```rust
Effect::Delay {
    duration: Duration::from_secs(60),
    action: Box::new(Action::TimeoutExpired),
}
```

**Fields:**
- `duration: Duration` - How long to wait
- `action: Box<Action>` - Action to dispatch after delay

##### `Effect::Parallel`

Execute multiple effects concurrently.

```rust
Effect::Parallel(smallvec![
    Effect::Future(Box::pin(send_email())),
    Effect::Future(Box::pin(send_sms())),
    Effect::Delay { /* ... */ },
])
```

**Behavior:**
- All effects execute concurrently (via `tokio::spawn`)
- Actions from effects are dispatched as they complete
- Order of completion is non-deterministic

##### `Effect::Sequential`

Execute effects in order (next starts after previous completes).

```rust
Effect::Sequential(smallvec![
    Effect::Future(Box::pin(save_to_db())),      // First
    Effect::Future(Box::pin(publish_event())),   // Then
    Effect::Delay { /* notify */ },              // Finally
])
```

**Behavior:**
- Effects execute one at a time in order
- If an effect produces an action, it's dispatched before the next effect starts
- Order of execution is deterministic

#### Methods

##### `map<B, F>(self, f: F) -> Effect<B>`

Transform the action type.

```rust
pub fn map<B, F>(self, f: F) -> Effect<B>
where
    F: Fn(Action) -> B + Send + Sync + 'static + Clone,
    Action: 'static,
    B: Send + 'static,
```

**Example:**

```rust
let effect: Effect<ActionA> = Effect::Delay {
    duration: Duration::from_secs(1),
    action: Box::new(ActionA::Timeout),
};

let transformed: Effect<ActionB> = effect.map(|a| ActionB::from(a));
```

**Use case:** Composing reducers with different action types.

##### `merge(self, other: Effect<Action>) -> Effect<Action>`

Combine two effects in parallel.

```rust
pub fn merge(self, other: Effect<Action>) -> Effect<Action>
```

**Example:**

```rust
let effect1 = Effect::Future(Box::pin(async { Some(Action::A) }));
let effect2 = Effect::Future(Box::pin(async { Some(Action::B) }));

let combined = effect1.merge(effect2);
// Equivalent to: Effect::Parallel(vec![effect1, effect2])
```

##### `chain(self, other: Effect<Action>) -> Effect<Action>`

Combine two effects sequentially.

```rust
pub fn chain(self, other: Effect<Action>) -> Effect<Action>
```

**Example:**

```rust
let effect1 = Effect::Future(Box::pin(save_order()));
let effect2 = Effect::Future(Box::pin(publish_event()));

let chained = effect1.chain(effect2);
// Equivalent to: Effect::Sequential(vec![effect1, effect2])
```

---

### Trait: `Clock`

Provides current time (for dependency injection).

```rust
pub trait Clock: Send + Sync + Clone {
    fn now(&self) -> DateTime<Utc>;
}
```

#### Method: `now`

Returns current UTC time.

**Returns:**
- `DateTime<Utc>` - Current time in UTC

**Implementations:**
- `SystemClock` - Uses `Utc::now()` (production)
- `FixedClock` - Returns fixed time, can be advanced (testing)

#### Example

```rust
use composable_rust_core::clock::Clock;
use chrono::Utc;

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
```

---

## Module: `composable_rust_runtime`

Runtime implementation (Store and effect execution).

### Struct: `Store<S, A, R, E>`

The runtime that coordinates reducers, state, and effects.

```rust
pub struct Store<S, A, R, E>
where
    S: Clone + Send + Sync + 'static,
    A: Send + 'static,
    R: Reducer<State = S, Action = A, Environment = E>,
    E: Send + Sync + 'static,
{
    // Internal fields (private)
}
```

**Type Parameters:**
- `S` - State type
- `A` - Action type
- `R` - Reducer type
- `E` - Environment type

**Traits:**
- `Clone` - Store can be cheaply cloned (all clones share same state)
- `Send + Sync` - Store can be shared across threads

#### Constructor: `new`

Creates a new store with initial state.

```rust
pub fn new(initial_state: S, reducer: R, environment: E) -> Self
```

**Parameters:**
- `initial_state: S` - Starting state
- `reducer: R` - Business logic
- `environment: E` - Dependencies

**Returns:**
- `Store` - New store instance

**Example:**

```rust
use composable_rust_runtime::Store;

let env = CounterEnvironment::new(test_clock());
let store = Store::new(
    CounterState::default(),
    CounterReducer::new(),
    env,
);
```

#### Method: `send`

Dispatches an action through the reducer.

```rust
pub async fn send(&self, action: A) -> EffectHandle
```

**Parameters:**
- `action: A` - Action to process

**Returns:**
- `EffectHandle` - Handle to track effect completion

**Behavior:**
1. Acquires write lock on state
2. Calls `reducer.reduce(&mut state, action, &env)`
3. Releases lock
4. Spawns effects for execution
5. Returns handle immediately (non-blocking)

**Example:**

```rust
let handle = store.send(CounterAction::Increment).await;
handle.wait().await;  // Optional: wait for effects to complete
```

**Note:** You must `.await` the `send()` call, but you don't need to `.await` the handle unless you want to wait for effects.

#### Method: `state`

Reads current state via closure.

```rust
pub async fn state<F, T>(&self, f: F) -> T
where
    F: FnOnce(&S) -> T,
```

**Parameters:**
- `f: F` - Closure that receives `&State` and returns some value

**Returns:**
- `T` - Whatever the closure returns

**Behavior:**
1. Acquires read lock on state
2. Calls closure with `&state`
3. Returns result
4. Releases lock

**Example:**

```rust
let count = store.state(|s| s.count).await;
println!("Current count: {count}");

// Clone entire state for snapshot
let snapshot = store.state(|s| s.clone()).await;
```

**Concurrency:**
- Multiple `state()` calls can happen concurrently (read lock)
- `state()` blocks if `send()` is currently executing (write lock)

---

### Struct: `EffectHandle`

Handle for tracking effect completion.

```rust
pub struct EffectHandle {
    // Internal fields (private)
}
```

#### Method: `wait`

Waits for all tracked effects to complete.

```rust
pub async fn wait(&mut self)
```

**Behavior:**
- **Direct mode** (default): Waits only for immediate effects
- **Cascading mode**: Waits for entire effect tree (including effects from actions produced by effects)

**Example:**

```rust
let mut handle = store.send(Action::TriggerEffects).await;
handle.wait().await;  // Blocks until effects complete

// Now it's safe to assert on state changes from effect feedback
let status = store.state(|s| s.status).await;
assert_eq!(status, Status::Completed);
```

**Use cases:**
- Integration tests: Wait for effects before asserting
- Shutdown: Ensure all effects complete before exiting
- Debugging: Synchronize on effect completion

#### Method: `cascading`

Switches to cascading tracking mode.

```rust
pub fn cascading(&mut self)
```

**Behavior:**
- Tracks entire effect tree, not just immediate effects
- Effects that produce actions that produce more effects are all tracked

**Example:**

```rust
let mut handle = store.send(Action::StartChain).await;
handle.cascading();  // Track the whole chain
handle.wait().await;  // Wait for entire cascade
```

**When to use:**
- Integration tests with effect chains
- When you need to wait for a complete workflow
- Default (direct mode) is sufficient for most cases

---

## Module: `composable_rust_testing`

Test utilities for deterministic testing.

### Struct: `TestStore<S, A, R, E>`

Store variant that queues actions instead of auto-dispatching.

```rust
pub struct TestStore<S, A, R, E>
where
    S: Clone + Send + Sync + 'static,
    A: Send + Clone + 'static,
    R: Reducer<State = S, Action = A, Environment = E>,
    E: Send + Sync + 'static,
{
    // Internal fields (private)
}
```

**Purpose:** Deterministic testing of effects that produce actions.

#### Constructor: `new`

Creates a new test store.

```rust
pub fn new(reducer: R, environment: E, initial_state: S) -> Self
```

**Parameters:**
- `reducer: R` - Business logic
- `environment: E` - Dependencies
- `initial_state: S` - Starting state

**Returns:**
- `TestStore` - New test store instance

#### Method: `send`

Dispatches an action.

```rust
pub async fn send(&self, action: A) -> EffectHandle
```

**Behavior:**
- Same as `Store::send()`
- Actions produced by effects are queued, not auto-dispatched

#### Method: `receive`

Asserts next queued action matches expected.

```rust
pub async fn receive(&self, expected: A) -> Result<(), TestStoreError>
where
    A: Debug + PartialEq,
```

**Parameters:**
- `expected: A` - Expected action

**Returns:**
- `Ok(())` - Action matched
- `Err(TestStoreError)` - Action didn't match or queue empty

**Example:**

```rust
let store = TestStore::new(MyReducer, env, initial_state);

// Trigger effect that produces action
let _ = store.send(Action::TriggerEffect).await;

// Assert on produced action
store.receive(Action::EffectCompleted).await?;
store.assert_no_pending_actions();
```

#### Method: `receive_unordered`

Receives multiple actions in any order.

```rust
pub async fn receive_unordered(
    &self,
    expected: Vec<A>,
) -> Result<(), TestStoreError>
where
    A: Debug + PartialEq,
```

**Parameters:**
- `expected: Vec<A>` - Expected actions (order doesn't matter)

**Returns:**
- `Ok(())` - All actions matched
- `Err(TestStoreError)` - Mismatch

**Example:**

```rust
let _ = store.send(Action::ParallelEffects).await;

// These actions might arrive in any order
store.receive_unordered(vec![
    Action::EmailSent,
    Action::SmsSent,
    Action::LogSaved,
]).await?;
```

#### Method: `assert_no_pending_actions`

Asserts action queue is empty.

```rust
pub fn assert_no_pending_actions(&self)
```

**Panics:** If queue is not empty

**Example:**

```rust
let _ = store.send(Action::NoEffects).await;
store.assert_no_pending_actions();  // Pass: no actions queued
```

#### Method: `state`

Reads current state (same as `Store::state`).

```rust
pub async fn state<F, T>(&self, f: F) -> T
where
    F: FnOnce(&S) -> T,
```

---

### Struct: `FixedClock`

Clock implementation with fixed time (for testing).

```rust
#[derive(Debug, Clone)]
pub struct FixedClock {
    time: Arc<RwLock<DateTime<Utc>>>,
}
```

**Purpose:** Deterministic time for tests.

#### Constructor: `new`

Creates clock with fixed time.

```rust
pub fn new(time: DateTime<Utc>) -> Self
```

**Parameters:**
- `time: DateTime<Utc>` - Initial time

**Returns:**
- `FixedClock` - New fixed clock

**Example:**

```rust
use chrono::Utc;
use composable_rust_testing::FixedClock;

let clock = FixedClock::new(Utc::now());
```

#### Method: `now`

Returns current time (implements `Clock` trait).

```rust
pub fn now(&self) -> DateTime<Utc>
```

**Returns:**
- `DateTime<Utc>` - Current fixed time

#### Method: `advance`

Advances time by duration.

```rust
pub fn advance(&self, duration: Duration)
```

**Parameters:**
- `duration: Duration` - How much to advance

**Example:**

```rust
let clock = FixedClock::new(test_time);

// Initial time
let before = clock.now();

// Advance 1 hour
clock.advance(Duration::from_secs(3600));

let after = clock.now();
assert_eq!(after - before, Duration::from_secs(3600));
```

#### Method: `set`

Sets time to specific value.

```rust
pub fn set(&self, time: DateTime<Utc>)
```

**Parameters:**
- `time: DateTime<Utc>` - New time

**Example:**

```rust
use chrono::Utc;

let clock = FixedClock::new(Utc::now());

// Jump to specific time
let specific_time = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
clock.set(specific_time);

assert_eq!(clock.now(), specific_time);
```

---

### Function: `test_clock`

Convenience function for creating test clock.

```rust
pub fn test_clock() -> FixedClock
```

**Returns:**
- `FixedClock` - Clock set to fixed test time

**Example:**

```rust
use composable_rust_testing::test_clock;

let env = MyEnvironment::new(test_clock());
let store = Store::new(initial_state, reducer, env);
```

**Note:** Uses `2025-01-01 00:00:00 UTC` as default test time.

---

## Module: `composable_rust_macros`

Procedural macros for reducing boilerplate (Section 3).

### Derive Macro: `Action`

Generates helper methods for action enums.

```rust
#[proc_macro_derive(Action, attributes(command, event))]
pub fn derive_action(input: TokenStream) -> TokenStream
```

#### Attributes

- `#[command]` - Mark a variant as a command
- `#[event]` - Mark a variant as an event

#### Generated Methods

**`is_command() -> bool`**
Returns `true` if the action is marked with `#[command]`.

**`is_event() -> bool`**
Returns `true` if the action is marked with `#[event]`.

**`event_type() -> &'static str`**
Returns the event type name for serialization (e.g., `"OrderPlaced.v1"`).
Returns `"unknown"` for non-events.

#### Example

```rust
use composable_rust_macros::Action;

#[derive(Action, Clone, Debug)]
enum OrderAction {
    #[command]
    PlaceOrder { customer_id: String, items: Vec<LineItem> },

    #[command]
    CancelOrder { order_id: String, reason: String },

    #[event]
    OrderPlaced { order_id: String, timestamp: DateTime<Utc> },

    #[event]
    OrderCancelled { order_id: String, reason: String },
}

// Usage:
let command = OrderAction::PlaceOrder {
    customer_id: "cust-1".into(),
    items: vec![],
};
assert!(command.is_command());
assert!(!command.is_event());

let event = OrderAction::OrderPlaced {
    order_id: "order-123".into(),
    timestamp: Utc::now(),
};
assert!(event.is_event());
assert_eq!(event.event_type(), "OrderPlaced.v1");  // Versioned for schema evolution
```

#### Benefits

- **CQRS enforcement**: Compile-time distinction between commands and events
- **Event sourcing**: Auto-generated event types for serialization
- **Zero boilerplate**: No manual `match` statements

---

### Derive Macro: `State`

Generates version tracking methods for event-sourced state.

```rust
#[proc_macro_derive(State, attributes(version))]
pub fn derive_state(input: TokenStream) -> TokenStream
```

#### Attributes

- `#[version]` - Mark a field as the version tracker (must be `Option<Version>`)

#### Generated Methods

**`version() -> Option<Version>`**
Returns the current version of the state.

**`set_version(&mut self, version: Version)`**
Sets the version of the state.

#### Example

```rust
use composable_rust_macros::State;
use composable_rust_core::stream::Version;

#[derive(State, Clone, Debug)]
pub struct OrderState {
    pub order_id: Option<String>,
    pub items: Vec<OrderItem>,
    pub status: OrderStatus,

    #[version]
    pub version: Option<Version>,
}

// Usage:
let mut state = OrderState::default();
assert_eq!(state.version(), None);

state.set_version(Version::new(5));
assert_eq!(state.version(), Some(Version::new(5)));
```

#### Benefits

- **Optimistic concurrency**: Version tracking for event store operations
- **Clean API**: No manual getter/setter boilerplate
- **Type safety**: Compile-time enforcement of `Option<Version>` type

---

## Effect Helper Macros

Declarative macros for common effect patterns (Section 3).

### Macro: `append_events!`

Creates `Effect::EventStore(AppendEvents)` with clean syntax.

```rust
append_events! {
    store: $event_store,
    stream: $stream_id,
    expected_version: $expected_version,
    events: $events,
    on_success: |$version| $success_body,
    on_error: |$error| $error_body
}
```

#### Parameters

- `store` - `Arc<dyn EventStore>` to use
- `stream` - Stream ID (converted to `StreamId::new()`)
- `expected_version` - `Option<Version>` for optimistic concurrency
- `events` - `Vec<SerializedEvent>` to append
- `on_success` - Closure receiving `Version`, returns `Option<Action>`
- `on_error` - Closure receiving error, returns `Option<Action>`

#### Example

```rust
use composable_rust_core::append_events;

// Instead of 18 lines:
Effect::EventStore(EventStoreOperation::AppendEvents {
    event_store: Arc::clone(&env.event_store),
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

// Write 7 lines (60% reduction):
append_events! {
    store: env.event_store,
    stream: "order-123",
    expected_version: Some(Version::new(5)),
    events: vec![event],
    on_success: |version| Some(OrderAction::EventsAppended { version }),
    on_error: |err| Some(OrderAction::AppendFailed { error: err.to_string() })
}
```

---

### Macro: `load_events!`

Creates `Effect::EventStore(LoadEvents)` with clean syntax.

```rust
load_events! {
    store: $event_store,
    stream: $stream_id,
    from_version: $from_version,
    on_success: |$events| $success_body,
    on_error: |$error| $error_body
}
```

#### Example

```rust
use composable_rust_core::load_events;

load_events! {
    store: env.event_store,
    stream: "order-123",
    from_version: None,
    on_success: |events| Some(OrderAction::EventsLoaded { events }),
    on_error: |err| Some(OrderAction::LoadFailed { error: err.to_string() })
}
```

---

### Macro: `async_effect!`

Creates `Effect::Future` from async block.

```rust
async_effect! {
    $body
}
```

#### Example

```rust
use composable_rust_core::async_effect;

async_effect! {
    let response = http_client.get("https://api.example.com").await?;
    Some(OrderAction::ResponseReceived { response })
}
```

---

### Macro: `delay!`

Creates `Effect::Delay` with clean syntax.

```rust
delay! {
    duration: $duration,
    action: $action
}
```

#### Example

```rust
use composable_rust_core::delay;
use std::time::Duration;

delay! {
    duration: Duration::from_secs(30),
    action: OrderAction::TimeoutExpired
}
```

---

### Struct: `ReducerTest<S, A, R, E>`

Fluent builder for testing reducers (Section 3).

```rust
pub struct ReducerTest<S, A, R, E>
where
    S: Clone + Send + Sync + 'static,
    A: Send + Clone + 'static,
    R: Reducer<State = S, Action = A, Environment = E>,
    E: Send + Sync + 'static,
```

**Purpose:** Given-When-Then style testing for reducers.

#### Constructor: `new`

```rust
pub fn new(reducer: R, environment: E) -> Self
```

**Parameters:**
- `reducer: R` - Reducer to test
- `environment: E` - Test environment

**Returns:**
- `ReducerTest` - Builder instance

#### Method: `given_state`

Sets initial state for test.

```rust
pub fn given_state(self, state: S) -> Self
```

#### Method: `when_action`

Specifies single action to dispatch.

```rust
pub fn when_action(self, action: A) -> Self
```

#### Method: `when_actions`

Specifies multiple actions to dispatch sequentially.

```rust
pub fn when_actions(self, actions: Vec<A>) -> Self
```

#### Method: `then_state`

Asserts on final state.

```rust
pub fn then_state<F>(self, assertion: F) -> Self
where
    F: FnOnce(&S),
```

#### Method: `assert_has_event_store_effect`

Asserts at least one `Effect::EventStore` was returned.

```rust
pub fn assert_has_event_store_effect(self) -> Self
```

#### Method: `assert_effect_count`

Asserts exact number of effects.

```rust
pub fn assert_effect_count(self, count: usize) -> Self
```

#### Method: `run`

Executes the test (consumes builder).

```rust
pub fn run(self)
```

#### Example

```rust
use composable_rust_testing::ReducerTest;

#[test]
fn test_place_order() {
    ReducerTest::new(OrderReducer, test_environment())
        .given_state(OrderState::default())
        .when_action(OrderAction::PlaceOrder {
            customer_id: "cust-1".into(),
            items: vec![test_item()],
        })
        .then_state(|state| {
            assert_eq!(state.status, OrderStatus::Placed);
            assert_eq!(state.items.len(), 1);
        })
        .assert_has_event_store_effect()
        .run();
}

#[test]
fn test_order_lifecycle() {
    ReducerTest::new(OrderReducer, test_environment())
        .given_state(OrderState::default())
        .when_actions(vec![
            OrderAction::PlaceOrder { /* ... */ },
            OrderAction::ShipOrder { /* ... */ },
        ])
        .then_state(|state| {
            assert_eq!(state.status, OrderStatus::Shipped);
        })
        .assert_effect_count(2)
        .run();
}
```

#### Benefits

- **Readable tests**: Given-When-Then makes intent clear
- **Composable**: Chain multiple assertions
- **Type-safe**: Full compile-time checking
- **No async**: Synchronous testing of pure reducers

---

## Complete Example

Putting it all together:

```rust
use composable_rust_core::{reducer::Reducer, effect::Effect, clock::Clock};
use composable_rust_runtime::Store;
use composable_rust_testing::{test_clock, TestStore};

// 1. Define State
#[derive(Clone, Debug, Default)]
struct CounterState {
    count: i64,
}

// 2. Define Actions
#[derive(Clone, Debug, PartialEq)]
enum CounterAction {
    Increment,
    Decrement,
    Reset,
}

// 3. Define Environment
struct CounterEnvironment<C: Clock> {
    clock: C,
}

impl<C: Clock> CounterEnvironment<C> {
    fn new(clock: C) -> Self {
        Self { clock }
    }
}

// 4. Implement Reducer
#[derive(Clone)]
struct CounterReducer;

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

// 5. Use Store
#[tokio::main]
async fn main() {
    let env = CounterEnvironment::new(test_clock());
    let store = Store::new(CounterState::default(), CounterReducer, env);

    // Send actions
    let _ = store.send(CounterAction::Increment).await;
    let _ = store.send(CounterAction::Increment).await;

    // Read state
    let count = store.state(|s| s.count).await;
    println!("Count: {count}");  // Count: 2
}

// 6. Test with TestStore
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reducer() {
        let mut state = CounterState::default();
        let env = CounterEnvironment::new(test_clock());
        let reducer = CounterReducer;

        let effects = reducer.reduce(
            &mut state,
            CounterAction::Increment,
            &env,
        );

        assert_eq!(state.count, 1);
        assert!(matches!(effects[0], Effect::None));
    }

    #[tokio::test]
    async fn test_store() {
        let env = CounterEnvironment::new(test_clock());
        let store = Store::new(CounterState::default(), CounterReducer, env);

        let _ = store.send(CounterAction::Increment).await;
        let count = store.state(|s| s.count).await;

        assert_eq!(count, 1);
    }
}
```

---

## Error Types

### `TestStoreError`

Errors from TestStore assertions.

```rust
pub enum TestStoreError {
    ActionMismatch {
        expected: String,
        actual: String,
    },
    NoActionReceived {
        expected: String,
    },
    UnexpectedAction {
        action: String,
    },
}
```

**Variants:**

- `ActionMismatch` - Received action didn't match expected
- `NoActionReceived` - Queue was empty when expecting action
- `UnexpectedAction` - Action left in queue (caught by drop guard)

---

## Cargo Features

Currently no optional features. All APIs are always available.

Future phases will add:
- `event-bus` - Redpanda/Kafka integration (Phase 3)
- `observability` - Metrics and tracing (Phase 4)

---

## See Also

- **Concepts**: [Core Concepts](concepts.md) - Deep dive into architecture
- **Tutorial**: [Getting Started](getting-started.md) - Step-by-step guide
- **Decisions**: [Implementation Decisions](implementation-decisions.md) - Design rationale
- **Macro Reference**: Effect macros in [`core/src/effect_macros.rs`](../core/src/effect_macros.rs)
- **Source Docs**: `cargo doc --open` - Full inline documentation

---

## Version

This API reference documents Composable Rust through **Phase 2** and **Section 3**:

- ✅ **Phase 1**: Core abstractions (Reducer, Effect, Store, Environment)
- ✅ **Phase 2**: Event sourcing & persistence (EventStore, Version tracking)
- ✅ **Section 3**: Developer tools & macros (`#[derive(Action)]`, `#[derive(State)]`, effect macros, ReducerTest)

**Stability:** APIs are subject to change before 1.0 release. Semantic versioning will be followed once published to crates.io.
