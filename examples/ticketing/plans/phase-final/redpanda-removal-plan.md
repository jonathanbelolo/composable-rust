# Plan: Remove Redpanda, Use Tokio Broadcast Channels

**Goal**: Remove Redpanda dependency from ticketing monolith and replace with fast in-process `tokio::broadcast` channels.

**Philosophy**: Keep it simple for monolith deployment. Add Redpanda later only when splitting into microservices.

---

## Architecture: Two-Level Channel System

### Level 1: Store-Local Channels (Already Exists)
- **Purpose**: Internal store coordination (`send_and_wait_for()`)
- **Lifecycle**: Per-request, ephemeral
- **Location**: `Store.action_broadcast` (runtime/src/lib.rs:1372)
- **Usage**: Same-request action waiting

### Level 2: Global Aggregate Channels (NEW)
- **Purpose**: Cross-aggregate coordination (sagas, projections)
- **Lifecycle**: Server lifetime, fixed
- **Location**: `ResourceManager` (shared infrastructure)
- **Usage**:
  - Sagas subscribe to coordinate multiple aggregates
  - Projections subscribe to build read models
  - All stores of same aggregate type publish to same global channel

---

## Design: Global Aggregate Action Channels

### Channel Structure

```rust
// In ResourceManager
pub struct ResourceManager {
    // ... existing fields ...

    /// Global channels for cross-aggregate coordination
    /// One channel per aggregate type, shared by ALL instances
    pub event_actions: broadcast::Sender<EventAction>,
    pub inventory_actions: broadcast::Sender<InventoryAction>,
    pub reservation_actions: broadcast::Sender<ReservationAction>,
    pub payment_actions: broadcast::Sender<PaymentAction>,
}
```

### Publishing Strategy

**Each Store publishes to BOTH:**
1. **Local channel** (`store.action_broadcast`) - for internal `send_and_wait_for()`
2. **Global channel** (`env.global_actions`) - for projections

**Implementation approach:**
- Pass global channel via Environment
- Stores publish to global channel after state changes
- Projections subscribe to global channels

---

## Bidirectional Communication Pattern

### The Challenge

**Ephemeral stores need responses from global projections:**
- Store is created per-request (short-lived)
- Projection is singleton (long-lived)
- Store publishes action to global channel
- Projection processes it
- **How does projection send result back to the SPECIFIC ephemeral store?**

### Solution: Response Channels in Actions

Include `oneshot::Sender` in actions that need responses.

#### Pattern Overview

```rust
use tokio::sync::oneshot;

// Action variant that needs projection response
pub enum EventAction {
    CreateEvent {
        id: EventId,
        name: String,
        // ... other fields ...

        /// Response channel for projection completion
        /// Projection sends result back via this channel
        respond_to: Option<oneshot::Sender<Result<(), ProjectionError>>>,
    },

    // Regular action without response
    EventCreated {
        id: EventId,
        // ...
    },
}
```

#### Requester Side (Ephemeral Store)

```rust
// Create response channel
let (response_tx, response_rx) = oneshot::channel();

// Publish action with response channel
let action = EventAction::CreateEvent {
    id: event_id,
    name: "Concert".to_string(),
    respond_to: Some(response_tx),
};

// Publish to global channel
env.global_actions.send(action)?;

// Wait for projection to respond
match tokio::time::timeout(Duration::from_secs(5), response_rx).await {
    Ok(Ok(Ok(()))) => { /* Projection succeeded */ }
    Ok(Ok(Err(e))) => { /* Projection failed */ }
    Ok(Err(_)) => { /* Channel closed (projection died) */ }
    Err(_) => { /* Timeout */ }
}
```

#### Projection Side

```rust
// Projection processes actions from global channel
while let Ok(action) = stream.recv().await {
    match action {
        EventAction::CreateEvent { id, name, respond_to, .. } => {
            // Process projection
            let result = self.process_event_created(id, name).await;

            // Send response back to requester (if they're still waiting)
            if let Some(tx) = respond_to {
                // Fails gracefully if requester dropped/timed out
                let _ = tx.send(result);
            }
        }
        _ => { /* Other actions without responses */ }
    }
}
```

### Why This Pattern is Optimal

✅ **O(1) direct point-to-point** - no filtering, no lookups
✅ **Zero shared state** - no global registry, no locks
✅ **Automatic cleanup** - when store drops, channel drops automatically
✅ **Lock-free** - no contention on shared data structures
✅ **Idiomatic Rust** - standard actor pattern used by Tokio ecosystem
✅ **Graceful degradation** - if requester times out, projection.send() fails silently

### When to Use Response Channels

**Use `respond_to: Option<oneshot::Sender<T>>` when:**
- ✅ Ephemeral store needs to wait for projection completion
- ✅ API must return only after projection updates (strong consistency)
- ✅ Error handling requires projection status

**Don't use response channels when:**
- ❌ Fire-and-forget updates (eventual consistency is fine)
- ❌ No one is waiting for the result
- ❌ Projection is purely read-side optimization

---

## Step-by-Step Implementation Plan

### Phase 1: Add Global Channels Infrastructure

#### Step 1.1: Add Global Channels to ResourceManager
**File**: `src/bootstrap/resources.rs`

```rust
// Add to imports
use tokio::sync::broadcast;
use crate::aggregates::{EventAction, InventoryAction, ReservationAction, PaymentAction};

// Add to struct
pub struct ResourceManager {
    // Existing fields...

    /// Global action channels for cross-aggregate coordination
    /// Channel capacity: 1000 (sufficient for high-throughput monolith)
    pub event_actions: broadcast::Sender<EventAction>,
    pub inventory_actions: broadcast::Sender<InventoryAction>,
    pub reservation_actions: broadcast::Sender<ReservationAction>,
    pub payment_actions: broadcast::Sender<PaymentAction>,
}

// In from_config():
let (event_actions, _) = broadcast::channel(1000);
let (inventory_actions, _) = broadcast::channel(1000);
let (reservation_actions, _) = broadcast::channel(1000);
let (payment_actions, _) = broadcast::channel(1000);
```

**Why capacity 1000?**
- Sufficient buffer for bursts
- Prevents slow subscribers from blocking fast publishers
- Lagging subscribers drop old messages (acceptable for projections - they rebuild from PostgreSQL)

#### Step 1.2: Remove Redpanda from ResourceManager
**File**: `src/bootstrap/resources.rs`

- ❌ Remove `event_bus: Arc<dyn EventBus>` field
- ❌ Remove RedpandaEventBus creation
- ❌ Remove topic initialization logic
- ❌ Remove `use composable_rust_redpanda::RedpandaEventBus`

---

### Phase 2: Add Response Channels to Actions

Actions that need projection responses must include `respond_to: Option<oneshot::Sender<T>>`.

#### Step 2.1: Update Action Enums with Response Channels
**Files**: `src/aggregates/*.rs`

Add response channel fields to action variants that trigger projections:

```rust
use tokio::sync::oneshot;

pub enum EventAction {
    // Command actions that trigger projections
    CreateEvent {
        id: EventId,
        name: String,
        owner_id: UserId,
        venue: Venue,
        date: NaiveDate,
        pricing_tiers: Vec<PricingTier>,

        /// Response channel for projection completion
        /// If provided, projection will send result back when done
        respond_to: Option<oneshot::Sender<Result<(), ProjectionError>>>,
    },

    // Event actions (no response needed)
    EventCreated {
        id: EventId,
        // ... other fields, NO respond_to
    },
}
```

**Apply to all aggregates:**
- EventAction::CreateEvent
- InventoryAction::InitializeInventory
- ReservationAction::CreateReservation
- PaymentAction::ProcessPayment

**When to add `respond_to`?**
- ✅ Action triggers projection updates
- ✅ Caller needs to wait for projection completion
- ❌ Fire-and-forget actions (eventual consistency)

---

### Phase 3: Modify Aggregate Environments

Each aggregate environment needs access to its global channel for publishing.

#### Step 3.1: Add Global Channel to EventEnvironment
**File**: `src/aggregates/event.rs`

```rust
pub struct EventEnvironment {
    pub clock: Arc<dyn Clock>,
    pub event_store: Arc<dyn EventStore>,
    pub stream_id: StreamId,
    pub projection: Arc<dyn EventProjectionQuery>,

    /// Global action channel for cross-aggregate coordination
    /// All Event aggregate instances publish to this shared channel
    pub global_actions: broadcast::Sender<EventAction>,
}

impl EventEnvironment {
    pub fn new(
        clock: Arc<dyn Clock>,
        event_store: Arc<dyn EventStore>,
        stream_id: StreamId,
        projection: Arc<dyn EventProjectionQuery>,
        global_actions: broadcast::Sender<EventAction>,
    ) -> Self {
        Self { clock, event_store, stream_id, projection, global_actions }
    }
}
```

**Remove:**
- ❌ `event_bus: Arc<dyn EventBus>` field
- ❌ `publish_event!` macro calls

#### Step 3.2: Repeat for Other Aggregates

Apply same pattern to:
- **InventoryEnvironment** (`src/aggregates/inventory.rs`)
  - Add `global_actions: broadcast::Sender<InventoryAction>`
- **ReservationEnvironment** (`src/aggregates/reservation.rs`)
  - Add `global_actions: broadcast::Sender<ReservationAction>`
- **PaymentEnvironment** (`src/aggregates/payment.rs`)
  - Add `global_actions: broadcast::Sender<PaymentAction>`

---

### Phase 4: Implement Publishing to Global Channels

Modify Store to publish actions to BOTH local and global channels.

#### Step 4.1: Pass Global Channel to Store
**File**: `runtime/src/lib.rs`

**Option A: Via Environment (Recommended)**

Stores get global channel from Environment and publish manually in reducer effects.

**Option B: Store-level global channel field**

```rust
pub struct Store<S, A, E, R> {
    // ... existing fields ...

    /// Global action channel for cross-aggregate coordination
    /// Optional - only set if aggregate participates in sagas/projections
    global_actions: Option<broadcast::Sender<A>>,
}

// Modify effect execution to dual-publish
async fn execute_effect(&self, effect: Effect<A>) -> Option<A> {
    // ... existing execution logic ...

    // After action is produced, publish to global channel
    if let Some(action) = result {
        if let Some(global) = &self.global_actions {
            // Ignore send errors (no subscribers = okay)
            let _ = global.send(action.clone());
        }
    }

    result
}
```

**Recommendation**: Use **Option A** (via Environment) for simplicity.
- Aggregates explicitly publish to global channel when needed
- More control, clearer intent
- No Store framework changes needed

#### Step 4.2: Publish to Global Channel in Reducer Effects

Each aggregate publishes to global channel after state changes.

**Example for Event aggregate:**

```rust
fn reduce(&self, state: &mut EventState, action: EventAction, env: &EventEnvironment) -> Vec<Effect<EventAction>> {
    match action {
        EventAction::CreateEvent { ... } => {
            // Validate and create event
            let event = EventCreated { ... };

            smallvec![
                // Persist to PostgreSQL
                append_events! { ... },

                // Publish to global channel for saga coordination
                Effect::Future(Box::pin(async move {
                    let _ = env.global_actions.send(EventAction::EventCreated { ... });
                    None
                }))
            ]
        }
        // ... other actions
    }
}
```

**Pattern**: Publish after successful state change, not on errors.

---

### Phase 5: Simplify Saga Pattern (Remove Choreography)

**Old pattern (choreography)**: Saga publishes commands to global channels, background consumers route events back to saga.

**New pattern (orchestration)**: Saga directly creates and calls child aggregate stores. No background routing needed.

#### Step 5.1: Remove Saga Consumer Spawning
**File**: `src/aggregates/event_inventory_saga.rs`

**Delete these functions** (no longer needed):
- ❌ `spawn_event_inventory_saga_consumers()`
- ❌ Any background task spawning for saga routing

**Why remove?**
- Sagas are per-request stores (not singletons)
- Sagas directly orchestrate child aggregates via factory functions
- No need for global event routing back to saga
- Simpler, more testable, follows Composable Rust principles

#### Step 5.2: Remove ReservationSaga Consumers
**File**: `src/aggregates/reservation.rs`

**Delete these functions**:
- ❌ `spawn_reservation_saga_consumers()`
- ❌ Background routing logic

**Replace with**: Direct orchestration pattern (same as EventInventorySaga in Step 6.3)

---

### Phase 6: Rewrite Projections with Local Channels

Projections subscribe to global aggregate channels instead of Redpanda.

#### Step 6.1: Create LocalProjectionStream
**File**: `src/projections/local_stream.rs` (NEW)

Replace `ProjectionStream` (Redpanda-based) with local channel variant:

```rust
use tokio::sync::broadcast;

/// Projection stream backed by local broadcast channel
/// Replaces RedpandaEventBus for in-process projections
pub struct LocalProjectionStream<A> {
    receiver: broadcast::Receiver<A>,
    name: String,
}

impl<A: Clone> LocalProjectionStream<A> {
    pub fn new(
        channel: broadcast::Sender<A>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            receiver: channel.subscribe(),
            name: name.into(),
        }
    }

    /// Receive next action (non-blocking)
    pub async fn recv(&mut self) -> Result<A, RecvError> {
        match self.receiver.recv().await {
            Ok(action) => Ok(action),
            Err(broadcast::error::RecvError::Lagged(n)) => {
                // Projection fell behind, dropped messages
                // This is OK - projection rebuilds from PostgreSQL on restart
                tracing::warn!(
                    projection = %self.name,
                    dropped = n,
                    "Projection lagged, dropped messages"
                );
                // Continue with next message
                self.receiver.recv().await
                    .map_err(|_| RecvError::ChannelClosed)
            }
            Err(broadcast::error::RecvError::Closed) => {
                Err(RecvError::ChannelClosed)
            }
        }
    }
}

pub enum RecvError {
    ChannelClosed,
}
```

**Key difference from Redpanda ProjectionStream:**
- ❌ No checkpointing (not needed - rebuild from PostgreSQL on restart)
- ❌ No consumer groups
- ✅ Much simpler
- ✅ Much faster
- ✅ Lagging = drop old messages (projections are eventually consistent anyway)

#### Step 6.2: Rewrite Projection Manager
**File**: `src/projections/manager.rs`

**Current** (Redpanda-based):
```rust
// Creates RedpandaEventBus per projection with consumer group
let event_bus = Arc::new(RedpandaEventBus::builder()...);
let stream = ProjectionStream::new(event_bus, checkpoint, topic, ...);
```

**New** (Local channels):
```rust
pub async fn spawn_projections(
    resources: &ResourceManager,
    shutdown: broadcast::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    let projection_pool = resources.projections_pool.clone();

    // Available Seats Projection (consumes Inventory actions)
    {
        let projection = Arc::new(PostgresAvailableSeatsProjection::new(
            Arc::new(projection_pool.clone())
        ));

        let mut stream = LocalProjectionStream::new(
            resources.inventory_actions.clone(),
            "available-seats"
        );

        let mut shutdown_rx = shutdown.resubscribe();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = stream.recv() => {
                        match result {
                            Ok(action) => {
                                if let Err(e) = projection.process_action(action).await {
                                    tracing::error!("Projection error: {}", e);
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        tracing::info!("Available seats projection shutting down");
                        break;
                    }
                }
            }
        });
    }

    // Repeat for other projections...
    // - Sales Analytics (consumes Reservation + Payment actions)
    // - Customer History (consumes Reservation actions)

    Ok(())
}
```

**No checkpointing needed because:**
- On server restart, projections rebuild from PostgreSQL event store
- Eventual consistency is acceptable for read models
- Much simpler than managing Redpanda offsets

#### Step 6.3: Add rebuild_from_event_store() to Projections

Each projection needs ability to rebuild state on startup.

```rust
impl PostgresAvailableSeatsProjection {
    /// Rebuild projection from PostgreSQL event store
    /// Called on server startup
    pub async fn rebuild_from_event_store(
        &self,
        event_store: &Arc<dyn EventStore>,
    ) -> Result<()> {
        tracing::info!("Rebuilding available seats projection...");

        // Clear existing projection data
        sqlx::query("DELETE FROM available_seats")
            .execute(&*self.pool)
            .await?;

        // Replay all inventory events from event store
        // (Implementation depends on event store query API)

        tracing::info!("Available seats projection rebuilt");
        Ok(())
    }
}
```

**Call on startup** in projection manager before spawning consumers.

---

### Phase 7: Update Bootstrap and AppState

Wire everything together with global channels.

#### Step 7.1: Update AppState Factory Methods
**File**: `src/server/state.rs`

Pass global channels to aggregate environments:

```rust
impl AppState {
    pub fn create_event_store(&self, event_id: EventId) -> Store<...> {
        let env = EventEnvironment::new(
            self.clock.clone(),
            self.event_store.clone(),
            StreamId::new(&format!("event-{}", event_id.as_uuid())),
            self.event_query.clone(),
            self.resources.event_actions.clone(), // ← NEW: global channel
        );

        Store::new(EventState::new(), EventReducer::new(), env)
    }

    pub fn create_inventory_store(&self, event_id: EventId) -> Store<...> {
        let env = InventoryEnvironment::new(
            self.clock.clone(),
            self.event_store.clone(),
            self.resources.event_bus.clone(),
            StreamId::new(&format!("inventory-{}", event_id.as_uuid())),
            self.inventory_query.clone(),
            self.resources.inventory_actions.clone(), // ← NEW: global channel
        );

        Store::new(InventoryState::new(), InventoryReducer::new(), env)
    }

    // Repeat for reservation_store, payment_store...
}
```

#### Step 7.2: Saga Environment Gets Store Factories
**File**: `src/aggregates/event_inventory_saga.rs`

**Sagas are just stores** - they directly orchestrate child aggregates, no background consumers needed.

```rust
use std::sync::Arc;

pub struct EventInventorySagaEnvironment {
    pub clock: Arc<dyn Clock>,
    pub event_store: Arc<dyn EventStore>,
    pub stream_id: StreamId,

    /// Factory functions to create child aggregate stores
    /// Saga calls these directly to orchestrate Event + Inventory
    pub create_event_store: Arc<dyn Fn(EventId) -> Store<EventState, EventAction, EventEnvironment, EventReducer> + Send + Sync>,
    pub create_inventory_store: Arc<dyn Fn(EventId) -> Store<InventoryState, InventoryAction, InventoryEnvironment, InventoryReducer> + Send + Sync>,
}
```

**Why factory functions?**
- Saga creates Event/Inventory stores on-demand during orchestration
- No global routing needed
- Saga directly calls `event_store.send()`, `inventory_store.send()`
- Clean, synchronous orchestration

#### Step 7.3: Implement Saga Orchestration Logic
**File**: `src/aggregates/event_inventory_saga.rs`

```rust
impl Reducer for EventInventorySaga {
    // ... existing trait bounds ...

    fn reduce(
        &self,
        state: &mut EventInventorySagaState,
        action: EventInventorySagaAction,
        env: &EventInventorySagaEnvironment,
    ) -> Vec<Effect<EventInventorySagaAction>> {
        match action {
            EventInventorySagaAction::CreateEventWithInventory {
                event_id,
                name,
                owner_id,
                venue,
                date,
                pricing_tiers,
            } => {
                // Update saga state
                state.event_id = Some(event_id);
                state.sections_to_initialize = venue.sections.iter()
                    .map(|s| s.name.clone())
                    .collect();

                // Orchestrate Event + Inventory creation
                let create_event_store = env.create_event_store.clone();
                let create_inventory_store = env.create_inventory_store.clone();
                let venue_clone = venue.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // Step 1: Create Event
                    let event_store = create_event_store(event_id);

                    let create_event_action = EventAction::CreateEvent {
                        id: event_id,
                        name,
                        owner_id,
                        venue: venue_clone.clone(),
                        date,
                        pricing_tiers,
                    };

                    // Wait for Event creation to complete
                    event_store.send_and_wait_for(
                        create_event_action,
                        |a| matches!(a, EventAction::EventCreated { .. }),
                        Duration::from_secs(5),
                    ).await.ok()?;

                    // Step 2: Initialize Inventory for each section
                    for section in &venue_clone.sections {
                        let inventory_store = create_inventory_store(event_id);

                        let init_action = InventoryAction::InitializeInventory {
                            event_id,
                            section: section.name.clone(),
                            total_seats: section.total_seats,
                        };

                        inventory_store.send(init_action).await.ok()?;
                    }

                    // Saga completed successfully
                    Some(EventInventorySagaAction::EventCreationCompleted { event_id })
                }))]
            }

            EventInventorySagaAction::EventCreationCompleted { event_id } => {
                state.completed = true;
                tracing::info!(?event_id, "Event inventory saga completed");
                smallvec![Effect::None]
            }

            _ => smallvec![Effect::None],
        }
    }
}
```

**Key points:**
- ✅ Saga directly creates Event and Inventory stores
- ✅ Synchronous orchestration via `send_and_wait_for()`
- ✅ No background consumers, no global routing
- ✅ Saga is just a regular store, per-request lifecycle
- ✅ Clean, testable, follows Composable Rust principles

#### Step 7.4: Create Saga in AppState
**File**: `src/server/state.rs`

```rust
impl AppState {
    pub fn create_event_inventory_saga(&self) -> Store<EventInventorySagaState, EventInventorySagaAction, EventInventorySagaEnvironment, EventInventorySaga> {
        let create_event_store = {
            let state = self.clone();
            Arc::new(move |event_id: EventId| state.create_event_store(event_id))
        };

        let create_inventory_store = {
            let state = self.clone();
            Arc::new(move |event_id: EventId| state.create_inventory_store(event_id))
        };

        let saga_env = EventInventorySagaEnvironment {
            clock: self.clock.clone(),
            event_store: self.event_store.clone(),
            stream_id: StreamId::new(&format!("event-inventory-saga-{}", Uuid::new_v4())),
            create_event_store,
            create_inventory_store,
        };

        Store::new(
            EventInventorySagaState::new(),
            EventInventorySaga::new(),
            saga_env,
        )
    }
}
```

**Pattern**: Factory closures capture AppState, allowing saga to create child stores on-demand.

---

### Phase 8: Update API to Use Saga

#### Step 8.1: Use Saga in API Handler
**File**: `src/api/events.rs`

```rust
pub async fn create_event(
    session: SessionUser,
    State(state): State<AppState>,
    Json(request): Json<CreateEventRequest>,
) -> Result<(StatusCode, Json<CreateEventResponse>), AppError> {
    let event_id = EventId::new();
    let (venue, date, pricing_tiers) = request.to_domain_types();

    // Create saga store (per-request)
    let saga = state.create_event_inventory_saga();

    let action = EventInventorySagaAction::CreateEventWithInventory {
        event_id,
        name: request.title.clone(),
        owner_id: session.user_id,
        venue,
        date,
        pricing_tiers,
    };

    // Wait for saga completion
    saga.send_and_wait_for(
        action,
        |a| matches!(a, EventInventorySagaAction::EventCreationCompleted { .. }),
        Duration::from_secs(10),
    ).await
        .map_err(|e| AppError::internal(format!("Saga failed: {e}")))?;

    Ok((StatusCode::CREATED, Json(CreateEventResponse {
        event_id: *event_id.as_uuid(),
        message: "Event created successfully".to_string(),
    })))
}
```

**Key changes:**
- ✅ Per-request saga store (not singleton)
- ✅ Saga orchestrates Event + Inventory creation synchronously
- ✅ API waits for saga completion before responding
- ✅ No background routing, no global saga state

---

### Phase 9: Remove Redpanda Dependencies

Clean up all Redpanda-related code.

#### Step 9.1: Remove from Cargo.toml
**File**: `examples/ticketing/Cargo.toml`

```toml
# REMOVE these dependencies:
# composable-rust-redpanda = { path = "../../redpanda" }
# kafka = "..."
```

#### Step 9.2: Remove from Imports

Search and remove across all files:
- `use composable_rust_redpanda::*`
- `use composable_rust_core::event_bus::EventBus`
- `publish_event!` macro calls

#### Step 9.3: Remove Redpanda Config
**File**: `src/config.rs`

```rust
// REMOVE RedpandaConfig struct
// REMOVE redpanda field from Config
```

#### Step 9.4: Remove Docker Compose Redpanda Service
**File**: `docker-compose.yml`

Remove Redpanda container definition.

#### Step 9.5: Remove `publish_event!` Macro Calls

Search for all occurrences:
```bash
rg "publish_event!" examples/ticketing/src/
```

Remove from:
- `src/aggregates/event.rs`
- `src/aggregates/inventory.rs`
- `src/aggregates/reservation.rs`
- `src/aggregates/payment.rs`

Replace with global channel publish (via Effect::Future).

---

### Phase 10: Update Tests

Adapt tests to use local channels instead of Redpanda.

#### Step 10.1: Remove Redpanda Test Containers
**Files**: All `tests/*.rs`

```rust
// REMOVE:
// use testcontainers::clients::Cli;
// use composable_rust_testing::redpanda::Redpanda;
// let docker = Cli::default();
// let redpanda_node = docker.run(Redpanda::default());
```

#### Step 10.2: Create Test ResourceManager Factory

```rust
// In test utilities
async fn create_test_resources() -> ResourceManager {
    let config = Config::test_defaults();
    ResourceManager::from_config(&config).await.unwrap()
}
```

Much simpler - no Redpanda container needed!

#### Step 10.3: Remove 500ms Sleeps

All those sleeps were waiting for Redpanda routing. With local channels:
- ✅ No serialization delay
- ✅ No network delay
- ✅ Microsecond latency
- ✅ No sleeps needed

---

## Migration Checklist

### Infrastructure
- [ ] Add global channels to ResourceManager
- [ ] Remove Redpanda from ResourceManager
- [ ] Remove Redpanda from docker-compose.yml
- [ ] Remove Redpanda config from Config struct
- [ ] Remove Redpanda dependency from Cargo.toml

### Aggregates
- [ ] Add global_actions to EventEnvironment
- [ ] Add global_actions to InventoryEnvironment
- [ ] Add global_actions to ReservationEnvironment
- [ ] Add global_actions to PaymentEnvironment
- [ ] Remove event_bus from all environments
- [ ] Remove publish_event! macro calls (replace with global channel publish)

### Sagas
- [ ] Create singleton EventInventorySaga in ResourceManager
- [ ] Rewrite spawn_event_inventory_saga_consumers() to use local channels
- [ ] Rewrite spawn_reservation_saga_consumers() to use local channels
- [ ] Update saga environments to use global channels
- [ ] Spawn saga consumers on server startup

### Projections
- [ ] Create LocalProjectionStream for local channels
- [ ] Rewrite projection manager to use LocalProjectionStream
- [ ] Add rebuild_from_event_store() to each projection
- [ ] Call rebuild on startup
- [ ] Remove Redpanda-based ProjectionStream usage

### AppState
- [ ] Update create_event_store() to pass global channel
- [ ] Update create_inventory_store() to pass global channel
- [ ] Update create_reservation_store() to pass global channel
- [ ] Update create_payment_store() to pass global channel
- [ ] Remove per-request saga factories (use singleton from ResourceManager)

### API
- [ ] Update create_event API to use singleton saga
- [ ] Remove any Redpanda-specific error handling

### Tests
- [ ] Remove Redpanda testcontainers setup
- [ ] Remove 500ms sleeps (no longer needed)
- [ ] Update test resource creation
- [ ] Verify all tests pass

### Documentation
- [ ] Update README to remove Redpanda setup instructions
- [ ] Document local channel architecture
- [ ] Document when to add Redpanda later (microservices split)

---

## Performance Expectations

### Before (Redpanda)
- Latency: 5-50ms per event (serialization + network + deserialization)
- Throughput: ~1,000 events/sec
- Infrastructure: Requires Kafka/Redpanda cluster
- Memory: Moderate (network buffers, consumer state)

### After (Local Channels)
- Latency: < 1μs per event (in-memory clone)
- Throughput: > 100,000 events/sec
- Infrastructure: None (pure Rust)
- Memory: Minimal (channel buffers only)

**Expected improvement**: 100-1000x faster 🚀

---

## Future: Adding Redpanda Back for Microservices

When you need to split the monolith:

### Step 1: Identify Service Boundary
Example: Split analytics into separate service

### Step 2: Add Redpanda for That Boundary Only
- Keep local channels for intra-service coordination
- Add Redpanda publish for cross-service events
- Analytics service subscribes to Redpanda

### Step 3: Dual Publish Pattern
```rust
// In aggregate reducer
smallvec![
    // PostgreSQL (durable)
    append_events! { ... },

    // Local channel (intra-service)
    Effect::Future(Box::pin(async move {
        let _ = env.global_actions.send(action.clone());
        None
    })),

    // Redpanda (inter-service) - ONLY for external consumers
    publish_event! {
        bus: env.event_bus,
        topic: "analytics.events",
        ...
    }
]
```

**Pattern**: Local channels for same-process, Redpanda only when crossing process boundaries.

---

## Questions to Address During Implementation

### Q1: Global channel capacity?
**Recommendation**: 1000
- Sufficient for bursts
- Lagging projections drop old messages (acceptable)
- Prevents blocking fast publishers

### Q2: What if projection lags and drops messages?
**Answer**: Rebuild from PostgreSQL on restart
- Projections are eventually consistent
- Dropping old messages is fine
- Simpler than Redpanda checkpointing

### Q3: How to handle projection failures?
**Answer**:
- Log error, continue processing
- Add circuit breaker if needed
- Rebuild from event store if corruption detected

### Q4: Singleton saga state management?
**Answer**: EventInventorySaga tracks ALL in-flight event creations
- State: `HashMap<EventId, SagaInstance>`
- Cleanup completed sagas after TTL
- Monitor for memory leaks

---

## Success Criteria

✅ **Functionality**: All features work identically
✅ **Performance**: 100x+ faster than Redpanda version
✅ **Simplicity**: No external infrastructure (except PostgreSQL)
✅ **Tests**: All tests pass without sleeps
✅ **Code**: Cleaner, more maintainable
✅ **Future-proof**: Easy to add Redpanda later for microservices split
