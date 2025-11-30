# Integration Test Guidelines for Ticketing System

## Executive Summary

**The Problem**: We have no integration tests that exercise the saga handlers with their child handlers. The only tests are:
1. **Unit tests**: Test `BusinessLogic.process()` in isolation
2. **E2E tests**: Test via HTTP + PostgreSQL (slow, hard to debug)

**The Gap**: Handler-level integration tests that call `handler.handle()` directly with in-memory infrastructure.

---

## Architecture Overview

### Handler Flow

```
Command → Handler.handle()
           │
           ├── 1. QueryFetcher.fetch(command, projections) → prepared_input + expected_version
           │
           ├── 2. BusinessLogic.process(prepared_input) → BusinessResult
           │          │
           │          ├── Done(events) → persist → project → broadcast → return
           │          │
           │          └── Continue { events, calls }
           │                    │
           │                    ├── persist → project → broadcast
           │                    │
           │                    └── CallExecutor.execute(calls)
           │                              │
           │                              ▼
           │                        Child Handlers
           │                              │
           │                              ▼
           │                        Vec<CallResult>
           │                              │
           │                    ◄─────────┘
           │
           └── feedback_input(results) → loop back to step 1
```

### Key Architecture: Projector and ProjectionQueries Are Separate Types

The `TicketingEnvironment` has **two separate but related** type parameters:

```rust
pub struct TicketingEnvironment<C, ES, P, EB, PQ>
where
    P: Projector,           // WRITES to projection store
    PQ: ProjectionQueries,  // READS from projection store
```

- **`Projector`**: Called after persist to update the read model
- **`ProjectionQueries`**: Used by `QueryFetcher` to read projection data

In production, both operate on **PostgreSQL**. For testing, both must share the **same in-memory store**.

### Key Insight: Projections Are The Glue

In a saga, the parent handler needs to read projection data that was written by child handlers:

```
Parent Saga                          Child Handler
     │                                    │
     │ ─── calls: [ReserveSeats] ───────► │
     │                                    │
     │                             persist InventoryEvent::SeatsReserved
     │                                    │
     │                             Projector.project() → shared in-memory store
     │                                    │
     │ ◄── results: [Ok(events)] ─────────┘
     │
     │ feedback_input(results)
     │
     │ QueryFetcher.fetch(Feedback { ... }, projections)
     │     │
     │     └── reads from shared in-memory store
```

**The problem**: Current tests use `InMemoryProjector` (generic) + `NoOpProjectionQueries` which don't share data.

---

## Existing In-Memory Infrastructure (What We Have)

### Saga Projections (Complete)

The saga infrastructure is **already complete**:

```rust
// In-memory projection stores (projector.rs)
pub type InMemorySagaProjection = Arc<RwLock<HashMap<EventId, SagaState>>>;
pub type InMemoryReservationSagaProjection = Arc<RwLock<HashMap<ReservationId, ReservationSagaState>>>;

// Domain-specific projectors that write to the stores
pub struct EventInventorySagaProjector {
    state: InMemorySagaProjection,  // Shared with query fetcher!
    logic: EventInventorySagaLogic,
}

pub struct ReservationSagaProjector {
    state: InMemoryReservationSagaProjection,  // Shared with query fetcher!
    logic: ReservationSagaLogic,
}
```

The pattern is:
1. Create shared `Arc<RwLock<HashMap<...>>>` store
2. Projector holds reference, deserializes events, updates store
3. Query fetcher holds same reference, reads from store

### Event Query Infrastructure (Partial)

```rust
// In-memory store (projection_queries.rs)
pub struct InMemoryEventProjectionQueries {
    events: RwLock<HashMap<EventId, EventDto>>,
}

// Query fetcher that uses internal store (ignores env.projections)
pub struct InMemoryEventQueryFetcher {
    projections: Arc<InMemoryEventProjectionQueries>,  // Has its own copy!
}
```

**Problem**: `InMemoryEventQueryFetcher` ignores the environment's `projections` parameter and uses its own internal store. No projector writes to `InMemoryEventProjectionQueries`.

### What's Missing

| Component | Saga | Event | Inventory | Payment |
|-----------|------|-------|-----------|---------|
| In-memory projection store | ✅ `InMemorySagaProjection` | ✅ `InMemoryEventProjectionQueries` | ❌ | ❌ |
| Projector that writes to store | ✅ `EventInventorySagaProjector` | ❌ | ❌ | ❌ |
| Query fetcher that reads from store | ✅ (via shared Arc) | ⚠️ (uses internal copy) | ❌ | ❌ |

### The Real Gap

**For sagas**: Infrastructure exists but tests don't wire it together properly. Tests use `InMemoryProjector` instead of `ReservationSagaProjector`.

**For aggregates**: Need in-memory projectors that write to stores.

---

## Test Levels

### Level 0: Pure BusinessLogic Unit Tests

**What**: Call `logic.process(input)` directly with hand-crafted input.

**Infrastructure**: None. Pure functions.

**Use for**:
- Testing business rules in isolation
- Testing edge cases with specific `fetched` data
- Fast TDD iteration

**Example**:
```rust
#[test]
fn reservation_saga_initiates_when_inventory_available() {
    let input = ReservationSagaInput::Initiate {
        reservation_id: ReservationId::new(),
        event_id: EventId::new(),
        section: "VIP".to_string(),
        quantity: 2,
        user_id: "user-123".to_string(),
        price_per_seat: Decimal::new(100, 0),
        fetched: Some(InventoryDto {
            available_seats: 10,
            ..Default::default()
        }),
    };

    let clock = FixedClock::new(Utc::now());
    let result = ReservationSagaLogic.process(input, &clock).unwrap();

    assert!(matches!(result, BusinessResult::Continue { .. }));
}
```

### Level 1: Single Handler Integration Tests (No Query Fetcher)

**What**: Test `Handler.handle()` with in-memory infrastructure, using `NoOpQueryFetcher`.

**Infrastructure**:
- `InMemoryEventStore`
- `InMemoryProjector`
- `InMemoryEventBus`
- `FixedClock`

**Use for**:
- Commands where `fetched: None` is acceptable (e.g., Create commands)
- Verifying persist → project → broadcast flow
- Testing optimistic concurrency (version conflicts)

**Example** (existing in `event.rs`):
```rust
#[tokio::test]
async fn handler_creates_event_and_persists() {
    let env = TestEnvironment::new(FixedClock::new(Utc::now()));
    let handler = Handler::new(EventBusinessLogic, NoOpCallExecutor, NoOpQueryFetcher, env.clone());

    let result = handler.handle(EventCommand::Create {
        event_id: EventId::new(),
        name: "Concert".to_string(),
        venue_id: VenueId::new(),
        date: Utc::now(),
        sections: vec![...],
        fetched: None,
    }).await.unwrap();

    // Assert events persisted
    let stored = env.event_store().events_for_stream("event-...");
    assert_eq!(stored.len(), 1);
}
```

### Level 2: Single Handler Integration Tests (With Query Fetcher)

**What**: Test `Handler.handle()` with a real query fetcher that reads from an in-memory projection store.

**Infrastructure**:
- `InMemoryEventStore`
- `InMemoryProjectionStore` (**NEW** - shared projection storage)
- `InMemoryProjector` that writes to the store
- `InMemoryQueryFetcher` that reads from the store
- `InMemoryEventBus`
- `FixedClock`

**Use for**:
- Commands that require validation against projection data
- Testing the complete fetch → process → persist → project cycle
- Verifying that projections are correctly written and read

**Example** (needs implementation):
```rust
#[tokio::test]
async fn reserve_seats_validates_against_inventory_projection() {
    // Shared projection store
    let projections = InMemoryTicketingProjections::new();

    // Pre-seed inventory projection (simulates prior inventory initialization)
    projections.insert_inventory(InventoryDto {
        event_id,
        section: "VIP".to_string(),
        available_seats: 10,
        ..Default::default()
    });

    let env = TestEnvironment::with_projections(clock, projections.clone());
    let handler = Handler::new(
        InventoryBusinessLogic,
        NoOpCallExecutor,
        InventoryQueryFetcher,  // Real fetcher that reads from projections
        env,
    );

    let result = handler.handle(InventoryCommand::Reserve {
        event_id,
        section: "VIP".to_string(),
        quantity: 2,
        reservation_id: ReservationId::new(),
        fetched: None,  // Query fetcher will populate this
    }).await.unwrap();

    // Assert reservation succeeded
    assert!(result.is_command());

    // Assert projection was updated
    let inventory = projections.get_inventory(&event_id, "VIP").unwrap();
    assert_eq!(inventory.available_seats, 8);
}
```

### Level 3: Saga Handler Integration Tests

**What**: Test saga handlers with real child handlers, all using shared in-memory infrastructure.

**Infrastructure**:
- **Shared** `InMemoryEventStore` - all handlers see each other's events
- **Shared** `InMemoryReservationSagaProjection` - saga projector writes, saga query fetcher reads
- Real `ReservationSagaCallExecutor` wired to real child handlers
- Each child handler has its own domain-specific projector
- `InMemoryEventBus`
- `FixedClock`

**Use for**:
- Testing complete saga flows (initiate → child calls → feedback → completion)
- Testing compensation flows (failure → rollback)
- Verifying cross-handler data flow via projections

**Example using EXISTING infrastructure** (projector.rs already has these types!):
```rust
use crate::next::{
    ReservationSagaLogic, ReservationSagaProjector, ReservationSagaCallExecutor,
    InMemoryReservationSagaProjection,  // Already exists!
    InventoryBusinessLogic, PaymentBusinessLogic,
};

#[tokio::test]
async fn reservation_saga_completes_full_flow() {
    // === SHARED INFRASTRUCTURE ===
    let event_store = InMemoryEventStore::new();
    let event_bus = InMemoryEventBus::new();
    let clock = FixedClock::new(Utc::now());

    // Shared saga state - THE KEY!
    // Both projector and query fetcher use this same Arc
    let saga_state: InMemoryReservationSagaProjection = Arc::new(RwLock::new(HashMap::new()));

    // === CHILD HANDLERS ===
    // For now, use simple handlers with NoOpProjector
    // (Phase 2 will add proper inventory/payment projectors)
    let inventory_env = TicketingEnvironment::new(
        clock.clone(),
        event_store.clone(),
        Some(NoOpProjector),  // TODO: InMemoryInventoryProjector
        Some(event_bus.clone()),
        "test",
    );
    let inventory_handler: Arc<dyn InventoryHandler> = Arc::new(Handler::new(
        InventoryBusinessLogic,
        NoOpCallExecutor,
        NoOpQueryFetcher,  // Pre-seed fetched data in command
        inventory_env,
    ));

    let payment_env = TicketingEnvironment::new(
        clock.clone(),
        event_store.clone(),
        Some(NoOpProjector),  // TODO: InMemoryPaymentProjector
        Some(event_bus.clone()),
        "test",
    );
    let payment_handler: Arc<dyn PaymentHandler> = Arc::new(Handler::new(
        PaymentBusinessLogic,
        NoOpCallExecutor,
        NoOpQueryFetcher,
        payment_env,
    ));

    // === SAGA HANDLER ===

    // Call executor dispatches to child handlers
    let call_executor = ReservationSagaCallExecutor::new(
        inventory_handler,
        payment_handler,
        event_store.clone(),
    );

    // Saga projector writes to shared state
    let saga_projector = ReservationSagaProjector::new(saga_state.clone());

    // Saga query fetcher reads from same shared state
    let saga_query_fetcher = ReservationSagaQueryFetcher::new(saga_state.clone());

    // Create saga environment with the domain-specific projector
    let saga_env = TicketingEnvironment::new(
        clock.clone(),
        event_store.clone(),
        Some(saga_projector),  // Domain-specific projector!
        Some(event_bus.clone()),
        "test",
    );

    let saga_handler = Handler::new(
        ReservationSagaLogic,
        call_executor,
        saga_query_fetcher,  // Reads from shared saga_state
        saga_env,
    );

    // === PRE-SEED: Inventory must exist ===
    // Since we use NoOpQueryFetcher for inventory, we pre-seed fetched data
    // (In Phase 2, InMemoryInventoryProjector would handle this)

    // Initialize inventory first via direct handler call
    inventory_handler.handle(InventoryCommand::Initialize {
        event_id,
        section: "General Admission".to_string(),
        capacity: Capacity::new(100),
        fetched: None,  // OK for Initialize
    }).await.unwrap();

    // === ACT ===
    let reservation_id = ReservationId::new();
    let result = saga_handler.handle(ReservationSagaInput::Initiate {
        reservation_id,
        event_id,
        section: "General Admission".to_string(),
        quantity: 2,
        user_id: "user-123".to_string(),
        price_per_seat: Decimal::new(50, 0),
        fetched: Some(InventoryDto {  // Pre-seed since NoOpQueryFetcher
            event_id,
            section: "General Admission".to_string(),
            total_capacity: 100,
            available_seats: 100,
            reserved_seats: 0,
        }),
    }).await;

    // === ASSERT ===
    assert!(result.is_ok(), "Saga should complete: {:?}", result);

    // Saga state is tracked in shared projection
    let states = saga_state.read().await;
    let state = states.get(&reservation_id).expect("Saga state should exist");
    assert_eq!(state.phase, ReservationSagaPhase::Completed);

    // Events were persisted to event store
    let saga_events = event_store.events_for_stream(&format!("reservation-saga-{reservation_id}"));
    assert!(!saga_events.is_empty(), "Saga events should be persisted");
}
```

**Key insight**: The `saga_state: InMemoryReservationSagaProjection` is shared between:
- `ReservationSagaProjector::new(saga_state.clone())` - WRITES
- `ReservationSagaQueryFetcher::new(saga_state.clone())` - READS

This is exactly how the production PostgreSQL setup works, just with `Arc<RwLock<HashMap>>` instead of a database connection pool.

### Level 4: HTTP Handler Integration Tests

**What**: Test HTTP routes with `axum::test` using in-memory infrastructure.

**Infrastructure**:
- All Level 3 infrastructure
- `axum::test` router

**Use for**:
- Testing HTTP request/response handling
- Testing authentication/authorization
- Testing error responses and status codes

**Example**:
```rust
#[tokio::test]
async fn post_reservations_returns_201() {
    let app = create_test_app().await;  // Creates router with in-memory handlers

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v2/reservations")
                .header("Content-Type", "application/json")
                .body(Body::from(json!({
                    "event_id": "...",
                    "section": "VIP",
                    "quantity": 2
                }).to_string()))
                .unwrap()
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
}
```

### Level 5: Full E2E Tests

**What**: Test against real running server with PostgreSQL.

**Infrastructure**:
- Running server process
- PostgreSQL database
- HTTP client

**Use for**:
- Final validation before deployment
- Testing database-specific behavior
- Integration with external systems

**This is what `full_deployment_test.rs` currently does.**

---

## What's Missing: InMemoryProjectionStore

The critical missing piece is an in-memory projection store that:

1. **Projectors write to**: When `InventoryProjector.project(SeatsReserved)` is called, it updates `available_seats` in the store
2. **Query fetchers read from**: When `InventoryQueryFetcher.fetch(Reserve { ... })` is called, it reads from the same store
3. **Is shared across handlers**: All handlers in a saga test use the same store instance

### Implementation Plan

#### Step 1: Create `InMemoryTicketingProjections`

```rust
/// In-memory projection store for ticketing system tests.
///
/// Stores all projection data in memory for fast, deterministic testing.
/// Implements `ProjectionQueries` trait so it can be used with `TestEnvironment`.
#[derive(Debug, Clone, Default)]
pub struct InMemoryTicketingProjections {
    events: Arc<RwLock<HashMap<EventId, EventDto>>>,
    inventories: Arc<RwLock<HashMap<(EventId, String), InventoryDto>>>,
    payments: Arc<RwLock<HashMap<PaymentId, PaymentDto>>>,
    reservations: Arc<RwLock<HashMap<ReservationId, ReservationDto>>>,
    saga_states: Arc<RwLock<HashMap<ReservationId, ReservationSagaStateDto>>>,
}

impl InMemoryTicketingProjections {
    pub fn new() -> Self { ... }

    // Write methods (used by projectors)
    pub fn insert_event(&self, event: EventDto) { ... }
    pub fn update_inventory(&self, event_id: &EventId, section: &str, f: impl FnOnce(&mut InventoryDto)) { ... }

    // Read methods (used by query fetchers)
    pub fn get_event(&self, id: &EventId) -> Option<EventDto> { ... }
    pub fn get_inventory(&self, event_id: &EventId, section: &str) -> Option<InventoryDto> { ... }
}
```

#### Step 2: Create `InMemoryInventoryProjector`

```rust
/// In-memory projector that writes to InMemoryTicketingProjections
pub struct InMemoryInventoryProjector {
    projections: InMemoryTicketingProjections,
}

impl Projector for InMemoryInventoryProjector {
    async fn project(&self, events: &[SerializedEvent]) -> Result<(), ProjectionError> {
        for event in events {
            let inventory_event: InventoryEvent = bincode::deserialize(&event.payload)?;
            match inventory_event {
                InventoryEvent::Initialized { event_id, section, capacity, .. } => {
                    self.projections.insert_inventory(InventoryDto {
                        event_id,
                        section,
                        total_capacity: capacity.as_u32(),
                        available_seats: capacity.as_u32(),
                        reserved_seats: 0,
                    });
                }
                InventoryEvent::SeatsReserved { event_id, section, quantity, .. } => {
                    self.projections.update_inventory(&event_id, &section, |inv| {
                        inv.available_seats -= quantity;
                        inv.reserved_seats += quantity;
                    });
                }
                // ... other events
            }
        }
        Ok(())
    }
}
```

#### Step 3: Create `InMemoryInventoryQueryFetcher`

```rust
/// Query fetcher that reads from InMemoryTicketingProjections
pub struct InMemoryInventoryQueryFetcher {
    projections: InMemoryTicketingProjections,
}

impl QueryFetcher<InventoryCommand, InMemoryTicketingProjections> for InMemoryInventoryQueryFetcher {
    async fn fetch(
        &self,
        command: InventoryCommand,
        projections: &InMemoryTicketingProjections,
    ) -> Result<FetchResult<InventoryCommand>, FetchError> {
        match &command {
            InventoryCommand::Reserve { event_id, section, .. } => {
                let dto = projections.get_inventory(event_id, section);
                let version = dto.as_ref().map(|_| Version::new(1)); // Simplified

                let prepared = InventoryCommand::Reserve {
                    fetched: dto,
                    ..command
                };

                Ok(FetchResult {
                    input: prepared,
                    expected_version: version,
                })
            }
            // ... other commands
        }
    }
}
```

#### Step 4: Create Test Setup Helpers

```rust
/// Test harness for saga integration tests
pub struct SagaTestHarness {
    pub event_store: InMemoryEventStore,
    pub projections: InMemoryTicketingProjections,
    pub event_bus: InMemoryEventBus,
    pub clock: FixedClock,
}

impl SagaTestHarness {
    pub fn new() -> Self { ... }

    /// Create a reservation saga handler with all child handlers wired up
    pub fn reservation_saga_handler(&self) -> Handler<
        ReservationSagaLogic,
        ReservationSagaCallExecutor<InMemoryEventStore>,
        ReservationSagaQueryFetcher,
        TestEnvironment<FixedClock, InMemoryTicketingProjections>,
    > {
        // Wire up inventory handler
        let inventory_handler = self.inventory_handler();

        // Wire up payment handler
        let payment_handler = self.payment_handler();

        // Create call executor
        let call_executor = ReservationSagaCallExecutor::new(
            Arc::new(inventory_handler),
            Arc::new(payment_handler),
            self.event_store.clone(),
        );

        // Create saga handler
        Handler::new(
            ReservationSagaLogic,
            call_executor,
            ReservationSagaQueryFetcher::new(self.projections.clone()),
            self.create_env(),
        )
    }

    pub fn inventory_handler(&self) -> Handler<...> { ... }
    pub fn payment_handler(&self) -> Handler<...> { ... }

    fn create_env(&self) -> TestEnvironment<...> { ... }
}
```

---

## Test File Organization

```
examples/ticketing/
├── src/
│   └── next/
│       ├── testing/                      # NEW: Test infrastructure
│       │   ├── mod.rs
│       │   ├── projections.rs            # InMemoryTicketingProjections
│       │   ├── projectors.rs             # In-memory projector implementations
│       │   ├── query_fetchers.rs         # In-memory query fetcher implementations
│       │   └── harness.rs                # SagaTestHarness
│       │
│       └── ...existing modules...
│
└── tests/
    ├── full_deployment_test.rs           # Level 5: E2E tests (existing)
    │
    └── integration/                       # NEW: Handler-level integration tests
        ├── mod.rs
        ├── event_handler_test.rs          # Level 1-2: Event handler tests
        ├── inventory_handler_test.rs      # Level 1-2: Inventory handler tests
        ├── payment_handler_test.rs        # Level 1-2: Payment handler tests
        ├── reservation_saga_test.rs       # Level 3: Saga integration tests
        └── event_inventory_saga_test.rs   # Level 3: Saga integration tests
```

---

## Guidelines for Writing Tests

### DO

1. **Share infrastructure in saga tests**: All handlers must use the same `EventStore` and `ProjectionStore` instance
2. **Pre-seed projections**: Set up the world state before calling the saga handler
3. **Assert on projections**: Verify the final state in the projection store
4. **Assert on events**: Verify the correct events were persisted
5. **Use deterministic data**: Fixed UUIDs, fixed timestamps via `FixedClock`
6. **Name tests descriptively**: `reservation_saga_compensates_on_payment_failure`

### DON'T

1. **Don't test via HTTP when handler test suffices**: HTTP tests are for HTTP-specific concerns
2. **Don't create separate projection stores**: Share one instance across all handlers
3. **Don't skip the query fetcher**: If the command has `fetched: Option<Dto>`, use a real query fetcher
4. **Don't mock child handlers in saga tests**: Use real handlers with in-memory infrastructure
5. **Don't rely on timing**: Use `FixedClock` and explicit state checks

---

## Revised Implementation Priority

**Bottom-up approach**: Test aggregates first, then sagas. You can't test orchestration on shaky foundations.

### Phase 1: Aggregate Infrastructure (Inventory)

Build and test the Inventory aggregate with full in-memory projection support:

**1.1. Create `InMemoryInventoryProjections`**
```rust
pub struct InMemoryInventoryProjections {
    inventories: Arc<RwLock<HashMap<(EventId, String), InventoryDto>>>,
}
```

**1.2. Create `InMemoryInventoryProjector`**
```rust
impl Projector for InMemoryInventoryProjector {
    async fn project(&self, events: &[SerializedEvent]) -> Result<(), ProjectionError> {
        for event in events {
            let inv_event: InventoryEvent = bincode::deserialize(&event.payload)?;
            match inv_event {
                InventoryEvent::Initialized { event_id, section, capacity, .. } => {
                    self.store.insert((event_id, section), InventoryDto { ... });
                }
                InventoryEvent::SeatsReserved { .. } => { ... }
                // etc.
            }
        }
        Ok(())
    }
}
```

**1.3. Write Inventory handler integration tests**
```rust
#[tokio::test]
async fn inventory_initialize_and_reserve() {
    let store = InMemoryInventoryProjections::new();
    let projector = InMemoryInventoryProjector::new(store.clone());
    let query_fetcher = InMemoryInventoryQueryFetcher::new(store.clone());

    let handler = Handler::new(InventoryBusinessLogic, NoOpCallExecutor, query_fetcher, env);

    // Initialize
    handler.handle(InventoryCommand::Initialize { ... }).await.unwrap();

    // Verify projection was updated
    let dto = store.get(&event_id, "GA").unwrap();
    assert_eq!(dto.available_seats, 100);

    // Reserve seats
    handler.handle(InventoryCommand::Reserve { ... }).await.unwrap();

    // Verify projection updated
    let dto = store.get(&event_id, "GA").unwrap();
    assert_eq!(dto.available_seats, 98);
}
```

### Phase 2: Aggregate Infrastructure (Payment)

Same pattern for Payment:

**2.1. `InMemoryPaymentProjections`** - Stores `PaymentDto` by payment_id

**2.2. `InMemoryPaymentProjector`** - Deserializes `PaymentEvent`, updates store

**2.3. Write Payment handler integration tests**

### Phase 3: Saga Integration Tests

**Only after aggregates are verified**, build saga tests:

**3.1. Create `SagaTestHarness`** that wires:
- Verified aggregate handlers with their projectors
- Saga projector (already exists)
- Shared `InMemoryEventStore`

**3.2. Write saga integration tests** that exercise:
- Full reservation flow (initiate → reserve → pay → complete)
- Compensation flow (reserve → pay fails → release seats)
- Edge cases (insufficient seats, payment timeout)

### Phase 4: Debug and Fix E2E Tests

With solid aggregate and saga tests in place:
1. Run handler test to isolate the issue
2. Compare handler behavior vs E2E behavior
3. Fix the discrepancy
4. Verify E2E test passes

---

## Debugging Strategy

When an E2E test fails:

1. **Write a handler-level test** that reproduces the scenario
2. **Add logging/assertions** at each step:
   - What did `QueryFetcher.fetch()` return?
   - What did `BusinessLogic.process()` return?
   - What events were persisted?
   - What did the projector write?
   - What did child handlers do?
3. **Fix the issue** in the handler test (fast iteration)
4. **Verify the E2E test passes**

This is the professional way to debug complex multi-handler flows.
