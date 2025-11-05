# Counter Example - Composable Rust Architecture Reference

This example demonstrates the core concepts of the Composable Rust architecture through the simplest possible implementation: a counter that can increment, decrement, and reset.

## Purpose

**Why start with a counter?** Following the principle "make it work, make it right, make it fast," the counter validates our core abstractions before adding complexity. It proves that:

✅ State management works correctly
✅ Actions flow through the system
✅ Reducers are pure and testable
✅ The Store coordinates everything properly
✅ Tests run at memory speed (< 100ms)

**"Can we explain the entire architecture using just the counter example?"** - This is the success criteria for Phase 1.

## Architecture Overview

The Composable Rust architecture is built on **five fundamental types**:

```
Action → Reducer → (State, Effects) → Effect Execution → More Actions
         ↑_______________________________________________|
                    Unidirectional Data Flow
```

### 1. **State** - What We Know

The domain state for our feature. Must be `Clone` for time-travel debugging and snapshots.

```rust
#[derive(Clone, Debug, Default)]
pub struct CounterState {
    pub count: i64,
}
```

**Key principle**: State is owned data, not references. This enables:
- Time-travel debugging (clone any past state)
- Snapshot testing (compare state equality)
- Event sourcing (rebuild state from events)

### 2. **Action** - What Happened

A unified type for all inputs: commands from users, events from the system, messages from other aggregates.

```rust
#[derive(Clone, Debug)]
pub enum CounterAction {
    Increment,
    Decrement,
    Reset,
}
```

**Key principle**: Actions are values describing what happened or should happen. They don't execute anything - that's the reducer's job.

### 3. **Reducer** - How We Respond

A pure function: `(State, Action, Environment) → (New State, Effects)`

```rust
impl Reducer for CounterReducer {
    type State = CounterState;
    type Action = CounterAction;
    type Environment = CounterEnvironment<C>;

    fn reduce(
        &self,
        state: &mut Self::State,  // ✅ Mutable for performance
        action: Self::Action,
        _env: &Self::Environment,
    ) -> Vec<Effect<Self::Action>> {
        match action {
            CounterAction::Increment => {
                state.count += 1;
                vec![Effect::None]  // Pure state machine
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

**Key principles**:
- **Pure**: Same inputs = same outputs, always
- **No I/O**: Database, HTTP, etc. are returned as effect *descriptions*
- **Fast**: Runs at memory speed (<1μs), enabling property-based testing
- **`&mut State`**: We mutate for performance (zero-copy), but it's still pure!

**Why `&mut`?** The reducer owns the state transition. Mutation is an implementation detail - from the caller's perspective, `reduce()` is a pure function.

### 4. **Effect** - What To Do Next

Side effect descriptions (values, not execution). The runtime executes these after the reducer returns.

```rust
pub enum Effect<Action> {
    None,                           // No side effect
    Future(/* async computation */), // Arbitrary async work
    Delay { duration, action },     // Delayed dispatch
    Parallel(Vec<Effect>),          // Concurrent execution
    Sequential(Vec<Effect>),        // Ordered execution

    // Phase 2+: Database, HTTP, Event Publishing
}
```

**Key principle**: Effects are data, not execution. This enables:
- Testing without mocks (effects are values you can assert on)
- Cancellation (effects haven't executed yet)
- Composition (combine/transform effects)
- Time-travel debugging (replay without side effects)

**Counter effects**: The counter is a pure state machine with `Effect::None` for all actions. Future examples will show:
- `Effect::Database(SaveEvent)` - persist state changes
- `Effect::Http { ... }` - call external services
- `Effect::PublishEvent(OrderPlaced)` - notify other aggregates

### 5. **Environment** - Dependencies We Need

Injected dependencies via traits. Enables production/test/dev implementations with static dispatch (zero-cost).

```rust
pub struct CounterEnvironment<C: Clock> {
    clock: C,  // For demonstration (not actually used)
}
```

**Three implementations pattern**:
1. **Production**: Real dependencies (`SystemClock`, `PostgresDatabase`)
2. **Test**: Fast, deterministic mocks (`FixedClock`, `MockDatabase`)
3. **Development**: Instrumented versions (`LoggingDatabase`)

**Why traits?** Static dispatch = zero-cost abstractions. No `Box<dyn>`, no virtual calls.

## The Store - Putting It All Together

The Store is the runtime that coordinates everything:

```rust
let env = CounterEnvironment::new(test_clock());
let store = Store::new(
    CounterState::default(),  // Initial state
    CounterReducer::new(),    // Business logic
    env,                      // Dependencies
);

// Send actions
let handle = store.send(CounterAction::Increment).await;
handle.wait().await;  // Wait for effects (none for counter)

// Read state
let count = store.state(|s| s.count).await;
assert_eq!(count, 1);
```

**What the Store does**:
1. **Action arrives** → Acquire write lock on state
2. **Call reducer** → Pure function runs synchronously
3. **Execute effects** → Spawn async tasks for each effect
4. **Feedback loop** → Effects can produce more actions

**Concurrency**:
- Multiple `send()` calls can happen concurrently
- Reducer execution serializes (one at a time via RwLock)
- Effects execute concurrently (each in its own tokio task)
- State reads block on writes, multiple reads can happen simultaneously

## Testing Philosophy

**Business logic tests run at memory speed - no I/O.**

### Unit Tests - Test the Reducer

```rust
#[test]
fn test_increment() {
    let mut state = CounterState::default();
    let env = CounterEnvironment::new(test_clock());
    let reducer = CounterReducer::new();

    let effects = reducer.reduce(&mut state, CounterAction::Increment, &env);

    assert_eq!(state.count, 1);  // State updated
    assert!(matches!(effects[0], Effect::None));  // No side effects
}
```

**Why fast?** No async, no I/O, just pure functions. Enables:
- Thousands of tests in milliseconds
- Property-based testing with `proptest`
- Mutation testing
- Fuzzing

### Integration Tests - Test the Store

```rust
#[tokio::test]
async fn test_counter_with_store() {
    let env = CounterEnvironment::new(test_clock());
    let store = Store::new(CounterState::default(), CounterReducer::new(), env);

    let _ = store.send(CounterAction::Increment).await;
    let count = store.state(|s| s.count).await;
    assert_eq!(count, 1);
}
```

**What we're testing**: The full end-to-end flow including Store coordination, concurrency, and state management.

### Testing with TestStore

For effects that produce actions (coming in Phase 2+):

```rust
let store = TestStore::new(MyReducer, env, initial_state);

let _ = store.send(Action::TriggerEffect).await;

// Assert on produced actions
store.receive(ExpectedAction).await?;
store.assert_no_pending_actions();
```

**TestStore queues actions** instead of auto-feeding them back, enabling deterministic effect testing.

## Key Architectural Patterns

### Functional Core, Imperative Shell

- **Core** (Reducer): Pure functions, fast, easily tested
- **Shell** (Store + Effects): I/O, side effects, async runtime

Reducers run in microseconds. Effects can take milliseconds or seconds. This separation enables testing business logic without mocking the world.

### Unidirectional Data Flow

```
User Action
  ↓
Store.send()
  ↓
Reducer (pure function)
  ↓
(New State, Effects)
  ↓
Effect Execution (async)
  ↓
More Actions (feedback)
  ↓
Back to Reducer
```

**No callback hell, no event emitters, no bidirectional bindings**. Data flows one way, making it easy to reason about.

### Effects as Values

```rust
// ❌ DON'T: Execute in reducer
fn reduce(...) {
    database.save(state).await;  // NO! This is I/O!
}

// ✅ DO: Return effect description
fn reduce(...) -> Vec<Effect> {
    vec![Effect::Database(SaveState { state })]  // YES! Just data
}
```

The Store executes effects after the reducer returns. This keeps reducers pure and enables:
- Testing effects without execution
- Effect cancellation
- Effect composition and transformation
- Deterministic replay

### Dependency Injection via Traits

```rust
// Generic over Clock implementation
pub struct CounterEnvironment<C: Clock> {
    clock: C,
}

// Production
let env = CounterEnvironment::new(SystemClock::new());

// Tests
let env = CounterEnvironment::new(FixedClock::new(test_time));
```

**Static dispatch** means zero runtime cost. The compiler monomorphizes each implementation.

## Performance Characteristics

Based on benchmarks (`cargo bench -p composable-rust-runtime`):

- **Reducer execution**: < 1μs (target met)
- **Store send+read**: ~1-5μs depending on state size
- **Effect overhead**: Minimal (tokio::spawn cost)
- **Concurrent throughput**: Scales with CPU cores

**Why fast?**
- Zero-cost abstractions (traits, generics)
- Minimal allocations (effects use `Pin<Box>` when needed)
- Lock contention minimized (write locks only during reduce)
- No serialization overhead (in-memory, bincode for Phase 2+)

## Common Questions

### Q: Why `&mut State` if reducers are pure?

**A:** Purity is about referential transparency (same inputs = same outputs), not about mutation. The `&mut` is an optimization - we own the state during reduction, so in-place mutation is safe and fast. The reducer is still pure from the caller's perspective.

### Q: Why not just use `async fn` in the reducer?

**A:** Async functions can hide side effects. By forcing effects to be values, we make all side effects explicit and visible. This enables testing without mocks and replay without re-execution.

### Q: How do I call a database/HTTP API?

**A:** Return an effect description from the reducer. Phase 2 adds:
```rust
vec![Effect::Database(SaveOrder { order })]
```

The Store executes this effect, which may produce a new action on completion.

### Q: What about transactions/sagas?

**A:** Coming in Phase 3. Sagas are just reducers with state machines:
```rust
match (state.current_step, action) {
    (Step1, Event1) => transition_to_step2(),
    (Step2, Failed) => compensate(),
    // ...
}
```

No framework needed - just pure functions managing state transitions.

### Q: How do I test side effects?

**A:** Two ways:
1. **Unit tests**: Assert on effect descriptions (no execution)
2. **Integration tests**: Use `TestStore` to receive and assert actions

Most tests should be unit tests (fast). Integration tests validate the runtime.

## Next Steps

**Phase 2**: Add persistence and event sourcing
- PostgreSQL event store
- Event replay for state reconstruction
- Snapshots for performance

**Phase 3**: Add event bus and sagas
- Redpanda for cross-aggregate events
- Saga pattern for distributed transactions
- Compensation logic

**Phase 4**: Production hardening
- Observability (tracing, metrics)
- Circuit breakers
- Retry policies
- Backpressure

## Running the Example

```bash
# Run the example binary
cargo run -p counter

# Run tests
cargo test -p counter

# Run with debug logging
RUST_LOG=debug cargo run -p counter
```

## Key Takeaways

1. **Five types**: State, Action, Reducer, Effect, Environment
2. **One-way flow**: Action → Reducer → (State, Effects) → More Actions
3. **Pure core**: Reducers are pure functions (< 1μs)
4. **Effects as values**: Side effects are data, not execution
5. **Fast tests**: Business logic tests run at memory speed
6. **Static dispatch**: Zero-cost abstractions via traits

**The counter proves the architecture works.** Everything else builds on these foundations.
