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
    ) -> Vec<Effect<Self::Action>>;
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
- `Vec<Effect<Self::Action>>` - Side effects to execute

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
    ) -> Vec<Effect<Self::Action>> {
        match action {
            CounterAction::Increment => {
                state.count += 1;
                vec![Effect::None]
            },
            CounterAction::Decrement => {
                state.count -= 1;
                vec![Effect::None]
            },
            CounterAction::Reset => {
                state.count = 0;
                vec![Effect::None]
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
    Parallel(Vec<Effect<Action>>),
    Sequential(Vec<Effect<Action>>),
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
Effect::Parallel(vec![
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
Effect::Sequential(vec![
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
    ) -> Vec<Effect<Self::Action>> {
        match action {
            CounterAction::Increment => {
                state.count += 1;
                vec![Effect::None]
            },
            CounterAction::Decrement => {
                state.count -= 1;
                vec![Effect::None]
            },
            CounterAction::Reset => {
                state.count = 0;
                vec![Effect::None]
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
- `event-sourcing` - Database and event store support (Phase 2)
- `event-bus` - Redpanda/Kafka integration (Phase 3)
- `observability` - Metrics and tracing (Phase 4)

---

## See Also

- **Concepts**: [Core Concepts](concepts.md) - Deep dive into architecture
- **Tutorial**: [Getting Started](getting-started.md) - Step-by-step guide
- **Decisions**: [Implementation Decisions](implementation-decisions.md) - Design rationale
- **Source Docs**: `cargo doc --open` - Full inline documentation

---

## Version

This API reference documents **Phase 1** of Composable Rust.

**Stability:** APIs are subject to change before 1.0 release. Semantic versioning will be followed once published to crates.io.
