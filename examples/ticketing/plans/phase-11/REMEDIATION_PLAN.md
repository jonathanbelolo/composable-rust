# Remediation Plan - Restore Composable Rust Architecture

**Goal:** Remove the service layer and restore the proper Composable Rust architecture where reducers control effects and Store executes them.

**Estimated Effort:** 16-20 hours (~2 days)
**Risk:** Medium (requires careful migration of working integration tests)

---

## Overview

The framework already provides everything we need:
- ✅ `Effect` enum with `EventStore`, `PublishEvent`, `Delay`, `Future`, `Stream` variants
- ✅ Macros: `append_events!`, `load_events!`, `publish_event!`, `async_effect!`, `delay!`
- ✅ `Store` runtime that executes all effect types
- ✅ `send_and_wait_for()` for request-response patterns (already in Store!)

**We just need to use them correctly.**

---

## Phase 1: Update Reducer Environments (1 hour)

Reducers need access to `EventStore` and `EventBus` to create effects.

### 1.1 Update Environment Definitions

```rust
// src/aggregates/inventory.rs
use composable_rust_postgres::PostgresEventStore;
use composable_rust_core::event_bus::EventBus;
use composable_rust_core::stream::StreamId;

#[derive(Clone)]
pub struct InventoryEnvironment {
    pub clock: Arc<dyn Clock>,
    pub event_store: Arc<dyn EventStore>,     // ✅ For Effect::EventStore
    pub event_bus: Arc<dyn EventBus>,         // ✅ For Effect::PublishEvent
    pub stream_id: StreamId,                  // ✅ For this aggregate instance
}

impl InventoryEnvironment {
    pub fn new(
        clock: Arc<dyn Clock>,
        event_store: Arc<dyn EventStore>,
        event_bus: Arc<dyn EventBus>,
        stream_id: StreamId,
    ) -> Self {
        Self {
            clock,
            event_store,
            event_bus,
            stream_id,
        }
    }
}
```

**Update for all aggregates:**
- [ ] `InventoryEnvironment` (src/aggregates/inventory.rs)
- [ ] `ReservationEnvironment` (src/aggregates/reservation.rs)
- [ ] `PaymentEnvironment` (src/aggregates/payment.rs)
- [ ] `EventEnvironment` (src/aggregates/event.rs)

### 1.2 Update Test Environments

```rust
// In test modules
#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_testing::{InMemoryEventStore, InMemoryEventBus};

    fn create_test_env() -> InventoryEnvironment {
        InventoryEnvironment::new(
            Arc::new(SystemClock),
            Arc::new(InMemoryEventStore::new()),
            Arc::new(InMemoryEventBus::new()),
            StreamId::new("test-inventory"),
        )
    }
}
```

**Note:** Check if `InMemoryEventStore` and `InMemoryEventBus` exist in `composable-rust-testing`. If not, create simple implementations.

---

## Phase 2: Create Serialization Helper (30 minutes)

We need a helper to serialize actions into `SerializedEvent`.

### 2.1 Create Serialization Module

```rust
// src/serialization.rs

use crate::projections::TicketingEvent;
use composable_rust_core::event::SerializedEvent;

/// Serialize a ticketing action into a `SerializedEvent`
///
/// # Errors
///
/// Returns error if JSON serialization fails.
pub fn serialize_ticketing_action(event: &TicketingEvent) -> Result<SerializedEvent, String> {
    // Extract event type name
    let event_type = match event {
        TicketingEvent::Inventory(action) => {
            format!("Inventory.{:?}", action)
                .split('(')
                .next()
                .unwrap_or("Inventory.Unknown")
                .to_string()
        }
        TicketingEvent::Reservation(action) => {
            format!("Reservation.{:?}", action)
                .split('(')
                .next()
                .unwrap_or("Reservation.Unknown")
                .to_string()
        }
        TicketingEvent::Payment(action) => {
            format!("Payment.{:?}", action)
                .split('(')
                .next()
                .unwrap_or("Payment.Unknown")
                .to_string()
        }
        TicketingEvent::Event(action) => {
            format!("Event.{:?}", action)
                .split('(')
                .next()
                .unwrap_or("Event.Unknown")
                .to_string()
        }
    };

    // Serialize to JSON
    let data = serde_json::to_vec(event)
        .map_err(|e| format!("Failed to serialize event: {}", e))?;

    Ok(SerializedEvent::new(event_type, data, None))
}
```

### 2.2 Export from lib.rs

```rust
// src/lib.rs
pub mod serialization;
pub use serialization::serialize_ticketing_action;
```

---

## Phase 3: Update Reducers to Return Effects (6-8 hours)

### 3.1 Pattern for Commands (Create Events)

Commands should:
1. Validate
2. Create event
3. Apply event to state (optimistic update)
4. Return effects to persist and publish

```rust
// src/aggregates/inventory.rs
use composable_rust_core::{append_events, publish_event, delay, SmallVec};
use crate::serialization::serialize_ticketing_action;

impl Reducer for InventoryReducer {
    type State = InventoryState;
    type Action = InventoryAction;
    type Environment = InventoryEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ========== COMMAND: Initialize Inventory ==========
            InventoryAction::InitializeInventory {
                event_id,
                section,
                capacity,
                seats,
            } => {
                // 1. Validate
                if let Err(error) = Self::validate_initialize_inventory(state, &event_id, &section) {
                    let error_event = InventoryAction::ValidationFailed { error };
                    Self::apply_event(state, &error_event);
                    return smallvec![Effect::None]; // Don't persist validation errors
                }

                // 2. Create event
                let event = InventoryAction::InventoryInitialized {
                    event_id,
                    section: section.clone(),
                    capacity,
                    seats: seats.clone(),
                    initialized_at: env.clock.now(),
                };

                // 3. Apply to state (optimistic)
                Self::apply_event(state, &event);

                // 4. Serialize
                let ticketing_event = TicketingEvent::Inventory(event.clone());
                let serialized = match serialize_ticketing_action(&ticketing_event) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!("Serialization failed: {}", e);
                        return smallvec![Effect::None];
                    }
                };

                // 5. Return effects
                smallvec![
                    // Effect 1: Save to event store
                    append_events! {
                        store: env.event_store,
                        stream: env.stream_id.as_str(),
                        expected_version: None,
                        events: vec![serialized.clone()],
                        on_success: |_version| None,
                        on_error: |error| {
                            tracing::error!("Failed to save event: {}", error);
                            Some(InventoryAction::SaveFailed {
                                error: error.to_string()
                            })
                        }
                    },

                    // Effect 2: Publish to event bus
                    publish_event! {
                        bus: env.event_bus,
                        topic: "inventory",
                        event: serialized,
                        on_success: || None,
                        on_error: |error| {
                            tracing::error!("Failed to publish event: {}", error);
                            Some(InventoryAction::PublishFailed {
                                error: error.to_string()
                            })
                        }
                    }
                ]
            }

            // ========== COMMAND: Reserve Seats (with timeout) ==========
            InventoryAction::ReserveSeats {
                reservation_id,
                event_id,
                section,
                quantity,
                specific_seats,
                expires_at,
            } => {
                // 1. Validate
                if let Err(error) = Self::validate_reserve_seats(state, &event_id, &section, quantity) {
                    let error_event = InventoryAction::ValidationFailed { error };
                    Self::apply_event(state, &error_event);
                    return smallvec![Effect::None];
                }

                // 2. Select seats
                let seats = if specific_seats.is_some() {
                    // TODO: Implement specific seat selection
                    Self::select_available_seats(state, &event_id, &section, quantity)
                } else {
                    Self::select_available_seats(state, &event_id, &section, quantity)
                };

                if seats.is_empty() {
                    let error_event = InventoryAction::ValidationFailed {
                        error: "No seats available".to_string()
                    };
                    Self::apply_event(state, &error_event);
                    return smallvec![Effect::None];
                }

                // 3. Create event
                let event = InventoryAction::SeatsReserved {
                    reservation_id,
                    event_id,
                    section: section.clone(),
                    seats: seats.clone(),
                    expires_at,
                    reserved_at: env.clock.now(),
                };

                // 4. Apply to state
                Self::apply_event(state, &event);

                // 5. Serialize
                let ticketing_event = TicketingEvent::Inventory(event);
                let serialized = match serialize_ticketing_action(&ticketing_event) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!("Serialization failed: {}", e);
                        return smallvec![Effect::None];
                    }
                };

                // 6. Calculate timeout duration
                let now = env.clock.now();
                let timeout_duration = if expires_at > now {
                    (expires_at - now).to_std().unwrap_or(std::time::Duration::from_secs(300))
                } else {
                    std::time::Duration::from_secs(0)
                };

                // 7. Return effects
                smallvec![
                    // Save to event store
                    append_events! {
                        store: env.event_store,
                        stream: env.stream_id.as_str(),
                        expected_version: None,
                        events: vec![serialized.clone()],
                        on_success: |_version| None,
                        on_error: |error| Some(InventoryAction::SaveFailed { error: error.to_string() })
                    },

                    // Publish to event bus
                    publish_event! {
                        bus: env.event_bus,
                        topic: "inventory",
                        event: serialized,
                        on_success: || None,
                        on_error: |error| Some(InventoryAction::PublishFailed { error: error.to_string() })
                    },

                    // Timeout: Auto-release seats if not confirmed
                    delay! {
                        duration: timeout_duration,
                        action: InventoryAction::ReleaseReservation { reservation_id }
                    }
                ]
            }

            // ========== EVENT: Already happened, just update state ==========
            InventoryAction::InventoryInitialized { .. } |
            InventoryAction::SeatsReserved { .. } |
            InventoryAction::SeatsConfirmed { .. } |
            InventoryAction::SeatsReleased { .. } => {
                // Events from event store replay - just apply to state
                Self::apply_event(state, &action);
                smallvec![Effect::None]
            }

            // ========== ERROR EVENTS ==========
            InventoryAction::ValidationFailed { .. } |
            InventoryAction::SaveFailed { .. } |
            InventoryAction::PublishFailed { .. } => {
                // Already applied by the command handler
                smallvec![Effect::None]
            }

            // Other actions...
            _ => {
                Self::apply_event(state, &action);
                smallvec![Effect::None]
            }
        }
    }
}
```

### 3.2 Pattern for Events (State Updates Only)

Events that come from event store replay or event bus should only update state, not create new effects.

```rust
// Events just update state
InventoryAction::SeatsReserved { .. } => {
    Self::apply_event(state, &action);
    smallvec![Effect::None]  // No effects - event already persisted
}
```

### 3.3 Add Error Action Variants

Each aggregate needs error action variants for effect failures:

```rust
// Add to InventoryAction enum
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum InventoryAction {
    // ... existing actions ...

    /// Event save failed
    #[event]
    SaveFailed {
        error: String,
    },

    /// Event publish failed
    #[event]
    PublishFailed {
        error: String,
    },
}
```

### 3.4 Update All Reducers

**Priority order:**
1. [ ] **InventoryReducer** (src/aggregates/inventory.rs) - 2 hours
   - InitializeInventory
   - ReserveSeats (with timeout)
   - ConfirmReservation
   - ReleaseReservation

2. [ ] **ReservationReducer** (src/aggregates/reservation.rs) - 2.5 hours
   - InitiateReservation (with timeout + cross-aggregate command)
   - SeatsAllocated
   - PaymentSucceeded
   - PaymentFailed (compensation)
   - ExpireReservation (compensation)

3. [ ] **PaymentReducer** (src/aggregates/payment.rs) - 1.5 hours
   - ProcessPayment
   - RefundPayment

4. [ ] **EventReducer** (src/aggregates/event.rs) - 1.5 hours
   - CreateEvent
   - PublishEvent
   - CancelEvent

**Total: ~7.5 hours**

---

## Phase 4: Cross-Aggregate Communication via Event Bus (2 hours)

Sagas need to send commands to other aggregates.

### 4.1 Pattern: Publish Commands to `.commands` Topics

```rust
// In ReservationReducer
ReservationAction::InitiateReservation { ... } => {
    // 1. Create and apply ReservationInitiated event
    let event = ReservationAction::ReservationInitiated { ... };
    Self::apply_event(state, &event);

    // 2. Serialize reservation event
    let reservation_event_serialized = serialize_ticketing_action(
        &TicketingEvent::Reservation(event)
    )?;

    // 3. Create command for Inventory aggregate
    let inventory_command = InventoryAction::ReserveSeats {
        reservation_id,
        event_id,
        section,
        quantity,
        specific_seats,
        expires_at,
    };

    // 4. Serialize command
    let inventory_command_serialized = serialize_ticketing_action(
        &TicketingEvent::Inventory(inventory_command)
    )?;

    // 5. Return effects
    smallvec![
        // Save reservation event
        append_events! { ... },

        // Publish reservation event
        publish_event! {
            bus: env.event_bus,
            topic: "reservations",
            event: reservation_event_serialized,
            on_success: || None,
            on_error: |e| Some(ReservationAction::PublishFailed { ... })
        },

        // Publish command to inventory
        publish_event! {
            bus: env.event_bus,
            topic: "inventory.commands",  // Command topic
            event: inventory_command_serialized,
            on_success: || None,
            on_error: |e| Some(ReservationAction::CommandPublishFailed { ... })
        },

        // Timeout
        delay! {
            duration: Duration::from_secs(300),
            action: ReservationAction::ExpireReservation { reservation_id }
        }
    ]
}
```

### 4.2 Subscribe to Command Topics in Coordinator

The coordinator wires up command subscriptions so Stores receive cross-aggregate commands.

```rust
// src/app/coordinator.rs

impl TicketingApp {
    pub async fn start(&self) -> Result<(), AppError> {
        // Existing: Subscribe to event topics for projections
        self.subscribe_projection_events().await?;

        // NEW: Subscribe to command topics for cross-aggregate communication
        self.subscribe_command_topics().await?;

        Ok(())
    }

    async fn subscribe_command_topics(&self) -> Result<(), AppError> {
        // Inventory commands
        let inventory_store = self.inventory_store.clone();
        let mut inventory_cmd_stream = self.event_bus
            .subscribe(&["inventory.commands"])
            .await?;

        tokio::spawn(async move {
            while let Some(result) = inventory_cmd_stream.next().await {
                match result {
                    Ok(serialized) => {
                        match serde_json::from_slice::<TicketingEvent>(&serialized.data) {
                            Ok(TicketingEvent::Inventory(action)) => {
                                if let Err(e) = inventory_store.send(action).await {
                                    tracing::error!("Failed to send inventory command: {}", e);
                                }
                            }
                            Ok(_) => {
                                tracing::warn!("Unexpected event type on inventory.commands");
                            }
                            Err(e) => {
                                tracing::error!("Failed to deserialize: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Event bus error: {}", e);
                    }
                }
            }
        });

        // Reservation commands
        let reservation_store = self.reservation_store.clone();
        let mut reservation_cmd_stream = self.event_bus
            .subscribe(&["reservations.commands"])
            .await?;

        tokio::spawn(async move {
            while let Some(result) = reservation_cmd_stream.next().await {
                match result {
                    Ok(serialized) => {
                        match serde_json::from_slice::<TicketingEvent>(&serialized.data) {
                            Ok(TicketingEvent::Reservation(action)) => {
                                if let Err(e) = reservation_store.send(action).await {
                                    tracing::error!("Failed to send reservation command: {}", e);
                                }
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        tracing::error!("Event bus error: {}", e);
                    }
                }
            }
        });

        // Payment commands
        let payment_store = self.payment_store.clone();
        let mut payment_cmd_stream = self.event_bus
            .subscribe(&["payments.commands"])
            .await?;

        tokio::spawn(async move {
            while let Some(result) = payment_cmd_stream.next().await {
                match result {
                    Ok(serialized) => {
                        match serde_json::from_slice::<TicketingEvent>(&serialized.data) {
                            Ok(TicketingEvent::Payment(action)) => {
                                if let Err(e) = payment_store.send(action).await {
                                    tracing::error!("Failed to send payment command: {}", e);
                                }
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        tracing::error!("Event bus error: {}", e);
                    }
                }
            }
        });

        tracing::info!("✓ Command topic subscriptions active");
        Ok(())
    }
}
```

---

## Phase 5: Remove Service Layer (30 minutes)

### 5.1 Delete Service Files

```bash
rm examples/ticketing/src/app/services.rs
```

### 5.2 Update Module Exports

```rust
// src/app/mod.rs
pub mod coordinator;
// REMOVE: pub mod services;

pub use coordinator::{TicketingApp, AppError};
```

### 5.3 Update Coordinator to Use Stores

```rust
// src/app/coordinator.rs
use composable_rust_runtime::Store;
use composable_rust_core::environment::SystemClock;

pub struct TicketingApp {
    /// Inventory store
    pub inventory_store: Arc<Store<
        InventoryState,
        InventoryAction,
        InventoryEnvironment,
        InventoryReducer,
    >>,

    /// Reservation store
    pub reservation_store: Arc<Store<
        ReservationState,
        ReservationAction,
        ReservationEnvironment,
        ReservationReducer,
    >>,

    /// Payment store
    pub payment_store: Arc<Store<
        PaymentState,
        PaymentAction,
        PaymentEnvironment,
        PaymentReducer,
    >>,

    /// Event store
    pub event_store_aggregate: Arc<Store<
        EventState,
        EventAction,
        EventEnvironment,
        EventReducer,
    >>,

    // Infrastructure
    event_store: Arc<PostgresEventStore>,
    event_bus: Arc<dyn EventBus>,

    // Projections (unchanged)
    pub available_seats: Arc<RwLock<AvailableSeatsProjection>>,
    pub sales_analytics: Arc<RwLock<SalesAnalyticsProjection>>,
    pub customer_history: Arc<RwLock<CustomerHistoryProjection>>,

    config: Config,
}

impl TicketingApp {
    pub async fn new(config: Config) -> Result<Self, AppError> {
        tracing::info!("Initializing Ticketing Application...");

        // 1. Initialize PostgreSQL event store
        let pool = PgPool::connect(&config.postgres.url).await?;
        sqlx::migrate!("../../migrations").run(&pool).await?;
        let event_store = Arc::new(PostgresEventStore::from_pool(pool));

        // 2. Initialize RedPanda event bus
        let event_bus = Arc::new(
            RedpandaEventBus::builder()
                .brokers(&config.redpanda.brokers)
                .consumer_group(&config.redpanda.consumer_group)
                .build()
                .map_err(|e| AppError::EventBus(e.to_string()))?
        ) as Arc<dyn EventBus>;

        // 3. Create Stores
        let inventory_store = Arc::new(Store::new(
            InventoryState::default(),
            InventoryReducer,
            InventoryEnvironment::new(
                Arc::new(SystemClock),
                event_store.clone(),
                event_bus.clone(),
                StreamId::new("inventory"),
            ),
        ));

        let reservation_store = Arc::new(Store::new(
            ReservationState::default(),
            ReservationReducer,
            ReservationEnvironment::new(
                Arc::new(SystemClock),
                event_store.clone(),
                event_bus.clone(),
                StreamId::new("reservations"),
            ),
        ));

        let payment_store = Arc::new(Store::new(
            PaymentState::default(),
            PaymentReducer,
            PaymentEnvironment::new(
                Arc::new(SystemClock),
                event_store.clone(),
                event_bus.clone(),
                StreamId::new("payments"),
            ),
        ));

        let event_store_aggregate = Arc::new(Store::new(
            EventState::default(),
            EventReducer,
            EventEnvironment::new(
                Arc::new(SystemClock),
                event_store.clone(),
                event_bus.clone(),
                StreamId::new("events"),
            ),
        ));

        // 4. Initialize projections
        let available_seats = Arc::new(RwLock::new(AvailableSeatsProjection::new()));
        let sales_analytics = Arc::new(RwLock::new(SalesAnalyticsProjection::new()));
        let customer_history = Arc::new(RwLock::new(CustomerHistoryProjection::new()));

        tracing::info!("✓ Ticketing application initialized");

        Ok(Self {
            inventory_store,
            reservation_store,
            payment_store,
            event_store_aggregate,
            event_store,
            event_bus,
            available_seats,
            sales_analytics,
            customer_history,
            config,
        })
    }
}
```

---

## Phase 6: Update HTTP Handlers to Use Store (2-3 hours)

### 6.1 Update AppState

```rust
// src/server/state.rs
use composable_rust_runtime::Store;

#[derive(Clone)]
pub struct AppState {
    pub inventory_store: Arc<Store<InventoryState, InventoryAction, InventoryEnvironment, InventoryReducer>>,
    pub reservation_store: Arc<Store<ReservationState, ReservationAction, ReservationEnvironment, ReservationReducer>>,
    pub payment_store: Arc<Store<PaymentState, PaymentAction, PaymentEnvironment, PaymentReducer>>,
    pub event_store: Arc<Store<EventState, EventAction, EventEnvironment, EventReducer>>,

    // Projections for queries
    pub available_seats: Arc<RwLock<AvailableSeatsProjection>>,
    pub sales_analytics: Arc<RwLock<SalesAnalyticsProjection>>,
    pub customer_history: Arc<RwLock<CustomerHistoryProjection>>,

    pub event_bus: Arc<dyn EventBus>,
}

impl AppState {
    pub fn from_app(app: &TicketingApp) -> Self {
        Self {
            inventory_store: app.inventory_store.clone(),
            reservation_store: app.reservation_store.clone(),
            payment_store: app.payment_store.clone(),
            event_store: app.event_store_aggregate.clone(),
            available_seats: app.available_seats.clone(),
            sales_analytics: app.sales_analytics.clone(),
            customer_history: app.customer_history.clone(),
            event_bus: app.event_bus.clone(),
        }
    }
}
```

### 6.2 Update API Handlers (Fire-and-Forget Pattern)

For operations that don't need immediate feedback:

```rust
// src/api/reservations.rs

/// Create a new reservation
pub async fn create_reservation(
    State(state): State<AppState>,
    session: AuthSession,
    Json(request): Json<CreateReservationRequest>,
) -> Result<Json<ReservationResponse>, AppError> {
    // 1. Validate request
    if request.quantity == 0 || request.quantity > 8 {
        return Err(AppError::validation("Quantity must be between 1 and 8"));
    }

    // 2. Generate IDs
    let reservation_id = ReservationId::new();
    let event_id = EventId::from_uuid(request.event_id);
    let customer_id = session.user_id;

    // 3. Create command
    let command = ReservationAction::InitiateReservation {
        reservation_id,
        event_id,
        customer_id,
        section: request.section.clone(),
        quantity: request.quantity,
        specific_seats: request.specific_seats.clone(),
    };

    // 4. Send to Store (fire-and-forget)
    state.reservation_store.send(command).await
        .map_err(|e| AppError::internal(format!("Failed to send command: {}", e)))?;

    // 5. Return response immediately
    Ok(Json(ReservationResponse {
        reservation_id: reservation_id.as_uuid(),
        status: "initiated".to_string(),
        message: "Reservation initiated successfully".to_string(),
    }))
}
```

### 6.3 Update Handlers (Request-Response Pattern)

For operations that need to wait for completion:

```rust
// src/api/reservations.rs

/// Create a reservation and wait for confirmation or failure
pub async fn create_reservation(
    State(state): State<AppState>,
    session: AuthSession,
    Json(request): Json<CreateReservationRequest>,
) -> Result<Json<ReservationResponse>, AppError> {
    let reservation_id = ReservationId::new();
    let event_id = EventId::from_uuid(request.event_id);
    let customer_id = session.user_id;

    let command = ReservationAction::InitiateReservation {
        reservation_id,
        event_id,
        customer_id,
        section: request.section.clone(),
        quantity: request.quantity,
        specific_seats: request.specific_seats.clone(),
    };

    // Use Store's send_and_wait_for (already exists!)
    let result = state.reservation_store
        .send_and_wait_for(
            command,
            |action| matches!(action,
                ReservationAction::ReservationInitiated { reservation_id: id, .. } if *id == reservation_id ||
                ReservationAction::ValidationFailed { .. }
            ),
            Duration::from_secs(10),
        )
        .await
        .map_err(|e| AppError::internal(format!("Request timeout: {}", e)))?;

    // Handle result
    match result {
        ReservationAction::ReservationInitiated { .. } => {
            Ok(Json(ReservationResponse {
                reservation_id: reservation_id.as_uuid(),
                status: "initiated".to_string(),
                message: "Reservation initiated successfully".to_string(),
            }))
        }
        ReservationAction::ValidationFailed { error } => {
            Err(AppError::validation(error))
        }
        _ => Err(AppError::internal("Unexpected action received")),
    }
}
```

### 6.4 Update All API Files

- [ ] `src/api/reservations.rs` - Use `reservation_store.send()` or `send_and_wait_for()`
- [ ] `src/api/payments.rs` - Use `payment_store.send()` or `send_and_wait_for()`
- [ ] `src/api/events.rs` - Use `event_store.send()` or `send_and_wait_for()`
- [ ] `src/api/availability.rs` - Already uses projections (no change)
- [ ] `src/api/analytics.rs` - Already uses projections (no change)

**Estimated time:** 1 hour per file × 3 = 3 hours

---

## Phase 7: State Reconstruction (2 hours)

### 7.1 Problem

Stores start with `State::default()`, but we need to reconstruct from event store on startup.

### 7.2 Solution: On-Demand Store Creation

For aggregates with many instances (one per event), use a factory pattern:

```rust
// src/app/store_factory.rs

use composable_rust_runtime::Store;
use composable_rust_core::stream::StreamId;

pub struct StoreFactory {
    event_store: Arc<PostgresEventStore>,
    event_bus: Arc<dyn EventBus>,
}

impl StoreFactory {
    pub fn new(
        event_store: Arc<PostgresEventStore>,
        event_bus: Arc<dyn EventBus>,
    ) -> Self {
        Self {
            event_store,
            event_bus,
        }
    }

    /// Create an inventory store for a specific event
    pub async fn create_inventory_store(
        &self,
        event_id: EventId,
    ) -> Result<Arc<Store<InventoryState, InventoryAction, InventoryEnvironment, InventoryReducer>>, String> {
        let stream_id = StreamId::new(&format!("inventory-{}", event_id));

        // 1. Load events from event store
        let events = self.event_store
            .load_events(stream_id.clone(), None)
            .await
            .map_err(|e| format!("Failed to load events: {}", e))?;

        // 2. Reconstruct state
        let mut state = InventoryState::default();
        let reducer = InventoryReducer;

        for event in events {
            let ticketing_event: TicketingEvent = serde_json::from_slice(&event.data)
                .map_err(|e| format!("Failed to deserialize: {}", e))?;

            if let TicketingEvent::Inventory(inventory_action) = ticketing_event {
                // Apply event to state (effects ignored during reconstruction)
                let env = InventoryEnvironment::new(
                    Arc::new(SystemClock),
                    self.event_store.clone(),
                    self.event_bus.clone(),
                    stream_id.clone(),
                );
                reducer.reduce(&mut state, inventory_action, &env);
            }
        }

        // 3. Create store with reconstructed state
        let store = Arc::new(Store::new(
            state,
            reducer,
            InventoryEnvironment::new(
                Arc::new(SystemClock),
                self.event_store.clone(),
                self.event_bus.clone(),
                stream_id,
            ),
        ));

        Ok(store)
    }

    // Similar for other aggregates
}
```

### 7.3 Alternative: Single Store Per Aggregate Type

For global aggregates (one instance), reconstruct in coordinator:

```rust
// In TicketingApp::new()

// Reconstruct global states
let reservation_events = event_store.load_events(StreamId::new("reservations"), None).await?;
let mut reservation_state = ReservationState::default();
for event in reservation_events {
    // Apply events to rebuild state
    // ...
}

let reservation_store = Arc::new(Store::new(
    reservation_state,  // ✅ Reconstructed state
    ReservationReducer,
    ReservationEnvironment::new(...),
));
```

**Decision:** Use global stores for Reservation/Payment (one instance), and optionally add factory for per-event inventory.

---

## Phase 8: Testing & Validation (3-4 hours)

### 8.1 Unit Tests

Existing reducer tests should mostly work. Update to assert effects:

```rust
#[test]
fn test_reserve_seats_returns_effects() {
    let env = create_test_env();

    ReducerTest::new(InventoryReducer)
        .with_env(env)
        .given_state(initialized_state())
        .when_action(InventoryAction::ReserveSeats {
            reservation_id: ReservationId::new(),
            event_id: EventId::new(),
            section: "VIP".to_string(),
            quantity: 2,
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .then_state(|state| {
            // Assert state changes
            assert_eq!(state.reserved_count, 2);
        })
        .then_effects(|effects| {
            // NOW we can assert effects!
            assert_eq!(effects.len(), 3); // Save + Publish + Delay
            assert!(matches!(effects[0], Effect::EventStore(_)));
            assert!(matches!(effects[1], Effect::PublishEvent(_)));
            assert!(matches!(effects[2], Effect::Delay { .. }));
        })
        .run();
}
```

### 8.2 Integration Tests

Update to use Store instead of services:

```rust
// tests/full_deployment_test.rs

#[tokio::test]
async fn test_full_reservation_flow() {
    // Setup
    let event_store = create_event_store().await;
    let event_bus = create_event_bus().await;

    let reservation_store = Arc::new(Store::new(
        ReservationState::default(),
        ReservationReducer,
        ReservationEnvironment::new(
            Arc::new(SystemClock),
            event_store.clone(),
            event_bus.clone(),
            StreamId::new("test-reservations"),
        ),
    ));

    // Send command
    let reservation_id = ReservationId::new();
    reservation_store.send(ReservationAction::InitiateReservation {
        reservation_id,
        event_id: EventId::new(),
        customer_id: CustomerId::new(),
        section: "VIP".to_string(),
        quantity: 2,
        specific_seats: None,
    }).await.expect("Failed to send");

    // Wait for effects to complete
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Assert via event store
    let events = event_store.load_events(StreamId::new("test-reservations"), None).await.unwrap();
    assert!(!events.is_empty());

    // Assert via projections
    // ...
}
```

### 8.3 End-to-End Test Scenarios

**Scenario 1: Happy Path**
```
1. POST /api/reservations → InitiateReservation
2. Store executes effects:
   - Save ReservationInitiated event
   - Publish to "reservations" topic
   - Publish InventoryAction::ReserveSeats to "inventory.commands"
   - Schedule timeout
3. Inventory store receives command from event bus
4. Inventory reducer returns effects:
   - Save SeatsReserved event
   - Publish to "inventory" topic
5. Projections update
6. Query GET /api/events/{id}/availability
7. Assert seats reserved
```

**Scenario 2: Timeout**
```
1. Create reservation (5-minute timeout)
2. Wait 5+ minutes
3. Effect::Delay fires ExpireReservation
4. Reservation reducer compensates
5. Seats released
```

### 8.4 Manual Testing Checklist

- [ ] Create reservation via API
- [ ] Check PostgreSQL for events
- [ ] Check RedPanda for published events
- [ ] Wait for timeout and verify compensation
- [ ] Query projections
- [ ] Test payment flow
- [ ] Test saga compensation

---

## Phase 9: Documentation Updates (1 hour)

### 9.1 Update README

```markdown
# Ticketing System Architecture

## Effect-Driven Architecture

The ticketing system follows the Composable Rust architecture where:

1. **HTTP handlers** receive requests
2. **Handlers** create command actions and send to **Store**
3. **Store** calls **Reducer** with (state, action, environment)
4. **Reducer** returns **Effects** (descriptions of side effects)
5. **Store executes effects**:
   - `Effect::EventStore` → PostgreSQL persistence
   - `Effect::PublishEvent` → RedPanda event bus
   - `Effect::Delay` → Tokio timers
   - `Effect::Future` → Async operations
6. **Effects produce new actions** (feedback loop)
7. **Store sends actions back to reducer**

## No Service Layer

Unlike traditional architectures, there is **no service layer**. The Store IS the runtime that executes effects. This keeps business logic (reducers) pure and testable.
```

### 9.2 Add Architecture Diagram

Create `docs/ARCHITECTURE.md` with diagrams showing the flow.

---

## Migration Checklist

### Pre-Migration
- [ ] Review current codebase state
- [ ] Create feature branch: `git checkout -b fix/restore-composable-architecture`
- [ ] Run current tests to establish baseline

### Phase 1: Update Environments (1h)
- [ ] Update `InventoryEnvironment` with event_store, event_bus, stream_id
- [ ] Update `ReservationEnvironment`
- [ ] Update `PaymentEnvironment`
- [ ] Update `EventEnvironment`
- [ ] Update test environments

### Phase 2: Serialization Helper (30m)
- [ ] Create `src/serialization.rs`
- [ ] Implement `serialize_ticketing_action()`
- [ ] Export from `src/lib.rs`
- [ ] Write tests

### Phase 3: Update Reducers (7.5h)
- [ ] Add error action variants (SaveFailed, PublishFailed)
- [ ] Update `InventoryReducer` to return effects
- [ ] Update `ReservationReducer` to return effects
- [ ] Update `PaymentReducer` to return effects
- [ ] Update `EventReducer` to return effects
- [ ] Update unit tests to assert effects

### Phase 4: Cross-Aggregate Communication (2h)
- [ ] Update saga reducers to publish commands
- [ ] Implement command topic subscriptions in coordinator
- [ ] Test cross-aggregate flow

### Phase 5: Remove Service Layer (30m)
- [ ] Delete `src/app/services.rs`
- [ ] Update `src/app/mod.rs`
- [ ] Update `TicketingApp` to use Stores
- [ ] Remove `#[allow(dead_code)]` on event_store

### Phase 6: Update HTTP Handlers (3h)
- [ ] Update `AppState` type
- [ ] Update `src/api/reservations.rs`
- [ ] Update `src/api/payments.rs`
- [ ] Update `src/api/events.rs`
- [ ] Choose fire-and-forget vs send_and_wait_for per endpoint

### Phase 7: State Reconstruction (2h)
- [ ] Implement state reconstruction helpers
- [ ] Update coordinator to reconstruct states on startup
- [ ] Or implement StoreFactory for on-demand creation
- [ ] Test state reconstruction

### Phase 8: Testing (4h)
- [ ] Run all unit tests
- [ ] Update integration tests
- [ ] Manual end-to-end testing
- [ ] Test timeout scenarios
- [ ] Test saga compensation

### Phase 9: Documentation (1h)
- [ ] Update README
- [ ] Create architecture docs
- [ ] Document migration

### Phase 10: Cleanup
- [ ] Run `cargo clippy --all-targets`
- [ ] Run `cargo fmt`
- [ ] Remove TODOs that are now completed
- [ ] Final review

---

## Rollback Plan

If migration fails:

```bash
# Discard changes
git checkout main
git branch -D fix/restore-composable-architecture

# Or archive for reference
git checkout main
git branch archive/architecture-fix-attempt fix/restore-composable-architecture
```

---

## Success Criteria

✅ **Architecture**
- [ ] No service layer exists
- [ ] HTTP handlers use Store directly
- [ ] Reducers return `Effect::EventStore`, `Effect::PublishEvent`, `Effect::Delay`
- [ ] Store executes all effect types
- [ ] No `let _effects = ...` (effects are used, not ignored)

✅ **Functionality**
- [ ] Events persisted to PostgreSQL via `Effect::EventStore`
- [ ] Events published to RedPanda via `Effect::PublishEvent`
- [ ] Projections update correctly
- [ ] Timeouts work via `Effect::Delay`
- [ ] Saga coordination works via command topics
- [ ] Compensation flows work

✅ **Testing**
- [ ] All unit tests pass
- [ ] All integration tests pass
- [ ] Manual testing successful
- [ ] Timeout scenarios tested

✅ **Code Quality**
- [ ] 0 clippy warnings (`cargo clippy --all-targets`)
- [ ] Code formatted (`cargo fmt`)
- [ ] Documentation updated
- [ ] No TODOs for completed items

---

## Timeline

| Phase | Description | Estimated Time | Dependencies |
|-------|-------------|----------------|--------------|
| 1 | Update environments | 1 hour | None |
| 2 | Serialization helper | 30 minutes | Phase 1 |
| 3 | Update reducers | 7.5 hours | Phase 1, 2 |
| 4 | Cross-aggregate communication | 2 hours | Phase 3 |
| 5 | Remove service layer | 30 minutes | Phase 3, 4 |
| 6 | Update HTTP handlers | 3 hours | Phase 5 |
| 7 | State reconstruction | 2 hours | Phase 5 |
| 8 | Testing | 4 hours | All previous |
| 9 | Documentation | 1 hour | Phase 8 |
| 10 | Cleanup | 30 minutes | Phase 9 |

**Total: 22 hours (~3 days)**

**Critical path: Phase 1 → 2 → 3 → 5 → 6 → 8**

---

## Key Differences from Original Plan

### Removed
- ❌ "Phase 1: Define Core Effect Types" - Framework already has them
- ❌ Custom effect helper functions - Using built-in macros instead
- ❌ Manual effect execution in services - Store already does this

### Simplified
- ✅ Reducers use `append_events!`, `publish_event!`, `delay!` macros directly
- ✅ Store already has `send_and_wait_for()` for request-response
- ✅ No custom effect executor needed - Store runtime handles everything

### Leveraged from Framework
- ✅ `Effect::EventStore` with callbacks (on_success, on_error)
- ✅ `Effect::PublishEvent` with callbacks
- ✅ `Effect::Delay` for timeouts
- ✅ `Store::send_and_wait_for()` for HTTP request-response
- ✅ Effect execution infrastructure in runtime crate

---

**Status:** Ready for execution
**Owner:** TBD
**Target Completion:** TBD
