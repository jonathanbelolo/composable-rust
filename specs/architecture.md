# Composable Rust: A Functional Architecture for Event-Driven Backend Systems

**Version:** 0.1.0
**Date:** 2025-11-05
**Status:** Draft Specification
**Edition:** Rust 2024 (updated to use modern patterns)

> **Note on Rust Edition 2024**: This specification has been updated to use Edition 2024 features including `async fn` in traits and RPITIT (Return Position Impl Trait In Traits). All implementations should use these modern patterns rather than older workarounds like `BoxFuture` or the `async-trait` crate.

---

## Vision

Modern backend systems face increasing complexity in managing state, coordinating distributed operations, and maintaining correctness under concurrent load. Traditional architectures often struggle with testability, forcing developers to choose between comprehensive testing and execution speed. Business logic becomes entangled with infrastructure concerns, making refactoring risky and onboarding difficult.

**Composable Rust** addresses these challenges by bringing the principles of functional architecture—popularized by frameworks like Swift's Composable Architecture (TCA)—to the Rust backend ecosystem. By combining Rust's unparalleled type safety and zero-cost abstractions with functional programming patterns and CQRS/Event Sourcing principles, we create a framework for building **battle-tested, industrial-grade business process management systems**.

### Core Vision Tenets

1. **Correctness First**: Leverage Rust's type system to make invalid states unrepresentable and ensure business logic correctness at compile time
2. **Fearless Refactoring**: Changes ripple through the type system, making large-scale refactoring safe and mechanical
3. **Lightning-Fast Tests**: Business logic tests run at memory speed with zero I/O, enabling comprehensive test suites that complete in seconds
4. **Production Performance**: Static dispatch and zero-cost abstractions ensure the architecture adds no runtime overhead
5. **Self-Documenting**: The type system and structure serve as living documentation of business capabilities
6. **Composability**: Complex systems emerge from the composition of simple, isolated components

This is not a rapid-prototyping framework. This is an architecture for systems that will run in production for years, where bugs are expensive, and correctness cannot be compromised.

---

## 1. Core Principles

### 1.1 Functional Core, Imperative Shell

The architecture separates pure business logic (functional core) from side effects (imperative shell):

- **Functional Core**: Pure functions that transform state and produce effect descriptions
- **Imperative Shell**: Runtime that executes effects (I/O, database, network)

This separation enables:
- Testing business logic without I/O
- Deterministic execution
- Effect composition and optimization
- Clear separation of concerns

### 1.2 Unidirectional Data Flow

```
Command/Event → Reducer → (New State, Effects) → Effect Execution → More Events
```

State flows in one direction, making the system easy to reason about:
1. Actions (commands/events) arrive
2. Reducers produce new state and effect descriptions
3. Effects execute and may produce more events
4. Cycle continues

### 1.3 Explicit Effects

Side effects are never hidden. Every I/O operation, every external interaction is:
- Described as a value (Effect enum)
- Returned from pure functions
- Executed by the runtime
- Testable via mocking
- Composable and cancellable

### 1.4 Dependency Injection via Environment

All external dependencies are:
- Abstracted behind traits
- Injected via generic environment parameter
- Swappable (production, test, development)
- Scoped to reducer needs
- Zero-cost via static dispatch

### 1.5 Pragmatic Functional Programming

We favor functional patterns but make pragmatic choices:
- **Prefer immutability** but allow `&mut self` when performance demands it
- **Prefer pure functions** but recognize async/await patterns
- **Prefer composition** but allow practical escape hatches
- **Favor readability** over theoretical purity

---

## 2. Architecture Overview

### 2.0 System-Level View

Before diving into individual components, here's how the pieces fit together in a complete system:

```
┌─────────────────────────────────────────────────────────────────┐
│                         External World                           │
│                    (HTTP, gRPC, Message Queue)                   │
└────────────────────────────┬────────────────────────────────────┘
                             │
                             │ Commands (as Actions)
                             ▼
                    ┌────────────────┐
                    │   Aggregate    │
                    │  (Store with   │
                    │   Reducer)     │
                    └────────┬───────┘
                             │
                             │ Effects
                             ▼
         ┌───────────────────┴───────────────────┐
         │                                       │
         │ Effect Execution                      │
         │                                       │
         ▼                    ▼                  ▼
    ┌─────────┐         ┌─────────┐       ┌─────────┐
    │Database │         │ Events  │       │  HTTP   │
    │  Save   │         │  Pub    │       │  Call   │
    └─────────┘         └────┬────┘       └─────────┘
                             │
                             │ Events (as Actions)
                             ▼
         ┌───────────────────┴───────────────────┐
         │            Event Bus/Router            │
         └─────┬──────────────────────────┬───────┘
               │                          │
               ▼                          ▼
       ┌──────────────┐          ┌──────────────┐
       │    Saga      │          │ Projection   │
       │ (Coordinates │          │ (Read Model) │
       │  Aggregates) │          └──────────────┘
       └──────┬───────┘
              │
              │ Commands to other Aggregates
              ▼
       (back to Aggregates)
```

**Key Data Flows**:

1. **Commands → Aggregate**: External systems send commands (as Actions) to aggregates
2. **Aggregate → Effects**: Reducers process actions and emit effect descriptions
3. **Effects → Side Effects**: Runtime executes effects (database, events, HTTP)
4. **Events → Event Bus**: Published events flow to interested subscribers
5. **Event Bus → Sagas**: Sagas receive events from multiple aggregates
6. **Sagas → Commands**: Sagas dispatch commands to coordinate workflows
7. **Events → Projections**: Projections build read models from event streams

**Core Concept**: Everything flows through the Action type—commands come in as actions, events go out as actions, and actions loop back to drive the system forward. The reducer is always at the center, transforming (State, Action) into (New State, Effects).

### 2.1 Core Types

Every feature in the system is built from five fundamental types:

#### **State**
```rust
/// Domain state for a feature
/// - Should be owned data (no lifetimes where possible)
/// - Should be Clone-able for time-travel and testing
/// - May use interior mutability where performance critical
#[derive(Clone, Debug)]
struct OrderState {
    orders: HashMap<OrderId, Order>,
    pending_payments: Vec<PaymentId>,
}
```

#### **Action**

**The Action type is the universal input to a reducer.** It represents all possible state transitions—both commands (requests to change state) and events (facts about what happened). This unified approach creates an elegant feedback loop: commands come in as actions, reducers emit events as actions, and events from other aggregates arrive as actions.

```rust
/// All possible inputs to a reducer
/// - Exhaustive enum of all state transitions
/// - Commands: External requests (PlaceOrder, CancelOrder)
/// - Events: Internal facts (OrderPlaced, PaymentReceived)
/// - Cross-Aggregate Events: Notifications from other parts of the system
#[derive(Clone, Debug)]
enum OrderAction {
    // Commands - External requests to change state
    PlaceOrder { customer_id: CustomerId, items: Vec<LineItem> },
    CancelOrder { order_id: OrderId, reason: String },
    ConfirmPayment { order_id: OrderId },

    // Events - Facts that occurred (often emitted by effects)
    OrderPlaced { order_id: OrderId, timestamp: DateTime<Utc> },
    PaymentReceived { order_id: OrderId, amount: Money },
    OrderShipped { order_id: OrderId, tracking: String },

    // Cross-Aggregate Events - From event bus/other aggregates
    InventoryReserved { order_id: OrderId, reservation_id: ReservationId },
    PaymentFailed { order_id: OrderId, reason: String },
}
```

**Key Insight**: By unifying commands and events into a single Action type, we create a consistent feedback mechanism. Effects can produce new actions (events), which flow back into the reducer, which produces more effects—all through the same type system and state machine.

#### **Reducer**
```rust
/// Pure function: (State, Action, Environment) -> (State, Effects)
/// - Contains all business logic
/// - Deterministic and testable
/// - May mutate state in place or return new state
trait Reducer {
    type State;
    type Action;
    type Environment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> Vec<Effect<Self::Action>>;
}
```

#### **Effect**

Effects describe side effects to be executed by the runtime. They are **values, not execution**—the reducer returns a description of what should happen, and the Store runtime executes them.

```rust
/// Description of side effects to perform
/// - Not executed immediately (just descriptions)
/// - Composable and cancellable
/// - May produce actions when executed (feedback loop)
///
/// Note: This is a conceptual definition. Actual implementations
/// may extend this with additional variants for specific needs.
enum Effect<Action> {
    /// No-op
    None,

    /// Database operation (concrete type defined during implementation)
    Database(DbOperation),

    /// HTTP request with response handlers
    Http {
        request: HttpRequest,
        on_success: fn(Response) -> Option<Action>,
        on_error: fn(Error) -> Option<Action>,
    },

    /// Publish event to message bus
    PublishEvent(Event),

    /// Delayed action (for timeouts, retries)
    Delay {
        duration: Duration,
        action: Box<Action>,
    },

    /// Run effects in parallel
    Parallel(Vec<Effect<Action>>),

    /// Run effects sequentially
    Sequential(Vec<Effect<Action>>),

    /// Arbitrary async computation
    Future(Pin<Box<dyn Future<Output = Option<Action>> + Send>>),

    /// Cancellable effect with ID
    Cancellable {
        id: EffectId,
        effect: Box<Effect<Action>>,
    },

    // Additional variants can be added for specific domains:
    // - DispatchCommand(Command) - for saga coordination
    // - SendEmail(EmailParams) - for notifications
    // - ScheduleJob(JobParams) - for background work
    // etc.
}
```

**Implementation Note**: Types like `DbOperation`, `HttpRequest`, `Event`, and `EffectId` are domain-specific and will be defined during Phase 1 implementation. The Effect enum is designed to be extended with additional variants as needed.

#### **Environment**
```rust
/// Injected dependencies for a reducer
/// - All dependencies are trait objects or generics
/// - Swappable for testing
/// - Passed down through composition
struct OrderEnvironment<D, C, E> {
    database: D,
    clock: C,
    event_publisher: E,
}
```

### 2.2 The Store

The **Store** is the runtime that brings these pieces together. It manages state, coordinates reducer execution, and handles the effect→action feedback loop.

```rust
/// The Store is the runtime for a reducer
///
/// Generic parameters:
/// - S: State type
/// - A: Action type
/// - E: Environment type
/// - R: Reducer implementation
struct Store<S, A, E, R>
where
    R: Reducer<State = S, Action = A, Environment = E>
{
    state: RwLock<S>,
    reducer: R,
    environment: E,
    // Additional fields for effect management, defined during implementation
}

impl<S, A, E, R> Store<S, A, E, R>
where
    R: Reducer<State = S, Action = A, Environment = E>,
    A: Send + 'static,
    S: Send + Sync,
{
    /// Create a new store
    pub fn new(initial_state: S, reducer: R, environment: E) -> Self {
        Self {
            state: RwLock::new(initial_state),
            reducer,
            environment,
        }
    }

    /// Send an action to the store
    ///
    /// This is the primary way to interact with the store:
    /// 1. Lock state for writing
    /// 2. Call reducer with (state, action, environment)
    /// 3. Apply effects (which may produce more actions)
    pub async fn send(&self, action: A) {
        let effects = {
            let mut state = self.state.write().await;
            self.reducer.reduce(&mut *state, action, &self.environment)
        };

        // Execute effects (which may feed actions back into the store)
        for effect in effects {
            self.execute_effect(effect).await;
        }
    }

    /// Read current state
    ///
    /// Access state via a closure to ensure the lock is released promptly:
    ///
    /// ```rust
    /// let order_count = store.state(|s| s.orders.len()).await;
    /// ```
    pub async fn state<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&S) -> T
    {
        let state = self.state.read().await;
        f(&*state)
    }

    // execute_effect implementation details defined during Phase 1
    async fn execute_effect(&self, effect: Effect<A>) {
        // Execution logic including:
        // - Pattern match on effect type
        // - Execute the side effect
        // - If effect produces an action, call self.send(action)
        // This creates the feedback loop: Effect → Action → Reducer → Effects...
    }
}
```

**Type Alias Pattern**: For ergonomics, define type aliases for concrete stores:

```rust
// Type alias for a specific store configuration
type OrderStore = Store<OrderState, OrderAction, OrderEnvironment, OrderReducer>;

// Usage
let store = OrderStore::new(
    OrderState::default(),
    OrderReducer,
    production_environment(),
);
```

**The Feedback Loop**: When an effect produces an action (e.g., `Future` returns `Some(action)`), the Store calls `self.send(action)`, feeding it back into the reducer. This creates a self-sustaining cycle where effects drive further actions.

---

## 3. Dependency Injection Model

### 3.1 Trait-Based Abstractions

Each capability is defined as a trait:

```rust
/// Database operations
trait Database: Send + Sync {
    async fn transaction<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Transaction) -> impl Future<Output = Result<T>> + Send;

    async fn save_aggregate(&self, id: AggregateId, events: &[Event]) -> Result<()>;
    async fn load_aggregate(&self, id: AggregateId) -> Result<Vec<Event>>;
}

/// Time operations
trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
    async fn sleep(&self, duration: Duration);
}

/// Event publishing
trait EventPublisher: Send + Sync {
    async fn publish(&self, event: Event) -> Result<()>;
    async fn publish_batch(&self, events: Vec<Event>) -> Result<()>;
}

/// HTTP client
trait HttpClient: Send + Sync {
    async fn get(&self, url: &str) -> Result<Response>;
    async fn post(&self, url: &str, body: &[u8]) -> Result<Response>;
}

/// ID generation
trait IdGenerator: Send + Sync {
    fn next_id(&self) -> Uuid;
}
```

### 3.2 Three-Tier Implementation Strategy

For each dependency, provide three implementations:

#### **Production** - Real, full-featured
```rust
struct PostgresDatabase {
    pool: PgPool,
}

impl Database for PostgresDatabase {
    async fn save_aggregate(&self, id: AggregateId, events: &[Event]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for event in events {
            sqlx::query(
                "INSERT INTO events (aggregate_id, event_type, payload, timestamp)
                 VALUES ($1, $2, $3, $4)"
            )
            .bind(id)
            .bind(event.event_type())
            .bind(serde_json::to_value(event)?)
            .bind(event.timestamp())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await
    }
}
```

#### **Test** - Fast, deterministic, in-memory
```rust
struct MockDatabase {
    events: Arc<Mutex<HashMap<AggregateId, Vec<Event>>>>,
    call_log: Arc<Mutex<Vec<String>>>,
}

impl MockDatabase {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(HashMap::new())),
            call_log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn verify_called(&self, method: &str) -> bool {
        self.call_log.lock().unwrap().contains(&method.to_string())
    }
}

impl Database for MockDatabase {
    async fn save_aggregate(&self, id: AggregateId, events: &[Event]) -> Result<()> {
        self.call_log.lock().unwrap().push("save_aggregate".to_string());
        self.events.lock().unwrap()
            .entry(id)
            .or_default()
            .extend(events.iter().cloned());
        Ok(())
    }

    async fn load_aggregate(&self, id: AggregateId) -> Result<Vec<Event>> {
        Ok(self.events.lock().unwrap()
            .get(&id)
            .cloned()
            .unwrap_or_default())
    }
}

struct FixedClock {
    time: DateTime<Utc>,
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.time // Always returns same time - deterministic!
    }

    async fn sleep(&self, _duration: Duration) {
        // Instant in tests - no actual sleeping
    }
}
```

#### **Development** - Instrumented, debuggable
```rust
struct LoggingDatabase<D> {
    inner: D,
    logger: Logger,
}

impl<D: Database> Database for LoggingDatabase<D> {
    async fn save_aggregate(&self, id: AggregateId, events: &[Event]) -> Result<()> {
        info!(self.logger, "save_aggregate";
              "aggregate_id" => ?id,
              "event_count" => events.len());

        let start = Instant::now();
        let result = self.inner.save_aggregate(id, events).await;

        info!(self.logger, "save_aggregate completed";
              "duration_ms" => start.elapsed().as_millis(),
              "success" => result.is_ok());

        result
    }
}
```

### 3.3 Environment Composition

Environments compose hierarchically:

```rust
/// Root environment with all dependencies
struct AppEnvironment {
    database: Arc<dyn Database>,
    clock: Arc<dyn Clock>,
    http: Arc<dyn HttpClient>,
    events: Arc<dyn EventPublisher>,
    ids: Arc<dyn IdGenerator>,
}

/// Child environments extract only what they need
struct OrderEnvironment {
    database: Arc<dyn Database>,
    clock: Arc<dyn Clock>,
    events: Arc<dyn EventPublisher>,
    ids: Arc<dyn IdGenerator>,
}

struct PaymentEnvironment {
    http: Arc<dyn HttpClient>,
    clock: Arc<dyn Clock>,
    events: Arc<dyn EventPublisher>,
}

impl AppEnvironment {
    fn order_env(&self) -> OrderEnvironment {
        OrderEnvironment {
            database: self.database.clone(),
            clock: self.clock.clone(),
            events: self.events.clone(),
            ids: self.ids.clone(),
        }
    }

    fn payment_env(&self) -> PaymentEnvironment {
        PaymentEnvironment {
            http: self.http.clone(),
            clock: self.clock.clone(),
            events: self.events.clone(),
        }
    }
}
```

### 3.4 Static vs Dynamic Dispatch

For maximum performance, prefer static dispatch:

```rust
/// Static - monomorphized at compile time
struct OrderStore<D, C, E, I> {
    state: RwLock<OrderState>,
    reducer: OrderReducer,
    environment: OrderEnvironment<D, C, E, I>,
}

/// Where clause ensures trait bounds
impl<D, C, E, I> OrderStore<D, C, E, I>
where
    D: Database,
    C: Clock,
    E: EventPublisher,
    I: IdGenerator,
{
    // Implementation...
}
```

For flexibility where needed, use dynamic dispatch:

```rust
/// Dynamic - uses trait objects
struct OrderStore {
    state: RwLock<OrderState>,
    reducer: OrderReducer,
    environment: OrderEnvironment<
        Arc<dyn Database>,
        Arc<dyn Clock>,
        Arc<dyn EventPublisher>,
        Arc<dyn IdGenerator>,
    >,
}
```

**Recommendation**: Use static dispatch by default. Only reach for dynamic dispatch when you need runtime polymorphism (e.g., hot-swapping implementations, plugin systems).

---

## 4. CQRS and Event Sourcing Integration

**Note on Terminology**: In this architecture, **Actions** encompass both Commands and Events (see Section 2.1). This unified approach simplifies the feedback loop. When discussing CQRS patterns, we'll sometimes refer to "commands" and "events" conceptually, but remember they're all represented as Action variants.

### 4.1 Command/Query Separation

The architecture naturally separates commands and queries:

```rust
/// Command Side - State mutations
enum Command {
    PlaceOrder { customer_id: CustomerId, items: Vec<LineItem> },
    CancelOrder { order_id: OrderId, reason: String },
    ShipOrder { order_id: OrderId },
}

/// Events - Facts that occurred
#[derive(Clone, Serialize, Deserialize)]
enum OrderEvent {
    OrderPlaced {
        order_id: OrderId,
        customer_id: CustomerId,
        items: Vec<LineItem>,
        timestamp: DateTime<Utc>,
    },
    OrderCancelled {
        order_id: OrderId,
        reason: String,
        timestamp: DateTime<Utc>,
    },
    OrderShipped {
        order_id: OrderId,
        tracking: String,
        timestamp: DateTime<Utc>,
    },
}

/// Query Side - Read models
#[derive(Clone)]
struct OrderListProjection {
    orders: Vec<OrderSummary>,
}

#[derive(Clone)]
struct OrderDetailsProjection {
    order: OrderDetails,
    history: Vec<StatusChange>,
}
```

### 4.2 Event Sourcing Pattern

State is derived from events:

```rust
/// Aggregate = fold over event stream
impl OrderState {
    fn from_events(events: impl Iterator<Item = OrderEvent>) -> Self {
        events.fold(Self::default(), |mut state, event| {
            state.apply_event(event);
            state
        })
    }

    fn apply_event(&mut self, event: OrderEvent) {
        match event {
            OrderEvent::OrderPlaced { order_id, customer_id, items, timestamp } => {
                self.orders.insert(order_id, Order {
                    id: order_id,
                    customer_id,
                    items,
                    status: OrderStatus::Pending,
                    placed_at: timestamp,
                });
            }
            OrderEvent::OrderShipped { order_id, tracking, .. } => {
                if let Some(order) = self.orders.get_mut(&order_id) {
                    order.status = OrderStatus::Shipped { tracking };
                }
            }
            OrderEvent::OrderCancelled { order_id, .. } => {
                if let Some(order) = self.orders.get_mut(&order_id) {
                    order.status = OrderStatus::Cancelled;
                }
            }
        }
    }
}
```

### 4.3 Command Handling

Commands are validated and produce events:

```rust
impl Reducer for OrderReducer {
    type State = OrderState;
    type Action = OrderAction;
    type Environment = OrderEnvironment;

    fn reduce(
        &self,
        state: &mut OrderState,
        action: OrderAction,
        env: &OrderEnvironment,
    ) -> Vec<Effect<OrderAction>> {
        match action {
            // Commands produce events
            OrderAction::PlaceOrder { customer_id, items } => {
                // Validation
                if items.is_empty() {
                    return vec![Effect::None];
                }

                let order_id = env.ids.next_id().into();
                let timestamp = env.clock.now();

                let event = OrderEvent::OrderPlaced {
                    order_id,
                    customer_id,
                    items: items.clone(),
                    timestamp,
                };

                // Apply event to state
                state.apply_event(event.clone());

                // Persist event and publish
                vec![
                    Effect::Database(DbOperation::SaveEvent(order_id, event.clone())),
                    Effect::PublishEvent(Event::Order(event)),
                ]
            }

            // Events just update state (idempotent)
            OrderAction::OrderPlaced { .. } => {
                // Already applied if coming from event replay
                vec![Effect::None]
            }

            _ => vec![Effect::None],
        }
    }
}
```

### 4.4 Projection Building

Events update read models:

```rust
struct OrderListProjection {
    orders: Vec<OrderSummary>,
}

impl EventHandler for OrderListProjection {
    fn handle_event(&mut self, event: &Event) {
        match event {
            Event::Order(OrderEvent::OrderPlaced { order_id, customer_id, items, timestamp }) => {
                self.orders.push(OrderSummary {
                    id: *order_id,
                    customer_id: *customer_id,
                    item_count: items.len(),
                    status: "pending".to_string(),
                    placed_at: *timestamp,
                });
            }
            Event::Order(OrderEvent::OrderShipped { order_id, .. }) => {
                if let Some(order) = self.orders.iter_mut().find(|o| o.id == *order_id) {
                    order.status = "shipped".to_string();
                }
            }
            _ => {}
        }
    }
}
```

### 4.5 Event Store Interface

```rust
trait EventStore: Send + Sync {
    /// Append events to an aggregate stream
    async fn append_events(
        &self,
        aggregate_id: AggregateId,
        expected_version: Version,
        events: Vec<Event>,
    ) -> Result<()>;

    /// Load all events for an aggregate
    async fn load_events(&self, aggregate_id: AggregateId) -> Result<Vec<Event>>;

    /// Stream all events (for projections)
    fn stream_all(&self) -> BoxStream<'_, Result<Event>>;

    /// Stream events from a position (for catch-up)
    fn stream_from(&self, position: Position) -> BoxStream<'_, Result<Event>>;
}
```

---

## 5. Effect System

### 5.1 Effect Types

```rust
enum Effect<Action> {
    /// No-op effect
    None,

    /// Database operation
    Database(DbOperation),

    /// HTTP request
    Http {
        request: HttpRequest,
        on_success: fn(Response) -> Option<Action>,
        on_error: fn(Error) -> Option<Action>,
    },

    /// Publish event to message bus
    PublishEvent(Event),

    /// Delayed action
    Delay {
        duration: Duration,
        action: Box<Action>,
    },

    /// Run effects in parallel
    Parallel(Vec<Effect<Action>>),

    /// Run effects sequentially
    Sequential(Vec<Effect<Action>>),

    /// Arbitrary async computation
    Future(Pin<Box<dyn Future<Output = Option<Action>> + Send>>),

    /// Cancellable effect with ID
    Cancellable {
        id: EffectId,
        effect: Box<Effect<Action>>,
    },
}
```

### 5.2 Effect Execution

```rust
impl<S, A, E, R> Store<S, A, E, R>
where
    R: Reducer<State = S, Action = A, Environment = E>,
    A: Send + 'static,
{
    async fn execute_effect(&self, effect: Effect<A>) {
        match effect {
            Effect::None => {}

            Effect::Database(op) => {
                if let Err(e) = self.execute_db_operation(op).await {
                    error!("Database operation failed: {}", e);
                }
            }

            Effect::Http { request, on_success, on_error } => {
                match self.environment.http.execute(request).await {
                    Ok(response) => {
                        if let Some(action) = on_success(response) {
                            self.send(action).await;
                        }
                    }
                    Err(e) => {
                        if let Some(action) = on_error(e) {
                            self.send(action).await;
                        }
                    }
                }
            }

            Effect::PublishEvent(event) => {
                if let Err(e) = self.environment.events.publish(event).await {
                    error!("Failed to publish event: {}", e);
                }
            }

            Effect::Delay { duration, action } => {
                let store = self.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(duration).await;
                    store.send(*action).await;
                });
            }

            Effect::Parallel(effects) => {
                let futures = effects.into_iter().map(|eff| self.execute_effect(eff));
                futures::future::join_all(futures).await;
            }

            Effect::Sequential(effects) => {
                for effect in effects {
                    self.execute_effect(effect).await;
                }
            }

            Effect::Future(fut) => {
                if let Some(action) = fut.await {
                    self.send(action).await;
                }
            }

            Effect::Cancellable { id, effect } => {
                self.effect_registry.register(id, effect);
                // Can be cancelled later via id
            }
        }
    }
}
```

### 5.3 Effect Composition

```rust
impl<A> Effect<A> {
    /// Combine effects to run in parallel
    pub fn merge(effects: Vec<Effect<A>>) -> Effect<A> {
        Effect::Parallel(effects)
    }

    /// Chain effects to run sequentially
    pub fn chain(effects: Vec<Effect<A>>) -> Effect<A> {
        Effect::Sequential(effects)
    }

    /// Map the action type
    ///
    /// # Implementation Note
    ///
    /// This will be implemented in Phase 1 to transform effect action types
    /// through the effect tree (Parallel, Sequential, Delay, etc.)
    pub fn map<B>(self, f: impl Fn(A) -> B + Send + 'static) -> Effect<B>
    where
        A: Send + 'static,
        B: Send + 'static,
    {
        // NOTE: Conceptual signature only - implementation deferred to Phase 1
        unimplemented!("Effect::map will be implemented in Phase 1")
    }

    /// Cancel effect by ID
    pub fn cancel(id: EffectId) -> Effect<A> {
        Effect::Cancel(id)
    }
}
```

### 5.4 Error Handling Strategy

**Key Principle**: Effects that fail should produce actions that flow back into the reducer, allowing business logic to decide how to handle errors.

#### Error-to-Action Pattern

When an effect fails, it can produce an error action:

```rust
Effect::Http {
    request: payment_request,
    on_success: |response| {
        Some(Action::PaymentSucceeded {
            payment_id: response.payment_id,
        })
    },
    on_error: |error| {
        Some(Action::PaymentFailed {
            reason: error.to_string(),
            is_retryable: error.is_transient(),
        })
    },
}
```

The reducer then decides how to handle the error:

```rust
match action {
    PaymentFailed { reason, is_retryable: true } if state.retry_count < MAX_RETRIES => {
        state.retry_count += 1;
        let delay = Duration::from_secs(2_u64.pow(state.retry_count));
        vec![
            Effect::Delay {
                duration: delay,
                action: Box::new(Action::RetryPayment),
            }
        ]
    }
    PaymentFailed { reason, .. } => {
        // Max retries or non-retryable error - compensate
        state.status = Status::Failed;
        self.compensate(state, env)
    }
}
```

#### Infallible Effects

Some effects should be fire-and-forget (logging, metrics):

```rust
Effect::PublishEvent(event) // If publishing fails, log but don't crash
```

The Store's effect executor should:
- Log the error
- Optionally publish to dead-letter queue
- Continue processing (don't block the reducer)

#### Fallible Effects

Critical effects should produce error actions:

```rust
Effect::Database(SaveOrder(order)) → if fails → Action::OrderSaveFailed
Effect::Http(ChargePayment)        → if fails → Action::PaymentFailed
```

#### Implementation Guidance

**Phase 1**: Simple error logging, no sophisticated retry
**Phase 2**: Add retry policies and circuit breakers
**Phase 3**: Dead-letter queues and error observability

**Principle**: The reducer is always in control. Effects describe what to do on error, but the reducer's business logic makes the final decision.

---

## 6. Composition Patterns

**Key Principle**: Complex systems emerge from composing simple, isolated reducers. This section covers how reducers combine to form larger systems and how long-running workflows (sagas) are modeled.

**Important Note on Sagas**: In this architecture, **sagas are just reducers**. There is no special saga framework or DSL—workflows are modeled as state machines using the same reducer pattern you already know. This elegant approach (covered in detail in Section 6.3) provides maximum flexibility, testability, and transparency.

### 6.1 Reducer Composition

Combine multiple reducers into one:

```rust
/// Combine reducers that operate on different parts of state
fn combine_reducers<S1, S2, A, E>(
    reducer1: impl Reducer<State = S1, Action = A, Environment = E>,
    reducer2: impl Reducer<State = S2, Action = A, Environment = E>,
) -> impl Reducer<State = (S1, S2), Action = A, Environment = E>
{
    CombinedReducer {
        reducer1,
        reducer2,
        _phantom: PhantomData,
    }
}

struct CombinedReducer<R1, R2, A, E> {
    reducer1: R1,
    reducer2: R2,
    _phantom: PhantomData<(A, E)>,
}

impl<R1, R2, A, E> Reducer for CombinedReducer<R1, R2, A, E>
where
    R1: Reducer<Action = A, Environment = E>,
    R2: Reducer<Action = A, Environment = E>,
{
    type State = (R1::State, R2::State);
    type Action = A;
    type Environment = E;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: A,
        env: &E,
    ) -> Vec<Effect<A>> {
        let effects1 = self.reducer1.reduce(&mut state.0, action.clone(), env);
        let effects2 = self.reducer2.reduce(&mut state.1, action, env);

        effects1.into_iter().chain(effects2).collect()
    }
}
```

### 6.2 Scoped Reducers

Lift a child reducer to work on parent state:

```rust
/// Scope a reducer to a subset of parent state
fn scope_reducer<ParentState, ChildState, A, E>(
    child_reducer: impl Reducer<State = ChildState, Action = A, Environment = E>,
    get_child: impl Fn(&mut ParentState) -> &mut ChildState,
) -> impl Reducer<State = ParentState, Action = A, Environment = E>
{
    ScopedReducer {
        child_reducer,
        get_child,
        _phantom: PhantomData,
    }
}

struct ScopedReducer<R, F, PS, A, E> {
    child_reducer: R,
    get_child: F,
    _phantom: PhantomData<(PS, A, E)>,
}

impl<R, F, PS, A, E> Reducer for ScopedReducer<R, F, PS, A, E>
where
    R: Reducer<Action = A, Environment = E>,
    F: Fn(&mut PS) -> &mut R::State,
{
    type State = PS;
    type Action = A;
    type Environment = E;

    fn reduce(&self, state: &mut PS, action: A, env: &E) -> Vec<Effect<A>> {
        let child_state = (self.get_child)(state);
        self.child_reducer.reduce(child_state, action, env)
    }
}
```

### 6.3 Sagas: Long-Running Workflows

**Sagas coordinate long-running business processes across multiple aggregates.** In this architecture, a saga is simply another reducer with its own state machine—no special framework needed.

#### Why Sagas as Reducers?

This approach is elegant because:
1. **Conceptual consistency**: Same patterns you already know (State, Action, Reducer, Effects)
2. **No new abstractions**: Just write reducers
3. **Explicit control flow**: State machine logic is visible in match statements
4. **Fully testable**: Test like any other reducer
5. **Composable**: Works with existing infrastructure

#### Saga Pattern

A saga is a **reducer that models a workflow as a state machine**:

```rust
// ============================================================================
// Saga State - Tracks workflow progress
// ============================================================================

#[derive(Clone, Debug)]
struct CheckoutSagaState {
    checkout_id: CheckoutId,
    customer_id: CustomerId,
    items: Vec<LineItem>,
    payment_method: PaymentMethod,

    // Current position in workflow
    current_step: CheckoutStep,

    // IDs from completed steps (for compensation)
    order_id: Option<OrderId>,
    reservation_id: Option<ReservationId>,
    payment_id: Option<PaymentId>,

    // Audit trail
    completed_steps: Vec<CheckoutStep>,
    started_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
enum CheckoutStep {
    Started,
    InventoryReserved,
    OrderCreated,
    PaymentProcessed,
    Completed,
    Failed { reason: String },
}

// ============================================================================
// Saga Actions - Trigger and respond to events
// ============================================================================

#[derive(Clone, Debug)]
enum CheckoutSagaAction {
    // Start the saga
    StartCheckout {
        customer_id: CustomerId,
        items: Vec<LineItem>,
        payment_method: PaymentMethod,
    },

    // Events from other aggregates (success cases)
    InventoryReserved {
        checkout_id: CheckoutId,
        reservation_id: ReservationId,
    },
    OrderCreated {
        checkout_id: CheckoutId,
        order_id: OrderId,
    },
    PaymentSucceeded {
        checkout_id: CheckoutId,
        payment_id: PaymentId,
    },
    OrderConfirmed {
        checkout_id: CheckoutId,
    },

    // Events from other aggregates (failure cases)
    InventoryReservationFailed {
        checkout_id: CheckoutId,
        reason: String,
    },
    PaymentFailed {
        checkout_id: CheckoutId,
        reason: String,
    },

    // Timeout events
    StepTimeout {
        checkout_id: CheckoutId,
        step: CheckoutStep,
    },
}

// ============================================================================
// Saga Reducer - The workflow logic!
// ============================================================================

struct CheckoutSagaReducer;

impl Reducer for CheckoutSagaReducer {
    type State = CheckoutSagaState;
    type Action = CheckoutSagaAction;
    type Environment = SagaEnvironment;

    fn reduce(
        &self,
        state: &mut CheckoutSagaState,
        action: CheckoutSagaAction,
        env: &SagaEnvironment,
    ) -> Vec<Effect<CheckoutSagaAction>> {
        use CheckoutSagaAction::*;
        use CheckoutStep::*;

        // Pattern match on (current_step, action) for state machine transitions
        match (state.current_step.clone(), action) {
            // ================================================================
            // Happy Path: Success Flow
            // ================================================================

            // Step 1: Start → Reserve Inventory
            (Started, StartCheckout { customer_id, items, payment_method }) => {
                state.customer_id = customer_id;
                state.items = items.clone();
                state.payment_method = payment_method;
                state.completed_steps.push(Started);

                vec![
                    // Dispatch command to inventory aggregate
                    Effect::DispatchCommand(
                        AggregateCommand::Inventory(InventoryCommand::Reserve {
                            checkout_id: state.checkout_id,
                            items: items.clone(),
                        })
                    ),
                    // Set timeout for this step
                    Effect::Delay {
                        duration: Duration::from_secs(30),
                        action: Box::new(StepTimeout {
                            checkout_id: state.checkout_id,
                            step: Started,
                        }),
                    },
                ]
            }

            // Step 2: Inventory Reserved → Create Order
            (Started, InventoryReserved { reservation_id, .. }) => {
                state.reservation_id = Some(reservation_id);
                state.current_step = InventoryReserved;
                state.completed_steps.push(InventoryReserved);

                vec![
                    Effect::DispatchCommand(
                        AggregateCommand::Order(OrderCommand::CreateOrder {
                            checkout_id: state.checkout_id,
                            customer_id: state.customer_id,
                            items: state.items.clone(),
                        })
                    ),
                    Effect::Delay {
                        duration: Duration::from_secs(30),
                        action: Box::new(StepTimeout {
                            checkout_id: state.checkout_id,
                            step: InventoryReserved,
                        }),
                    },
                ]
            }

            // Step 3: Order Created → Process Payment
            (InventoryReserved, OrderCreated { order_id, .. }) => {
                state.order_id = Some(order_id);
                state.current_step = OrderCreated;
                state.completed_steps.push(OrderCreated);

                let total = calculate_total(&state.items);

                vec![
                    Effect::DispatchCommand(
                        AggregateCommand::Payment(PaymentCommand::ProcessPayment {
                            checkout_id: state.checkout_id,
                            order_id,
                            amount: total,
                            payment_method: state.payment_method.clone(),
                        })
                    ),
                    Effect::Delay {
                        duration: Duration::from_secs(60), // Longer timeout for payment
                        action: Box::new(StepTimeout {
                            checkout_id: state.checkout_id,
                            step: OrderCreated,
                        }),
                    },
                ]
            }

            // Step 4: Payment Succeeded → Confirm Order
            (OrderCreated, PaymentSucceeded { payment_id, .. }) => {
                state.payment_id = Some(payment_id);
                state.current_step = PaymentProcessed;
                state.completed_steps.push(PaymentProcessed);

                vec![
                    Effect::DispatchCommand(
                        AggregateCommand::Order(OrderCommand::ConfirmOrder {
                            order_id: state.order_id.unwrap(),
                        })
                    ),
                ]
            }

            // Step 5: Order Confirmed → Complete!
            (PaymentProcessed, OrderConfirmed { .. }) => {
                state.current_step = Completed;
                state.completed_steps.push(Completed);

                vec![
                    Effect::PublishEvent(Event::CheckoutCompleted {
                        checkout_id: state.checkout_id,
                        order_id: state.order_id.unwrap(),
                        completed_at: env.clock.now(),
                    }),
                ]
            }

            // ================================================================
            // Unhappy Path: Failures Trigger Compensation
            // ================================================================

            (Started, InventoryReservationFailed { reason, .. }) => {
                state.current_step = Failed { reason: reason.clone() };

                vec![
                    Effect::PublishEvent(Event::CheckoutFailed {
                        checkout_id: state.checkout_id,
                        reason,
                        failed_at: env.clock.now(),
                    }),
                ]
            }

            (OrderCreated, PaymentFailed { reason, .. }) => {
                state.current_step = Failed { reason: reason.clone() };

                // Compensate: Cancel order and release inventory
                self.compensate(state, env)
            }

            (step, StepTimeout { step: timeout_step, .. })
                if step == timeout_step =>
            {
                state.current_step = Failed {
                    reason: format!("Timeout at step: {:?}", timeout_step)
                };

                self.compensate(state, env)
            }

            // ================================================================
            // Ignore out-of-order or duplicate events
            // ================================================================
            _ => vec![Effect::None],
        }
    }
}

impl CheckoutSagaReducer {
    /// Compensation logic - undo completed steps in reverse order
    fn compensate(
        &self,
        state: &CheckoutSagaState,
        env: &SagaEnvironment,
    ) -> Vec<Effect<CheckoutSagaAction>> {
        let mut effects = vec![];

        // Cancel order if created
        if let Some(order_id) = state.order_id {
            effects.push(Effect::DispatchCommand(
                AggregateCommand::Order(OrderCommand::CancelOrder {
                    order_id,
                    reason: "Checkout failed".to_string(),
                })
            ));
        }

        // Release inventory reservation
        if let Some(reservation_id) = state.reservation_id {
            effects.push(Effect::DispatchCommand(
                AggregateCommand::Inventory(InventoryCommand::ReleaseReservation {
                    reservation_id,
                })
            ));
        }

        // Publish failure event
        effects.push(Effect::PublishEvent(Event::CheckoutFailed {
            checkout_id: state.checkout_id,
            reason: match &state.current_step {
                CheckoutStep::Failed { reason } => reason.clone(),
                _ => "Unknown failure".to_string(),
            },
            failed_at: env.clock.now(),
        }));

        effects
    }
}

// ============================================================================
// Saga Environment
// ============================================================================

struct SagaEnvironment {
    clock: Arc<dyn Clock>,
    events: Arc<dyn EventPublisher>,
    command_dispatcher: Arc<dyn CommandDispatcher>,
}

// ============================================================================
// Effect Extensions for Sagas
// ============================================================================

enum Effect<Action> {
    // ... existing effects ...

    /// Dispatch command to another aggregate
    DispatchCommand(AggregateCommand),
}
```

#### Testing Sagas

The beauty of this approach—**sagas are trivial to test**:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_successful_checkout_workflow() {
        // Arrange
        let env = test_saga_environment();
        let saga = CheckoutSagaStore::new(CheckoutSagaReducer, env);

        // Act: Execute the happy path
        saga.send(CheckoutSagaAction::StartCheckout {
            customer_id: CustomerId::new(1),
            items: vec![LineItem { sku: "WIDGET".into(), quantity: 1 }],
            payment_method: PaymentMethod::CreditCard,
        }).await;

        // Simulate events from other aggregates
        let checkout_id = saga.state(|s| s.checkout_id).await;
        saga.send(CheckoutSagaAction::InventoryReserved {
            checkout_id,
            reservation_id: ReservationId::new(1),
        }).await;

        let checkout_id = saga.state(|s| s.checkout_id).await;
        saga.send(CheckoutSagaAction::OrderCreated {
            checkout_id,
            order_id: OrderId::new(1),
        }).await;

        let checkout_id = saga.state(|s| s.checkout_id).await;
        saga.send(CheckoutSagaAction::PaymentSucceeded {
            checkout_id,
            payment_id: PaymentId::new(1),
        }).await;

        let checkout_id = saga.state(|s| s.checkout_id).await;
        saga.send(CheckoutSagaAction::OrderConfirmed {
            checkout_id,
        }).await;

        // Assert
        let state = saga.state(|s| s.clone()).await;
        assert_eq!(state.current_step, CheckoutStep::Completed);
        assert!(state.order_id.is_some());
        assert!(state.payment_id.is_some());
        assert_eq!(state.completed_steps.len(), 5);
    }

    #[tokio::test]
    async fn test_payment_failure_triggers_compensation() {
        // Arrange
        let env = test_saga_environment();
        let saga = CheckoutSagaStore::new(CheckoutSagaReducer, env.clone());

        // Progress through workflow
        saga.send(CheckoutSagaAction::StartCheckout {
            customer_id: CustomerId::new(1),
            items: vec![LineItem { sku: "WIDGET".into(), quantity: 1 }],
            payment_method: PaymentMethod::CreditCard,
        }).await;

        let checkout_id = saga.state(|s| s.checkout_id).await;
        saga.send(CheckoutSagaAction::InventoryReserved {
            checkout_id,
            reservation_id: ReservationId::new(1),
        }).await;

        let checkout_id = saga.state(|s| s.checkout_id).await;
        saga.send(CheckoutSagaAction::OrderCreated {
            checkout_id,
            order_id: OrderId::new(1),
        }).await;

        // Payment fails!
        let checkout_id = saga.state(|s| s.checkout_id).await;
        saga.send(CheckoutSagaAction::PaymentFailed {
            checkout_id,
            reason: "Card declined".to_string(),
        }).await;

        // Assert compensation occurred
        let state = saga.state(|s| s.clone()).await;
        assert!(matches!(state.current_step, CheckoutStep::Failed { .. }));

        // Verify compensating commands were dispatched
        let commands = env.command_dispatcher.dispatched_commands();
        assert!(commands.iter().any(|c| matches!(c,
            AggregateCommand::Order(OrderCommand::CancelOrder { .. })
        )));
        assert!(commands.iter().any(|c| matches!(c,
            AggregateCommand::Inventory(InventoryCommand::ReleaseReservation { .. })
        )));
    }

    #[tokio::test]
    async fn test_timeout_triggers_compensation() {
        let env = test_saga_environment();
        let saga = CheckoutSagaStore::new(CheckoutSagaReducer, env.clone());

        saga.send(CheckoutSagaAction::StartCheckout {
            customer_id: CustomerId::new(1),
            items: vec![LineItem { sku: "WIDGET".into(), quantity: 1 }],
            payment_method: PaymentMethod::CreditCard,
        }).await;

        // Simulate timeout (no InventoryReserved event arrives)
        let checkout_id = saga.state(|s| s.checkout_id).await;
        saga.send(CheckoutSagaAction::StepTimeout {
            checkout_id,
            step: CheckoutStep::Started,
        }).await;

        let state = saga.state(|s| s.clone()).await;
        assert!(matches!(state.current_step, CheckoutStep::Failed { .. }));
    }
}
```

### 6.4 Saga Patterns and Best Practices

#### Pattern 1: State Machine with Explicit Steps

The most common pattern—explicitly model workflow steps:

```rust
#[derive(Clone, Debug, PartialEq)]
enum WorkflowStep {
    Initial,
    Step1Complete,
    Step2Complete,
    Step3Complete,
    Finished,
    Failed { at_step: usize, reason: String },
}

// Match on (current_step, incoming_event) for clear state transitions
match (state.step, action) {
    (Initial, StartWorkflow) => { /* transition to step 1 */ }
    (Step1Complete, Event1) => { /* transition to step 2 */ }
    // ...
}
```

**When to use**: Most workflows with 3-10 sequential steps

#### Pattern 2: Parallel Steps with Synchronization

For workflows where multiple steps can run concurrently:

```rust
#[derive(Clone, Debug)]
struct ParallelWorkflowState {
    payment_complete: bool,
    inventory_reserved: bool,
    shipping_calculated: bool,
}

impl Reducer for ParallelWorkflowReducer {
    fn reduce(&self, state: &mut State, action: Action, env: &Env) -> Vec<Effect> {
        match action {
            StartWorkflow => {
                // Launch all parallel operations at once
                vec![
                    Effect::DispatchCommand(ProcessPayment { .. }),
                    Effect::DispatchCommand(ReserveInventory { .. }),
                    Effect::DispatchCommand(CalculateShipping { .. }),
                ]
            }
            PaymentCompleted => {
                state.payment_complete = true;
                self.check_all_complete(state)
            }
            InventoryReserved => {
                state.inventory_reserved = true;
                self.check_all_complete(state)
            }
            ShippingCalculated => {
                state.shipping_calculated = true;
                self.check_all_complete(state)
            }
        }
    }
}

impl ParallelWorkflowReducer {
    fn check_all_complete(&self, state: &State) -> Vec<Effect> {
        if state.payment_complete && state.inventory_reserved && state.shipping_calculated {
            vec![Effect::DispatchCommand(CompleteOrder)]
        } else {
            vec![Effect::None]
        }
    }
}
```

**When to use**: Steps that don't depend on each other and can run concurrently

#### Pattern 3: Retry with Exponential Backoff

For handling transient failures:

```rust
#[derive(Clone, Debug)]
struct WorkflowStateWithRetry {
    current_step: Step,
    retry_count: u32,
    max_retries: u32,
}

impl Reducer for RetryableWorkflowReducer {
    fn reduce(&self, state: &mut State, action: Action, env: &Env) -> Vec<Effect> {
        match action {
            StepFailed { error } if error.is_transient() => {
                if state.retry_count < state.max_retries {
                    state.retry_count += 1;
                    let delay = Duration::from_secs(2_u64.pow(state.retry_count));

                    vec![
                        Effect::Delay {
                            duration: delay,
                            action: Box::new(RetryStep),
                        }
                    ]
                } else {
                    // Max retries exceeded, compensate
                    state.current_step = Step::Failed;
                    self.compensate(state, env)
                }
            }
            StepSucceeded => {
                state.retry_count = 0; // Reset on success
                // Continue to next step...
                vec![/* ... */]
            }
            _ => vec![Effect::None],
        }
    }
}
```

**When to use**: External API calls or operations prone to transient failures

#### Pattern 4: Idempotency Keys

Ensure saga operations are idempotent:

```rust
#[derive(Clone, Debug)]
struct IdempotentSagaState {
    saga_id: SagaId,
    idempotency_key: String,
    completed_operations: HashSet<OperationId>,
}

impl Reducer for IdempotentSagaReducer {
    fn reduce(&self, state: &mut State, action: Action, env: &Env) -> Vec<Effect> {
        match action {
            ExecuteOperation { operation_id, .. } => {
                // Check if already executed
                if state.completed_operations.contains(&operation_id) {
                    return vec![Effect::None]; // Already done, skip
                }

                state.completed_operations.insert(operation_id);

                vec![
                    Effect::DispatchCommand(
                        Command::WithIdempotencyKey {
                            key: format!("{}-{}", state.saga_id, operation_id),
                            command: /* ... */
                        }
                    )
                ]
            }
        }
    }
}
```

**When to use**: All production sagas (prevents duplicate operations on replay)

#### Pattern 5: Saga Event Sourcing

Store saga state as events for complete audit trail:

```rust
#[derive(Clone, Serialize, Deserialize)]
enum SagaEvent {
    SagaStarted { saga_id: SagaId, timestamp: DateTime<Utc> },
    StepCompleted { step: Step, timestamp: DateTime<Utc> },
    StepFailed { step: Step, reason: String, timestamp: DateTime<Utc> },
    CompensationStarted { timestamp: DateTime<Utc> },
    SagaCompleted { timestamp: DateTime<Utc> },
}

impl SagaState {
    fn from_events(events: impl Iterator<Item = SagaEvent>) -> Self {
        events.fold(Self::default(), |mut state, event| {
            match event {
                SagaEvent::SagaStarted { saga_id, .. } => {
                    state.saga_id = saga_id;
                    state.status = SagaStatus::InProgress;
                }
                SagaEvent::StepCompleted { step, .. } => {
                    state.completed_steps.push(step);
                }
                SagaEvent::StepFailed { reason, .. } => {
                    state.status = SagaStatus::Failed { reason };
                }
                SagaEvent::SagaCompleted { .. } => {
                    state.status = SagaStatus::Completed;
                }
                _ => {}
            }
            state
        })
    }
}
```

**When to use**: When you need complete audit trail and time-travel debugging

#### Key Design Principles

1. **Make Illegal States Unrepresentable**
   ```rust
   // Bad: can have payment_id without order_id
   struct State {
       order_id: Option<OrderId>,
       payment_id: Option<PaymentId>,
   }

   // Good: payment_id only exists after order
   enum State {
       Initial,
       OrderCreated { order_id: OrderId },
       PaymentProcessed { order_id: OrderId, payment_id: PaymentId },
   }
   ```

2. **Always Track What You Need to Undo**
   - Store IDs of created resources
   - Track completed steps
   - Maintain compensation state

3. **Timeout Every Step**
   - External systems can hang
   - Always have a timeout escape hatch
   - Use `Effect::Delay` to schedule timeout actions

4. **Idempotency is Critical**
   - Events may be replayed
   - Use idempotency keys
   - Track completed operations

5. **Test Both Paths**
   - Happy path (all steps succeed)
   - Unhappy path (each possible failure point)
   - Timeout scenarios
   - Compensation logic

#### Common Pitfalls to Avoid

**❌ Don't await in reducers**
```rust
// BAD - this violates the pure function principle
fn reduce(&self, state: &mut State, action: Action, env: &Env) -> Vec<Effect> {
    let result = env.database.save(state).await; // ❌ Don't await!
    vec![]
}
```

**✓ Return effects instead**
```rust
// GOOD - return effect description
fn reduce(&self, state: &mut State, action: Action, env: &Env) -> Vec<Effect> {
    vec![Effect::Database(DbOperation::Save(state.clone()))]
}
```

**❌ Don't forget compensation**
```rust
// BAD - no way to undo partial work
(Step3, StepFailed) => {
    vec![Effect::PublishEvent(Event::Failed)] // What about step 1 and 2?
}
```

**✓ Always compensate**
```rust
// GOOD - undo everything we did
(Step3, StepFailed) => {
    self.compensate(state, env) // Undoes step 1, 2, and 3
}
```

**❌ Don't ignore out-of-order events**
```rust
// BAD - might process events in wrong order
(current_step, action) => {
    // Process any action regardless of step
}
```

**✓ Validate state transitions**
```rust
// GOOD - only process valid transitions
match (state.current_step, action) {
    (Step1, Event1) => { /* ok */ }
    (Step2, Event2) => { /* ok */ }
    _ => vec![Effect::None] // Ignore invalid transitions
}
```

### 6.5 Event Routing and Inter-Aggregate Communication

A critical question: **How do events from one aggregate reach sagas and other aggregates?**

#### The Event Bus Pattern

When an aggregate emits an event via `Effect::PublishEvent`, the Store's effect executor publishes it to an **Event Bus**. Other stores (sagas, projections, other aggregates) subscribe to events they care about.

**Conceptual Flow**:

```
OrderAggregate
    ↓ emit Effect::PublishEvent(OrderPlaced)
    ↓
Store.execute_effect()
    ↓ publish to event bus
    ↓
EventBus
    ├─→ CheckoutSaga (subscribed to OrderPlaced)
    ├─→ OrderProjection (subscribed to OrderPlaced)
    └─→ InventoryAggregate (subscribed to OrderPlaced)
```

#### Subscription Mechanisms

**Option 1: Direct Subscription** (simplest, for single-process systems)

```rust
// Event bus routes events to subscribed stores
struct EventBus {
    subscribers: HashMap<EventType, Vec<Arc<dyn Store>>>,
}

impl EventBus {
    async fn publish(&self, event: Event) {
        let event_type = event.event_type();
        if let Some(stores) = self.subscribers.get(&event_type) {
            for store in stores {
                // Convert event to appropriate action and send
                let action = event.to_action();
                store.send(action).await;
            }
        }
    }
}
```

**Option 2: Message Queue** (distributed systems)

```rust
// Aggregates publish to Kafka/NATS
Effect::PublishEvent(event) → Kafka topic

// Sagas/Projections consume from topics
kafka_consumer.consume(topic)
    ↓ deserialize event
    ↓ convert to action
    ↓ saga_store.send(action)
```

#### Event Correlation

**How does a saga know which events are "for it"?**

Events carry correlation IDs:

```rust
#[derive(Clone)]
enum OrderAction {
    // Saga includes its ID in commands
    PlaceOrder {
        saga_id: SagaId,  // ← Added by saga
        customer_id: CustomerId,
        items: Vec<LineItem>,
    },

    // Events echo the saga_id back
    OrderPlaced {
        saga_id: SagaId,  // ← Saga can match this
        order_id: OrderId,
        timestamp: DateTime<Utc>,
    },
}

// Saga filters events
impl Reducer for CheckoutSagaReducer {
    fn reduce(&self, state: &mut State, action: Action, env: &Env) -> Vec<Effect> {
        match action {
            OrderPlaced { saga_id, order_id, .. } if saga_id == state.saga_id => {
                // This event is for us!
                state.order_id = Some(order_id);
                // ... proceed with workflow
            }
            _ => vec![Effect::None]  // Not for us, ignore
        }
    }
}
```

#### Implementation Strategies

**Phase 1**: In-memory event bus for single-process systems
- Simple HashMap-based routing
- Good for development and testing
- Easy to reason about

**Phase 2**: Message queue for distributed systems
- Kafka, NATS, or RabbitMQ
- Durable event log
- Multiple consumers per event
- At-least-once delivery semantics

**Phase 3**: Event Store with projections
- Purpose-built event storage (EventStoreDB, custom)
- Event sourcing native
- Time-travel and replay
- Subscription with catch-up

#### Key Design Principles

1. **Events are broadcast**: Any number of consumers can react to an event
2. **Stores are decoupled**: Aggregates don't know who subscribes to their events
3. **Correlation is explicit**: Use IDs to track which saga/workflow an event belongs to
4. **Routing is pluggable**: The event bus mechanism is swappable (in-memory → Kafka → EventStore)

**Implementation Note**: The exact event routing mechanism will be designed during Phase 2 (CQRS/Event Sourcing implementation). The architecture supports multiple approaches—choose based on system requirements (single-process vs distributed, throughput needs, durability requirements).

---

## 7. Testing Strategy

### 7.1 Unit Testing Reducers

Reducers are pure functions - trivial to test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_place_order() {
        // Arrange
        let mut state = OrderState::default();
        let env = test_environment();
        let reducer = OrderReducer;

        let action = OrderAction::PlaceOrder {
            customer_id: CustomerId::new(1),
            items: vec![
                LineItem { sku: "WIDGET-1".into(), quantity: 2 },
            ],
        };

        // Act
        let effects = reducer.reduce(&mut state, action, &env);

        // Assert
        assert_eq!(state.orders.len(), 1);
        assert_eq!(effects.len(), 2); // Save + Publish

        let order = state.orders.values().next().unwrap();
        assert_eq!(order.customer_id, CustomerId::new(1));
        assert_eq!(order.items.len(), 1);
    }

    fn test_environment() -> OrderEnvironment {
        OrderEnvironment {
            database: Arc::new(MockDatabase::new()),
            clock: Arc::new(FixedClock::new(test_time())),
            events: Arc::new(MockEventPublisher::new()),
            ids: Arc::new(SequentialIdGenerator::new()),
        }
    }
}
```

### 7.2 Property-Based Testing

Use proptest for exhaustive testing:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn order_state_is_consistent(events: Vec<OrderEvent>) {
        let state = OrderState::from_events(events.iter().cloned());

        // Invariants
        for order in state.orders.values() {
            prop_assert!(!order.items.is_empty());
            prop_assert!(order.placed_at <= Utc::now());
        }
    }

    #[test]
    fn events_are_idempotent(events: Vec<OrderEvent>) {
        let state1 = OrderState::from_events(events.iter().cloned());
        let state2 = OrderState::from_events(
            events.iter().cloned().chain(events.iter().cloned())
        );

        // Applying events twice should be same as once
        prop_assert_eq!(state1.orders.len(), state2.orders.len());
    }
}
```

### 7.3 Integration Testing

Test with real dependencies in isolated environments:

```rust
#[tokio::test]
async fn test_order_flow_integration() {
    // Use testcontainers for real Postgres
    let docker = clients::Cli::default();
    let postgres = docker.run(images::postgres::Postgres::default());

    let db = PostgresDatabase::connect(&postgres.connection_string()).await.unwrap();

    let env = OrderEnvironment {
        database: Arc::new(db),
        clock: Arc::new(SystemClock),
        events: Arc::new(InMemoryEventPublisher::new()),
        ids: Arc::new(UuidGenerator),
    };

    let store = OrderStore::new(OrderReducer, env);

    // Exercise the full flow
    store.send(OrderAction::PlaceOrder {
        customer_id: CustomerId::new(1),
        items: vec![LineItem { sku: "TEST".into(), quantity: 1 }],
    }).await;

    // Verify state
    let state = store.state(|s| s.clone()).await;
    assert_eq!(state.orders.len(), 1);
}
```

### 7.4 Test Helpers

Provide builders for common test scenarios:

```rust
mod test_helpers {
    pub struct OrderBuilder {
        customer_id: CustomerId,
        items: Vec<LineItem>,
    }

    impl OrderBuilder {
        pub fn new() -> Self {
            Self {
                customer_id: CustomerId::new(1),
                items: vec![],
            }
        }

        pub fn customer(mut self, id: u64) -> Self {
            self.customer_id = CustomerId::new(id);
            self
        }

        pub fn item(mut self, sku: &str, quantity: u32) -> Self {
            self.items.push(LineItem {
                sku: sku.to_string(),
                quantity,
            });
            self
        }

        pub fn build(self) -> OrderAction {
            OrderAction::PlaceOrder {
                customer_id: self.customer_id,
                items: self.items,
            }
        }
    }
}

// Usage in tests
#[test]
fn test_with_builder() {
    let action = OrderBuilder::new()
        .customer(42)
        .item("WIDGET-1", 2)
        .item("WIDGET-2", 1)
        .build();

    // Test with action...
}
```

---

## 8. Performance Considerations

### 8.1 Zero-Cost Abstractions

With static dispatch, the architecture compiles to optimal code:

```rust
// This generic code:
fn reduce<D: Database>(state: &mut State, action: Action, db: &D) -> Vec<Effect> {
    db.save(state);
    vec![]
}

// Monomorphizes to this at compile time:
fn reduce_with_postgres(state: &mut State, action: Action, db: &PostgresDatabase) -> Vec<Effect> {
    db.save(state);  // Direct call, no vtable
    vec![]
}
```

LLVM can then inline, optimize, and vectorize as if you wrote it by hand.

### 8.2 Allocation Strategy

Minimize allocations in hot paths:

```rust
// Bad: allocates on every reduction
fn reduce(&self, state: &mut State, action: Action, env: &Env) -> Vec<Effect> {
    if should_save {
        vec![Effect::Save]  // Allocation
    } else {
        vec![]  // Another allocation
    }
}

// Good: use SmallVec or static array
fn reduce(&self, state: &mut State, action: Action, env: &Env) -> SmallVec<[Effect; 4]> {
    let mut effects = SmallVec::new();  // Stack allocated for <= 4 items
    if should_save {
        effects.push(Effect::Save);
    }
    effects
}
```

### 8.3 State Mutation vs Immutability

Allow pragmatic mutation in reducers:

```rust
// Immutable - clear but allocates
fn reduce(state: State, action: Action) -> (State, Vec<Effect>) {
    let mut new_state = state.clone();
    new_state.apply(action);
    (new_state, vec![])
}

// Mutable - faster, still testable
fn reduce(&self, state: &mut State, action: Action, env: &Env) -> Vec<Effect> {
    state.apply(action);  // In-place mutation
    vec![]
}
```

For most backends, the mutable approach is fine and performant.

### 8.4 Async Performance

Use structured concurrency and avoid spawning when possible:

```rust
// Sequential - slow
async fn execute_effects(effects: Vec<Effect>) {
    for effect in effects {
        execute(effect).await;  // Wait for each
    }
}

// Concurrent - fast
async fn execute_effects(effects: Vec<Effect>) {
    let futures = effects.into_iter().map(execute);
    futures::future::join_all(futures).await;  // All at once
}
```

### 8.5 Benchmarking Targets

Expected performance characteristics:

- **Reducer execution**: < 100ns for simple actions (pure function, no allocation)
- **State mutation**: < 1μs for typical aggregate updates
- **Effect dispatch**: < 10μs to enqueue effects
- **Test throughput**: > 10,000 reducer tests per second
- **Event replay**: > 100,000 events/second for rebuilding state

Use criterion.rs for benchmarking:

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn benchmark_reduce(c: &mut Criterion) {
    let mut state = OrderState::default();
    let env = test_environment();
    let reducer = OrderReducer;
    let action = place_order_action();

    c.bench_function("reduce_place_order", |b| {
        b.iter(|| {
            reducer.reduce(black_box(&mut state), black_box(action.clone()), &env)
        })
    });
}

criterion_group!(benches, benchmark_reduce);
criterion_main!(benches);
```

---

## 9. Expected Dependencies

The architecture relies on several well-established Rust ecosystem crates. Exact versions will be determined during implementation, but the core dependencies are:

**Core Runtime**:
- `tokio` - Async runtime for effect execution
- `futures` - Async utilities (note: `BoxFuture` no longer needed with Edition 2024's `async fn` in traits)

**Serialization** (for event sourcing):
- `serde` - Serialization framework
- `serde_json` - JSON serialization for events

**Time** (for timestamps, scheduling):
- `chrono` or `time` - DateTime handling

**Data Structures** (optimization):
- `smallvec` - Stack-allocated vectors for effects
- `dashmap` - Concurrent HashMap (if needed)

**Database** (Phase 2+):
- `sqlx` - Async SQL (Postgres support)
- `tokio-postgres` - Alternative Postgres driver

**Messaging** (Phase 2+):
- `rdkafka` or `async-nats` - Event bus implementation

**Testing**:
- `proptest` - Property-based testing
- `criterion` - Benchmarking
- `testcontainers` - Integration tests with real databases

**Observability** (Phase 4):
- `tracing` - Structured logging and distributed tracing
- `metrics` - Metrics collection
- `opentelemetry` - Distributed tracing standard

**Implementation Note**: These are guidelines, not requirements. Phase-specific implementation specs will determine exact dependencies and versions.

---

## 10. Implementation Roadmap

### Phase 1: Core Foundation (Weeks 1-2)

**Deliverables:**
- [ ] Core trait definitions (Reducer, Environment)
- [ ] Store implementation with basic effect execution
- [ ] Effect type and combinators
- [ ] Basic dependency traits (Database, Clock, EventPublisher)
- [ ] Documentation and examples

**Success Criteria:**
- Can implement a simple reducer
- Can test reducer in isolation
- Can swap environments (prod/test)

### Phase 2: CQRS/Event Sourcing (Weeks 3-4)

**Deliverables:**
- [ ] Event store trait and implementations
- [ ] Command/Event patterns
- [ ] Aggregate root abstraction
- [ ] Event replay and state reconstruction
- [ ] Snapshot support for large aggregates

**Success Criteria:**
- Can build event-sourced aggregates
- Can replay events to rebuild state
- Can take and restore from snapshots

### Phase 3: Composition & Coordination (Weeks 5-6)

**Deliverables:**
- [ ] Reducer composition utilities (combine, scope)
- [ ] Saga testing utilities and helpers
- [ ] Effect middleware (logging, tracing, retry policies)
- [ ] Event bus implementation (in-memory, then message queue)
- [ ] Multi-store coordination patterns
- [ ] Distributed tracing integration

**Success Criteria:**
- Can compose multiple reducers
- Can implement and test complex workflows (sagas)
- Events route correctly between aggregates
- Sagas can coordinate multi-aggregate transactions

### Phase 4: Production Hardening (Weeks 7-8)

**Deliverables:**
- [ ] Performance optimization pass
- [ ] Production database implementations (Postgres)
- [ ] Event bus implementations (Kafka, NATS)
- [ ] Observability (metrics, tracing)
- [ ] Error handling and recovery
- [ ] Comprehensive test suite

**Success Criteria:**
- Benchmarks meet targets
- Can run in production
- Full observability
- Failure scenarios handled gracefully

### Phase 5: Developer Experience (Weeks 9-10)

**Deliverables:**
- [ ] Macro for deriving Reducer
- [ ] Testing utilities and helpers
- [ ] Code generation for boilerplate
- [ ] Example applications
- [ ] Tutorial and guides
- [ ] API documentation

**Success Criteria:**
- Minimal boilerplate for new features
- Easy to write tests
- Clear examples for common patterns
- Comprehensive documentation

---

## 10. Example Application

### 10.1 Domain: Order Processing

A complete example of an order processing system:

```rust
// ============================================================================
// Domain Types
// ============================================================================

#[derive(Clone, Debug, PartialEq)]
struct Order {
    id: OrderId,
    customer_id: CustomerId,
    items: Vec<LineItem>,
    status: OrderStatus,
    placed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
enum OrderStatus {
    Pending,
    PaymentConfirmed,
    Shipped { tracking: String },
    Delivered,
    Cancelled { reason: String },
}

// ============================================================================
// State
// ============================================================================

#[derive(Clone, Debug, Default)]
struct OrderState {
    orders: HashMap<OrderId, Order>,
    pending_payments: HashSet<OrderId>,
}

// ============================================================================
// Actions
// ============================================================================

#[derive(Clone, Debug)]
enum OrderAction {
    // Commands
    PlaceOrder {
        customer_id: CustomerId,
        items: Vec<LineItem>,
    },
    ConfirmPayment {
        order_id: OrderId,
    },
    ShipOrder {
        order_id: OrderId,
        tracking: String,
    },
    CancelOrder {
        order_id: OrderId,
        reason: String,
    },

    // Events (for replay)
    OrderPlaced {
        order_id: OrderId,
        customer_id: CustomerId,
        items: Vec<LineItem>,
        timestamp: DateTime<Utc>,
    },
    PaymentConfirmed {
        order_id: OrderId,
        timestamp: DateTime<Utc>,
    },
    OrderShipped {
        order_id: OrderId,
        tracking: String,
        timestamp: DateTime<Utc>,
    },
    OrderCancelled {
        order_id: OrderId,
        reason: String,
        timestamp: DateTime<Utc>,
    },
}

// ============================================================================
// Environment
// ============================================================================

struct OrderEnvironment<D, C, E, I> {
    database: D,
    clock: C,
    events: E,
    ids: I,
}

// ============================================================================
// Reducer
// ============================================================================

struct OrderReducer;

impl<D, C, E, I> Reducer for OrderReducer
where
    D: Database,
    C: Clock,
    E: EventPublisher,
    I: IdGenerator,
{
    type State = OrderState;
    type Action = OrderAction;
    type Environment = OrderEnvironment<D, C, E, I>;

    fn reduce(
        &self,
        state: &mut OrderState,
        action: OrderAction,
        env: &OrderEnvironment<D, C, E, I>,
    ) -> Vec<Effect<OrderAction>> {
        match action {
            // Command: Place order
            OrderAction::PlaceOrder { customer_id, items } => {
                if items.is_empty() {
                    return vec![Effect::None];
                }

                let order_id = env.ids.next_id().into();
                let timestamp = env.clock.now();

                let event = OrderAction::OrderPlaced {
                    order_id,
                    customer_id,
                    items: items.clone(),
                    timestamp,
                };

                // Apply event
                state.orders.insert(order_id, Order {
                    id: order_id,
                    customer_id,
                    items,
                    status: OrderStatus::Pending,
                    placed_at: timestamp,
                });
                state.pending_payments.insert(order_id);

                vec![
                    Effect::Database(DbOperation::SaveEvent(order_id, event.clone())),
                    Effect::PublishEvent(Event::Order(event)),
                ]
            }

            // Command: Confirm payment
            OrderAction::ConfirmPayment { order_id } => {
                if !state.pending_payments.contains(&order_id) {
                    return vec![Effect::None];
                }

                let timestamp = env.clock.now();
                let event = OrderAction::PaymentConfirmed { order_id, timestamp };

                if let Some(order) = state.orders.get_mut(&order_id) {
                    order.status = OrderStatus::PaymentConfirmed;
                    state.pending_payments.remove(&order_id);
                }

                vec![
                    Effect::Database(DbOperation::SaveEvent(order_id, event.clone())),
                    Effect::PublishEvent(Event::Order(event)),
                ]
            }

            // Command: Ship order
            OrderAction::ShipOrder { order_id, tracking } => {
                let Some(order) = state.orders.get(&order_id) else {
                    return vec![Effect::None];
                };

                if !matches!(order.status, OrderStatus::PaymentConfirmed) {
                    return vec![Effect::None];
                }

                let timestamp = env.clock.now();
                let event = OrderAction::OrderShipped {
                    order_id,
                    tracking: tracking.clone(),
                    timestamp,
                };

                if let Some(order) = state.orders.get_mut(&order_id) {
                    order.status = OrderStatus::Shipped { tracking };
                }

                vec![
                    Effect::Database(DbOperation::SaveEvent(order_id, event.clone())),
                    Effect::PublishEvent(Event::Order(event)),
                ]
            }

            // Events (idempotent)
            OrderAction::OrderPlaced { .. } |
            OrderAction::PaymentConfirmed { .. } |
            OrderAction::OrderShipped { .. } |
            OrderAction::OrderCancelled { .. } => {
                // Events are applied during command handling
                // When replaying, we'd apply them here
                vec![Effect::None]
            }

            _ => vec![Effect::None],
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_order_flow() {
        let env = OrderEnvironment {
            database: MockDatabase::new(),
            clock: FixedClock::new(test_time()),
            events: MockEventPublisher::new(),
            ids: SequentialIdGenerator::new(),
        };

        let store = OrderStore::new(OrderReducer, env);

        // Place order
        store.send(OrderAction::PlaceOrder {
            customer_id: CustomerId::new(1),
            items: vec![
                LineItem { sku: "WIDGET-1".into(), quantity: 2 },
            ],
        }).await;

        let state = store.state(|s| s.clone()).await;
        assert_eq!(state.orders.len(), 1);

        let order_id = state.orders.keys().next().unwrap();

        // Confirm payment
        store.send(OrderAction::ConfirmPayment {
            order_id: *order_id
        }).await;

        let state = store.state(|s| s.clone()).await;
        let order = state.orders.get(order_id).unwrap();
        assert!(matches!(order.status, OrderStatus::PaymentConfirmed));

        // Ship order
        store.send(OrderAction::ShipOrder {
            order_id: *order_id,
            tracking: "TRACK123".to_string(),
        }).await;

        let state = store.state(|s| s.clone()).await;
        let order = state.orders.get(order_id).unwrap();
        assert!(matches!(order.status, OrderStatus::Shipped { .. }));
    }
}
```

---

## 11. Open Questions & Future Considerations

### 11.1 Alternative Saga Patterns

**Current Approach**: Sagas are modeled as reducers with explicit state machines (see Section 6.3).

**Future Considerations**: If we observe repetitive patterns across many sagas, we could explore:

1. **Declarative Workflow DSL**
   ```rust
   // Define workflows declaratively
   let workflow = WorkflowBuilder::new()
       .step("reserve_inventory")
           .execute(|state, env| /* ... */)
           .wait_for(|action| matches!(action, InventoryReserved { .. }))
           .compensate(|state, env| /* ... */)
       .step("create_order")
           .execute(|state, env| /* ... */)
           .wait_for(|action| matches!(action, OrderCreated { .. }))
       .build();
   ```

   **Pros**: Less boilerplate for simple workflows
   **Cons**: Hides control flow, harder to debug, adds abstraction

2. **Workflow Effect Interpreter**
   ```rust
   // Effects describe multi-step workflows
   Effect::Workflow {
       steps: vec![
           Step::command(ReserveInventory),
           Step::wait_for(|a| matches!(a, InventoryReserved { .. })),
           Step::command(CreateOrder),
       ],
       on_failure: compensate_workflow,
   }
   ```

   **Pros**: Compact representation
   **Cons**: Complex runtime, harder to reason about

**Decision**: Only add these abstractions if:
- We have 50+ sagas and see clear repetitive patterns
- The benefits outweigh the added complexity
- The DSL/interpreter can be built **on top of** the reducer pattern (not replacing it)

**Recommendation**: Start with Approach 1 (Saga as Reducer). Wait for real-world usage to inform whether higher-level abstractions are needed.

### 11.2 Concurrency Model

**Question**: How do we handle concurrent commands to the same aggregate?

**Options**:
1. Optimistic concurrency with version numbers
2. Lock-based (one command at a time per aggregate)
3. Actor model (one task per aggregate)

**Recommendation**: Start with option 2 (simple), evolve to 3 if needed.

### 11.2 Snapshot Strategy

**Question**: When should we snapshot large aggregates?

**Options**:
1. Every N events
2. Time-based (every hour)
3. Size-based (when state > X bytes)
4. Manual triggers

**Recommendation**: Configurable strategy, default to every 100 events.

### 11.3 Schema Evolution

**Question**: How do we handle event schema changes over time?

**Options**:
1. Upcasting (transform old events to new schema on read)
2. Versioned events (multiple versions coexist)
3. Event migration (background process)

**Recommendation**: Versioned events with upcasting layer.

### 11.4 Distributed Tracing

**Question**: How do we trace commands through sagas and multiple aggregates?

**Recommendation**: OpenTelemetry integration with correlation IDs in all events and effects.

### 11.5 Hot Reloading

**Question**: Can we hot-reload reducers in development?

**Feasibility**: Difficult with static dispatch, but possible with dynamic dispatch + plugin system.

---

## 12. Success Metrics

The architecture is successful if it delivers on these goals:

### Developer Experience
- [ ] Simple features can be added in < 1 hour (once familiar with patterns)
- [ ] Refactoring is safe and compiler-guided (exhaustive matching catches regressions)
- [ ] New developers productive within 1 week (with good examples and docs)
- [ ] Code is self-documenting (Action enum shows all capabilities)

### Testing
- [ ] 100% test coverage of business logic (reducers are pure, easy to test)
- [ ] Test suite scales well (target: 1000 unit tests run in < 5 seconds)
- [ ] Integration tests are fast (target: < 1 minute for typical suite)
- [ ] Tests are deterministic (no flakes due to controlled time/dependencies)

### Performance
- [ ] Reducer overhead is negligible (< 1% of request time for typical aggregates)
- [ ] Scales with cores (target: 10,000+ simple commands/sec per core)
- [ ] Event replay is fast (target: > 100,000 events/sec for aggregate reconstruction)
- [ ] Zero-cost abstractions verified (benchmarks show static dispatch has no overhead)

### Reliability
- [ ] No data loss (event sourcing guarantees)
- [ ] Full audit trail for debugging
- [ ] Graceful degradation on failure
- [ ] Observable (logs, metrics, traces)

### Maintainability
- [ ] Refactoring large changes takes days, not weeks
- [ ] Type system catches bugs at compile time
- [ ] Clear boundaries between components
- [ ] Documentation stays in sync with code

---

## Conclusion

**Composable Rust** brings the battle-tested principles of functional architecture to Rust backends, creating a framework optimized for:

- **Correctness**: Type-safe state machines and event sourcing
- **Performance**: Zero-cost abstractions with static dispatch
- **Testability**: Pure functions and dependency injection
- **Maintainability**: Self-documenting code and safe refactoring

This is an architecture for the long haul—systems that will run in production for years, where bugs are expensive, and correctness cannot be compromised.

The upfront investment in structure and types pays dividends in faster development, safer refactoring, and dramatically faster test suites.

**Next steps**: Proceed with Phase 1 implementation and validate core concepts with a working prototype.
