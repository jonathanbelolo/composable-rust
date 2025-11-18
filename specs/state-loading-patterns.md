# State Loading Patterns in Composable Rust

This document describes the architectural patterns for loading aggregate state in event-sourced systems using Composable Rust.

**Last Updated**: 2025-01-17
**Status**: Canonical Reference

---

## Table of Contents

1. [The Problem](#the-problem)
2. [Architectural Principles](#architectural-principles)
3. [Pattern 1: State Machine Within Store](#pattern-1-state-machine-within-store)
4. [Pattern 2: Parent-Child Saga Coordination](#pattern-2-parent-child-saga-coordination)
5. [When to Use Each Pattern](#when-to-use-each-pattern)
6. [Implementation Guide](#implementation-guide)
7. [Complete Example](#complete-example)
8. [Common Pitfalls](#common-pitfalls)

---

## The Problem

In event-sourced systems with CQRS, we face a fundamental challenge:

**Commands need to read current state, but aggregates are stateless.**

### The Naive Approach (WRONG ❌)

```rust
// BAD: Assumes state is pre-loaded in memory
InventoryAction::ReserveSeats { event_id, section, quantity } => {
    // Validate using in-memory state
    if state.get_inventory(&event_id, &section).available() < quantity {
        return validation_failed("Insufficient seats");
    }

    // Emit event
    emit(SeatsReserved { ... });
}
```

**Why this fails:**
- State is empty after server restart
- No mechanism to load data from persistence
- Commands fail with "inventory not found"

### The Solution: Load State on Demand

Commands must **load required state from projections** before validation.

**Key Insight:** Use the **Effect → Action feedback loop** to load data.

---

## Architectural Principles

### Principle 1: Aggregates Are Stateless

**Aggregate stores do NOT maintain global state.**

| Concern | Where It Lives |
|---------|----------------|
| **Query optimization** | Projections (read models) |
| **Command validation** | Loaded on-demand from projections |
| **Event persistence** | Event Store |

**Aggregate state** only holds the minimum needed for **current command validation**:
- Not a cache of all entities
- Not a projection/read model
- Just enough to answer: "Can I execute this command?"

### Principle 2: Commands Read from Projections

When a command arrives:
1. Load current state from **projection** (optimized read model)
2. Validate using loaded state
3. Emit events to **event store** (write model)
4. Events update **both** event store AND projections

```
┌─────────────┐
│   Command   │
│ ReserveSeats│
└──────┬──────┘
       │
       ↓ Load state
┌─────────────────┐
│   Projection    │  ← Optimized for reads
│  (PostgreSQL)   │
└─────────────────┘
       │
       ↓ Validate
┌─────────────────┐
│    Reducer      │  ← Business logic
└─────────────────┘
       │
       ↓ Emit events
┌─────────────────┐
│  Event Store    │  ← Source of truth
└─────────────────┘
       │
       ↓ Update
┌─────────────────┐
│   Projection    │  ← Eventually consistent
└─────────────────┘
```

### Principle 3: Load State via Effect Feedback

**Use the Effect → Action feedback loop to load data:**

```
1. Command arrives
   ↓
2. Reducer: "I need inventory data"
   → Returns Effect::LoadInventory
   ↓
3. Effect executes async load from projection
   ↓
4. Effect produces Action::InventoryLoaded
   ↓
5. Action feeds back to reducer
   ↓
6. Reducer: Hydrate state, continue...
```

This keeps reducers **pure** (no async I/O) while enabling state loading.

### Principle 4: Layered Coordination

**Two coordination patterns, used at different layers:**

- **Within one store**: State machine in data (Pattern 1)
- **Between multiple stores**: Parent-child saga (Pattern 2)

**Parent coordinators do NOT micromanage child workflows.**

---

## Pattern 1: State Machine Within Store

**Use when:** Single aggregate needs to load its own data before processing.

### Concept

The aggregate's state includes a **workflow state machine** that tracks:
- Where we are in the load-validate-execute flow
- What command is pending
- What data has been loaded

### State Structure

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InventoryState {
    // ===== Domain Data =====
    /// Inventories indexed by (event_id, section)
    pub inventories: HashMap<(EventId, String), Inventory>,
    /// Seat assignments
    pub seat_assignments: HashMap<SeatId, SeatAssignment>,

    // ===== Workflow State Machine =====
    /// Current workflow state
    pub workflow: InventoryWorkflow,

    // ===== Error Tracking =====
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum InventoryWorkflow {
    /// Idle, no pending operations
    Idle,

    /// Loading data from projection
    LoadingData {
        event_id: EventId,
        section: String,
        /// The command waiting for data
        pending_command: Box<InventoryAction>,
    },

    /// Data loaded, ready to process
    Ready {
        /// What data is loaded
        loaded_scope: (EventId, String),
    },
}
```

### Environment with Query Interface

```rust
/// Query interface for loading inventory state
#[async_trait::async_trait]
pub trait InventoryProjection: Send + Sync {
    /// Load inventory and seat data for an event/section
    async fn load_inventory(
        &self,
        event_id: EventId,
        section: &str,
    ) -> Result<InventoryData, ProjectionError>;
}

pub struct InventoryData {
    pub inventory: Option<Inventory>,
    pub seat_assignments: Vec<SeatAssignment>,
}

/// Environment for Inventory aggregate
#[derive(Clone)]
pub struct InventoryEnvironment {
    pub clock: Arc<dyn Clock>,
    pub event_store: Arc<dyn EventStore>,
    pub event_bus: Arc<dyn EventBus>,
    pub stream_id: StreamId,

    // ===== Query Interface =====
    /// Projection for loading current state
    pub projection: Arc<dyn InventoryProjection>,
}
```

### Reducer Logic

```rust
impl Reducer for InventoryReducer {
    fn reduce(
        &self,
        state: &mut InventoryState,
        action: InventoryAction,
        env: &InventoryEnvironment,
    ) -> SmallVec<[Effect<InventoryAction>; 4]> {
        match action {
            // ========== Command: Reserve Seats ==========
            InventoryAction::ReserveSeats {
                event_id,
                section,
                quantity,
                reservation_id,
                expires_at,
                specific_seats,
            } => {
                // Check workflow state
                match &state.workflow {
                    // Data not loaded - initiate load
                    InventoryWorkflow::Idle => {
                        state.workflow = InventoryWorkflow::LoadingData {
                            event_id,
                            section: section.clone(),
                            pending_command: Box::new(action.clone()),
                        };

                        // Return effect to load data
                        let projection = env.projection.clone();
                        let section_clone = section.clone();

                        return smallvec![
                            Effect::Future(Box::pin(async move {
                                match projection.load_inventory(event_id, &section_clone).await {
                                    Ok(data) => Some(InventoryAction::DataLoaded {
                                        event_id,
                                        section: section_clone,
                                        data,
                                    }),
                                    Err(e) => Some(InventoryAction::DataLoadFailed {
                                        event_id,
                                        section: section_clone,
                                        error: e.to_string(),
                                    }),
                                }
                            }))
                        ];
                    }

                    // Data is loaded and matches scope - process command
                    InventoryWorkflow::Ready { loaded_scope }
                        if *loaded_scope == (event_id, section.clone()) =>
                    {
                        // Validate using loaded state
                        if let Err(error) = Self::validate_reserve_seats(
                            state, &event_id, &section, quantity
                        ) {
                            Self::apply_event(
                                state,
                                &InventoryAction::ValidationFailed { error }
                            );
                            return SmallVec::new();
                        }

                        // Select seats
                        let seats = Self::select_available_seats(
                            state, &event_id, &section, quantity
                        );

                        // Emit event
                        let event = InventoryAction::SeatsReserved {
                            reservation_id,
                            event_id,
                            section,
                            seats,
                            expires_at,
                            reserved_at: env.clock.now(),
                        };
                        Self::apply_event(state, &event);

                        // Persist and publish
                        Self::create_effects(event, env)
                    }

                    // Wrong scope loaded or still loading - fail
                    _ => {
                        Self::apply_event(state, &InventoryAction::ValidationFailed {
                            error: "Invalid workflow state for command".to_string(),
                        });
                        SmallVec::new()
                    }
                }
            }

            // ========== Data Loaded ==========
            InventoryAction::DataLoaded { event_id, section, data } => {
                // Hydrate state with loaded data
                if let Some(inventory) = data.inventory {
                    state.inventories.insert((event_id, section.clone()), inventory);
                }

                for seat in data.seat_assignments {
                    state.seat_assignments.insert(seat.seat_id, seat);
                }

                // Extract pending command BEFORE updating workflow
                let pending_command = if let InventoryWorkflow::LoadingData {
                    pending_command, ..
                } = &state.workflow
                {
                    Some(*pending_command.clone())
                } else {
                    None
                };

                // Update workflow to Ready
                state.workflow = InventoryWorkflow::Ready {
                    loaded_scope: (event_id, section.clone()),
                };

                // Continue with pending command
                if let Some(cmd) = pending_command {
                    return smallvec![
                        Effect::Future(Box::pin(async move {
                            Some(cmd)
                        }))
                    ];
                }

                SmallVec::new()
            }

            // ========== Data Load Failed ==========
            InventoryAction::DataLoadFailed { error, .. } => {
                state.workflow = InventoryWorkflow::Idle;
                Self::apply_event(state, &InventoryAction::ValidationFailed {
                    error: format!("Failed to load inventory data: {}", error),
                });
                SmallVec::new()
            }

            // ... other actions ...
        }
    }
}
```

### Key Points

1. **State machine is data** - serializable, debuggable
2. **Load happens via Effect** - keeps reducer pure
3. **Automatic continuation** - after load, pending command continues automatically
4. **Scope checking** - ensures loaded data matches command
5. **Graceful failure** - load errors handled explicitly

---

## Pattern 2: Parent-Child Saga Coordination

**Use when:** Workflow involves **multiple aggregates** that need coordination.

### Concept

A **parent saga coordinator** orchestrates multiple child stores:
- Sends commands to children
- Waits for results (events)
- Makes decisions based on outcomes
- Handles timeouts and compensation

**Parent does NOT manage child's internal workflow.**

### Architecture

```
┌───────────────────────────────────────────────────────┐
│ ReservationStore (Parent Saga)                        │
│ ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ │
│                                                       │
│ Coordinates BETWEEN stores:                          │
│ • InitiateReservation → LoadInventory?               │
│ • Send ReserveSeats → InventoryStore                 │
│ • Wait for SeatsReserved event                       │
│ • Send ProcessPayment → PaymentStore                 │
│ • Wait for PaymentSucceeded event                    │
│ • Handle timeouts → Compensation                     │
│                                                       │
│ State Machine:                                        │
│ Idle → ReservingSeats → ProcessingPayment →          │
│   Completed                                           │
│     ↓            ↓                 ↓                  │
│   Failed     Failed           Failed/Timeout         │
│     ↓            ↓                 ↓                  │
│ Compensating → Compensating → Compensating           │
└───────────────────────────────────────────────────────┘
                    │                    │
        ┌───────────┴──────────┐  ┌─────┴──────────────┐
        │                      │  │                     │
        ↓                      ↓  ↓                     ↓
┌──────────────────┐  ┌──────────────────┐
│ InventoryStore   │  │ PaymentStore     │
│ (Pattern 1)      │  │ (Pattern 1)      │
│ ━━━━━━━━━━━━━━━━│  │ ━━━━━━━━━━━━━━━━│
│                  │  │                  │
│ Own workflow:    │  │ Own workflow:    │
│ 1. Load data     │  │ 1. Load data     │
│ 2. Validate      │  │ 2. Validate      │
│ 3. Reserve       │  │ 3. Process       │
│ 4. Return result │  │ 4. Return result │
└──────────────────┘  └──────────────────┘
```

### Parent State (Saga)

```rust
#[derive(Clone)]
pub struct ReservationState {
    // ===== Domain Data =====
    pub reservations: HashMap<ReservationId, Reservation>,

    // ===== Saga State Machine =====
    pub saga_state: ReservationSagaState,

    // ===== Child Stores =====
    pub inventory_store: Arc<Store<InventoryState, InventoryAction, ...>>,
    pub payment_store: Arc<Store<PaymentState, PaymentAction, ...>>,

    // ===== Error Tracking =====
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ReservationSagaState {
    Idle,
    ReservingSeats {
        reservation_id: ReservationId,
        event_id: EventId,
        section: String,
        quantity: u32,
    },
    ProcessingPayment {
        reservation_id: ReservationId,
        seats_reserved: Vec<SeatId>,
    },
    Completed {
        reservation_id: ReservationId,
    },
    Failed {
        reservation_id: ReservationId,
        reason: String,
    },
    Compensating {
        reservation_id: ReservationId,
        seats_to_release: Option<Vec<SeatId>>,
    },
}
```

### Parent Reducer (Saga Coordinator)

```rust
impl Reducer for ReservationReducer {
    fn reduce(
        &self,
        state: &mut ReservationState,
        action: ReservationAction,
        env: &ReservationEnvironment,
    ) -> SmallVec<[Effect<ReservationAction>; 4]> {
        match action {
            // ========== Initiate Reservation ==========
            ReservationAction::InitiateReservation {
                reservation_id,
                event_id,
                customer_id,
                section,
                quantity,
                specific_seats,
            } => {
                // Create reservation record
                let reservation = Reservation::new(
                    reservation_id,
                    event_id,
                    customer_id,
                    section.clone(),
                    quantity,
                );
                state.reservations.insert(reservation_id, reservation);

                // Update saga state
                state.saga_state = ReservationSagaState::ReservingSeats {
                    reservation_id,
                    event_id,
                    section: section.clone(),
                    quantity,
                };

                // Emit event
                let event = ReservationAction::ReservationInitiated { ... };
                Self::apply_event(state, &event);

                // Send command to Inventory child (Pattern 1 handles load)
                let inventory_store = Arc::clone(&state.inventory_store);
                let expires_at = env.clock.now() + Duration::minutes(5);

                let mut effects = Self::create_effects(event, env);
                effects.push(Effect::Future(Box::pin(async move {
                    // Child handles its own load-then-process workflow
                    let _ = inventory_store.send(InventoryAction::ReserveSeats {
                        reservation_id,
                        event_id,
                        section,
                        quantity,
                        specific_seats,
                        expires_at,
                    }).await;
                    None // Wait for SeatsReserved event via EventBus
                })));

                effects
            }

            // ========== Seats Reserved (from Inventory) ==========
            ReservationAction::SeatsReserved {
                reservation_id,
                seats,
                ..
            } => {
                // Update saga state
                if let ReservationSagaState::ReservingSeats { .. } = &state.saga_state {
                    state.saga_state = ReservationSagaState::ProcessingPayment {
                        reservation_id,
                        seats_reserved: seats.clone(),
                    };

                    // Update reservation
                    if let Some(reservation) = state.reservations.get_mut(&reservation_id) {
                        reservation.allocated_seats = Some(seats.clone());
                        reservation.status = ReservationStatus::PaymentPending;
                    }

                    // Send command to Payment child (Pattern 1 handles load)
                    let payment_store = Arc::clone(&state.payment_store);
                    let payment_id = PaymentId::new();

                    // Calculate amount (simplified)
                    let amount = Money::from_dollars(100);

                    return smallvec![
                        Effect::Future(Box::pin(async move {
                            let _ = payment_store.send(PaymentAction::ProcessPayment {
                                payment_id,
                                reservation_id,
                                amount,
                                payment_method: PaymentMethod::CreditCard {
                                    last_four: "4242".to_string(),
                                },
                            }).await;
                            None // Wait for PaymentSucceeded event
                        }))
                    ];
                }

                SmallVec::new()
            }

            // ========== Payment Succeeded (from Payment) ==========
            ReservationAction::PaymentSucceeded {
                reservation_id,
                payment_id,
                ..
            } => {
                // Complete saga
                state.saga_state = ReservationSagaState::Completed { reservation_id };

                if let Some(reservation) = state.reservations.get_mut(&reservation_id) {
                    reservation.status = ReservationStatus::Completed;
                    reservation.payment_id = Some(payment_id);
                }

                // Emit completion event
                let event = ReservationAction::ReservationCompleted {
                    reservation_id,
                    completed_at: env.clock.now(),
                };
                Self::apply_event(state, &event);
                Self::create_effects(event, env)
            }

            // ========== Compensation (on failure) ==========
            ReservationAction::CancelReservation { reservation_id } => {
                // Determine what needs compensation
                let seats_to_release = match &state.saga_state {
                    ReservationSagaState::ProcessingPayment { seats_reserved, .. }
                        => Some(seats_reserved.clone()),
                    _ => None,
                };

                state.saga_state = ReservationSagaState::Compensating {
                    reservation_id,
                    seats_to_release: seats_to_release.clone(),
                };

                // Release seats if they were reserved
                if let Some(seats) = seats_to_release {
                    let inventory_store = Arc::clone(&state.inventory_store);
                    return smallvec![
                        Effect::Future(Box::pin(async move {
                            let _ = inventory_store.send(
                                InventoryAction::ReleaseReservation { reservation_id }
                            ).await;
                            Some(ReservationAction::SeatsReleased { reservation_id })
                        }))
                    ];
                }

                SmallVec::new()
            }

            // ... other actions ...
        }
    }
}
```

### Key Points

1. **Parent coordinates, children execute** - clear separation
2. **Children are autonomous** - handle own workflows (Pattern 1)
3. **Event-driven communication** - parent sends commands, waits for events
4. **Compensation flows** - parent handles rollback across children
5. **No micromanagement** - parent doesn't know about child's load steps

---

## When to Use Each Pattern

### Decision Tree

```
Does the workflow involve multiple aggregates?
│
├─ NO → Pattern 1: State Machine Within Store
│        Example: Load inventory data before reserving seats
│
└─ YES → Pattern 2: Parent-Child Saga Coordination
         Example: Reserve seats, then process payment, with compensation

         For each child in the saga:
         └─ Does the child need to load data before processing?
            └─ YES → Use Pattern 1 within the child
```

### Pattern 1 Indicators

Use **state machine within store** when:
- ✅ Single aggregate (InventoryStore, PaymentStore)
- ✅ Need to load data before command validation
- ✅ Workflow is internal to one domain concept
- ✅ No cross-aggregate coordination needed

**Examples:**
- Load inventory before reserving seats
- Load payment history before processing refund
- Load user profile before updating preferences

### Pattern 2 Indicators

Use **parent-child saga** when:
- ✅ Multiple aggregates involved (Reservation + Inventory + Payment)
- ✅ Multi-step workflow with decision points
- ✅ Compensation needed on failure
- ✅ Timeouts and deadlines matter

**Examples:**
- Reservation saga (inventory → payment → confirmation)
- Order processing (inventory → shipping → billing)
- Booking flow (availability → reservation → payment → confirmation)

### Anti-Patterns ❌

**DON'T:**
- ❌ Use Pattern 2 for single-aggregate workflows (overengineering)
- ❌ Make parent manage child's internal load workflow (tight coupling)
- ❌ Put workflow state machine in projection (wrong layer)
- ❌ Load all data eagerly on startup (performance/memory waste)
- ❌ Cache loaded state indefinitely (staleness)

---

## Implementation Guide

### Step 1: Define Projection Query Interface

```rust
/// Query interface for the aggregate
#[async_trait::async_trait]
pub trait MyAggregateProjection: Send + Sync {
    async fn load_data(&self, id: AggregateId) -> Result<AggregateData, Error>;
}

pub struct AggregateData {
    pub entity: Option<MyEntity>,
    pub related_data: Vec<RelatedItem>,
}
```

### Step 2: Add Workflow State to Aggregate State

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MyAggregateState {
    // Domain data
    pub entities: HashMap<Id, Entity>,

    // Workflow state machine
    pub workflow: MyWorkflow,

    // Error tracking
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MyWorkflow {
    Idle,
    LoadingData {
        id: Id,
        pending_command: Box<MyAction>,
    },
    Ready {
        loaded_scope: Id,
    },
}
```

### Step 3: Add Query Interface to Environment

```rust
#[derive(Clone)]
pub struct MyEnvironment {
    pub clock: Arc<dyn Clock>,
    pub event_store: Arc<dyn EventStore>,
    pub event_bus: Arc<dyn EventBus>,
    pub stream_id: StreamId,

    // Query interface
    pub projection: Arc<dyn MyAggregateProjection>,
}
```

### Step 4: Implement Load-Then-Process Pattern

```rust
impl Reducer for MyReducer {
    fn reduce(...) -> SmallVec<[Effect; 4]> {
        match action {
            MyAction::SomeCommand { id, ... } => {
                match &state.workflow {
                    Idle => {
                        // Initiate load
                        state.workflow = LoadingData {
                            id,
                            pending_command: Box::new(action.clone()),
                        };

                        let projection = env.projection.clone();
                        return smallvec![
                            Effect::Future(Box::pin(async move {
                                match projection.load_data(id).await {
                                    Ok(data) => Some(MyAction::DataLoaded { id, data }),
                                    Err(e) => Some(MyAction::DataLoadFailed {
                                        id,
                                        error: e.to_string()
                                    }),
                                }
                            }))
                        ];
                    }

                    Ready { loaded_scope } if *loaded_scope == id => {
                        // Process command with loaded state
                        // ... validation, event emission ...
                    }

                    _ => {
                        // Invalid state
                        validation_failed("Invalid workflow state");
                    }
                }
            }

            MyAction::DataLoaded { id, data } => {
                // Hydrate state
                state.entities.insert(id, data.entity);

                // Extract pending command BEFORE updating workflow
                let pending_command = if let LoadingData { pending_command, .. } = &state.workflow {
                    Some(*pending_command.clone())
                } else {
                    None
                };

                state.workflow = Ready { loaded_scope: id };

                // Continue with pending command
                if let Some(cmd) = pending_command {
                    return smallvec![
                        Effect::Future(Box::pin(async move {
                            Some(cmd)
                        }))
                    ];
                }

                SmallVec::new()
            }
        }
    }
}
```

### Step 5: Implement Projection Reconstruction (Separate Concern)

**Note:** This is a **different** problem from command-time state loading.

```rust
// On startup, rebuild projections from event store
async fn rebuild_projections(event_store: &EventStore) -> Result<()> {
    let mut projection = MyProjection::new();

    let events = event_store.load_all_events().await?;
    for event in events {
        projection.handle_event(&event)?;
    }

    Ok(())
}
```

---

## Complete Example

See the **Event Ticketing System** for a full implementation:

### Pattern 1 Example: InventoryStore

**File:** `examples/ticketing/src/aggregates/inventory.rs`

- Implements state machine for loading inventory data
- Query interface: `InventoryProjection::load_inventory()`
- Workflow states: `Idle → LoadingData → Ready`
- Commands: `ReserveSeats`, `ConfirmReservation`, `ReleaseReservation`

### Pattern 2 Example: ReservationStore (Saga)

**File:** `examples/ticketing/src/aggregates/reservation.rs`

- Coordinates InventoryStore + PaymentStore
- Saga states: `Idle → ReservingSeats → ProcessingPayment → Completed`
- Compensation: Releases seats if payment fails
- Children use Pattern 1 internally

### Integration

**File:** `examples/ticketing/src/main.rs`

Shows how to:
1. Create projection implementations
2. Wire up environment with query interfaces
3. Initialize stores with proper dependencies
4. Handle startup projection reconstruction

---

## Common Pitfalls

### Pitfall 1: Loading State Eagerly on Startup ❌

**WRONG:**
```rust
// DON'T: Load all inventory data on server start
let inventory_state = InventoryState::from_projection(
    projection.load_all().await?
);
let store = Store::new(inventory_state, reducer, env);
```

**Why it's wrong:**
- Slow startup for large datasets
- Memory waste (load data you may not need)
- Staleness (data changes after load)

**CORRECT:**
```rust
// DO: Start with empty state, load on-demand
let inventory_state = InventoryState::new(); // Empty
let store = Store::new(inventory_state, reducer, env);

// Load happens lazily when commands arrive
```

### Pitfall 2: Parent Micromanaging Child Workflow ❌

**WRONG:**
```rust
// DON'T: Parent saga controls child's internal steps
ReservationAction::InitiateReservation { ... } => {
    // Step 1: Tell child to load data
    inventory_store.send(LoadInventoryData { ... }).await;

    // Step 2: Wait for data loaded
    // ...

    // Step 3: Tell child to validate
    inventory_store.send(ValidateInventory { ... }).await;

    // Step 4: Tell child to reserve
    inventory_store.send(ReserveSeats { ... }).await;
}
```

**Why it's wrong:**
- Tight coupling between parent and child internals
- Parent knows too much about child workflow
- Hard to test child in isolation

**CORRECT:**
```rust
// DO: Parent sends high-level command, child handles workflow
ReservationAction::InitiateReservation { ... } => {
    // Child handles load → validate → reserve internally
    inventory_store.send(ReserveSeats { ... }).await;

    // Wait for result
}
```

### Pitfall 3: Caching Loaded State Forever ❌

**WRONG:**
```rust
// DON'T: Keep state indefinitely
Ready { loaded_scope } => {
    // Process command using state loaded hours ago!
}
```

**Why it's wrong:**
- Stale reads (data changed in projection)
- Memory leak (unbounded growth)
- Inconsistency with actual state

**BETTER:**
```rust
// Option 1: Reload on every command (simple, always fresh)
Idle => { load_and_process() }

// Option 2: TTL-based cache (balance freshness vs performance)
Ready { loaded_scope, loaded_at } => {
    if loaded_at + TTL < now() {
        // Reload stale data
    } else {
        // Use cached data
    }
}

// Option 3: Explicit cache invalidation (on events)
InventoryAction::SeatsReserved { ... } => {
    // Invalidate cache for this event/section
    state.workflow = Idle;
}
```

### Pitfall 4: Putting Workflow State in Projection ❌

**WRONG:**
```rust
// DON'T: Store workflow state in read model
pub struct InventoryProjection {
    pub inventories: HashMap<...>,
    pub workflow_state: WorkflowState, // ❌ Wrong layer!
}
```

**Why it's wrong:**
- Projection is for **queries**, not command processing
- Multiple command instances would conflict
- Violates CQRS separation

**CORRECT:**
```rust
// DO: Workflow state lives in aggregate state
pub struct InventoryState {
    pub inventories: HashMap<...>,
    pub workflow: InventoryWorkflow, // ✅ Right layer!
}

pub struct InventoryProjection {
    pub inventories: HashMap<...>, // ✅ Just data for queries
}
```

### Pitfall 5: Forgetting to Handle Load Failures ❌

**WRONG:**
```rust
// DON'T: Assume load always succeeds
Effect::Future(async {
    let data = projection.load_data(id).await.unwrap(); // ❌ Can panic!
    Some(DataLoaded { data })
})
```

**CORRECT:**
```rust
// DO: Handle load errors explicitly
Effect::Future(async {
    match projection.load_data(id).await {
        Ok(data) => Some(DataLoaded { data }),
        Err(e) => Some(DataLoadFailed {
            id,
            error: e.to_string()
        }),
    }
})
```

---

## Summary

### Key Takeaways

1. **Aggregates are stateless** - load state on-demand, not eagerly
2. **Commands read from projections** - optimized read models
3. **Two patterns, two layers**:
   - Pattern 1: State machine within single store
   - Pattern 2: Parent saga coordinates multiple stores
4. **Load via Effect feedback** - keeps reducers pure
5. **Each child is autonomous** - handles own workflow
6. **Parent coordinates, doesn't micromanage** - clean separation

### When to Use What

| Scenario | Pattern | Why |
|----------|---------|-----|
| Load inventory before reserve | Pattern 1 | Single store, internal workflow |
| Reserve + Payment + Timeout | Pattern 2 | Multi-aggregate, compensation |
| Load user profile before update | Pattern 1 | Single store, simple load |
| Order processing saga | Pattern 2 | Complex workflow, multiple domains |
| Each child in saga needs data | Pattern 1 | Per-child, internal load |

### Implementation Checklist

For **Pattern 1** (State Machine):
- [ ] Define projection query interface
- [ ] Add workflow state to aggregate state
- [ ] Add projection to environment
- [ ] Implement load-then-process in reducer
- [ ] Handle load success and failure
- [ ] Test with empty state (simulates restart)

For **Pattern 2** (Parent-Child Saga):
- [ ] Define parent saga state machine
- [ ] Identify coordination points between children
- [ ] Implement compensation flows
- [ ] Handle timeouts and failures
- [ ] Let children use Pattern 1 for internal loads
- [ ] Test end-to-end workflow

---

## Related Documentation

- **Event Sourcing**: `docs/event-design-guidelines.md`
- **CQRS Patterns**: `docs/concepts.md`
- **Saga Patterns**: `docs/saga-patterns.md`
- **Projection Management**: `docs/production-database.md`
- **Architecture Spec**: `specs/architecture.md`

---

**Version**: 1.0
**Authors**: Composable Rust Team
**Last Reviewed**: 2025-01-17
