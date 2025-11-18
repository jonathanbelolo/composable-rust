# Phase 12: Production Completion Plan

**Goal**: Complete the ticketing application to the point where it can be deployed in production and start selling tickets.

**Current Status**: 60% complete
- Domain logic: 95% complete (world-class)
- Infrastructure integration: 15% complete (critical gaps)
- Production readiness: 4/10

**Estimated Total Effort**: 50-60 hours

**Target Production Readiness**: 10/10

---

## Critical Path Overview

```
1. Event Aggregate Persistence (BLOCKER)
   ↓
2. API-to-Aggregate Integration (BLOCKER)
   ↓
3. Authorization Implementation (SECURITY)
   ↓
4. Query Layer Implementation
   ↓
5. Payment Gateway Integration
   ↓
6. Production Hardening
   ↓
7. Testing & Validation
   ↓
8. Deployment Preparation
```

---

## Phase 12.1: Fix Event Aggregate Persistence (CRITICAL)

**Priority**: BLOCKER
**Estimated Time**: 2-3 hours
**Dependencies**: None

### Tasks

#### Task 1.1: Add Event Persistence Helper
**File**: `src/aggregates/event.rs`

**Current Issue**:
```rust
// Line 369-482: All commands return SmallVec::new()
EventAction::CreateEvent { ... } => {
    Self::apply_event(state, &event);
    SmallVec::new()  // ❌ No persistence!
}
```

**Action**:
1. Add `create_effects()` helper method similar to inventory/reservation aggregates
2. Pattern to follow from `inventory.rs:565-593`:
```rust
fn create_effects(
    event: EventEvent,
    env: &Self::Environment,
    correlation_id: Option<CorrelationId>,
) -> SmallVec<[Effect<Self::Action>; 4]> {
    let mut effects = smallvec![];

    // 1. Serialize event
    let mut serialized = SerializedEvent {
        aggregate_id: event.event_id().to_string(),
        aggregate_type: "event".to_string(),
        event_type: event.event_type().to_string(),
        data: bincode::serialize(&event).unwrap(),
        metadata: None,
        timestamp: env.clock.now(),
    };

    // 2. Add correlation_id to metadata
    if let Some(cid) = correlation_id {
        let metadata = serialized.metadata.get_or_insert_with(EventMetadata::new);
        metadata.correlation_id = Some(cid.to_string());
    }

    // 3. Persist to event store
    effects.push(Effect::Database(DatabaseEffect::AppendEvent(serialized.clone())));

    // 4. Publish to event bus
    effects.push(Effect::PublishEvent {
        topic: "events".to_string(),
        event: serialized,
    });

    effects
}
```

3. Update all command handlers to call `create_effects()`
4. Add proper error handling for validation failures

**Acceptance Criteria**:
- [ ] `EventReducer::reduce()` returns effects for all commands
- [ ] Events persisted to `events` table in PostgreSQL
- [ ] Events published to `ticketing-events` topic in RedPanda
- [ ] Unit tests verify effects are generated
- [ ] Integration test verifies end-to-end persistence

**Files to Modify**:
- `src/aggregates/event.rs` (lines 369-482)

**Test File**:
- Create `tests/event_persistence_test.rs`

---

## Phase 12.2: Wire API Endpoints to Aggregates (CRITICAL)

**Priority**: BLOCKER
**Estimated Time**: 16-20 hours
**Dependencies**: Phase 12.1

### Task 2.1: Create Event via API
**File**: `src/api/events.rs`

**Current Issue** (line 156):
```rust
// TODO: Send CreateEvent action to event aggregate via event store
Json(CreateEventResponse {
    event_id: Uuid::new_v4(),  // ❌ Fake response
    message: "Event created successfully".to_string(),
})
```

**Action**:
1. Inject `EventStore` and `EventBus` into handler via `State`
2. Build `EventAction::CreateEvent` from request
3. Load current state from event store (or create new)
4. Call `EventReducer::reduce()` to get effects
5. Execute effects (persist, publish)
6. Return real event_id

**Pattern to Follow**: `tests/saga_integration_test.rs:40-120` (E2E test flow)

**Implementation**:
```rust
pub async fn create_event(
    State(event_store): State<Arc<PostgresEventStore>>,
    State(event_bus): State<Arc<RedpandaEventBus>>,
    State(clock): State<Arc<SystemClock>>,
    correlation_id: CorrelationId,
    Json(request): Json<CreateEventRequest>,
) -> Result<(StatusCode, Json<CreateEventResponse>), AppError> {
    // 1. Build action
    let action = EventAction::CreateEvent {
        correlation_id: correlation_id.0,
        name: request.name,
        description: request.description,
        venue: request.venue,
        date_time: request.date_time,
        sections: request.sections,
    };

    // 2. Build environment
    let env = EventEnvironment {
        event_store: event_store.as_ref(),
        event_bus: event_bus.as_ref(),
        clock: clock.as_ref(),
    };

    // 3. Load current state (or create new)
    let event_id = EventId::new();
    let events = event_store
        .load_events("event", &event_id.0.to_string())
        .await
        .map_err(|e| AppError::internal(format!("Failed to load events: {e}")))?;

    let mut state = EventState::from_events(
        events.into_iter().map(|se| bincode::deserialize(&se.data).unwrap())
    );

    // 4. Reduce
    let reducer = EventReducer;
    let effects = reducer.reduce(&mut state, action, &env);

    // 5. Execute effects
    for effect in effects {
        execute_effect(effect, &event_store, &event_bus).await?;
    }

    // 6. Return response
    Ok((
        StatusCode::CREATED,
        Json(CreateEventResponse {
            event_id: event_id.0,
            message: "Event created successfully".to_string(),
        }),
    ))
}
```

**Acceptance Criteria**:
- [ ] POST `/events` creates real event in database
- [ ] Event appears in event store with correct aggregate_id
- [ ] Event published to event bus
- [ ] Response contains real event_id
- [ ] Integration test verifies E2E flow

**Files to Modify**:
- `src/api/events.rs` (line 156)
- `src/server/routes.rs` (add State injections)

**Test File**:
- `tests/api_event_creation_test.rs`

---

### Task 2.2: Add Inventory via API
**File**: `src/api/inventory.rs`

**Current Issue** (line 192):
```rust
// TODO: Send AddInventory action to inventory aggregate via event store
```

**Action**:
1. Follow same pattern as Task 2.1
2. Build `InventoryAction::AddInventory`
3. Load state from projection (use `InventoryProjection::load_inventory()`)
4. Reduce and execute effects
5. Return real inventory section details

**Key Difference**: Inventory uses projection-based state loading (not full event replay)

**Implementation Note**: Line 705-774 in `inventory.rs` shows the pattern:
```rust
if !state.is_loaded(&event_id, &section) {
    // Trigger load from projection
    return smallvec![Effect::Sequential(vec![
        Effect::Future(Box::pin(async move {
            projection.load_inventory(&event_id_copy, &section_copy).await
        })),
        Effect::Future(Box::pin(async move { Some(original_command) }))
    ])];
}
```

**Acceptance Criteria**:
- [ ] POST `/inventory` adds real inventory in database
- [ ] State loads from projection before processing
- [ ] Projection updated with new inventory
- [ ] Response contains real section details

**Files to Modify**:
- `src/api/inventory.rs` (lines 192, 318)

**Test File**:
- `tests/api_inventory_test.rs`

---

### Task 2.3: Reserve Seats via API
**File**: `src/api/reservations.rs`

**Current Issue** (line 151):
```rust
// TODO: Send InitiateReservation action to reservation aggregate via event store
```

**Action**:
1. Build `ReservationAction::InitiateReservation`
2. Load reservation state from events
3. Reduce and execute effects
4. **Critical**: This triggers the saga! Effects will include:
   - Publishing to inventory aggregate (reserve seats)
   - Scheduling timeout (5 minutes)
5. Return reservation_id

**Saga Flow** (reference `reservation.rs:500-593`):
```
InitiateReservation
  → Persist ReservationInitiated
  → Publish InventoryAction::ReserveSeats (child aggregate)
  → Schedule timeout (5 min)
```

**Acceptance Criteria**:
- [ ] POST `/reservations` creates real reservation
- [ ] Saga triggers: inventory reservation command published
- [ ] Timeout scheduled (verify in integration test)
- [ ] Correlation ID propagates to child aggregate
- [ ] Response contains real reservation_id

**Files to Modify**:
- `src/api/reservations.rs` (lines 151, 251, 366)

**Test File**:
- `tests/api_reservation_saga_test.rs` (full saga E2E)

---

### Task 2.4: Process Payment via API
**File**: `src/api/payments.rs`

**Current Issue** (lines 241-266):
```rust
// TODO: Send ProcessPayment command to payment aggregate via event store
Json(ProcessPaymentResponse {
    payment_id: Uuid::new_v4(),  // ❌ Fake
    status: "completed".to_string(),
    amount: 200.0,  // TODO: Get from reservation
    message: "Payment processed successfully".to_string(),
})
```

**Action**:
1. Load reservation to get total_amount (from query adapter)
2. Build `PaymentAction::ProcessPayment`
3. Reduce and execute effects
4. **Critical**: Payment reducer calls payment gateway (async effect)
5. Payment success triggers `ReservationAction::CompleteReservation`
6. Return payment_id and transaction_id

**Saga Flow** (reference `payment.rs:317-405`):
```
ProcessPayment
  → Call PaymentGateway::process_payment()
  → Persist PaymentCompleted
  → Publish ReservationAction::CompleteReservation (parent aggregate)
```

**Acceptance Criteria**:
- [ ] POST `/payments` processes real payment
- [ ] Payment gateway called with correct amount
- [ ] Payment success completes reservation (saga step 4)
- [ ] Response contains real payment_id and transaction_id
- [ ] Integration test verifies full saga: reserve → pay → complete

**Files to Modify**:
- `src/api/payments.rs` (lines 241-266, 390-404)

**Test File**:
- `tests/api_payment_saga_test.rs`

---

### Task 2.5: Query Endpoints
**File**: `src/api/reservations.rs`, `src/api/payments.rs`

**Current Issue**: Query endpoints always return 404

**Action**:
1. Wire query handlers to query adapters
2. Query adapters will be implemented in Phase 12.4
3. For now, add proper error handling for None case

**Files to Modify**:
- `src/api/reservations.rs` (line 251)
- `src/api/payments.rs` (line 390)

**Acceptance Criteria**:
- [ ] GET `/reservations/:id` queries database (returns 404 if not found)
- [ ] GET `/payments/:id` queries database (returns 404 if not found)
- [ ] Proper error handling with AppError

---

### Task 2.6: Health Check Endpoints
**File**: `src/api/health.rs`

**Current Issue** (lines 66-121): All checks are stubbed

**Action**:
1. Implement real database health check (PostgreSQL ping)
2. Implement real Redis health check (PING command)
3. Implement real event bus health check (RedPanda broker metadata)
4. Implement projection consumer lag check

**Implementation**:
```rust
// Database check
let db_healthy = sqlx::query("SELECT 1")
    .fetch_one(event_store.pool())
    .await
    .is_ok();

// Redis check
let redis_healthy = redis_client
    .get::<_, Option<String>>("health_check")
    .await
    .is_ok();

// Event bus check
let bus_healthy = event_bus.get_metadata().await.is_ok();

// Projection lag check
let projection_lag = projection_store.get_consumer_lag().await.unwrap_or(0);
let projection_healthy = projection_lag < 1000; // < 1000 messages behind
```

**Acceptance Criteria**:
- [ ] GET `/health` returns real system status
- [ ] GET `/health/ready` checks all dependencies
- [ ] GET `/health/live` checks application is running
- [ ] Prometheus metrics exposed at `/metrics`

**Files to Modify**:
- `src/api/health.rs` (lines 66-121)

---

## Phase 12.3: Implement Authorization (SECURITY)

**Priority**: SECURITY CRITICAL
**Estimated Time**: 6-8 hours
**Dependencies**: Phase 12.2

### Task 3.1: Implement Reservation Ownership Check
**File**: `src/auth/middleware.rs`

**Current Issue** (line 356):
```rust
// TEMPORARY: Allow all for development
return Ok(RequireReservationOwnership {
    user_id: UserId::new(),  // ❌ Fake user ID!
    reservation_id,
});
```

**Action**:
1. Query reservation from database/projection
2. Compare reservation.user_id with authenticated user_id
3. Return Forbidden if mismatch

**Implementation**:
```rust
// Query reservation ownership
let reservation = query_adapters::find_reservation(&reservation_id)
    .await
    .map_err(|e| AuthError::Internal(e))?
    .ok_or_else(|| AuthError::Forbidden("Reservation not found".to_string()))?;

// Verify ownership
if reservation.user_id != user_id {
    return Err(AuthError::Forbidden(
        "You do not have permission to access this reservation".to_string()
    ));
}

Ok(RequireReservationOwnership {
    user_id,
    reservation_id,
})
```

**Acceptance Criteria**:
- [ ] Authenticated user can only access own reservations
- [ ] Accessing other user's reservation returns 403 Forbidden
- [ ] Non-existent reservation returns 404 Not Found
- [ ] Integration test verifies authorization

**Files to Modify**:
- `src/auth/middleware.rs` (line 356)

**Test File**:
- `tests/auth_reservation_ownership_test.rs`

---

### Task 3.2: Implement Payment Ownership Check
**File**: `src/auth/middleware.rs`

**Current Issue** (line 398):
```rust
// TEMPORARY: Allow all for development
return Ok(RequirePaymentOwnership {
    user_id: UserId::new(),  // ❌ Fake user ID!
    payment_id,
});
```

**Action**:
1. Query payment from database/projection
2. Query associated reservation (payment → reservation → user_id)
3. Compare with authenticated user_id
4. Return Forbidden if mismatch

**Implementation**:
```rust
// Query payment
let payment = query_adapters::find_payment(&payment_id)
    .await
    .map_err(|e| AuthError::Internal(e))?
    .ok_or_else(|| AuthError::Forbidden("Payment not found".to_string()))?;

// Query associated reservation to get user_id
let reservation = query_adapters::find_reservation(&payment.reservation_id)
    .await
    .map_err(|e| AuthError::Internal(e))?
    .ok_or_else(|| AuthError::Forbidden("Reservation not found".to_string()))?;

// Verify ownership
if reservation.user_id != user_id {
    return Err(AuthError::Forbidden(
        "You do not have permission to access this payment".to_string()
    ));
}

Ok(RequirePaymentOwnership {
    user_id,
    payment_id,
})
```

**Acceptance Criteria**:
- [ ] Authenticated user can only access payments for own reservations
- [ ] Accessing other user's payment returns 403 Forbidden
- [ ] Non-existent payment returns 404 Not Found
- [ ] Integration test verifies authorization

**Files to Modify**:
- `src/auth/middleware.rs` (line 398)

**Test File**:
- `tests/auth_payment_ownership_test.rs`

---

### Task 3.3: Add Admin Role Support
**File**: `src/auth/middleware.rs`

**Action**:
1. Add `RequireAdmin` extractor
2. Check user role from session (requires auth library integration)
3. Allow admins to bypass ownership checks (for customer support)

**Implementation**:
```rust
pub struct RequireAdmin {
    pub user_id: UserId,
}

#[async_trait]
impl<S> FromRequestParts<S> for RequireAdmin
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Extract authenticated session
        let auth_session = RequireAuthSession::from_request_parts(parts, state).await?;

        // Check if user has admin role
        // TODO: Query user role from database
        let is_admin = check_user_role(&auth_session.user_id).await?;

        if !is_admin {
            return Err(AuthError::Forbidden("Admin access required".to_string()));
        }

        Ok(RequireAdmin {
            user_id: auth_session.user_id,
        })
    }
}
```

**Acceptance Criteria**:
- [ ] Admin users can access all reservations/payments
- [ ] Non-admin users get 403 when trying to access admin endpoints
- [ ] Admin role stored in database (user_roles table)

**Files to Create**:
- `src/auth/roles.rs` (role checking logic)

**Test File**:
- `tests/auth_admin_role_test.rs`

---

## Phase 12.4: Implement Query Layer (HIGH PRIORITY)

**Priority**: HIGH
**Estimated Time**: 8-10 hours
**Dependencies**: Phase 12.2

### Task 4.1: Implement Payment Query Adapter
**File**: `src/projections/query_adapters.rs`

**Current Issue** (line 110):
```rust
pub async fn find_payment(_payment_id: &PaymentId) -> Result<Option<Payment>, String> {
    // TODO: Implement when we have a payment projection
    Ok(None)
}
```

**Action**:
1. Create `payments` projection table (schema below)
2. Implement projection consumer that listens to payment events
3. Implement query function that reads from projection table

**Schema**:
```sql
CREATE TABLE payments (
    payment_id UUID PRIMARY KEY,
    reservation_id UUID NOT NULL,
    user_id UUID NOT NULL,
    amount DECIMAL(10, 2) NOT NULL,
    currency VARCHAR(3) NOT NULL,
    status VARCHAR(50) NOT NULL,
    gateway_transaction_id VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    failed_at TIMESTAMPTZ,
    failure_reason TEXT,
    metadata JSONB
);

CREATE INDEX idx_payments_reservation_id ON payments(reservation_id);
CREATE INDEX idx_payments_user_id ON payments(user_id);
CREATE INDEX idx_payments_status ON payments(status);
```

**Implementation**:
```rust
pub async fn find_payment(
    pool: &PgPool,
    payment_id: &PaymentId,
) -> Result<Option<Payment>, String> {
    let row = sqlx::query_as!(
        PaymentRow,
        r#"
        SELECT
            payment_id,
            reservation_id,
            user_id,
            amount,
            currency,
            status,
            gateway_transaction_id,
            created_at,
            completed_at,
            failed_at,
            failure_reason,
            metadata
        FROM payments
        WHERE payment_id = $1
        "#,
        payment_id.0
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Database error: {e}"))?;

    Ok(row.map(|r| r.into()))
}
```

**Projection Consumer**:
```rust
// In src/projections/payment.rs
pub struct PaymentProjection {
    pool: PgPool,
}

impl PaymentProjection {
    pub async fn handle_event(&self, event: PaymentEvent) -> Result<(), String> {
        match event {
            PaymentEvent::PaymentInitiated { payment_id, reservation_id, amount, currency, .. } => {
                sqlx::query!(
                    "INSERT INTO payments (payment_id, reservation_id, amount, currency, status, created_at)
                     VALUES ($1, $2, $3, $4, 'initiated', $5)
                     ON CONFLICT (payment_id) DO NOTHING",
                    payment_id.0,
                    reservation_id.0,
                    amount,
                    currency,
                    Utc::now()
                )
                .execute(&self.pool)
                .await
                .map_err(|e| format!("Failed to insert payment: {e}"))?;
            }
            PaymentEvent::PaymentCompleted { payment_id, transaction_id, .. } => {
                sqlx::query!(
                    "UPDATE payments
                     SET status = 'completed',
                         gateway_transaction_id = $2,
                         completed_at = $3
                     WHERE payment_id = $1",
                    payment_id.0,
                    transaction_id,
                    Utc::now()
                )
                .execute(&self.pool)
                .await
                .map_err(|e| format!("Failed to update payment: {e}"))?;
            }
            PaymentEvent::PaymentFailed { payment_id, error, .. } => {
                sqlx::query!(
                    "UPDATE payments
                     SET status = 'failed',
                         failure_reason = $2,
                         failed_at = $3
                     WHERE payment_id = $1",
                    payment_id.0,
                    error,
                    Utc::now()
                )
                .execute(&self.pool)
                .await
                .map_err(|e| format!("Failed to update payment: {e}"))?;
            }
        }
        Ok(())
    }
}
```

**Acceptance Criteria**:
- [ ] `payments` table created with schema
- [ ] Projection consumer running in background
- [ ] Query function returns real data from projection
- [ ] Integration test verifies: payment event → projection → query

**Files to Create**:
- `migrations/007_create_payments_projection.sql`
- `src/projections/payment.rs`

**Files to Modify**:
- `src/projections/query_adapters.rs` (line 110)
- `src/projections/mod.rs` (export payment projection)

**Test File**:
- `tests/payment_projection_test.rs`

---

### Task 4.2: Implement Reservation Query Adapter
**File**: `src/projections/query_adapters.rs`

**Current Issue** (line 151):
```rust
pub async fn find_reservation(_reservation_id: &ReservationId) -> Result<Option<Reservation>, String> {
    // TODO: Implement when we have a reservation projection
    Ok(None)
}
```

**Action**:
1. Create `reservations` projection table (schema below)
2. Implement projection consumer that listens to reservation events
3. Implement query function that reads from projection table

**Schema**:
```sql
CREATE TABLE reservations (
    reservation_id UUID PRIMARY KEY,
    user_id UUID NOT NULL,
    event_id UUID NOT NULL,
    status VARCHAR(50) NOT NULL,
    seats JSONB NOT NULL,  -- Array of {section, seat_id, price}
    total_amount DECIMAL(10, 2) NOT NULL,
    currency VARCHAR(3) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    expired_at TIMESTAMPTZ,
    compensated_at TIMESTAMPTZ,
    metadata JSONB
);

CREATE INDEX idx_reservations_user_id ON reservations(user_id);
CREATE INDEX idx_reservations_event_id ON reservations(event_id);
CREATE INDEX idx_reservations_status ON reservations(status);
```

**Implementation**: Follow same pattern as Task 4.1

**Acceptance Criteria**:
- [ ] `reservations` table created with schema
- [ ] Projection consumer running in background
- [ ] Query function returns real data from projection
- [ ] Integration test verifies: reservation event → projection → query

**Files to Create**:
- `migrations/008_create_reservations_projection.sql`
- `src/projections/reservation.rs`

**Files to Modify**:
- `src/projections/query_adapters.rs` (line 151)

**Test File**:
- `tests/reservation_projection_test.rs`

---

### Task 4.3: Add User Query Functions
**File**: `src/projections/query_adapters.rs`

**Action**:
1. Implement `list_user_reservations(user_id)` → Vec<Reservation>
2. Implement `list_user_payments(user_id)` → Vec<Payment>
3. Add pagination support (limit, offset)

**Implementation**:
```rust
pub async fn list_user_reservations(
    pool: &PgPool,
    user_id: &UserId,
    limit: i64,
    offset: i64,
) -> Result<Vec<Reservation>, String> {
    let rows = sqlx::query_as!(
        ReservationRow,
        r#"
        SELECT *
        FROM reservations
        WHERE user_id = $1
        ORDER BY created_at DESC
        LIMIT $2 OFFSET $3
        "#,
        user_id.0,
        limit,
        offset
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Database error: {e}"))?;

    Ok(rows.into_iter().map(|r| r.into()).collect())
}
```

**Acceptance Criteria**:
- [ ] User can list own reservations
- [ ] User can list own payments
- [ ] Pagination works correctly
- [ ] Results ordered by created_at DESC (newest first)

**Files to Modify**:
- `src/projections/query_adapters.rs` (add new functions)
- `src/api/reservations.rs` (add list endpoint)
- `src/api/payments.rs` (add list endpoint)

**Test File**:
- `tests/user_query_test.rs`

---

## Phase 12.5: Payment Gateway Integration (HIGH PRIORITY)

**Priority**: HIGH
**Estimated Time**: 12-16 hours
**Dependencies**: Phase 12.2, Phase 12.4

### Task 5.1: Stripe Integration
**File**: `src/payment_gateway.rs`

**Current Issue**: Only mock implementation exists

**Action**:
1. Add Stripe SDK dependency: `stripe-rust = "0.28"`
2. Implement `StripePaymentGateway` struct
3. Add configuration for Stripe API key
4. Implement payment processing with Stripe Payment Intents API
5. Handle webhook callbacks for async payment confirmation

**Configuration**:
```rust
// src/config.rs - Add to Config
pub struct PaymentConfig {
    pub stripe_api_key: String,
    pub stripe_webhook_secret: String,
    pub payment_timeout_seconds: u64,
}
```

**Implementation**:
```rust
use stripe::{Client, CreatePaymentIntent, PaymentIntent, PaymentIntentStatus};

pub struct StripePaymentGateway {
    client: Client,
    timeout: Duration,
}

impl StripePaymentGateway {
    pub fn new(api_key: &str, timeout: Duration) -> Self {
        let client = Client::new(api_key);
        Self { client, timeout }
    }
}

#[async_trait]
impl PaymentGateway for StripePaymentGateway {
    fn process_payment(
        &self,
        amount: f64,
        currency: &str,
        payment_method: PaymentMethod,
        metadata: Option<HashMap<String, String>>,
    ) -> Pin<Box<dyn Future<Output = GatewayResult<PaymentTransaction>> + Send>> {
        let client = self.client.clone();
        let currency_str = currency.to_string();
        let timeout = self.timeout;

        Box::pin(async move {
            // Convert amount to cents (Stripe uses smallest currency unit)
            let amount_cents = (amount * 100.0) as i64;

            // Create payment intent
            let mut create_intent = CreatePaymentIntent::new(amount_cents, stripe::Currency::USD);
            create_intent.payment_method = Some(payment_method.stripe_payment_method_id());
            create_intent.confirm = Some(true); // Auto-confirm
            create_intent.metadata = metadata;

            // Process with timeout
            let result = tokio::time::timeout(
                timeout,
                PaymentIntent::create(&client, create_intent),
            )
            .await
            .map_err(|_| GatewayError::Timeout)?
            .map_err(|e| GatewayError::External(format!("Stripe error: {e}")))?;

            // Check status
            match result.status {
                PaymentIntentStatus::Succeeded => {
                    Ok(PaymentTransaction {
                        transaction_id: result.id.to_string(),
                        status: TransactionStatus::Completed,
                        amount,
                        currency: currency_str,
                        processed_at: Utc::now(),
                        gateway_response: Some(serde_json::to_value(&result).unwrap()),
                    })
                }
                PaymentIntentStatus::Processing => {
                    // Payment is async (bank transfer, etc.)
                    Ok(PaymentTransaction {
                        transaction_id: result.id.to_string(),
                        status: TransactionStatus::Pending,
                        amount,
                        currency: currency_str,
                        processed_at: Utc::now(),
                        gateway_response: Some(serde_json::to_value(&result).unwrap()),
                    })
                }
                PaymentIntentStatus::RequiresAction => {
                    // 3D Secure or additional verification needed
                    Err(GatewayError::RequiresAction {
                        client_secret: result.client_secret.unwrap(),
                        next_action: result.next_action,
                    })
                }
                _ => {
                    // Failed, canceled, etc.
                    Err(GatewayError::PaymentFailed(format!("Status: {:?}", result.status)))
                }
            }
        })
    }

    fn refund_payment(
        &self,
        transaction_id: &str,
        amount: Option<f64>,
        reason: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = GatewayResult<RefundTransaction>> + Send>> {
        let client = self.client.clone();
        let transaction_id_str = transaction_id.to_string();
        let amount_opt = amount;
        let reason_str = reason.map(|s| s.to_string());

        Box::pin(async move {
            use stripe::{CreateRefund, Refund};

            let mut create_refund = CreateRefund::new(transaction_id_str);
            if let Some(amt) = amount_opt {
                create_refund.amount = Some((amt * 100.0) as i64);
            }
            create_refund.reason = reason_str.map(|r| match r.as_str() {
                "duplicate" => stripe::RefundReason::Duplicate,
                "fraudulent" => stripe::RefundReason::Fraudulent,
                _ => stripe::RefundReason::RequestedByCustomer,
            });

            let result = Refund::create(&client, create_refund)
                .await
                .map_err(|e| GatewayError::External(format!("Stripe refund error: {e}")))?;

            Ok(RefundTransaction {
                refund_id: result.id.to_string(),
                status: RefundStatus::Completed,
                amount: (result.amount as f64) / 100.0,
                refunded_at: Utc::now(),
            })
        })
    }
}
```

**Webhook Handler**:
```rust
// src/api/webhooks.rs
pub async fn stripe_webhook(
    State(webhook_secret): State<String>,
    headers: HeaderMap,
    body: String,
) -> Result<StatusCode, AppError> {
    // Verify webhook signature
    let signature = headers
        .get("stripe-signature")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| AppError::unauthorized("Missing signature"))?;

    let event = stripe::Webhook::construct_event(&body, signature, &webhook_secret)
        .map_err(|e| AppError::unauthorized(format!("Invalid signature: {e}")))?;

    // Handle event
    match event.type_ {
        stripe::EventType::PaymentIntentSucceeded => {
            let payment_intent: PaymentIntent = serde_json::from_value(event.data.object)?;

            // Extract our payment_id from metadata
            let payment_id = payment_intent.metadata
                .get("payment_id")
                .ok_or_else(|| AppError::internal("Missing payment_id in metadata"))?;

            // Send PaymentCompleted action to payment aggregate
            // (This handles async payment confirmations)
        }
        stripe::EventType::PaymentIntentPaymentFailed => {
            // Handle payment failure
        }
        _ => {
            // Ignore other events
        }
    }

    Ok(StatusCode::OK)
}
```

**Acceptance Criteria**:
- [ ] Stripe SDK integrated and configured
- [ ] Real payments processed through Stripe
- [ ] Webhook endpoint handles async payment confirmations
- [ ] Payment failures handled gracefully
- [ ] Refunds supported
- [ ] Integration test with Stripe test mode
- [ ] Environment variables for API keys

**Files to Create**:
- `src/payment_gateway/stripe.rs`
- `src/api/webhooks.rs`

**Files to Modify**:
- `src/payment_gateway.rs` (export StripePaymentGateway)
- `src/config.rs` (add PaymentConfig)
- `Cargo.toml` (add stripe-rust dependency)

**Test File**:
- `tests/stripe_integration_test.rs`

---

### Task 5.2: PayPal Integration (Optional)
**Estimated Time**: 8-10 hours

**Action**: Follow same pattern as Task 5.1, using PayPal SDK

**Files to Create**:
- `src/payment_gateway/paypal.rs`

---

### Task 5.3: Payment Gateway Selection Logic
**File**: `src/payment_gateway.rs`

**Action**:
1. Add `PaymentGatewayRouter` that selects gateway based on payment method
2. Support multiple gateways (Stripe for cards, PayPal for PayPal)

**Implementation**:
```rust
pub struct PaymentGatewayRouter {
    stripe: StripePaymentGateway,
    paypal: Option<PayPalPaymentGateway>,
    mock: MockPaymentGateway,
    use_mock: bool,
}

impl PaymentGatewayRouter {
    pub fn get_gateway(&self, payment_method: &PaymentMethod) -> &dyn PaymentGateway {
        if self.use_mock {
            return &self.mock;
        }

        match payment_method {
            PaymentMethod::CreditCard { .. } => &self.stripe,
            PaymentMethod::PayPal { .. } => {
                self.paypal.as_ref().unwrap_or(&self.stripe)
            }
        }
    }
}
```

**Acceptance Criteria**:
- [ ] Router selects correct gateway based on payment method
- [ ] Mock gateway available for testing
- [ ] Configuration controls which gateways are enabled

**Files to Modify**:
- `src/payment_gateway.rs`

---

## Phase 12.6: Production Hardening (MEDIUM PRIORITY)

**Priority**: MEDIUM
**Estimated Time**: 8-10 hours
**Dependencies**: Phases 12.1-12.5

### Task 6.1: WebSocket Rate Limiting
**File**: `src/request_lifecycle/mod.rs`

**Current Issue** (line 253):
```rust
// TODO: Implement rate limiting (per user)
```

**Action**:
1. Add Redis-backed rate limiter
2. Limit WebSocket messages per user (e.g., 100 messages per minute)
3. Return error if limit exceeded

**Implementation**:
```rust
use composable_rust_auth::RedisRateLimiter;

pub async fn check_rate_limit(
    user_id: &UserId,
    rate_limiter: &RedisRateLimiter,
) -> Result<(), String> {
    let key = format!("ws:rate_limit:{}", user_id.0);
    let limit = 100; // messages
    let window = Duration::from_secs(60); // per minute

    if !rate_limiter.check_rate_limit(&key, limit, window).await? {
        return Err("Rate limit exceeded. Please slow down.".to_string());
    }

    Ok(())
}
```

**Acceptance Criteria**:
- [ ] Rate limiting enforced per user
- [ ] Error message returned when limit exceeded
- [ ] Redis stores rate limit counters
- [ ] Integration test verifies rate limiting

**Files to Modify**:
- `src/request_lifecycle/mod.rs` (line 253)

**Test File**:
- `tests/websocket_rate_limit_test.rs`

---

### Task 6.2: Add Idempotency Keys
**File**: `src/api/payments.rs`, `src/api/reservations.rs`

**Action**:
1. Add `Idempotency-Key` header to payment/reservation endpoints
2. Store processed keys in Redis with 24-hour TTL
3. Return cached response if duplicate request detected

**Implementation**:
```rust
use axum::http::HeaderMap;

pub async fn process_payment(
    headers: HeaderMap,
    State(redis): State<Arc<RedisClient>>,
    State(store): State<Arc<PaymentStore>>,
    Json(request): Json<ProcessPaymentRequest>,
) -> Result<Json<ProcessPaymentResponse>, AppError> {
    // Extract idempotency key
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| AppError::bad_request("Missing Idempotency-Key header"))?;

    // Check if already processed
    let cache_key = format!("idempotency:payment:{idempotency_key}");
    if let Some(cached_response) = redis.get::<_, Option<String>>(&cache_key).await? {
        let response: ProcessPaymentResponse = serde_json::from_str(&cached_response)?;
        return Ok(Json(response));
    }

    // Process payment
    let response = process_payment_internal(store, request).await?;

    // Cache response for 24 hours
    redis.set_ex(
        &cache_key,
        serde_json::to_string(&response)?,
        24 * 60 * 60, // 24 hours
    ).await?;

    Ok(Json(response))
}
```

**Acceptance Criteria**:
- [ ] Idempotency keys required for payment/reservation endpoints
- [ ] Duplicate requests return cached response
- [ ] Keys expire after 24 hours
- [ ] Integration test verifies idempotency

**Files to Modify**:
- `src/api/payments.rs`
- `src/api/reservations.rs`

**Test File**:
- `tests/idempotency_test.rs`

---

### Task 6.3: Add Circuit Breakers
**File**: `src/payment_gateway.rs`, `src/projections/mod.rs`

**Action**:
1. Wrap payment gateway calls in circuit breaker
2. Wrap event bus publishing in circuit breaker
3. Use `composable-rust-runtime` circuit breaker implementation

**Implementation**:
```rust
use composable_rust_runtime::CircuitBreaker;

pub struct ResilientPaymentGateway {
    inner: Box<dyn PaymentGateway>,
    circuit_breaker: CircuitBreaker,
}

impl PaymentGateway for ResilientPaymentGateway {
    fn process_payment(...) -> Pin<Box<dyn Future<Output = GatewayResult<PaymentTransaction>> + Send>> {
        let inner = self.inner.clone();
        let cb = self.circuit_breaker.clone();

        Box::pin(async move {
            cb.call(|| inner.process_payment(...)).await
        })
    }
}
```

**Acceptance Criteria**:
- [ ] Circuit breaker protects payment gateway calls
- [ ] Circuit opens after 5 consecutive failures
- [ ] Circuit closes after 30-second timeout
- [ ] Integration test verifies circuit breaker behavior

**Files to Modify**:
- `src/payment_gateway.rs`

**Test File**:
- `tests/circuit_breaker_test.rs`

---

### Task 6.4: Add Request Tracing
**File**: All API handlers

**Action**:
1. Add `tracing` spans to all HTTP handlers
2. Log correlation_id, user_id, request duration
3. Configure OpenTelemetry exporter for Jaeger/Zipkin

**Implementation**:
```rust
use tracing::{info_span, Instrument};

pub async fn create_event(
    State(store): State<Arc<EventStore>>,
    correlation_id: CorrelationId,
    Json(request): Json<CreateEventRequest>,
) -> Result<Json<CreateEventResponse>, AppError> {
    let span = info_span!(
        "create_event",
        correlation_id = %correlation_id.0,
        event_name = %request.name,
    );

    async move {
        // Handler implementation
    }
    .instrument(span)
    .await
}
```

**Acceptance Criteria**:
- [ ] All API handlers have tracing spans
- [ ] Spans include correlation_id, user_id
- [ ] Distributed tracing works across services
- [ ] Jaeger UI shows end-to-end traces

**Files to Modify**:
- All files in `src/api/`
- `src/server/main.rs` (configure tracing)

---

### Task 6.5: Add Prometheus Metrics
**File**: `src/server/metrics.rs`

**Action**:
1. Add custom metrics for business operations
2. Track: reservations_total, payments_total, reservation_duration_seconds
3. Expose at `/metrics` endpoint

**Implementation**:
```rust
use prometheus::{Counter, Histogram, Registry};

lazy_static! {
    pub static ref RESERVATIONS_TOTAL: Counter = Counter::new(
        "ticketing_reservations_total",
        "Total number of reservations created"
    ).unwrap();

    pub static ref PAYMENTS_TOTAL: Counter = Counter::new(
        "ticketing_payments_total",
        "Total number of payments processed"
    ).unwrap();

    pub static ref RESERVATION_DURATION: Histogram = Histogram::with_opts(
        HistogramOpts::new(
            "ticketing_reservation_duration_seconds",
            "Time from reservation to completion"
        )
    ).unwrap();
}

pub fn register_metrics(registry: &Registry) {
    registry.register(Box::new(RESERVATIONS_TOTAL.clone())).unwrap();
    registry.register(Box::new(PAYMENTS_TOTAL.clone())).unwrap();
    registry.register(Box::new(RESERVATION_DURATION.clone())).unwrap();
}
```

**Usage**:
```rust
// In reservation handler
RESERVATIONS_TOTAL.inc();

// In payment handler
PAYMENTS_TOTAL.inc();

// Track duration
let start = Instant::now();
// ... process reservation ...
RESERVATION_DURATION.observe(start.elapsed().as_secs_f64());
```

**Acceptance Criteria**:
- [ ] Custom business metrics defined
- [ ] Metrics incremented in handlers
- [ ] Prometheus can scrape `/metrics`
- [ ] Grafana dashboard can visualize metrics

**Files to Create**:
- `src/server/metrics.rs`

**Files to Modify**:
- `src/api/reservations.rs` (increment metrics)
- `src/api/payments.rs` (increment metrics)
- `src/server/routes.rs` (add /metrics endpoint)

---

### Task 6.6: Add Graceful Shutdown
**File**: `src/server/main.rs`

**Action**:
1. Handle SIGTERM/SIGINT signals
2. Stop accepting new requests
3. Drain in-flight requests (30-second timeout)
4. Close database connections
5. Flush event bus messages

**Implementation**:
```rust
use tokio::signal;

pub async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received, starting graceful shutdown");
}

#[tokio::main]
async fn main() {
    // ... setup ...

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();

    // Cleanup
    event_store.close().await;
    event_bus.close().await;

    tracing::info!("Graceful shutdown complete");
}
```

**Acceptance Criteria**:
- [ ] SIGTERM triggers graceful shutdown
- [ ] In-flight requests complete before exit
- [ ] Database connections closed cleanly
- [ ] Event bus consumer commits offsets
- [ ] Integration test verifies graceful shutdown

**Files to Modify**:
- `src/server/main.rs`

**Test File**:
- `tests/graceful_shutdown_test.rs`

---

## Phase 12.7: Testing & Validation (HIGH PRIORITY)

**Priority**: HIGH
**Estimated Time**: 12-16 hours
**Dependencies**: Phases 12.1-12.6

### Task 7.1: End-to-End Integration Tests
**Test File**: `tests/e2e_ticket_purchase_test.rs`

**Action**:
1. Create full E2E test: Create event → Add inventory → Reserve → Pay → Complete
2. Use testcontainers for real PostgreSQL + RedPanda + Redis
3. Test with real Stripe test mode
4. Verify all projections updated

**Implementation**:
```rust
#[tokio::test]
async fn test_complete_ticket_purchase_flow() {
    // Setup testcontainers
    let postgres = PostgresContainer::default().start().await;
    let redpanda = RedpandaContainer::default().start().await;
    let redis = RedisContainer::default().start().await;

    // Run migrations
    run_migrations(&postgres).await;

    // Build stores
    let event_store = PostgresEventStore::new(&postgres.connection_string()).await;
    let event_bus = RedpandaEventBus::new(&redpanda.broker_address()).await;
    let projection_store = ProjectionStore::new(&postgres.connection_string()).await;

    // Start projection consumers
    let inventory_projection = InventoryProjection::start(event_bus.clone()).await;
    let reservation_projection = ReservationProjection::start(event_bus.clone()).await;
    let payment_projection = PaymentProjection::start(event_bus.clone()).await;

    // Test flow
    // Step 1: Create event
    let event_id = create_event_via_api(&event_store, "Concert", "2025-12-31").await;

    // Step 2: Add inventory
    add_inventory_via_api(&event_store, event_id, "VIP", 100, 150.0).await;

    // Wait for projection
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Step 3: Reserve seats
    let reservation_id = reserve_seats_via_api(&event_store, event_id, "VIP", 2).await;

    // Wait for saga
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify reservation status
    let reservation = query_reservation(&projection_store, reservation_id).await.unwrap();
    assert_eq!(reservation.status, ReservationStatus::SeatsReserved);

    // Step 4: Process payment
    let payment_id = process_payment_via_api(
        &event_store,
        reservation_id,
        PaymentMethod::CreditCard {
            stripe_token: "tok_visa".to_string(), // Stripe test token
        },
    ).await;

    // Wait for saga completion
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify reservation completed
    let reservation = query_reservation(&projection_store, reservation_id).await.unwrap();
    assert_eq!(reservation.status, ReservationStatus::Completed);

    // Verify payment completed
    let payment = query_payment(&projection_store, payment_id).await.unwrap();
    assert_eq!(payment.status, PaymentStatus::Completed);

    // Verify inventory updated
    let inventory = query_inventory(&projection_store, event_id, "VIP").await.unwrap();
    assert_eq!(inventory.available(), 98); // 100 - 2
    assert_eq!(inventory.reserved(), 0);
    assert_eq!(inventory.sold(), 2);
}
```

**Acceptance Criteria**:
- [ ] E2E test covers complete flow: create → reserve → pay → complete
- [ ] Test uses real dependencies (PostgreSQL, RedPanda, Redis)
- [ ] Test uses Stripe test mode
- [ ] All projections verified
- [ ] Test passes consistently (no flakiness)

**Files to Create**:
- `tests/e2e_ticket_purchase_test.rs`

---

### Task 7.2: Saga Timeout Test
**Test File**: `tests/saga_timeout_test.rs`

**Action**:
1. Create reservation
2. Wait 5 minutes (use FixedClock to advance time)
3. Verify reservation expires
4. Verify seats released (compensation)

**Implementation**:
```rust
#[tokio::test]
async fn test_reservation_timeout_triggers_compensation() {
    // Setup with FixedClock
    let clock = Arc::new(FixedClock::new(test_time()));

    // Create reservation
    let reservation_id = create_reservation(...).await;

    // Verify initial state
    let reservation = query_reservation(reservation_id).await.unwrap();
    assert_eq!(reservation.status, ReservationStatus::SeatsReserved);

    // Advance clock by 5 minutes
    clock.advance(Duration::from_secs(5 * 60));

    // Trigger timeout (in production, this is scheduled effect)
    send_timeout_action(reservation_id).await;

    // Wait for compensation
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify reservation expired
    let reservation = query_reservation(reservation_id).await.unwrap();
    assert_eq!(reservation.status, ReservationStatus::Expired);

    // Verify seats released
    let inventory = query_inventory(event_id, section).await.unwrap();
    assert_eq!(inventory.reserved(), 0);
    assert_eq!(inventory.available(), original_available);
}
```

**Acceptance Criteria**:
- [ ] Test verifies timeout triggers compensation
- [ ] Seats released when reservation expires
- [ ] Test uses FixedClock for time control

**Files to Create**:
- `tests/saga_timeout_test.rs`

---

### Task 7.3: Payment Failure Test
**Test File**: `tests/payment_failure_test.rs`

**Action**:
1. Create reservation
2. Process payment with failing payment method
3. Verify payment fails
4. Verify reservation compensated
5. Verify seats released

**Implementation**:
```rust
#[tokio::test]
async fn test_payment_failure_triggers_compensation() {
    // Create reservation
    let reservation_id = create_reservation(...).await;

    // Process payment with failing card
    let result = process_payment(
        reservation_id,
        PaymentMethod::CreditCard {
            stripe_token: "tok_chargeDeclined".to_string(), // Stripe test token
        },
    ).await;

    // Verify payment failed
    assert!(result.is_err());

    // Wait for compensation
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify reservation compensated
    let reservation = query_reservation(reservation_id).await.unwrap();
    assert_eq!(reservation.status, ReservationStatus::Compensated);

    // Verify seats released
    let inventory = query_inventory(event_id, section).await.unwrap();
    assert_eq!(inventory.reserved(), 0);
}
```

**Acceptance Criteria**:
- [ ] Test verifies payment failure triggers compensation
- [ ] Reservation moves to Compensated status
- [ ] Seats released

**Files to Create**:
- `tests/payment_failure_test.rs`

---

### Task 7.4: Concurrency Tests
**Test File**: `tests/concurrency_stress_test.rs`

**Action**:
1. Add inventory with 1 seat
2. Launch 100 concurrent reservation attempts
3. Verify exactly 1 reservation succeeds
4. Verify 99 reservations fail with "insufficient inventory"

**Implementation**:
```rust
#[tokio::test]
async fn test_last_seat_concurrency() {
    // Add inventory with 1 seat
    add_inventory(event_id, "VIP", 1, 100.0).await;

    // Launch 100 concurrent reservation attempts
    let mut handles = vec![];
    for i in 0..100 {
        let handle = tokio::spawn(async move {
            reserve_seats(event_id, "VIP", 1).await
        });
        handles.push(handle);
    }

    // Collect results
    let results: Vec<Result<_, _>> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // Verify exactly 1 success
    let successes: Vec<_> = results.iter().filter(|r| r.is_ok()).collect();
    assert_eq!(successes.len(), 1, "Exactly 1 reservation should succeed");

    // Verify 99 failures
    let failures: Vec<_> = results.iter().filter(|r| r.is_err()).collect();
    assert_eq!(failures.len(), 99, "Exactly 99 reservations should fail");

    // Verify inventory
    let inventory = query_inventory(event_id, "VIP").await.unwrap();
    assert_eq!(inventory.available(), 0);
    assert_eq!(inventory.reserved(), 1);
}
```

**Acceptance Criteria**:
- [ ] Test proves no double-booking under concurrency
- [ ] Exactly 1 winner for last seat
- [ ] Test passes consistently (run 10 times)

**Files to Create**:
- `tests/concurrency_stress_test.rs`

---

### Task 7.5: Authorization Tests
**Test File**: `tests/auth_authorization_test.rs`

**Action**:
1. Create reservation as User A
2. Try to access reservation as User B
3. Verify 403 Forbidden
4. Repeat for payments

**Implementation**:
```rust
#[tokio::test]
async fn test_cannot_access_other_users_reservation() {
    // Create reservation as User A
    let user_a_token = create_user_and_login("alice@example.com").await;
    let reservation_id = create_reservation_with_auth(event_id, user_a_token).await;

    // Try to access as User B
    let user_b_token = create_user_and_login("bob@example.com").await;
    let result = get_reservation_with_auth(reservation_id, user_b_token).await;

    // Verify 403 Forbidden
    assert_eq!(result.status(), StatusCode::FORBIDDEN);
}
```

**Acceptance Criteria**:
- [ ] Users cannot access other users' reservations
- [ ] Users cannot access other users' payments
- [ ] Proper 403 Forbidden responses

**Files to Create**:
- `tests/auth_authorization_test.rs`

---

### Task 7.6: Load Testing
**Tool**: `k6` (Grafana k6)

**Action**:
1. Write k6 script for realistic load
2. Target: 100 concurrent users, 10 reservations/second
3. Run for 5 minutes
4. Measure: p95 latency, error rate, throughput

**Script**:
```javascript
// load_test.js
import http from 'k6/http';
import { check, sleep } from 'k6';

export let options = {
  stages: [
    { duration: '1m', target: 50 },   // Ramp up to 50 users
    { duration: '3m', target: 100 },  // Stay at 100 users
    { duration: '1m', target: 0 },    // Ramp down
  ],
  thresholds: {
    http_req_duration: ['p(95)<500'], // 95% of requests under 500ms
    http_req_failed: ['rate<0.01'],   // Error rate < 1%
  },
};

export default function () {
  // Login
  let loginRes = http.post('http://localhost:8080/auth/magic-link/request',
    JSON.stringify({ email: 'test@example.com' }));

  check(loginRes, { 'login successful': (r) => r.status === 200 });

  // Get token from response (testing mode)
  let token = JSON.parse(loginRes.body).magic_link_token;

  // Verify magic link
  let verifyRes = http.post('http://localhost:8080/auth/magic-link/verify',
    JSON.stringify({ token: token }));

  let sessionToken = JSON.parse(verifyRes.body).session_token;

  // Reserve seats
  let reserveRes = http.post('http://localhost:8080/reservations',
    JSON.stringify({
      event_id: 'test-event-id',
      section: 'VIP',
      quantity: 2,
    }),
    {
      headers: {
        'Authorization': `Bearer ${sessionToken}`,
        'Content-Type': 'application/json',
      },
    }
  );

  check(reserveRes, { 'reservation successful': (r) => r.status === 201 });

  sleep(1); // Think time
}
```

**Run**:
```bash
k6 run load_test.js
```

**Acceptance Criteria**:
- [ ] p95 latency < 500ms
- [ ] Error rate < 1%
- [ ] Throughput > 10 reservations/second
- [ ] No database connection exhaustion
- [ ] No memory leaks

**Files to Create**:
- `scripts/load_test.js`

---

## Phase 12.8: Deployment Preparation (CRITICAL)

**Priority**: CRITICAL
**Estimated Time**: 6-8 hours
**Dependencies**: All previous phases

### Task 8.1: Docker Configuration
**File**: `Dockerfile`

**Action**:
1. Create multi-stage Dockerfile for optimal image size
2. Use Rust builder stage + minimal runtime stage
3. Copy only necessary binaries

**Implementation**:
```dockerfile
# Builder stage
FROM rust:1.85-slim as builder

WORKDIR /app

# Install dependencies
RUN apt-get update && apt-get install -y \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY core/Cargo.toml core/
COPY runtime/Cargo.toml runtime/
COPY postgres/Cargo.toml postgres/
COPY redpanda/Cargo.toml redpanda/
COPY projections/Cargo.toml projections/
COPY web/Cargo.toml web/
COPY auth/Cargo.toml auth/
COPY examples/ticketing/Cargo.toml examples/ticketing/

# Build dependencies (cache layer)
RUN mkdir -p core/src runtime/src postgres/src redpanda/src projections/src web/src auth/src examples/ticketing/src && \
    echo "fn main() {}" > examples/ticketing/src/main.rs && \
    cargo build --release --package ticketing && \
    rm -rf target/release/.fingerprint/ticketing-*

# Copy source
COPY . .

# Build application
RUN cargo build --release --package ticketing

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy binary
COPY --from=builder /app/target/release/ticketing /app/ticketing

# Copy migrations
COPY examples/ticketing/migrations /app/migrations

# Expose ports
EXPOSE 8080 9090

# Run
CMD ["/app/ticketing"]
```

**Acceptance Criteria**:
- [ ] Docker image builds successfully
- [ ] Image size < 100MB (runtime stage)
- [ ] Binary runs correctly in container
- [ ] Migrations bundled in image

**Files to Create**:
- `examples/ticketing/Dockerfile`

---

### Task 8.2: Docker Compose for Local Development
**File**: `docker-compose.yml`

**Action**:
1. Define services: app, postgres (events), postgres (projections), postgres (auth), redis, redpanda
2. Configure networks and volumes
3. Add health checks

**Implementation**:
```yaml
version: '3.8'

services:
  # Application
  ticketing:
    build:
      context: ../..
      dockerfile: examples/ticketing/Dockerfile
    ports:
      - "8080:8080"
      - "9090:9090"
    environment:
      DATABASE_URL: postgres://postgres:postgres@postgres-events:5432/ticketing_events
      PROJECTION_DATABASE_URL: postgres://postgres:postgres@postgres-projections:5432/ticketing_projections
      REDIS_URL: redis://redis:6379
      REDPANDA_BROKERS: redpanda:9092
      AUTH_BASE_URL: http://localhost:8080
      AUTH_JWT_SECRET: ${AUTH_JWT_SECRET:-dev-secret-change-in-production}
      STRIPE_API_KEY: ${STRIPE_API_KEY}
      STRIPE_WEBHOOK_SECRET: ${STRIPE_WEBHOOK_SECRET}
      RUST_LOG: info
    depends_on:
      postgres-events:
        condition: service_healthy
      postgres-projections:
        condition: service_healthy
      postgres-auth:
        condition: service_healthy
      redis:
        condition: service_healthy
      redpanda:
        condition: service_healthy
    restart: unless-stopped

  # PostgreSQL - Events (write side)
  postgres-events:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: ticketing_events
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: postgres
    ports:
      - "5432:5432"
    volumes:
      - postgres-events-data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 5s
      timeout: 5s
      retries: 5

  # PostgreSQL - Projections (read side)
  postgres-projections:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: ticketing_projections
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: postgres
    ports:
      - "5433:5432"
    volumes:
      - postgres-projections-data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 5s
      timeout: 5s
      retries: 5

  # PostgreSQL - Auth
  postgres-auth:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: ticketing_auth
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: postgres
    ports:
      - "5435:5432"
    volumes:
      - postgres-auth-data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 5s
      timeout: 5s
      retries: 5

  # Redis - Sessions and rate limiting
  redis:
    image: redis:7-alpine
    ports:
      - "6379:6379"
    volumes:
      - redis-data:/data
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 5s
      timeout: 3s
      retries: 5

  # RedPanda - Event bus
  redpanda:
    image: vectorized/redpanda:latest
    command:
      - redpanda
      - start
      - --smp 1
      - --memory 1G
      - --reserve-memory 0M
      - --overprovisioned
      - --node-id 0
      - --kafka-addr PLAINTEXT://0.0.0.0:29092,OUTSIDE://0.0.0.0:9092
      - --advertise-kafka-addr PLAINTEXT://redpanda:29092,OUTSIDE://localhost:9092
    ports:
      - "9092:9092"
      - "9644:9644"
    volumes:
      - redpanda-data:/var/lib/redpanda/data
    healthcheck:
      test: ["CMD-SHELL", "rpk cluster health | grep -E 'Healthy:.+true'"]
      interval: 10s
      timeout: 5s
      retries: 5

volumes:
  postgres-events-data:
  postgres-projections-data:
  postgres-auth-data:
  redis-data:
  redpanda-data:
```

**Acceptance Criteria**:
- [ ] `docker-compose up` starts all services
- [ ] Application connects to all dependencies
- [ ] Health checks pass for all services
- [ ] Data persists across restarts (volumes)

**Files to Create**:
- `examples/ticketing/docker-compose.yml`

---

### Task 8.3: Kubernetes Manifests
**Directory**: `examples/ticketing/k8s/`

**Action**:
1. Create Deployment manifest
2. Create Service manifest
3. Create ConfigMap for configuration
4. Create Secrets for sensitive data
5. Create Ingress for HTTPS

**Implementation**:
```yaml
# k8s/deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ticketing
  labels:
    app: ticketing
spec:
  replicas: 3
  selector:
    matchLabels:
      app: ticketing
  template:
    metadata:
      labels:
        app: ticketing
    spec:
      containers:
      - name: ticketing
        image: your-registry/ticketing:latest
        ports:
        - containerPort: 8080
          name: http
        - containerPort: 9090
          name: metrics
        env:
        - name: DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: ticketing-secrets
              key: database-url
        - name: REDIS_URL
          valueFrom:
            secretKeyRef:
              name: ticketing-secrets
              key: redis-url
        - name: STRIPE_API_KEY
          valueFrom:
            secretKeyRef:
              name: ticketing-secrets
              key: stripe-api-key
        envFrom:
        - configMapRef:
            name: ticketing-config
        livenessProbe:
          httpGet:
            path: /health/live
            port: 8080
          initialDelaySeconds: 10
          periodSeconds: 10
        readinessProbe:
          httpGet:
            path: /health/ready
            port: 8080
          initialDelaySeconds: 5
          periodSeconds: 5
        resources:
          requests:
            memory: "256Mi"
            cpu: "250m"
          limits:
            memory: "512Mi"
            cpu: "500m"

---
# k8s/service.yaml
apiVersion: v1
kind: Service
metadata:
  name: ticketing
spec:
  selector:
    app: ticketing
  ports:
  - name: http
    port: 80
    targetPort: 8080
  - name: metrics
    port: 9090
    targetPort: 9090
  type: LoadBalancer

---
# k8s/configmap.yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: ticketing-config
data:
  RUST_LOG: "info"
  HOST: "0.0.0.0"
  PORT: "8080"
  REDPANDA_BROKERS: "redpanda:9092"
  AUTH_BASE_URL: "https://ticketing.example.com"

---
# k8s/secrets.yaml (template - populate with real values)
apiVersion: v1
kind: Secret
metadata:
  name: ticketing-secrets
type: Opaque
stringData:
  database-url: "postgres://postgres:password@postgres:5432/ticketing_events"
  projection-database-url: "postgres://postgres:password@postgres:5432/ticketing_projections"
  redis-url: "redis://redis:6379"
  auth-jwt-secret: "your-secret-here"
  stripe-api-key: "sk_live_..."
  stripe-webhook-secret: "whsec_..."

---
# k8s/ingress.yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: ticketing
  annotations:
    cert-manager.io/cluster-issuer: "letsencrypt-prod"
spec:
  tls:
  - hosts:
    - ticketing.example.com
    secretName: ticketing-tls
  rules:
  - host: ticketing.example.com
    http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: ticketing
            port:
              number: 80
```

**Acceptance Criteria**:
- [ ] Manifests deploy successfully to Kubernetes
- [ ] Application scales to 3 replicas
- [ ] Health checks work correctly
- [ ] Ingress provides HTTPS access
- [ ] Secrets managed securely

**Files to Create**:
- `examples/ticketing/k8s/deployment.yaml`
- `examples/ticketing/k8s/service.yaml`
- `examples/ticketing/k8s/configmap.yaml`
- `examples/ticketing/k8s/secrets.yaml.template`
- `examples/ticketing/k8s/ingress.yaml`

---

### Task 8.4: Database Migration Strategy
**File**: `examples/ticketing/migrations/README.md`

**Action**:
1. Document migration process
2. Add rollback strategy
3. Add zero-downtime migration guide

**Documentation**:
```markdown
# Database Migrations

## Running Migrations

### Development
```bash
# Events database
DATABASE_URL="postgres://..." sqlx migrate run

# Projections database
PROJECTION_DATABASE_URL="postgres://..." sqlx migrate run

# Auth database
AUTH_DATABASE_URL="postgres://..." sqlx migrate run
```

### Production

**Zero-Downtime Strategy:**

1. **Expand Phase**: Add new columns/tables (backward compatible)
   ```bash
   kubectl exec -it deployment/ticketing -- /app/ticketing migrate
   ```

2. **Migrate Phase**: Deploy new application version
   ```bash
   kubectl rollout restart deployment/ticketing
   kubectl rollout status deployment/ticketing
   ```

3. **Contract Phase**: Remove old columns/tables (after old version drained)
   ```bash
   # Run cleanup migration
   psql $DATABASE_URL -f migrations/cleanup.sql
   ```

## Rollback Strategy

### Application Rollback
```bash
kubectl rollout undo deployment/ticketing
```

### Database Rollback
```bash
# Manual rollback - migrations are one-way
# Use point-in-time recovery or manual ALTER TABLE statements
# See migrations/rollback/ directory for rollback scripts
```

## Best Practices

1. **Always test migrations in staging first**
2. **Backup database before migration**
3. **Migrations should be idempotent** (ON CONFLICT DO NOTHING)
4. **Never drop columns in first deployment** (expand → migrate → contract)
5. **Monitor application errors during migration**
```

**Acceptance Criteria**:
- [ ] Migration process documented
- [ ] Rollback strategy defined
- [ ] Zero-downtime strategy explained
- [ ] Best practices listed

**Files to Create**:
- `examples/ticketing/migrations/README.md`

---

### Task 8.5: Environment Configuration
**File**: `.env.example`

**Action**:
1. Document all environment variables
2. Provide example values
3. Mark required vs optional

**Implementation**:
```bash
# Database Configuration (REQUIRED)
DATABASE_URL=postgres://postgres:postgres@localhost:5432/ticketing_events
PROJECTION_DATABASE_URL=postgres://postgres:postgres@localhost:5433/ticketing_projections
AUTH_DATABASE_URL=postgres://postgres:postgres@localhost:5435/ticketing_auth

# Redis Configuration (REQUIRED)
REDIS_URL=redis://localhost:6379

# Event Bus Configuration (REQUIRED)
REDPANDA_BROKERS=localhost:9092
CONSUMER_GROUP=ticketing-projections

# Event Topics (OPTIONAL - defaults provided)
INVENTORY_TOPIC=ticketing-inventory-events
RESERVATION_TOPIC=ticketing-reservation-events
PAYMENT_TOPIC=ticketing-payment-events

# Server Configuration (OPTIONAL - defaults provided)
HOST=0.0.0.0
PORT=8080
RUST_LOG=info
METRICS_HOST=0.0.0.0
METRICS_PORT=9090

# Authentication Configuration (REQUIRED in production)
AUTH_BASE_URL=http://localhost:8080
AUTH_JWT_SECRET=dev-secret-change-in-production
AUTH_SESSION_TTL=604800  # 7 days
AUTH_MAGIC_LINK_TTL=900  # 15 minutes
AUTH_EXPOSE_MAGIC_LINKS_FOR_TESTING=false  # MUST be false in production

# Payment Gateway Configuration (REQUIRED in production)
STRIPE_API_KEY=sk_test_...  # Use sk_live_... in production
STRIPE_WEBHOOK_SECRET=whsec_...
PAYMENT_TIMEOUT_SECONDS=30

# Database Connection Pooling (OPTIONAL - defaults provided)
DATABASE_MAX_CONNECTIONS=10
DATABASE_MIN_CONNECTIONS=2
DATABASE_CONNECT_TIMEOUT=30
DATABASE_STATEMENT_TIMEOUT=60

# Security Configuration (OPTIONAL - defaults provided)
AUTH_MAX_CONCURRENT_SESSIONS=5
AUTH_RATE_LIMIT_REQUESTS=10
AUTH_RATE_LIMIT_WINDOW=60
```

**Acceptance Criteria**:
- [ ] All environment variables documented
- [ ] Example values provided
- [ ] Required vs optional marked
- [ ] Security warnings included

**Files to Create**:
- `examples/ticketing/.env.example`

---

### Task 8.6: Deployment Checklist
**File**: `examples/ticketing/DEPLOYMENT.md`

**Action**:
1. Create pre-deployment checklist
2. Document deployment steps
3. Add post-deployment verification

**Documentation**:
```markdown
# Deployment Checklist

## Pre-Deployment

### 1. Code Quality
- [ ] All tests passing (`cargo test --all-features`)
- [ ] No clippy warnings (`cargo clippy --all-targets --all-features -- -D warnings`)
- [ ] Code formatted (`cargo fmt --all --check`)
- [ ] Documentation builds (`cargo doc --no-deps --all-features`)

### 2. Security Audit
- [ ] No TODOs or FIXME comments in production code paths
- [ ] No hardcoded secrets or credentials
- [ ] `AUTH_EXPOSE_MAGIC_LINKS_FOR_TESTING=false` in production
- [ ] HTTPS enabled (TLS certificates valid)
- [ ] Database passwords rotated
- [ ] Stripe API keys are `sk_live_...` (not test keys)

### 3. Database
- [ ] Backup created and verified
- [ ] Migrations tested in staging
- [ ] Connection pool sizes appropriate for load
- [ ] Indexes created for all query patterns

### 4. Infrastructure
- [ ] PostgreSQL cluster healthy (events, projections, auth)
- [ ] Redis cluster healthy
- [ ] RedPanda cluster healthy
- [ ] Monitoring/alerting configured (Prometheus, Grafana, PagerDuty)
- [ ] Log aggregation configured (ELK, Datadog, etc.)

### 5. Load Testing
- [ ] Load tests passed in staging
- [ ] p95 latency < 500ms
- [ ] Error rate < 1%
- [ ] No memory leaks detected

## Deployment Steps

### 1. Deploy Application
```bash
# Build and push Docker image
docker build -t your-registry/ticketing:v1.0.0 .
docker push your-registry/ticketing:v1.0.0

# Update Kubernetes deployment
kubectl set image deployment/ticketing ticketing=your-registry/ticketing:v1.0.0

# Watch rollout
kubectl rollout status deployment/ticketing
```

### 2. Run Migrations
```bash
# Events database
kubectl exec -it deployment/ticketing -- /app/ticketing migrate --database events

# Projections database
kubectl exec -it deployment/ticketing -- /app/ticketing migrate --database projections

# Auth database
kubectl exec -it deployment/ticketing -- /app/ticketing migrate --database auth
```

### 3. Verify Deployment
```bash
# Check pods
kubectl get pods -l app=ticketing

# Check logs
kubectl logs -f deployment/ticketing

# Check health endpoint
curl https://ticketing.example.com/health
```

## Post-Deployment Verification

### 1. Smoke Tests
- [ ] Health check returns 200 (`GET /health`)
- [ ] Readiness check returns 200 (`GET /health/ready`)
- [ ] Metrics endpoint accessible (`GET /metrics`)

### 2. Functional Tests
- [ ] User can register/login
- [ ] User can browse events
- [ ] User can reserve seats
- [ ] User can complete payment
- [ ] User can view reservations

### 3. Monitoring
- [ ] Error rate < 1% (Prometheus)
- [ ] p95 latency < 500ms (Prometheus)
- [ ] No memory leaks (Grafana)
- [ ] No database connection exhaustion (Grafana)
- [ ] Event bus consumer lag < 100 messages (RedPanda console)

### 4. Alerts
- [ ] PagerDuty alerts configured
- [ ] On-call engineer notified
- [ ] Runbook accessible

## Rollback Procedure

If issues detected:

```bash
# Rollback application
kubectl rollout undo deployment/ticketing

# Verify rollback
kubectl rollout status deployment/ticketing

# Rollback database (if needed)
# See migrations/README.md for rollback strategy
```

## Production Monitoring

Monitor for first 24 hours:
- Error rates
- Latency (p50, p95, p99)
- Throughput (requests/second)
- Database connection pool usage
- Event bus consumer lag
- Payment gateway success rate
```

**Acceptance Criteria**:
- [ ] Checklist covers all critical steps
- [ ] Deployment steps documented
- [ ] Verification steps included
- [ ] Rollback procedure defined

**Files to Create**:
- `examples/ticketing/DEPLOYMENT.md`

---

## Summary

### Estimated Total Time
- **Phase 12.1**: 2-3 hours (Event persistence)
- **Phase 12.2**: 16-20 hours (API integration)
- **Phase 12.3**: 6-8 hours (Authorization)
- **Phase 12.4**: 8-10 hours (Query layer)
- **Phase 12.5**: 12-16 hours (Payment gateway)
- **Phase 12.6**: 8-10 hours (Production hardening)
- **Phase 12.7**: 12-16 hours (Testing)
- **Phase 12.8**: 6-8 hours (Deployment prep)

**Total**: 70-91 hours (~2-3 weeks full-time)

### Critical Path
1. Event Aggregate Persistence (BLOCKER) → 2-3 hours
2. API-to-Aggregate Integration (BLOCKER) → 16-20 hours
3. Authorization (SECURITY) → 6-8 hours
4. Testing → 12-16 hours
5. Deployment → 6-8 hours

**Minimum Viable Production**: ~42-55 hours (Phases 12.1, 12.2, 12.3, 12.7, 12.8)

### Success Criteria

Application is production-ready when:
- [ ] All API endpoints call real aggregates (no TODOs)
- [ ] Authorization enforced (no fake user IDs)
- [ ] Payment gateway integrated (Stripe)
- [ ] Query layer implemented (projections + query adapters)
- [ ] E2E tests passing (create → reserve → pay → complete)
- [ ] Load tests passing (100 concurrent users, <500ms p95)
- [ ] Health checks working
- [ ] Monitoring/alerting configured
- [ ] Docker + Kubernetes deployment tested
- [ ] Deployment checklist complete

### Production Readiness Scorecard

| Category | Before | After |
|----------|--------|-------|
| **Domain Logic** | 95% | 95% |
| **Infrastructure Integration** | 15% | 95% |
| **Security** | 40% | 95% |
| **Testing** | 70% | 95% |
| **Observability** | 60% | 90% |
| **Documentation** | 70% | 95% |
| **Deployment** | 0% | 95% |
| **Overall** | 60% | 95% |

---

## Next Steps

1. Start with **Phase 12.1** (Event Aggregate Persistence) - this unblocks everything else
2. Move to **Phase 12.2** (API Integration) - this is the bulk of the work
3. Prioritize **Phase 12.3** (Authorization) for security
4. Complete **Phase 12.7** (Testing) to gain confidence
5. Finish with **Phase 12.8** (Deployment) to go live

**Recommendation**: Work through phases sequentially. Each phase has clear acceptance criteria. Test thoroughly at each step before moving forward.
