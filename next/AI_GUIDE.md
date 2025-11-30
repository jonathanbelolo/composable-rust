# Composable Rust Next: AI Implementation Guide

This document teaches AI assistants how to understand, use, and implement business logic using the `composable-rust-next` library. It covers architecture, patterns, guidelines, and common pitfalls.

---

## Table of Contents

1. [Core Philosophy](#core-philosophy)
2. [Architecture Overview](#architecture-overview)
3. [The Five Fundamental Types](#the-five-fundamental-types)
4. [Aggregates vs Sagas vs Queries](#aggregates-vs-sagas-vs-queries)
5. [The CQRS Flow](#the-cqrs-flow)
6. [Implementation Guide](#implementation-guide)
7. [Testing Patterns](#testing-patterns)
8. [Guidelines and Best Practices](#guidelines-and-best-practices)
9. [Common Mistakes to Avoid](#common-mistakes-to-avoid)
10. [Quick Reference](#quick-reference)

---

## Core Philosophy

### Separation of Concerns

The Next library enforces a strict separation between:

| Layer | Responsibility | Purity |
|-------|---------------|--------|
| **BusinessLogic** | Domain rules, validation, state transitions | Pure (no I/O) |
| **Handler** | Orchestration, persistence, broadcasting | Impure (I/O) |
| **Environment** | Infrastructure dependencies | Configuration |

### CQRS (Command Query Responsibility Segregation)

**Critical Understanding**: This library implements true CQRS.

- **Reads**: Always from projections (read models), NEVER from event store
- **Writes**: Validate against projection data, append to event store
- **Event Store Purpose**: Only for (1) appending events and (2) rebuilding projections

### Event Sourcing

State is derived from events, not stored directly:

```
Events → fold/apply → Current State
```

The `apply()` method rebuilds state from events. This is used for projection rebuilding, NOT for command validation.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                              Handler                                     │
│                                                                         │
│  INPUT ──► QueryFetcher.fetch() ──► BusinessLogic.process()             │
│                │                            │                           │
│                ▼                            ▼                           │
│         (prepared_input,              BusinessResult                    │
│          expected_version)                  │                           │
│                                             │                           │
│         ┌───────────────────────────────────┼─────────────────────────┐ │
│         │                                   │                         │ │
│         ▼                                   ▼                         ▼ │
│      Respond                           Done/Continue                    │
│         │                                   │                           │
│         ▼                                   ▼                           │
│   Return query data              Persist → Project → Broadcast          │
│   (no persistence)                         │                           │
│                                            ▼                           │
│                                    Return HandleResult                  │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## The Five Fundamental Types

### 1. `BusinessLogic` Trait

The unified trait for all domain logic—aggregates, sagas, and queries.

```rust
pub trait BusinessLogic: Send + Sync + 'static {
    type State: Default + Send + Sync;      // For projection rebuilding
    type Input: Send;                        // Commands/events with fetched data
    type Event: Serialize + DeserializeOwned + Send + Sync + Clone;
    type Error: std::error::Error + Send + Sync;
    type Call: Send;                         // Saga calls (Infallible for aggregates)
    type CallResult: Send;                   // Saga results (Infallible for aggregates)
    type Response: Send;                     // Query response type

    fn stream_id(input: &Self::Input) -> StreamId;
    fn process(&self, input: Self::Input, clock: &dyn Clock)
        -> Result<BusinessResult<Self::Event, Self::Call, Self::Response>, Self::Error>;
    fn apply(&self, state: &mut Self::State, event: &Self::Event);
    fn event_type_name(event: &Self::Event) -> &'static str;

    // Only for sagas - default panics (aggregates never call this)
    fn feedback_input(results: Vec<Self::CallResult>) -> Self::Input;
}
```

### 2. `BusinessResult` Enum

The return type from `process()` indicating what happens next:

```rust
pub enum BusinessResult<E, C, R = ()> {
    Done(Vec<E>),                    // Emit events and finish
    Continue { events: Vec<E>, calls: Vec<C> },  // Emit, call aggregates, continue
    Respond(R),                      // Return query data (no persistence)
}
```

| Variant | Used By | Behavior |
|---------|---------|----------|
| `Done(events)` | Aggregates, Sagas (final step) | Persist events, finish |
| `Continue { events, calls }` | Sagas only | Persist, execute calls, loop back |
| `Respond(data)` | Queries | Return data, no persistence |

### 3. `Handler` Struct

Orchestrates the complete flow with retry logic and safety guards:

```rust
let handler = Handler::new(
    MyBusinessLogic,      // Your domain logic
    NoOpCallExecutor,     // Or saga call executor
    MyQueryFetcher,       // Fetches projection data
    environment,          // Infrastructure dependencies
);

let result = handler.handle(command).await?;
```

**Important**: The `Handler` requires `T::Input: Clone` because inputs may need to be cloned for retry logic on version conflicts.

### 4. `HandlerEnvironment` Trait

Provides infrastructure dependencies to the Handler:

```rust
pub trait HandlerEnvironment: Send + Sync {
    type Clock: Clock;
    type EventStore: EventStore;
    type Projector: Projector;
    type EventBus: EventBus;
    type Projections: ProjectionQueries;

    fn clock(&self) -> &Self::Clock;
    fn event_store(&self) -> &Self::EventStore;
    fn projector(&self) -> Option<&Self::Projector>;
    fn event_bus(&self) -> Option<&Self::EventBus>;
    fn broadcast_topic(&self) -> &str;
    fn projections(&self) -> &Self::Projections;
    fn metadata(&self) -> &MetadataContext;
}
```

### 5. `QueryFetcher` Trait

Fetches projection data BEFORE business logic runs:

```rust
pub trait QueryFetcher<Input, Projections>: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn fetch(
        &self,
        input: Input,
        projections: &Projections,
    ) -> Result<FetchResult<Input>, Self::Error>;
}
```

**Key Insight**: The fetcher returns `FetchResult { input, expected_version }`. The `input` has its `fetched` field populated with projection data.

---

## Aggregates vs Sagas vs Queries

### Aggregates

Aggregates process commands and emit events. They NEVER call other aggregates.

```rust
impl BusinessLogic for OrderAggregate {
    type Call = Infallible;        // Never makes calls
    type CallResult = Infallible;  // Never receives results
    type Response = OrderResponse; // For queries

    fn process(&self, input: OrderCommand, clock: &dyn Clock)
        -> Result<BusinessResult<OrderEvent, Infallible, OrderResponse>, OrderError>
    {
        match input {
            OrderCommand::Create { order_id, items, fetched } => {
                // Validate using fetched projection data
                if fetched.is_some() {
                    return Err(OrderError::AlreadyExists);
                }
                Ok(BusinessResult::Done(vec![OrderEvent::Created {
                    order_id,
                    items,
                    created_at: clock.now(),
                }]))
            }
            OrderCommand::GetOrder { fetched, .. } => {
                let data = fetched.ok_or(OrderError::NotFound)?;
                Ok(BusinessResult::Respond(OrderResponse::Single(data)))
            }
        }
    }
}
```

### Sagas

Sagas coordinate multiple aggregates. They use `Continue` to make calls and receive feedback.

```rust
impl BusinessLogic for CheckoutSaga {
    type Call = CheckoutCall;
    type CallResult = CheckoutCallResult;
    type Response = ();

    fn process(&self, input: SagaInput, clock: &dyn Clock)
        -> Result<BusinessResult<SagaEvent, CheckoutCall, ()>, SagaError>
    {
        match input {
            SagaInput::Start { saga_id, order_id, amount, fetched } => {
                // Start the saga - call the first aggregate
                Ok(BusinessResult::Continue {
                    events: vec![SagaEvent::Started { saga_id, order_id, amount }],
                    calls: vec![CheckoutCall::ReserveInventory { order_id }],
                })
            }
            SagaInput::Feedback { results, fetched } => {
                let saga_state = fetched.ok_or(SagaError::NotFound)?;
                match (saga_state.phase, &results[..]) {
                    (Phase::ReservingInventory, [CheckoutCallResult::InventoryReserved]) => {
                        // Next step: process payment (amount from saga state)
                        Ok(BusinessResult::Continue {
                            events: vec![SagaEvent::InventoryReserved],
                            calls: vec![CheckoutCall::ProcessPayment {
                                amount: saga_state.amount,
                            }],
                        })
                    }
                    (Phase::ProcessingPayment, [CheckoutCallResult::PaymentProcessed]) => {
                        // Done!
                        Ok(BusinessResult::Done(vec![SagaEvent::Completed]))
                    }
                    (_, [CheckoutCallResult::Failed { reason }]) => {
                        // Compensate (clone reason from the result)
                        Ok(BusinessResult::Continue {
                            events: vec![SagaEvent::Failed { reason: reason.clone() }],
                            calls: vec![CheckoutCall::ReleaseInventory],
                        })
                    }
                    _ => Err(SagaError::InvalidTransition),
                }
            }
        }
    }

    // REQUIRED for sagas - converts call results to input
    fn feedback_input(results: Vec<CheckoutCallResult>) -> SagaInput {
        SagaInput::Feedback { results, fetched: None }
    }
}
```

### Queries

Queries return data without persistence. They use the `Respond` variant.

```rust
// Inside process()
OrderCommand::GetOrderHistory { user_id, fetched } => {
    let orders = fetched.ok_or(OrderError::NotFound)?;
    Ok(BusinessResult::Respond(OrderResponse::History(orders)))
}
```

---

## The CQRS Flow

### Step-by-Step Execution

1. **Input arrives** at `handler.handle(input)`

2. **QueryFetcher.fetch()** is called:
   - Examines the input to determine what data is needed
   - Queries projections (NOT event store)
   - Returns prepared input with `fetched` field populated
   - Returns `expected_version` for optimistic concurrency

3. **BusinessLogic.process()** is called:
   - Receives prepared input with fetched data
   - Validates against fetched projection data
   - Returns `BusinessResult`

4. **Based on BusinessResult**:
   - `Respond(data)`: Return immediately, no persistence
   - `Done(events)`: Persist → Project → Broadcast → Return
   - `Continue { events, calls }`: Persist → Project → Broadcast → Execute calls → Loop back

5. **On version conflict**: Retry from step 2 (up to `max_retries`)

6. **Safety guard**: Saga loops abort after `max_saga_iterations` (default 100)

### Version Tracking

```rust
// QueryFetcher returns expected version from projection
FetchResult {
    input: prepared_input,
    expected_version: Some(Version::new(5)), // From projection
}

// Handler passes to event store
event_store.append(stream_id, expected_version, events).await?;

// On conflict (another write happened), retry with fresh data
```

---

## Implementation Guide

### Step 1: Define Your Types

```rust
// Events (what happened)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrderEvent {
    Created { order_id: Uuid, items: Vec<Item>, created_at: DateTime<Utc> },
    Confirmed { confirmed_at: DateTime<Utc> },
    Cancelled { reason: String, cancelled_at: DateTime<Utc> },
}

// Commands (what to do) - includes `fetched` field for projection data
// NOTE: Must derive Clone - Handler requires Input: Clone for retry logic
#[derive(Debug, Clone)]
pub enum OrderCommand {
    Create { order_id: Uuid, items: Vec<Item>, fetched: Option<OrderDto> },
    Confirm { order_id: Uuid, fetched: Option<OrderDto> },
    GetOrder { order_id: Uuid, fetched: Option<OrderDto> },
}

// State (for projection rebuilding only)
#[derive(Debug, Default)]
pub struct OrderState {
    pub order_id: Option<Uuid>,
    pub status: OrderStatus,
    pub items: Vec<Item>,
}

// Errors
#[derive(Debug, thiserror::Error)]
pub enum OrderError {
    #[error("order already exists")]
    AlreadyExists,
    #[error("order not found")]
    NotFound,
    #[error("invalid state transition")]
    InvalidTransition,
}

// Response for queries
#[derive(Debug, Clone)]
pub enum OrderResponse {
    Single(OrderDto),
    List(Vec<OrderDto>),
}
```

### Step 2: Implement BusinessLogic

```rust
pub struct OrderBusinessLogic;

impl BusinessLogic for OrderBusinessLogic {
    type State = OrderState;
    type Input = OrderCommand;
    type Event = OrderEvent;
    type Error = OrderError;
    type Call = Infallible;
    type CallResult = Infallible;
    type Response = OrderResponse;

    fn stream_id(input: &OrderCommand) -> StreamId {
        match input {
            OrderCommand::Create { order_id, .. } |
            OrderCommand::Confirm { order_id, .. } |
            OrderCommand::GetOrder { order_id, .. } => {
                StreamId::new(format!("order-{order_id}"))
            }
        }
    }

    fn process(
        &self,
        input: OrderCommand,
        clock: &dyn Clock,
    ) -> Result<BusinessResult<OrderEvent, Infallible, OrderResponse>, OrderError> {
        match input {
            OrderCommand::Create { order_id, items, fetched } => {
                // Validate: order must NOT exist
                if fetched.is_some() {
                    return Err(OrderError::AlreadyExists);
                }
                Ok(BusinessResult::Done(vec![
                    OrderEvent::Created {
                        order_id,
                        items,
                        created_at: clock.now(),
                    }
                ]))
            }

            OrderCommand::Confirm { fetched, .. } => {
                // Validate: order must exist and be in correct state
                let order = fetched.ok_or(OrderError::NotFound)?;
                if order.status != OrderStatus::Pending {
                    return Err(OrderError::InvalidTransition);
                }
                Ok(BusinessResult::Done(vec![
                    OrderEvent::Confirmed { confirmed_at: clock.now() }
                ]))
            }

            OrderCommand::GetOrder { fetched, .. } => {
                let order = fetched.ok_or(OrderError::NotFound)?;
                Ok(BusinessResult::Respond(OrderResponse::Single(order)))
            }
        }
    }

    fn apply(&self, state: &mut OrderState, event: &OrderEvent) {
        match event {
            OrderEvent::Created { order_id, items, .. } => {
                state.order_id = Some(*order_id);
                state.items = items.clone();
                state.status = OrderStatus::Pending;
            }
            OrderEvent::Confirmed { .. } => {
                state.status = OrderStatus::Confirmed;
            }
            OrderEvent::Cancelled { .. } => {
                state.status = OrderStatus::Cancelled;
            }
        }
    }

    fn event_type_name(event: &OrderEvent) -> &'static str {
        match event {
            OrderEvent::Created { .. } => "OrderCreated",
            OrderEvent::Confirmed { .. } => "OrderConfirmed",
            OrderEvent::Cancelled { .. } => "OrderCancelled",
        }
    }
}
```

### Step 3: Implement QueryFetcher

```rust
pub struct OrderQueryFetcher;

impl QueryFetcher<OrderCommand, OrderProjectionQueries> for OrderQueryFetcher {
    type Error = sqlx::Error;

    async fn fetch(
        &self,
        input: OrderCommand,
        projections: &OrderProjectionQueries,
    ) -> Result<FetchResult<OrderCommand>, Self::Error> {
        match input {
            OrderCommand::Create { order_id, items, .. } => {
                // For create: check if already exists
                let existing = projections.get_order(order_id).await?;
                let version = existing.as_ref().map(|o| o.version);
                Ok(FetchResult::new(
                    OrderCommand::Create { order_id, items, fetched: existing },
                    version,
                ))
            }

            OrderCommand::Confirm { order_id, .. } => {
                // For update: fetch current state
                let existing = projections.get_order(order_id).await?;
                let version = existing.as_ref().map(|o| o.version);
                Ok(FetchResult::new(
                    OrderCommand::Confirm { order_id, fetched: existing },
                    version,
                ))
            }

            OrderCommand::GetOrder { order_id, .. } => {
                // For query: no version needed
                let existing = projections.get_order(order_id).await?;
                Ok(FetchResult::new(
                    OrderCommand::GetOrder { order_id, fetched: existing },
                    None, // Queries don't need version
                ))
            }
        }
    }
}
```

### Step 4: Implement Environment

```rust
pub struct OrderEnvironment<C, ES, P, EB, PQ> {
    clock: C,
    event_store: ES,
    projector: P,
    event_bus: Option<EB>,
    projections: PQ,
    broadcast_topic: String,
    metadata: MetadataContext,
}

impl<C, ES, P, EB, PQ> HandlerEnvironment for OrderEnvironment<C, ES, P, EB, PQ>
where
    C: Clock,
    ES: EventStore,
    P: Projector,
    EB: EventBus,
    PQ: ProjectionQueries,
{
    type Clock = C;
    type EventStore = ES;
    type Projector = P;
    type EventBus = EB;
    type Projections = PQ;

    fn clock(&self) -> &Self::Clock { &self.clock }
    fn event_store(&self) -> &Self::EventStore { &self.event_store }
    fn projector(&self) -> Option<&Self::Projector> { Some(&self.projector) }
    fn event_bus(&self) -> Option<&Self::EventBus> { self.event_bus.as_ref() }
    fn broadcast_topic(&self) -> &str { &self.broadcast_topic }
    fn projections(&self) -> &Self::Projections { &self.projections }
    fn metadata(&self) -> &MetadataContext { &self.metadata }
}
```

### Step 5: Wire It Together

```rust
let handler = Handler::new(
    OrderBusinessLogic,
    NoOpCallExecutor,  // Aggregates don't make calls
    OrderQueryFetcher,
    environment,
);

// Handle a command
let result = handler.handle(OrderCommand::Create {
    order_id: Uuid::new_v4(),
    items: vec![item],
    fetched: None,  // Will be populated by QueryFetcher
}).await?;

match result {
    HandleResult::Command { version, event_count } => {
        println!("Created order at version {version}, {event_count} events");
    }
    HandleResult::Query(response) => {
        println!("Query response: {response:?}");
    }
}
```

---

## Testing Patterns

### Using TestEnvironment

```rust
use composable_rust_next::testing::TestEnvironment;
use composable_rust_next::{Handler, FixedClock, NoOpCallExecutor, NoOpQueryFetcher};
use chrono::Utc;
use uuid::Uuid;

#[tokio::test]
async fn test_order_creation() {
    // Create test environment with fixed clock
    let env = TestEnvironment::new(FixedClock::new(Utc::now()));

    let handler = Handler::new(
        OrderBusinessLogic,
        NoOpCallExecutor,
        NoOpQueryFetcher,  // Or your test fetcher
        env.clone(),
    );

    let order_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

    // Execute command
    let result = handler.handle(OrderCommand::Create {
        order_id,
        items: vec![],
        fetched: None,
    }).await.unwrap();

    // Assert on result
    assert!(result.is_command());
    assert_eq!(result.event_count(), 1);

    // Assert on stored events
    let stored = env.event_store().events_for_stream("order-550e8400-e29b-41d4-a716-446655440000");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].event_type, "OrderCreated");

    // Assert on projected events
    let projected = env.projector().projected_events();
    assert_eq!(projected.len(), 1);

    // Assert on published events
    let published = env.event_bus().published_events();
    assert_eq!(published.len(), 1);
}
```

### Testing with FixedClock

```rust
use composable_rust_next::{FixedClock, Clock};
use composable_rust_next::testing::TestEnvironment;
use chrono::{TimeZone, Utc};
use std::time::Duration;

#[tokio::test]
async fn test_time_dependent_logic() {
    let fixed_time = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
    let clock = FixedClock::new(fixed_time);
    let env = TestEnvironment::new(clock.clone());

    // Create handler and execute initial command...
    // let handler = Handler::new(..., env.clone());
    // handler.handle(some_command).await.unwrap();

    // Advance time for testing expiration logic
    clock.advance(Duration::from_secs(3600)); // 1 hour later

    // Execute another command that checks expiration
    // The business logic sees the advanced time via clock.now()
    assert_eq!(
        clock.now(),
        Utc.with_ymd_and_hms(2025, 1, 15, 11, 0, 0).unwrap()
    );
}
```

### Testing Error Cases

```rust
use composable_rust_next::{Handler, FixedClock, NoOpCallExecutor, HandlerError};
use composable_rust_next::testing::TestEnvironment;
use chrono::Utc;
use uuid::Uuid;

#[tokio::test]
async fn test_duplicate_creation_fails() {
    let env = TestEnvironment::new(FixedClock::new(Utc::now()));

    // Use a query fetcher that returns existing data (simulates order exists)
    let fetcher = MockQueryFetcher::with_existing_order(existing_order_dto);

    let handler = Handler::new(
        OrderBusinessLogic,
        NoOpCallExecutor,
        fetcher,
        env,
    );

    let result = handler.handle(OrderCommand::Create {
        order_id: Uuid::new_v4(),
        items: vec![],
        fetched: None,  // Will be populated by MockQueryFetcher
    }).await;

    assert!(matches!(
        result,
        Err(HandlerError::Business(OrderError::AlreadyExists))
    ));
}
```

### Testing Version Conflicts

```rust
#[tokio::test]
async fn test_version_conflict_retry() {
    // The TestEnvironment's InMemoryEventStore supports version checking
    let env = TestEnvironment::new(FixedClock::new(Utc::now()));

    // Pre-populate the stream to create a version mismatch
    env.event_store().append(
        &StreamId::new("order-123"),
        None,
        vec![some_event],
    ).await.unwrap();

    // Now a command expecting version 0 will fail
    // Handler will retry with fresh data
}
```

---

## Guidelines and Best Practices

### DO: Keep BusinessLogic Pure

```rust
// GOOD: Pure business logic
fn process(&self, input: Input, clock: &dyn Clock) -> Result<BusinessResult, Error> {
    let now = clock.now();  // Use injected clock
    // Validate against input.fetched (already loaded)
    // Return events/response
}
```

### DO: Validate Against Fetched Data

```rust
// GOOD: Validation uses fetched projection data
OrderCommand::Confirm { fetched, .. } => {
    let order = fetched.ok_or(OrderError::NotFound)?;
    if order.status != OrderStatus::Pending {
        return Err(OrderError::InvalidTransition);
    }
    // Proceed with business logic...
    Ok(BusinessResult::Done(vec![
        OrderEvent::Confirmed { confirmed_at: clock.now() }
    ]))
}
```

### DO: Use Meaningful Event Type Names

```rust
// GOOD: Stable, descriptive names
fn event_type_name(event: &OrderEvent) -> &'static str {
    match event {
        OrderEvent::Created { .. } => "OrderCreated",      // Past tense
        OrderEvent::Confirmed { .. } => "OrderConfirmed",  // Matches domain language
        OrderEvent::Cancelled { .. } => "OrderCancelled",
    }
}
```

### DO: Include All Relevant Data in Events

```rust
// GOOD: Fat event with all data needed for projections
OrderEvent::Created {
    order_id,
    items: items.clone(),        // Include items
    created_at: clock.now(),     // Include timestamp
    customer_id,                 // Include who created it
    total_amount,                // Include computed values
}
```

### DO: Use Infallible for Aggregate Call Types

```rust
// GOOD: Aggregates never make calls
impl BusinessLogic for MyAggregate {
    type Call = Infallible;
    type CallResult = Infallible;
    // ...
}
```

### DO: Implement feedback_input for Sagas

```rust
// GOOD: Sagas must implement this
fn feedback_input(results: Vec<SagaCallResult>) -> SagaInput {
    SagaInput::Feedback { results, fetched: None }
}
```

---

## Common Mistakes to Avoid

### DON'T: Read from Event Store for Validation

```rust
// BAD: Trying to load from event store in business logic
// NOTE: This won't even compile! process() is NOT async, so you can't use .await
fn process(&self, input: Input, clock: &dyn Clock) -> Result<...> {
    // let events = self.event_store.load(&stream_id).await?;  // COMPILE ERROR!
    // The type system prevents this mistake.
}

// GOOD: Use fetched projection data (already loaded by QueryFetcher)
fn process(&self, input: Input, clock: &dyn Clock) -> Result<...> {
    let data = input.fetched.ok_or(Error::NotFound)?;  // From projection
    // Validation uses pre-fetched data
    Ok(BusinessResult::Done(vec![...]))
}
```

### DON'T: Execute Side Effects in BusinessLogic

```rust
// BAD: Trying to execute side effects in business logic
// NOTE: This won't compile! process() is NOT async.
fn process(&self, input: Input, clock: &dyn Clock) -> Result<...> {
    // self.database.save(&data).await?;  // COMPILE ERROR!
    // self.email_service.send(&email).await?;  // COMPILE ERROR!
    // The non-async signature enforces purity.
}

// GOOD: Return events, let Handler handle persistence
fn process(&self, input: Input, clock: &dyn Clock) -> Result<...> {
    // Pure logic only - return what should happen
    Ok(BusinessResult::Done(vec![Event::Created { id, name, created_at: clock.now() }]))
}
```

### DON'T: Use Real Time in BusinessLogic

```rust
// BAD: Using system time directly
fn process(&self, input: Input, clock: &dyn Clock) -> Result<...> {
    let now = Utc::now();  // WRONG! Not deterministic
    // ...
}

// GOOD: Use injected clock
fn process(&self, input: Input, clock: &dyn Clock) -> Result<...> {
    let now = clock.now();  // Deterministic in tests
    // ...
}
```

### DON'T: Forget to Handle All Command Variants

```rust
// BAD: Non-exhaustive match with wildcard
fn process(&self, input: Input, clock: &dyn Clock) -> Result<...> {
    match input {
        Command::Create { .. } => Ok(BusinessResult::Done(vec![...])),
        _ => Ok(BusinessResult::Done(vec![])),  // Dangerous! Silently ignores new variants
    }
}

// GOOD: Exhaustive match - compiler warns about missing variants
fn process(&self, input: Input, clock: &dyn Clock) -> Result<...> {
    match input {
        Command::Create { .. } => Ok(BusinessResult::Done(vec![...])),
        Command::Update { .. } => Ok(BusinessResult::Done(vec![...])),
        Command::Delete { .. } => Ok(BusinessResult::Done(vec![...])),
        Command::Get { .. } => Ok(BusinessResult::Respond(...)),
    }
}
```

### DON'T: Change Event Type Names After Production Use

```rust
// BAD: Renaming event types
fn event_type_name(event: &Event) -> &'static str {
    match event {
        Event::Created { .. } => "ItemCreated",  // Was "OrderCreated" - BREAKS DESERIALIZATION!
    }
}

// GOOD: Keep names stable, add new versions if needed
fn event_type_name(event: &Event) -> &'static str {
    match event {
        Event::CreatedV1 { .. } => "OrderCreated",      // Original
        Event::CreatedV2 { .. } => "OrderCreatedV2",    // New version
    }
}
```

### DON'T: Return Continue from Aggregates

```rust
// BAD: Aggregate returning Continue
impl BusinessLogic for MyAggregate {
    type Call = SomeCall;  // Should be Infallible!

    fn process(&self, input: Input, clock: &dyn Clock)
        -> Result<BusinessResult<Event, SomeCall, Response>, Error>
    {
        // WRONG! Aggregates should never orchestrate other aggregates
        Ok(BusinessResult::Continue {
            events: vec![Event::Created { id, name }],
            calls: vec![SomeCall::DoSomething],
        })
    }
}

// GOOD: Aggregates always return Done or Respond
impl BusinessLogic for MyAggregate {
    type Call = Infallible;  // Type system prevents Continue

    fn process(&self, input: Input, clock: &dyn Clock)
        -> Result<BusinessResult<Event, Infallible, Response>, Error>
    {
        // Can only return Done or Respond - Continue is impossible with Infallible
        Ok(BusinessResult::Done(vec![Event::Created { id, name }]))
    }
}
```

---

## Quick Reference

### Type Mapping

| Concept | Aggregate | Saga | Query-Only |
|---------|-----------|------|------------|
| `Call` | `Infallible` | Domain-specific enum | `Infallible` |
| `CallResult` | `Infallible` | Domain-specific enum | `Infallible` |
| `Response` | Domain response or `()` | `()` | Domain response |
| Returns | `Done` or `Respond` | `Done`, `Continue`, or `Respond` | `Respond` |

### Handler Configuration

```rust
// Default values
DEFAULT_MAX_RETRIES = 3           // Version conflict retries
DEFAULT_MAX_SAGA_ITERATIONS = 100 // Saga loop safety guard

// Custom configuration via builder
let handler = HandlerBuilder::new(MyBusinessLogic)
    .call_executor(MyCallExecutor)
    .query_fetcher(MyQueryFetcher)
    .environment(env)
    .max_retries(5)
    .max_saga_iterations(50)
    .build();
```

### Error Types

| Error | Meaning | Action |
|-------|---------|--------|
| `HandlerError::Business(e)` | Domain validation failed | Return 400 |
| `HandlerError::QueryFetch(e)` | Projection query failed | Return 503, retry |
| `HandlerError::Load(e)` | Event store read failed | Return 503, retry |
| `HandlerError::Persist(VersionConflict)` | Concurrent modification | Auto-retried |
| `HandlerError::Persist(other)` | Event store write failed | Return 503 |
| `HandlerError::Projection(e)` | Read model update failed | Events saved, retry projection |
| `HandlerError::Broadcast(e)` | Event bus publish failed | Events saved, retry broadcast |
| `HandlerError::Serialization(e)` | Event serialization failed | Bug in event schema |
| `HandlerError::SagaIterationsExceeded` | Infinite loop detected | Bug in saga logic |

### Testing Utilities

```rust
use composable_rust_next::testing::{
    InMemoryEventStore,   // Test event store with version tracking
    InMemoryEventBus,     // Captures published events
    InMemoryProjector,    // Records projected events (can simulate failures)
    TestEnvironment,      // Pre-configured environment bundling all above
};

use composable_rust_next::{
    FixedClock,           // Deterministic clock for testing
    SystemClock,          // Real clock for production
    NoOpCallExecutor,     // For aggregates
    NoOpQueryFetcher,     // Pass-through fetcher
    NoOpProjectionQueries,// For simple cases
};
```

---

## Summary

The `composable-rust-next` library provides a clean architecture for building event-sourced systems with CQRS. Key principles:

1. **BusinessLogic is pure** - no I/O, just decisions
2. **Projections are the source of truth for reads** - never query the event store for validation
3. **Events are the source of truth for writes** - append-only, immutable
4. **Handler orchestrates everything** - retry, project, broadcast
5. **Type system enforces correctness** - `Infallible` for aggregates, real types for sagas

When implementing:
1. Define your events, commands (with `fetched` field), and errors
2. Implement `BusinessLogic` with pure validation logic
3. Implement `QueryFetcher` to load projection data
4. Implement `HandlerEnvironment` with your infrastructure
5. Wire together with `Handler` and you're done

The framework handles persistence, projections, broadcasting, retries, and safety guards automatically.
