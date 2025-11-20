# Query Actions Refactor - Architectural Consistency Fix

## ❌ CRITICAL ARCHITECTURAL VIOLATION

**Problem**: API handlers are directly calling projection methods, **bypassing the store/reducer pattern**.

**Impact**:
- ❌ No testability via mock injection
- ❌ Cannot add business rules to queries
- ❌ Inconsistent with write operations
- ❌ Not future-proof for changing requirements

**Root Cause**: Query operations implemented with direct projection calls instead of query actions flowing through reducers.

---

## 📋 Complete Audit Results

### ✅ CORRECT IMPLEMENTATION (Reference Pattern)

**Reservation API** (`src/api/reservations.rs:254-295`)
- ✅ Uses `GetReservation` query action
- ✅ Has `ReservationQueried` result event
- ✅ Reducer handles query (line 872-880 in `src/aggregates/reservation.rs`)
- ✅ API uses `store.send_and_wait_for()` pattern

**THIS IS THE REFERENCE IMPLEMENTATION TO FOLLOW FOR ALL OTHER AGGREGATES**

---

### ❌ VIOLATIONS FOUND

#### 1. Event API (`src/api/events.rs`)
**5 direct projection calls:**
- Line 242: `state.events_projection.get(&event_id)` in `get_event()`
- Line 291: `state.events_projection.list(status_str)` in `list_events()`
- Line 347: `state.events_projection.get(&event_id)` in `update_event()` (pre-check)
- Line 389: `state.events_projection.get(&event_id)` in `update_event()` (verification)
- Line 427: `state.events_projection.get(&event_id)` in `delete_event()`

**Missing Query Actions:**
- `GetEvent { event_id }` → `EventQueried { event: Option<Event> }`
- `ListEvents { status_filter }` → `EventsListed { events: Vec<Event> }`

---

#### 2. Payment API (`src/api/payments.rs`)
**3 direct projection calls:**
- Line 322: `state.payments_projection.get_payment(&PaymentId::from_uuid(payment_id))` in `get_payment()`
- Line 408: `state.payments_projection.get_payment(&PaymentId::from_uuid(payment_id))` in `refund_payment()` (pre-check)
- Line 530: `state.payments_projection.list_customer_payments(&customer_id_typed)` in `list_user_payments()`

**Missing Query Actions:**
- `GetPayment { payment_id }` → `PaymentQueried { payment: Option<Payment> }`
- `ListCustomerPayments { customer_id }` → `CustomerPaymentsListed { payments: Vec<Payment> }`

---

#### 3. Availability API (`src/api/availability.rs`)
**2 direct projection calls:**
- Line 92: `state.available_seats_projection.get_all_sections(&event_id)` in `get_event_availability()`
- Line 150: `state.available_seats_projection.get_availability(&event_id, &section)` in `get_section_availability()`

**Missing Query Actions:**
- `GetAllSections { event_id }` → `AllSectionsQueried { sections: Vec<SectionAvailability> }`
- `GetSectionAvailability { event_id, section }` → `SectionAvailabilityQueried { availability: Option<SectionAvailability> }`

**Note**: These should be added to **Inventory aggregate** (availability is inventory state)

---

#### 4. WebSocket API (`src/api/websocket.rs`)
**2 direct projection calls:**
- Line 305: `state.available_seats_projection.get_all_sections(&event_id_typed)` in `handle_availability_socket()`
- Line 356: `event_projection.get_all_sections(&event_id_typed)` in WebSocket event stream

**Special Case**: WebSocket handlers are **streaming real-time updates**, but initial snapshot should still use query actions for consistency.

---

#### 5. Analytics API (`src/api/analytics.rs`)
**5 direct projection calls:**
- Lines 225-232: `state.sales_analytics_projection.read()` in `get_event_sales()`
- Lines 295-298: `state.sales_analytics_projection.read()` in `get_popular_sections()`
- Lines 376-379: `state.sales_analytics_projection.read()` in `get_total_revenue()`
- Lines 445-448: `state.customer_history_projection.read()` in `get_top_spenders()`
- Lines 525-528: `state.customer_history_projection.read()` in `get_customer_profile()`

**Special Case**: Analytics projections are **in-memory** (`Arc<RwLock<T>>`), not PostgreSQL-backed.

**Missing Query Actions:**
- `GetEventSales { event_id }` → `EventSalesQueried { metrics: Option<EventSalesMetrics> }`
- `GetPopularSections { event_id }` → `PopularSectionsQueried { ... }`
- `GetTotalRevenue` → `TotalRevenueQueried { ... }`
- `GetTopSpenders { limit }` → `TopSpendersQueried { ... }`
- `GetCustomerProfile { customer_id }` → `CustomerProfileQueried { ... }`

**Note**: Analytics queries may need a **new Analytics aggregate** or be added to existing aggregates.

---

## 🎯 Reference Implementation Pattern

From **Reservation API** (`src/api/reservations.rs:254-295`):

```rust
pub async fn get_reservation(
    ownership: RequireOwnership<ReservationId>,
    Path(reservation_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<ReservationResponse>, AppError> {
    let reservation_id_typed = ReservationId::from_uuid(reservation_id);

    // ✅ Create per-request store
    let reservation_store = state.create_reservation_store();

    // ✅ Send query action
    let action = ReservationAction::GetReservation {
        reservation_id: reservation_id_typed,
    };

    // ✅ Wait for query result event
    let result_action = reservation_store
        .send_and_wait_for(
            action,
            |a| matches!(a, ReservationAction::ReservationQueried { .. }),
            std::time::Duration::from_secs(5),
        )
        .await?;

    // ✅ Extract result from event
    let reservation = match result_action {
        ReservationAction::ReservationQueried { reservation: Some(r), .. } => r,
        ReservationAction::ReservationQueried { reservation: None, .. } => {
            return Err(AppError::not_found("Reservation", reservation_id));
        }
        _ => return Err(AppError::internal("Unexpected action returned")),
    };

    // Convert to response...
}
```

From **Reservation Aggregate** (`src/aggregates/reservation.rs:872-880`):

```rust
ReservationAction::GetReservation { reservation_id } => {
    let projection = env.projection.clone();
    smallvec![Effect::Future(Box::pin(async move {
        match projection.load_reservation(&reservation_id).await {
            Ok(reservation) => Some(ReservationAction::ReservationQueried {
                reservation_id,
                reservation,
            }),
            Err(e) => Some(ReservationAction::ValidationFailed {
                reason: format!("Failed to load reservation: {e}"),
            }),
        }
    }))]
}
```

---

## 📝 Step-by-Step Refactoring Plan

### Phase 1: Event Aggregate (Highest Priority - Most Violations)

#### Step 1.1: Add Query Actions to Event Aggregate
**File**: `src/aggregates/event.rs`

**Add to EventAction enum**:
```rust
/// Query a single event by ID
#[command]
GetEvent {
    event_id: EventId,
},

/// Query events with optional status filter
#[command]
ListEvents {
    status_filter: Option<EventStatus>,
},

/// Event was queried (query result)
#[event]
EventQueried {
    event_id: EventId,
    event: Option<Event>,
},

/// Events were listed (query result)
#[event]
EventsListed {
    events: Vec<Event>,
    status_filter: Option<EventStatus>,
},
```

#### Step 1.2: Add Projection Query Trait to Event Environment
**File**: `src/aggregates/event.rs`

**Add trait**:
```rust
/// Event projection query trait for loading event state.
#[async_trait::async_trait]
pub trait EventProjectionQuery: Send + Sync {
    /// Load a single event by ID
    async fn load_event(&self, event_id: &EventId) -> Result<Option<Event>, String>;

    /// Load events with optional status filter
    async fn load_events(&self, status_filter: Option<EventStatus>) -> Result<Vec<Event>, String>;
}
```

**Update EventEnvironment**:
```rust
pub struct EventEnvironment {
    pub clock: Arc<dyn Clock>,
    pub event_store: Arc<dyn EventStore>,
    pub event_bus: Arc<dyn EventBus>,
    pub stream_id: StreamId,
    pub projection: Arc<dyn EventProjectionQuery>,  // NEW - dynamic dispatch like all other deps
}

impl EventEnvironment {
    /// Creates a new `EventEnvironment`
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        event_store: Arc<dyn EventStore>,
        event_bus: Arc<dyn EventBus>,
        stream_id: StreamId,
        projection: Arc<dyn EventProjectionQuery>,  // NEW parameter
    ) -> Self {
        Self {
            clock,
            event_store,
            event_bus,
            stream_id,
            projection,  // NEW field
        }
    }
}
```

#### Step 1.3: Implement Query Trait for PostgresEventsProjection
**File**: `src/projections/events_postgres.rs`

```rust
#[async_trait::async_trait]
impl EventProjectionQuery for PostgresEventsProjection {
    async fn load_event(&self, event_id: &EventId) -> Result<Option<Event>, String> {
        self.get(event_id.as_uuid())
            .await
            .map_err(|e| format!("Failed to load event: {e}"))
    }

    async fn load_events(&self, status_filter: Option<EventStatus>) -> Result<Vec<Event>, String> {
        let status_str = status_filter.as_ref().map(|s| s.as_str());
        self.list(status_str)
            .await
            .map_err(|e| format!("Failed to load events: {e}"))
    }
}
```

#### Step 1.4: Add Reducer Handlers
**File**: `src/aggregates/event.rs`

```rust
EventAction::GetEvent { event_id } => {
    let projection = env.projection.clone();
    smallvec![Effect::Future(Box::pin(async move {
        match projection.load_event(&event_id).await {
            Ok(event) => Some(EventAction::EventQueried {
                event_id,
                event,
            }),
            Err(e) => Some(EventAction::ValidationFailed {
                error: format!("Failed to load event: {e}"),
            }),
        }
    }))]
}

EventAction::ListEvents { status_filter } => {
    let projection = env.projection.clone();
    smallvec![Effect::Future(Box::pin(async move {
        match projection.load_events(status_filter).await {
            Ok(events) => Some(EventAction::EventsListed {
                events,
                status_filter,
            }),
            Err(e) => Some(EventAction::ValidationFailed {
                error: format!("Failed to load events: {e}"),
            }),
        }
    }))]
}

EventAction::EventQueried { .. } | EventAction::EventsListed { .. } => {
    // Query results don't produce effects
    smallvec![]
}
```

#### Step 1.5: Update AppState to Create Event Store with Projection
**File**: `src/server/state.rs`

**Add field**:
```rust
pub struct AppState {
    // ... existing fields ...

    /// Events projection for querying event data (PostgreSQL-backed)
    pub events_projection: Arc<PostgresEventsProjection>,
}
```

**Add method**:
```rust
pub fn create_event_store(
    &self,
) -> composable_rust_runtime::Store<
    crate::types::EventState,
    crate::aggregates::event::EventAction,
    crate::aggregates::event::EventEnvironment,  // ✅ No generics!
    crate::aggregates::event::EventReducer,
> {
    use crate::aggregates::event::{EventEnvironment, EventReducer};
    use crate::types::EventState;
    use composable_rust_core::stream::StreamId;
    use composable_rust_runtime::Store;

    let env = EventEnvironment::new(
        self.clock.clone(),
        self.event_store.clone(),
        self.event_bus.clone(),
        StreamId::new("event"),
        self.events_projection.clone(),  // Arc<dyn EventProjectionQuery>
    );

    Store::new(EventState::new(), EventReducer::new(), env)
}
```

#### Step 1.6: Update Event API Handlers
**File**: `src/api/events.rs`

**Replace `get_event()` handler** (line 240-244):
```rust
pub async fn get_event(
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<EventResponse>, AppError> {
    let event_id_typed = EventId::from_uuid(event_id);

    // ✅ Create per-request store
    let event_store = state.create_event_store();

    // ✅ Send query action
    let action = EventAction::GetEvent {
        event_id: event_id_typed,
    };

    // ✅ Wait for query result event
    let result_action = event_store
        .send_and_wait_for(
            action,
            |a| matches!(a, EventAction::EventQueried { .. }),
            std::time::Duration::from_secs(5),
        )
        .await?;

    // ✅ Extract result from event
    let event = match result_action {
        EventAction::EventQueried { event: Some(e), .. } => e,
        EventAction::EventQueried { event: None, .. } => {
            return Err(AppError::not_found("Event", event_id));
        }
        _ => return Err(AppError::internal("Unexpected action returned")),
    };

    // Convert to response...
    Ok(Json(EventResponse {
        id: *event.id.as_uuid(),
        name: event.name,
        venue: event.venue,
        date: event.date,
        pricing_tiers: event.pricing_tiers,
        status: event.status.as_str().to_string(),
        created_at: event.created_at,
        owner_id: event.owner_id.0,
    }))
}
```

**Replace `list_events()` handler** (line 289-293):
```rust
pub async fn list_events(
    Query(params): Query<ListEventsQuery>,
    State(state): State<AppState>,
) -> Result<Json<ListEventsResponse>, AppError> {
    let event_store = state.create_event_store();

    let action = EventAction::ListEvents {
        status_filter: params.status,
    };

    let result_action = event_store
        .send_and_wait_for(
            action,
            |a| matches!(a, EventAction::EventsListed { .. }),
            std::time::Duration::from_secs(5),
        )
        .await?;

    let events = match result_action {
        EventAction::EventsListed { events, .. } => events,
        _ => return Err(AppError::internal("Unexpected action returned")),
    };

    // Apply pagination...
    let total = events.len();
    let start = params.page * params.page_size;
    let end = (start + params.page_size).min(total);
    let page_events = if start < total {
        events[start..end].to_vec()
    } else {
        Vec::new()
    };

    // Convert to response...
}
```

**Update other handlers** (lines 347, 389, 427) to use query actions for pre-checks.

#### Step 1.7: Add Tests
**File**: `src/aggregates/event.rs` (tests module)

```rust
#[tokio::test]
async fn test_get_event_query() {
    let mut env = create_test_environment();
    let mut state = EventState::new();
    let reducer = EventReducer::new();

    // Setup: Create event first
    let event_id = EventId::new();
    create_test_event(&mut state, &reducer, &env, event_id);

    // Execute: Query event
    let effects = reducer.reduce(
        &mut state,
        EventAction::GetEvent { event_id },
        &env,
    );

    // Execute effect
    let result = execute_effect(&effects[0]).await;

    // Verify
    assert!(matches!(
        result,
        Some(EventAction::EventQueried { event: Some(_), .. })
    ));
}
```

---

### Phase 2: Payment Aggregate

Follow same pattern as Event aggregate:
1. Add `GetPayment`, `ListCustomerPayments` query actions
2. Add `PaymentQueried`, `CustomerPaymentsListed` result events
3. Add `PaymentProjectionQuery` trait
4. Implement trait for `PostgresPaymentsProjection`
5. Update `PaymentEnvironment` to include projection
6. Add reducer handlers
7. Update `AppState::create_payment_store()` to pass projection
8. Update API handlers in `src/api/payments.rs` (lines 322, 408, 530)
9. Add tests

---

### Phase 3: Inventory Aggregate (for Availability API)

Follow same pattern:
1. Add `GetAllSections`, `GetSectionAvailability` query actions
2. Add `AllSectionsQueried`, `SectionAvailabilityQueried` result events
3. Add `InventoryProjectionQuery` trait
4. Implement trait for `PostgresAvailableSeatsProjection`
5. Update `InventoryEnvironment` to include projection
6. Add reducer handlers
7. Update `AppState::create_inventory_store()` to pass projection
8. Update API handlers in `src/api/availability.rs` (lines 92, 150)
9. Update WebSocket initial snapshot (line 305) to use query action
10. Add tests

---

### Phase 4: Analytics Aggregate (NEW - Option A Implementation)

**DECISION**: Create dedicated Analytics aggregate for architectural consistency.

Analytics is special because it:
- Queries **two in-memory projections** (`SalesAnalyticsProjection`, `CustomerHistoryProjection`)
- Spans multiple concerns (revenue, customer behavior)
- Is **read-only** (no state mutations)

#### Step 4.1: Create Analytics Aggregate
**File**: `src/aggregates/analytics.rs` (NEW)

**Create Action enum**:
```rust
use crate::types::{CustomerId, EventId, EventSalesMetrics, CustomerProfile};

/// Analytics query actions and results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AnalyticsAction {
    // Query commands
    #[command]
    GetEventSales { event_id: EventId },

    #[command]
    GetPopularSections { event_id: EventId },

    #[command]
    GetTotalRevenue,

    #[command]
    GetTopSpenders { limit: usize },

    #[command]
    GetCustomerProfile { customer_id: CustomerId },

    // Query result events
    #[event]
    EventSalesQueried {
        event_id: EventId,
        metrics: Option<EventSalesMetrics>,
    },

    #[event]
    PopularSectionsQueried {
        event_id: EventId,
        most_popular: Option<(String, u32)>,
        highest_revenue: Option<(String, Money)>,
    },

    #[event]
    TotalRevenueQueried {
        total_revenue: Money,
        total_tickets: u32,
        events_with_sales: usize,
    },

    #[event]
    TopSpendersQueried {
        customers: Vec<CustomerProfile>,
        total_customers: usize,
    },

    #[event]
    CustomerProfileQueried {
        customer_id: CustomerId,
        profile: Option<CustomerProfile>,
    },

    #[event]
    ValidationFailed { error: String },
}
```

**Create State** (read-only, always empty):
```rust
#[derive(Debug, Clone, Default)]
pub struct AnalyticsState {
    // Analytics is stateless - all queries go to projections
}

impl AnalyticsState {
    pub fn new() -> Self {
        Self::default()
    }
}
```

#### Step 4.2: Create Analytics Projection Query Trait
**File**: `src/aggregates/analytics.rs`

```rust
/// Analytics projection query trait for loading analytics data.
#[async_trait::async_trait]
pub trait AnalyticsProjectionQuery: Send + Sync {
    /// Load sales metrics for an event
    async fn load_event_sales(&self, event_id: &EventId) -> Result<Option<EventSalesMetrics>, String>;

    /// Load most popular section by ticket count
    async fn load_most_popular_section(&self, event_id: &EventId) -> Result<Option<(String, u32)>, String>;

    /// Load highest revenue section
    async fn load_highest_revenue_section(&self, event_id: &EventId) -> Result<Option<(String, Money)>, String>;

    /// Load total revenue across all events
    async fn load_total_revenue(&self) -> Result<(Money, u32, usize), String>;

    /// Load top spending customers
    async fn load_top_spenders(&self, limit: usize) -> Result<(Vec<CustomerProfile>, usize), String>;

    /// Load customer profile
    async fn load_customer_profile(&self, customer_id: &CustomerId) -> Result<Option<CustomerProfile>, String>;
}
```

**Create Environment**:
```rust
pub struct AnalyticsEnvironment {
    pub clock: Arc<dyn Clock>,
    pub projection: Arc<dyn AnalyticsProjectionQuery>,  // ✅ Dynamic dispatch
}

impl AnalyticsEnvironment {
    pub fn new(clock: Arc<dyn Clock>, projection: Arc<dyn AnalyticsProjectionQuery>) -> Self {
        Self { clock, projection }
    }
}
```

#### Step 4.3: Create Analytics Projection Adapter
**File**: `src/projections/analytics_adapter.rs` (NEW)

This adapter wraps both in-memory projections:
```rust
use crate::projections::{SalesAnalyticsProjection, CustomerHistoryProjection};
use crate::aggregates::analytics::AnalyticsProjectionQuery;
use std::sync::{Arc, RwLock};

/// Adapter that wraps both analytics projections.
pub struct AnalyticsProjectionAdapter {
    sales: Arc<RwLock<SalesAnalyticsProjection>>,
    customers: Arc<RwLock<CustomerHistoryProjection>>,
}

impl AnalyticsProjectionAdapter {
    pub fn new(
        sales: Arc<RwLock<SalesAnalyticsProjection>>,
        customers: Arc<RwLock<CustomerHistoryProjection>>,
    ) -> Self {
        Self { sales, customers }
    }
}

#[async_trait::async_trait]
impl AnalyticsProjectionQuery for AnalyticsProjectionAdapter {
    async fn load_event_sales(&self, event_id: &EventId) -> Result<Option<EventSalesMetrics>, String> {
        let projection = self.sales
            .read()
            .map_err(|_| "Failed to acquire read lock".to_string())?;

        Ok(projection.get_metrics(event_id).cloned())
    }

    async fn load_most_popular_section(&self, event_id: &EventId) -> Result<Option<(String, u32)>, String> {
        let projection = self.sales
            .read()
            .map_err(|_| "Failed to acquire read lock".to_string())?;

        Ok(projection.get_most_popular_section(event_id))
    }

    // ... implement other methods similarly
}
```

#### Step 4.4: Implement Analytics Reducer
**File**: `src/aggregates/analytics.rs`

```rust
pub struct AnalyticsReducer;

impl AnalyticsReducer {
    pub fn new() -> Self {
        Self
    }
}

impl Reducer for AnalyticsReducer {
    type State = AnalyticsState;
    type Action = AnalyticsAction;
    type Environment = AnalyticsEnvironment;  // ✅ No generics!

    fn reduce(
        &self,
        _state: &mut Self::State,  // Analytics is stateless
        action: Self::Action,
        env: &Self::Environment,
    ) -> Vec<Effect<Self::Action>> {
        match action {
            AnalyticsAction::GetEventSales { event_id } => {
                let projection = env.projection.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    match projection.load_event_sales(&event_id).await {
                        Ok(metrics) => Some(AnalyticsAction::EventSalesQueried {
                            event_id,
                            metrics,
                        }),
                        Err(e) => Some(AnalyticsAction::ValidationFailed {
                            error: format!("Failed to load event sales: {e}"),
                        }),
                    }
                }))]
            }

            AnalyticsAction::GetTopSpenders { limit } => {
                let projection = env.projection.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    match projection.load_top_spenders(limit).await {
                        Ok((customers, total)) => Some(AnalyticsAction::TopSpendersQueried {
                            customers,
                            total_customers: total,
                        }),
                        Err(e) => Some(AnalyticsAction::ValidationFailed {
                            error: format!("Failed to load top spenders: {e}"),
                        }),
                    }
                }))]
            }

            // ... implement other query handlers

            // Result events don't produce effects
            AnalyticsAction::EventSalesQueried { .. }
            | AnalyticsAction::PopularSectionsQueried { .. }
            | AnalyticsAction::TotalRevenueQueried { .. }
            | AnalyticsAction::TopSpendersQueried { .. }
            | AnalyticsAction::CustomerProfileQueried { .. }
            | AnalyticsAction::ValidationFailed { .. } => smallvec![],
        }
    }
}
```

#### Step 4.5: Update AppState to Create Analytics Store
**File**: `src/server/state.rs`

**Add method**:
```rust
pub fn create_analytics_store(
    &self,
) -> composable_rust_runtime::Store<
    crate::aggregates::analytics::AnalyticsState,
    crate::aggregates::analytics::AnalyticsAction,
    crate::aggregates::analytics::AnalyticsEnvironment<AnalyticsProjectionAdapter>,
    crate::aggregates::analytics::AnalyticsReducer,
> {
    use crate::aggregates::analytics::{AnalyticsEnvironment, AnalyticsReducer, AnalyticsState};
    use crate::projections::AnalyticsProjectionAdapter;
    use composable_rust_runtime::Store;

    let projection_adapter = Arc::new(AnalyticsProjectionAdapter::new(
        self.sales_analytics_projection.clone(),
        self.customer_history_projection.clone(),
    ));

    let env = AnalyticsEnvironment::new(
        self.clock.clone(),
        projection_adapter,
    );

    Store::new(AnalyticsState::new(), AnalyticsReducer::new(), env)
}
```

#### Step 4.6: Update Analytics API Handlers
**File**: `src/api/analytics.rs`

**Replace all 5 handlers** to use store/reducer pattern:
```rust
pub async fn get_event_sales(
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<EventSalesResponse>, AppError> {
    let event_id_typed = EventId::from_uuid(event_id);
    let analytics_store = state.create_analytics_store();

    let action = AnalyticsAction::GetEventSales {
        event_id: event_id_typed,
    };

    let result_action = analytics_store
        .send_and_wait_for(
            action,
            |a| matches!(a, AnalyticsAction::EventSalesQueried { .. }),
            std::time::Duration::from_secs(5),
        )
        .await?;

    let metrics = match result_action {
        AnalyticsAction::EventSalesQueried { metrics: Some(m), .. } => m,
        AnalyticsAction::EventSalesQueried { metrics: None, .. } => {
            return Err(AppError::not_found("Sales data", event_id));
        }
        _ => return Err(AppError::internal("Unexpected action returned")),
    };

    // Convert metrics to EventSalesResponse...
}
```

Repeat pattern for:
- `get_popular_sections()` (line 288)
- `get_total_revenue()` (line 371)
- `get_top_spenders()` (line 432)
- `get_customer_profile()` (line 512)

#### Step 4.7: Add Tests
**File**: `src/aggregates/analytics.rs` (tests module)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct MockAnalyticsProjection {
        sales_data: HashMap<EventId, EventSalesMetrics>,
        customer_data: HashMap<CustomerId, CustomerProfile>,
    }

    #[async_trait::async_trait]
    impl AnalyticsProjectionQuery for MockAnalyticsProjection {
        async fn load_event_sales(&self, event_id: &EventId) -> Result<Option<EventSalesMetrics>, String> {
            Ok(self.sales_data.get(event_id).cloned())
        }
        // ... implement other methods
    }

    #[tokio::test]
    async fn test_get_event_sales_query() {
        // Setup mock projection with test data
        let mut sales_data = HashMap::new();
        let event_id = EventId::new();
        sales_data.insert(event_id, create_test_metrics());

        let mock = MockAnalyticsProjection { sales_data, customer_data: HashMap::new() };
        let env = AnalyticsEnvironment::new(
            Arc::new(FixedClock::new(Utc::now())),
            Arc::new(mock),
        );

        let mut state = AnalyticsState::new();
        let reducer = AnalyticsReducer::new();

        // Execute query
        let effects = reducer.reduce(
            &mut state,
            AnalyticsAction::GetEventSales { event_id },
            &env,
        );

        // Execute effect
        let result = execute_effect(&effects[0]).await;

        // Verify
        assert!(matches!(
            result,
            Some(AnalyticsAction::EventSalesQueried { metrics: Some(_), .. })
        ));
    }
}
```

---

### Phase 4 (Original): Analytics API (Special Case - In-Memory Projections)

**Decision Point**: Analytics queries are different because:
- They query **in-memory projections** (`Arc<RwLock<T>>`)
- They don't correspond to a single aggregate
- They span multiple aggregates (sales analytics, customer history)

**Options**:
1. **Option A**: Create dedicated `Analytics` aggregate
   - Pro: Consistent with other patterns
   - Con: Adds complexity for simple read-only queries

2. **Option B**: Keep direct projection access for analytics
   - Pro: Simple, fast, read-only
   - Con: Inconsistent with architectural pattern

3. **Option C**: Create `AnalyticsService` that wraps projections
   - Pro: Encapsulation, testability
   - Con: Not using reducer pattern

**DECISION**: **✅ Option A CHOSEN** - Create dedicated Analytics aggregate for consistency.

This maintains architectural integrity across the entire codebase.

---

### Phase 5: WebSocket Real-Time Updates

**WebSocket Special Case**: Real-time event streams from EventBus.

**Current Approach** (line 356):
```rust
if let Ok(sections) = event_projection.get_all_sections(&event_id_typed).await {
    // Send update...
}
```

**Decision Point**:
- Initial snapshot (line 305): Should use query action ✅
- Real-time updates from EventBus (line 356): Can query projection directly
  - Reason: These are **event-driven updates**, not user-initiated queries
  - The projection is already updated by the event
  - No business logic needed here

**Action**: Only fix initial snapshot (line 305) to use query action.

---

## 🧪 Testing Strategy

### 1. Unit Tests (Per Aggregate)
Test query actions in isolation:
```rust
#[tokio::test]
async fn test_get_event_returns_event_when_exists() {
    // Arrange: Mock projection returns Some(event)
    // Act: Send GetEvent action
    // Assert: EventQueried with Some(event)
}

#[tokio::test]
async fn test_get_event_returns_none_when_not_exists() {
    // Arrange: Mock projection returns None
    // Act: Send GetEvent action
    // Assert: EventQueried with None
}

#[tokio::test]
async fn test_list_events_filters_by_status() {
    // Arrange: Mock projection with multiple events
    // Act: Send ListEvents with status filter
    // Assert: EventsListed with filtered events
}
```

### 2. Integration Tests (API Level)
Test API handlers use store/reducer pattern:
```rust
#[tokio::test]
async fn test_get_event_api_uses_store() {
    // Arrange: Setup test app with mock projection
    // Act: GET /api/events/:id
    // Assert: Response matches expected event
    // Assert: Mock projection was called via reducer
}
```

### 3. Mock Projections for Testing
Create mock implementations:
```rust
pub struct MockEventProjectionQuery {
    events: HashMap<EventId, Event>,
}

#[async_trait::async_trait]
impl EventProjectionQuery for MockEventProjectionQuery {
    async fn load_event(&self, event_id: &EventId) -> Result<Option<Event>, String> {
        Ok(self.events.get(event_id).cloned())
    }

    async fn load_events(&self, status_filter: Option<EventStatus>) -> Result<Vec<Event>, String> {
        let events: Vec<Event> = self.events
            .values()
            .filter(|e| status_filter.is_none_or(|s| e.status == s))
            .cloned()
            .collect();
        Ok(events)
    }
}
```

---

## 📊 Progress Tracking

### Phase 1: Event Aggregate
- [ ] Step 1.1: Add query actions to EventAction enum
- [ ] Step 1.2: Add EventProjectionQuery trait + update EventEnvironment
- [ ] Step 1.3: Implement trait for PostgresEventsProjection
- [ ] Step 1.4: Add reducer handlers
- [ ] Step 1.5: Update AppState::create_event_store()
- [ ] Step 1.6: Update Event API handlers (5 handlers)
  - [ ] `get_event()` (line 240-244)
  - [ ] `list_events()` (line 289-293)
  - [ ] `update_event()` pre-check (line 347)
  - [ ] `update_event()` verification (line 389)
  - [ ] `delete_event()` (line 427)
- [ ] Step 1.7: Add tests

### Phase 2: Payment Aggregate
- [ ] Step 2.1: Add query actions to PaymentAction enum
- [ ] Step 2.2: Add PaymentProjectionQuery trait + update PaymentEnvironment
- [ ] Step 2.3: Implement trait for PostgresPaymentsProjection
- [ ] Step 2.4: Add reducer handlers
- [ ] Step 2.5: Update AppState::create_payment_store()
- [ ] Step 2.6: Update Payment API handlers (3 handlers)
  - [ ] `get_payment()` (line 322)
  - [ ] `refund_payment()` pre-check (line 408)
  - [ ] `list_user_payments()` (line 530)
- [ ] Step 2.7: Add tests

### Phase 3: Inventory Aggregate
- [ ] Step 3.1: Add query actions to InventoryAction enum
- [ ] Step 3.2: Add InventoryProjectionQuery trait + update InventoryEnvironment
- [ ] Step 3.3: Implement trait for PostgresAvailableSeatsProjection
- [ ] Step 3.4: Add reducer handlers
- [ ] Step 3.5: Update AppState::create_inventory_store()
- [ ] Step 3.6: Update Availability API handlers (2 handlers)
  - [ ] `get_event_availability()` (line 92)
  - [ ] `get_section_availability()` (line 150)
- [ ] Step 3.7: Update WebSocket initial snapshot (line 305)
- [ ] Step 3.8: Add tests

### Phase 4: Analytics Aggregate (Option A - DECIDED)
- [ ] Step 4.1: Create Analytics aggregate (Action, State)
- [ ] Step 4.2: Create AnalyticsProjectionQuery trait + AnalyticsEnvironment
- [ ] Step 4.3: Create AnalyticsProjectionAdapter (wraps in-memory projections)
- [ ] Step 4.4: Implement AnalyticsReducer
- [ ] Step 4.5: Update AppState::create_analytics_store()
- [ ] Step 4.6: Update Analytics API handlers (5 handlers)
  - [ ] `get_event_sales()` (line 218)
  - [ ] `get_popular_sections()` (line 288)
  - [ ] `get_total_revenue()` (line 371)
  - [ ] `get_top_spenders()` (line 432)
  - [ ] `get_customer_profile()` (line 512)
- [ ] Step 4.7: Add tests with mock AnalyticsProjectionQuery

### Phase 5: Final Verification
- [ ] Run full integration test suite
- [ ] Verify no direct projection calls remain (grep audit)
- [ ] Update documentation
- [ ] Code review

---

## 🎓 Key Learnings

### Why This Matters
1. **Testability**: Mock projections in tests without touching infrastructure
2. **Business Rules**: Can add authorization, caching, rate limiting to queries
3. **Consistency**: Same pattern for reads and writes
4. **Future-Proof**: Easy to add complex query logic later

### Reservation as Gold Standard
The Reservation aggregate shows the correct pattern:
- Query actions defined in Action enum
- Projection query trait for dependency injection
- Reducer handles query by executing Effect::Future
- Result event returned to caller
- API handler uses `send_and_wait_for()` pattern

**THIS PATTERN MUST BE FOLLOWED FOR ALL AGGREGATES.**

---

## 🚨 Critical Notes

1. **Do NOT skip any aggregate** - consistency is non-negotiable
2. **Use Reservation as reference** - copy the pattern exactly
3. **Test with mocks** - verify dependency injection works
4. **Update bootstrap** - ensure all stores created with projection dependency
5. **Grep audit after completion** - verify no direct projection calls remain

---

## 📝 Files to Modify

### Aggregates
- `src/aggregates/event.rs` - Add query actions + projection trait + update EventEnvironment
- `src/aggregates/payment.rs` - Add query actions + projection trait + update PaymentEnvironment
- `src/aggregates/inventory.rs` - Add query actions + projection trait + update InventoryEnvironment
- `src/aggregates/analytics.rs` - **NEW**: Create Analytics aggregate (Action, State, Reducer, Environment)
- `src/aggregates/mod.rs` - Add `pub mod analytics;` and re-export types

### Projections
- `src/projections/events_postgres.rs` - Implement EventProjectionQuery
- `src/projections/payments_postgres.rs` - Implement PaymentProjectionQuery
- `src/projections/available_seats_postgres.rs` - Implement InventoryProjectionQuery
- `src/projections/analytics_adapter.rs` - **NEW**: Create AnalyticsProjectionAdapter
- `src/projections/mod.rs` - Add analytics_adapter module and re-export AnalyticsProjectionAdapter

### API Handlers
- `src/api/events.rs` - 5 handlers (lines 242, 291, 347, 389, 427)
- `src/api/payments.rs` - 3 handlers (lines 322, 408, 530)
- `src/api/availability.rs` - 2 handlers (lines 92, 150)
- `src/api/websocket.rs` - 1 handler (line 305 - initial snapshot only)
- `src/api/analytics.rs` - 5 handlers (lines 218, 288, 371, 432, 512)

### Infrastructure
- `src/server/state.rs` - Update store creation methods
- `src/bootstrap/builder.rs` - Ensure projections passed to stores

### Tests
- `src/aggregates/event.rs` - Add query action tests
- `src/aggregates/payment.rs` - Add query action tests
- `src/aggregates/inventory.rs` - Add query action tests
- `tests/integration/` - Add API integration tests with mocks

---

## ✅ Success Criteria

1. **Zero direct projection calls in API handlers** (verified by grep)
2. **All queries flow through store/reducer pattern**
3. **All tests pass** (unit + integration)
4. **Mock projections used in tests** (no real databases)
5. **Consistent with Reservation pattern** (code review)
6. **Documentation updated** (architecture docs reflect query actions)

---

## 🔍 Verification Commands

After refactoring is complete, run these commands:

```bash
# Verify no direct projection calls in API handlers
grep -n "\.events_projection\." src/api/*.rs
grep -n "\.payments_projection\." src/api/*.rs
grep -n "\.available_seats_projection\." src/api/*.rs
grep -n "\.sales_analytics_projection\." src/api/*.rs
grep -n "\.customer_history_projection\." src/api/*.rs

# Should return ZERO matches (except in test files)

# Run tests
cargo test --all-features

# Run clippy
cargo clippy --all-targets --all-features -- -D warnings

# Run integration tests
cargo test --test '*' --all-features
```

Expected output: **ALL CLEAR** ✅

---

End of refactor plan. Follow step-by-step, test thoroughly, and verify consistency.
