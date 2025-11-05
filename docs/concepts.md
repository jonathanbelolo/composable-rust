# Core Concepts

This document provides a deep dive into the fundamental concepts of Composable Rust. For a tutorial introduction, see [Getting Started](getting-started.md).

## Overview

Composable Rust is built on a simple but powerful idea: **separate pure business logic from side effects**. This separation enables:

- **Fast tests** - Business logic runs at memory speed (no I/O)
- **Deterministic behavior** - Same inputs always produce same outputs
- **Time-travel debugging** - Replay state transitions without re-executing effects
- **Clear architecture** - Unidirectional data flow makes reasoning easy

The architecture is built on **five fundamental types** that compose together:

```
Action → Reducer → (State, Effects) → Effect Execution → More Actions
         ↑_________________________________________________|
                      Unidirectional Data Flow
```

## The Five Fundamental Types

### 1. State

**State represents what your feature knows about the world.**

#### Definition

```rust
#[derive(Clone, Debug)]
pub struct MyState {
    // Domain data
}
```

#### Requirements

- **Must be `Clone`**: Enables snapshots, time-travel debugging, event replay
- **Must be `Debug`**: Helps with logging and debugging
- **Owned data**: No references (state must be self-contained)
- **Public fields**: Makes testing easier (no getters/setters needed)

#### Design Principles

**Principle 1: State is owned data, not views**

```rust
// ✅ GOOD: Owned data
pub struct OrderState {
    order_id: String,
    items: Vec<OrderItem>,
    total: Decimal,
}

// ❌ BAD: References (can't be cloned)
pub struct OrderState<'a> {
    order_id: &'a str,
    items: &'a [OrderItem],
}
```

**Principle 2: State should be as simple as possible**

```rust
// ✅ GOOD: Plain data
pub struct CounterState {
    count: i64,
}

// ❌ BAD: Embedded dependencies
pub struct CounterState {
    count: i64,
    database: Arc<dyn Database>,  // Dependencies belong in Environment
}
```

**Principle 3: State is the source of truth**

All domain knowledge lives in state. Don't duplicate state in the environment or effects.

#### Why `Clone`?

1. **Snapshots**: Store can clone state for time-travel debugging
2. **Event Sourcing**: Rebuild state by replaying events
3. **Testing**: Easily compare expected vs actual state
4. **Concurrency**: Multiple readers can each have their own copy

**Performance concern?** Cloning is fast:
- Small states (< 1KB): Negligible overhead
- Large states: Use `Arc<T>` for expensive fields
- Event sourcing: Clone only for snapshots, not every action

#### State vs View

State is **domain data**, not UI data. If you're building a backend system:

```rust
// ✅ State: Domain model
pub struct OrderState {
    order_id: String,
    items: Vec<OrderItem>,
    status: OrderStatus,
    total: Decimal,
}

// ❌ Not State: Presentation logic (this goes in the frontend)
pub struct OrderViewModel {
    formatted_total: String,
    display_color: Color,
}
```

---

### 2. Action

**Actions represent events that happen in your system.**

#### Definition

```rust
#[derive(Clone, Debug)]
pub enum MyAction {
    // Command from user
    PlaceOrder { items: Vec<Item> },

    // Event from system
    OrderPlaced { order_id: String },

    // Response from effect
    PaymentSucceeded { transaction_id: String },
}
```

#### Requirements

- **Must be `Clone`**: Actions can be logged, replayed, or duplicated
- **Must be `Debug`**: Essential for logging and debugging
- **Typically an enum**: Variants represent different event types
- **Should be values**: Actions describe what happened, not what to do

#### Design Principles

**Principle 1: Actions are values, not commands**

```rust
// ✅ GOOD: Describes what happened
pub enum OrderAction {
    OrderPlaced { order_id: String, items: Vec<Item> },
    PaymentReceived { transaction_id: String },
}

// ❌ BAD: Contains behavior
pub enum OrderAction {
    PlaceOrder {
        order: Order,
        on_success: Box<dyn Fn()>,  // NO! Actions are data
    },
}
```

**Principle 2: Actions unify all inputs**

Everything is an action:
- User commands: `Action::CreateOrder`
- System events: `Action::OrderCreated`
- Effect responses: `Action::PaymentCompleted`
- Timer events: `Action::TimeoutExpired`
- External messages: `Action::RefundRequested`

This unification simplifies the architecture - there's only one input type.

**Principle 3: Actions should be past tense for events**

```rust
// ✅ GOOD: Events in past tense
OrderPlaced
PaymentReceived
OrderCancelled

// ❌ BAD: Commands in imperative mood (use for user input)
PlaceOrder   // OK for command
ReceivePayment  // Better: PaymentReceived
CancelOrder  // OK for command
```

#### Action Design Patterns

**Pattern 1: Commands vs Events**

```rust
pub enum OrderAction {
    // Commands (from user/external system)
    PlaceOrder { items: Vec<Item> },
    CancelOrder { reason: String },

    // Events (from our system)
    OrderPlaced { order_id: String },
    OrderCancelled { order_id: String, reason: String },

    // Responses (from effects)
    PaymentSucceeded { transaction_id: String },
    PaymentFailed { error: String },
}
```

**Pattern 2: Result Actions**

Effects that can fail should produce Result-style actions:

```rust
pub enum OrderAction {
    // Request
    ChargePayment { amount: Decimal },

    // Result
    PaymentSucceeded { transaction_id: String },
    PaymentFailed { error: String },
}
```

**Pattern 3: Cross-Aggregate Events**

For Phase 3+ (event bus), actions can represent events from other aggregates:

```rust
pub enum OrderAction {
    // Local events
    OrderPlaced { order_id: String },

    // Events from Inventory aggregate
    InventoryReserved { order_id: String, items: Vec<Item> },
    InventoryReservationFailed { order_id: String, reason: String },
}
```

---

### 3. Reducer

**Reducers are pure functions that implement business logic.**

#### Definition

```rust
impl Reducer for MyReducer {
    type State = MyState;
    type Action = MyAction;
    type Environment = MyEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> Vec<Effect<Self::Action>> {
        // Business logic here
        vec![Effect::None]
    }
}
```

#### Requirements

- **Must be pure**: Same inputs always produce same outputs
- **No I/O**: Database, HTTP, file system are returned as effects
- **No hidden state**: All state is in `State`, all deps in `Environment`
- **Fast**: Should complete in microseconds (< 1μs target)

#### Why Pure Functions?

**Benefit 1: Trivial to test**

```rust
#[test]
fn test_place_order() {
    let mut state = OrderState::default();
    let reducer = OrderReducer;
    let env = test_environment();

    let effects = reducer.reduce(
        &mut state,
        OrderAction::PlaceOrder { items },
        &env,
    );

    // No async, no mocks, just assertions
    assert_eq!(state.status, OrderStatus::Placed);
    assert!(matches!(effects[0], Effect::Database(_)));
}
```

**Benefit 2: Property-based testing**

```rust
#[test]
fn test_total_never_negative() {
    proptest!(|(actions: Vec<OrderAction>)| {
        let mut state = OrderState::default();
        for action in actions {
            let _ = reducer.reduce(&mut state, action, &env);
            assert!(state.total >= Decimal::ZERO);
        }
    });
}
```

**Benefit 3: Time-travel debugging**

Because reducers are pure, you can replay any sequence of actions to reconstruct state at any point in time.

#### The `&mut State` Question

**Why mutate if we're functional?**

Performance. Copying large state structs on every action would be wasteful. But the reducer is still pure:

```rust
// What the compiler sees (mutation for performance)
fn reduce(&self, state: &mut State, action: Action) -> Vec<Effect> {
    state.count += 1;
    vec![Effect::None]
}

// What we reason about (pure function)
// reduce(state, action) -> (new_state, effects)
// Same inputs = same outputs, always
```

The mutation is an **implementation detail**. From the caller's perspective (Store), `reduce()` is a pure function with no observable side effects.

**Trade-off**: We lose structural sharing (like persistent data structures), but we gain:
- 10-100x faster execution
- Simpler code (no builder pattern needed)
- Familiar imperative style for business logic

See `docs/implementation-decisions.md` for the full analysis.

#### Reducer Patterns

**Pattern 1: State machine**

```rust
fn reduce(&self, state: &mut State, action: Action, env: &Env) -> Vec<Effect> {
    match (state.status, action) {
        (Status::Draft, Action::Submit) => {
            state.status = Status::Pending;
            vec![Effect::Database(SaveState)]
        },
        (Status::Pending, Action::Approve) => {
            state.status = Status::Approved;
            vec![Effect::PublishEvent(OrderApproved)]
        },
        (Status::Pending, Action::Reject) => {
            state.status = Status::Rejected;
            vec![Effect::PublishEvent(OrderRejected)]
        },
        _ => vec![Effect::None],  // Invalid transitions ignored
    }
}
```

**Pattern 2: Validation then mutation**

```rust
fn reduce(&self, state: &mut State, action: Action, env: &Env) -> Vec<Effect> {
    match action {
        Action::PlaceOrder { items } => {
            // 1. Validate
            if items.is_empty() {
                return vec![Effect::None];  // Or return error action
            }
            if state.status != Status::Draft {
                return vec![Effect::None];  // Can't place twice
            }

            // 2. Mutate state
            state.status = Status::Placed;
            state.items = items;
            state.placed_at = env.clock.now();

            // 3. Return effects
            vec![
                Effect::Database(SaveOrder),
                Effect::PublishEvent(OrderPlaced),
            ]
        },
        // ...
    }
}
```

**Pattern 3: Effect composition**

```rust
fn reduce(&self, state: &mut State, action: Action, env: &Env) -> Vec<Effect> {
    match action {
        Action::PlaceOrder { items } => {
            state.status = Status::Placed;

            // Sequential: Save first, then publish event
            vec![Effect::Sequential(vec![
                Effect::Database(SaveOrder),
                Effect::PublishEvent(OrderPlaced),
            ])]
        },
        Action::NotifyCustomer => {
            // Parallel: Send email and SMS concurrently
            vec![Effect::Parallel(vec![
                Effect::Http { /* send email */ },
                Effect::Http { /* send SMS */ },
            ])]
        },
        // ...
    }
}
```

---

### 4. Effect

**Effects are descriptions of side effects, not execution.**

#### Definition

```rust
pub enum Effect<Action> {
    None,
    Future(Pin<Box<dyn Future<Output = Option<Action>> + Send>>),
    Delay { duration: Duration, action: Box<Action> },
    Parallel(Vec<Effect<Action>>),
    Sequential(Vec<Effect<Action>>),
}
```

#### Key Insight: Effects Are Values

This is the most important concept in the architecture:

```rust
// ❌ BAD: Executing side effect in reducer
fn reduce(...) {
    env.database.save(state).await;  // NO! This is I/O!
}

// ✅ GOOD: Returning effect description
fn reduce(...) -> Vec<Effect> {
    vec![Effect::Database(SaveState)]  // YES! Just data
}
```

**Why this matters:**

1. **Testing without mocks**: Assert on effect values
   ```rust
   let effects = reducer.reduce(...);
   assert!(matches!(effects[0], Effect::Database(SaveOrder)));
   // No mocking, no I/O, just value comparison
   ```

2. **Time-travel debugging**: Replay without side effects
   ```rust
   for action in history {
       let effects = reducer.reduce(&mut state, action, &env);
       // Effects aren't executed, so no duplicate DB writes
   }
   ```

3. **Effect cancellation**: Effects haven't run yet
   ```rust
   let effects = reducer.reduce(...);
   if should_cancel {
       return;  // Effects never execute
   }
   store.execute(effects);
   ```

4. **Effect composition**: Transform and combine effects
   ```rust
   let effect = Effect::Database(Save);
   let transformed = effect.map(|action| NewAction::from(action));
   ```

#### Effect Variants (Phase 1)

**`Effect::None`**
No side effect. Used for pure state machines.

```rust
CounterAction::Increment => {
    state.count += 1;
    vec![Effect::None]
}
```

**`Effect::Future`**
Arbitrary async computation that may produce an action.

```rust
Effect::Future(Box::pin(async move {
    let result = some_async_work().await;
    Some(Action::WorkCompleted(result))
}))
```

**`Effect::Delay`**
Delayed action dispatch (like `setTimeout` in JavaScript).

```rust
Effect::Delay {
    duration: Duration::from_secs(60),
    action: Box::new(Action::TimeoutExpired),
}
```

**`Effect::Parallel`**
Execute multiple effects concurrently.

```rust
Effect::Parallel(vec![
    Effect::Http { /* email */ },
    Effect::Http { /* SMS */ },
    Effect::Database(SaveLog),
])
```

**`Effect::Sequential`**
Execute effects in order (next starts after previous completes).

```rust
Effect::Sequential(vec![
    Effect::Database(SaveOrder),      // First
    Effect::PublishEvent(OrderPlaced), // Then
    Effect::Http { /* notify */ },     // Finally
])
```

#### Effect Composition

Effects can be transformed and combined:

**`Effect::map()`** - Transform action type

```rust
let effect: Effect<ActionA> = Effect::Delay { /* ... */ };
let transformed: Effect<ActionB> = effect.map(|a| ActionB::from(a));
```

**`Effect::merge()`** - Combine effects in parallel

```rust
let effect1 = Effect::Database(Save);
let effect2 = Effect::Http { /* ... */ };
let combined = effect1.merge(effect2);
// Equivalent to: Effect::Parallel(vec![effect1, effect2])
```

**`Effect::chain()`** - Combine effects sequentially

```rust
let effect1 = Effect::Database(Save);
let effect2 = Effect::PublishEvent(Saved);
let chained = effect1.chain(effect2);
// Equivalent to: Effect::Sequential(vec![effect1, effect2])
```

See `core/src/lib.rs` for implementation details and tests.

#### The Feedback Loop

Effects can produce actions, which feed back into the reducer:

```
1. User → Action::ChargePayment
2. Reducer → (State, [Effect::Http { charge API }])
3. Effect executes → HTTP call completes
4. Effect → Some(Action::PaymentSucceeded)
5. Action feeds back → Reducer processes PaymentSucceeded
6. Loop continues...
```

This creates a **self-sustaining event loop** where everything is an action.

---

### 5. Environment

**Environment provides dependencies via dependency injection.**

#### Definition

```rust
pub struct MyEnvironment<D, C>
where
    D: Database,
    C: Clock,
{
    pub database: D,
    pub clock: C,
}
```

#### Purpose

Environment holds all **external dependencies** your reducer needs:
- **Time**: `Clock` trait (system time vs fixed time for tests)
- **I/O**: `Database`, `HttpClient`, `EventPublisher` traits
- **Configuration**: Feature flags, rate limits, etc.
- **Identity**: ID generators, random number generators

#### The Three Implementations Pattern

For every dependency, implement three versions:

```rust
// 1. Production: Real implementation
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

// 2. Test: Fast, deterministic mock
pub struct FixedClock {
    time: Arc<RwLock<DateTime<Utc>>>,
}
impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self.time.read().expect("lock poisoned")
    }
}

// 3. Development: Instrumented version
pub struct LoggingClock<C: Clock> {
    inner: C,
}
impl<C: Clock> Clock for LoggingClock<C> {
    fn now(&self) -> DateTime<Utc> {
        let now = self.inner.now();
        tracing::debug!("Clock::now() -> {now}");
        now
    }
}
```

#### Static Dispatch

Environment uses **generic parameters**, not trait objects:

```rust
// ✅ GOOD: Static dispatch
pub struct Environment<D: Database, C: Clock> {
    database: D,
    clock: C,
}

// ❌ BAD: Dynamic dispatch
pub struct Environment {
    database: Box<dyn Database>,
    clock: Box<dyn Clock>,
}
```

**Why static dispatch?**
- **Zero cost**: Compiler monomorphizes each implementation
- **Inlining**: Functions can be inlined across trait boundaries
- **No heap allocation**: No `Box`, just stack data
- **Better optimization**: Dead code elimination, constant folding

**Trade-off**: More verbose type signatures, but faster runtime and zero overhead.

#### Environment Design Principles

**Principle 1: Keep it minimal**

Only include dependencies the reducer actually needs:

```rust
// ✅ GOOD: Minimal dependencies
pub struct OrderEnvironment<C: Clock, D: Database> {
    clock: C,
    database: D,
}

// ❌ BAD: Kitchen sink
pub struct OrderEnvironment<...> {
    clock: C,
    database: D,
    http_client: H,  // Not needed by Order reducer
    email_service: E,  // Not needed by Order reducer
    // ...
}
```

**Principle 2: Traits, not concrete types**

```rust
// ✅ GOOD: Generic over trait
pub struct Environment<C: Clock> {
    clock: C,
}

// ❌ BAD: Concrete type
pub struct Environment {
    clock: SystemClock,  // Can't swap for tests
}
```

**Principle 3: No behavior in Environment**

Environment is a **bag of dependencies**, not a service:

```rust
// ✅ GOOD: Just holds dependencies
pub struct Environment<D: Database> {
    database: D,
}

// ❌ BAD: Has behavior
impl<D: Database> Environment<D> {
    pub async fn save_order(&self, order: &Order) {
        // NO! This logic belongs in a trait method
    }
}
```

---

## Architecture Principles

### Principle 1: Unidirectional Data Flow

Data flows in one direction only:

```
Action → Reducer → (State, Effects) → Effect Execution → More Actions
         ↑_____________________________________________________|
```

**No callbacks, no bidirectional bindings, no event emitters.** This makes the system easy to reason about:
- Where did this action come from? Trace backwards
- What happens after this action? Trace forwards
- Can this code be reordered? Check dependencies

### Principle 2: Functional Core, Imperative Shell

- **Core (Reducer)**: Pure functions, fast (< 1μs), no I/O
- **Shell (Store + Effects)**: Async, I/O, side effects

Business logic lives in the core. Infrastructure lives in the shell. Tests focus on the core (fast), integration tests verify the shell (slower).

### Principle 3: Make Illegal States Unrepresentable

Use the type system to prevent invalid states:

```rust
// ❌ BAD: Can have nonsensical states
pub struct Order {
    status: OrderStatus,
    paid_at: Option<DateTime>,
    cancelled_at: Option<DateTime>,
}
// What if status = Paid but paid_at = None?
// What if cancelled_at is Some but status != Cancelled?

// ✅ GOOD: Only valid states possible
pub enum OrderState {
    Draft { items: Vec<Item> },
    Placed { order_id: String, placed_at: DateTime },
    Paid { order_id: String, paid_at: DateTime, transaction_id: String },
    Cancelled { reason: String, cancelled_at: DateTime },
}
```

### Principle 4: Explicit Over Implicit

Make everything visible:

```rust
// ❌ IMPLICIT: Hidden side effect
async fn place_order(order: Order) {
    database.save(&order).await;  // Hidden!
}

// ✅ EXPLICIT: Effect as return value
fn place_order(state: &mut State, order: Order) -> Vec<Effect> {
    state.orders.push(order);
    vec![Effect::Database(SaveOrder)]  // Visible!
}
```

### Principle 5: Pure Functions Are Fast Functions

No I/O means reducers run at CPU speed:
- **Target**: < 1μs per action
- **Achieved**: ~200ns for simple actions, ~800ns for complex
- **Benefit**: Can test thousands of scenarios in milliseconds

### Principle 6: Composition Over Inheritance

No inheritance hierarchies. Just small, composable pieces:

```rust
// Compose effects
let effect = effect1
    .chain(effect2)
    .merge(effect3)
    .map(transform_action);

// Compose reducers (Phase 2+)
let app_reducer = counter_reducer
    .combine(todo_reducer)
    .combine(auth_reducer);
```

### Principle 7: Static Dispatch, Zero Cost

Use generics and traits for abstraction, but pay no runtime cost:

```rust
// Generic function
fn make_env<C: Clock>(clock: C) -> Environment<C> {
    Environment { clock }
}

// Compiler generates specialized versions:
// fn make_env_system_clock(clock: SystemClock) -> Environment<SystemClock>
// fn make_env_fixed_clock(clock: FixedClock) -> Environment<FixedClock>

// No virtual dispatch, no `dyn`, no `Box`
```

---

## Testing Philosophy

### Unit Tests: Test Reducers

**Reducers are pure functions, so tests are trivial:**

```rust
#[test]
fn test_increment() {
    let mut state = CounterState { count: 0 };
    let reducer = CounterReducer;
    let env = test_environment();

    let effects = reducer.reduce(&mut state, CounterAction::Increment, &env);

    assert_eq!(state.count, 1);
    assert!(matches!(effects[0], Effect::None));
}
```

**Fast**: No async, no I/O, runs in nanoseconds
**Simple**: No mocking, no setup, just assertions
**Comprehensive**: Can easily test thousands of scenarios

### Integration Tests: Test Store

**Store tests verify the full end-to-end flow:**

```rust
#[tokio::test]
async fn test_with_store() {
    let env = test_environment();
    let store = Store::new(CounterState::default(), CounterReducer, env);

    let _ = store.send(CounterAction::Increment).await;
    let count = store.state(|s| s.count).await;

    assert_eq!(count, 1);
}
```

**Slower**: Requires async runtime
**More complex**: Tests coordination, concurrency
**Focused**: Test happy path + critical failure scenarios

### Effect Tests: Use TestStore

**For effects that produce actions, use `TestStore`:**

```rust
#[tokio::test]
async fn test_payment_effect() {
    let env = test_environment();
    let store = TestStore::new(OrderReducer, env, OrderState::default());

    // Trigger action that produces effect
    let _ = store.send(OrderAction::ChargePayment { amount }).await;

    // TestStore queues resulting actions instead of auto-dispatching
    store.receive(OrderAction::PaymentSucceeded { transaction_id }).await?;
    store.assert_no_pending_actions();
}
```

**Deterministic**: You control when actions are processed
**Explicit**: Clear expectations about what actions are produced
**Safe**: Drop guard catches unprocessed actions

See `testing/src/lib.rs` for `TestStore` implementation.

---

## Error Handling

### Three-Tier Model

**Tier 1: Reducer Panics → Halt Store**

Reducers should only panic for bugs (logic errors):

```rust
fn reduce(...) {
    assert!(state.balance >= withdrawal, "Balance check failed - bug!");
}
```

**Result**: Store's `RwLock` is poisoned, all subsequent operations panic. This is intentional - it forces you to fix the bug.

**Tier 2: Effect Panics → Isolate, Log, Continue**

Effects can panic due to runtime failures (network, disk, etc.):

```rust
Effect::Future(Box::pin(async move {
    // If this panics, tokio::spawn isolates it
    let result = flaky_api_call().await?;
    Some(Action::Success(result))
}))
```

**Result**: Effect failure is logged, but Store continues operating. Other effects and actions are unaffected.

**Tier 3: Lock Poisoning → Unrecoverable**

If a reducer panics, the Store is permanently poisoned:

```rust
let count = store.state(|s| s.count).await;  // Panics: lock poisoned
```

**Result**: Application must restart. This is correct - a bug in business logic means the state may be corrupted.

### Domain Errors

Model expected errors as actions:

```rust
pub enum OrderAction {
    PlaceOrder { items: Vec<Item> },

    // Success
    OrderPlaced { order_id: String },

    // Failure
    OrderFailed { reason: String },
}
```

Reducers handle errors like any other action:

```rust
match action {
    OrderAction::OrderFailed { reason } => {
        state.status = OrderStatus::Failed;
        state.error_message = Some(reason);
        vec![Effect::PublishEvent(OrderFailed)]
    },
    // ...
}
```

See `docs/error-handling.md` for comprehensive guidance.

---

## Performance Characteristics

Based on Phase 1 benchmarks (`cargo bench -p composable-rust-runtime`):

| Operation | Target | Actual | Status |
|-----------|--------|--------|--------|
| Reducer execution | < 1μs | ~200-800ns | ✅ |
| Store send+read | < 5μs | ~1-3μs | ✅ |
| Effect::None overhead | < 1μs | ~500ns | ✅ |
| Effect::Future spawn | < 10μs | ~5-8μs | ✅ |

**Why so fast?**
- Zero-cost abstractions (static dispatch, no `Box<dyn>`)
- Minimal allocations (effects use `Pin<Box>` only when needed)
- Lock contention minimized (write locks only during reduce)
- No serialization overhead (in-memory for Phase 1)

**Scalability**:
- Reducer throughput: > 1M actions/sec (single-threaded)
- Store throughput: Scales with CPU cores (concurrent sends)
- Effect throughput: Limited by I/O, not framework

---

## Phase 1 Scope

Phase 1 implements the **core abstractions**:

✅ **Reducer trait**: Pure function for business logic
✅ **Effect enum**: Five variants (None, Future, Delay, Parallel, Sequential)
✅ **Store**: Runtime with effect execution and feedback loop
✅ **Environment traits**: Clock (with SystemClock, FixedClock)
✅ **TestStore**: Deterministic testing of effect chains
✅ **Effect composition**: map, chain, merge methods
✅ **Error handling**: Three-tier model with panic isolation

**Not in Phase 1**:
- ❌ Database effects (Phase 2)
- ❌ HTTP effects (Phase 2)
- ❌ Event publishing (Phase 3)
- ❌ Saga pattern (Phase 3)
- ❌ Event sourcing (Phase 2)

See `plans/implementation-roadmap.md` for the full phased plan.

---

## Next Steps

- **Getting Started**: See [Getting Started](getting-started.md) for a tutorial walkthrough
- **API Reference**: See [API Reference](api-reference.md) for detailed API docs
- **Implementation Decisions**: See [Implementation Decisions](implementation-decisions.md) for design rationale
- **Architecture Spec**: See [Architecture](../specs/architecture.md) for the complete 2800+ line specification

**You now understand the core concepts.** Everything else is application of these five types and principles.
