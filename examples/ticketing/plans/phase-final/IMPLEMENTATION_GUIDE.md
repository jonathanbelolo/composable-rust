# Stream ID Fix - Implementation Quick Reference

**Purpose**: Concrete before/after examples for each file type
**Use**: Copy-paste patterns during implementation

---

## Pattern 1: Store Creation Methods (server/state.rs)

### Payment Store

```rust
// ❌ BEFORE - Singleton stream
pub fn create_payment_store(
    &self,
) -> Store<PaymentState, PaymentAction, PaymentEnvironment, PaymentReducer> {
    let env = PaymentEnvironment::new(
        self.clock.clone(),
        self.event_store.clone(),
        self.event_bus.clone(),
        StreamId::new("payment"),  // 🔴 WRONG
        self.payment_query.clone(),
    );
    Store::new(PaymentState::new(), PaymentReducer::new(), env)
}

// ✅ AFTER - Per-instance stream
pub fn create_payment_store(
    &self,
    payment_id: PaymentId,  // ← ADD THIS
) -> Store<PaymentState, PaymentAction, PaymentEnvironment, PaymentReducer> {
    let env = PaymentEnvironment::new(
        self.clock.clone(),
        self.event_store.clone(),
        self.event_bus.clone(),
        StreamId::new(&format!("payment-{}", payment_id.as_uuid())),  // ✅ CORRECT
        self.payment_query.clone(),
    );
    Store::new(PaymentState::new(), PaymentReducer::new(), env)
}
```

### Reservation Store

```rust
// ❌ BEFORE
pub fn create_reservation_store(&self) -> Store<...> {
    StreamId::new("reservation")  // 🔴 WRONG
}

// ✅ AFTER
pub fn create_reservation_store(&self, reservation_id: ReservationId) -> Store<...> {
    StreamId::new(&format!("reservation-{}", reservation_id.as_uuid()))  // ✅ CORRECT
}
```

### Event Store

```rust
// ❌ BEFORE
pub fn create_event_store(&self) -> Store<...> {
    StreamId::new("event")  // 🔴 WRONG
}

// ✅ AFTER
pub fn create_event_store(&self, event_id: EventId) -> Store<...> {
    StreamId::new(&format!("event-{}", event_id.as_uuid()))  // ✅ CORRECT
}
```

### Inventory Store

```rust
// ❌ BEFORE
pub fn create_inventory_store(&self) -> Store<...> {
    StreamId::new("inventory")  // 🔴 WRONG
}

// ✅ AFTER
pub fn create_inventory_store(&self, event_id: EventId) -> Store<...> {
    StreamId::new(&format!("inventory-{}", event_id.as_uuid()))  // ✅ CORRECT
}
```

---

## Pattern 2: CREATE Endpoints (New Entity)

### Payment - Process Payment

```rust
// ❌ BEFORE
pub async fn process_payment(
    session: SessionUser,
    Extension(correlation_uuid): Extension<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<ProcessPaymentRequest>,
) -> Result<(StatusCode, Json<ProcessPaymentResponse>), AppError> {
    // ... validation ...

    let payment_id = PaymentId::new();  // Generate ID

    let store = state.create_payment_store();  // 🔴 MISSING ID

    let action = PaymentAction::ProcessPayment {
        payment_id,
        // ...
    };

    store.send_with_metadata(action, Some(metadata)).await?;
    // ...
}

// ✅ AFTER
pub async fn process_payment(
    session: SessionUser,
    Extension(correlation_uuid): Extension<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<ProcessPaymentRequest>,
) -> Result<(StatusCode, Json<ProcessPaymentResponse>), AppError> {
    // ... validation ...

    let payment_id = PaymentId::new();  // Generate ID

    let store = state.create_payment_store(payment_id);  // ✅ PASS ID

    let action = PaymentAction::ProcessPayment {
        payment_id,
        // ...
    };

    store.send_with_metadata(action, Some(metadata)).await?;
    // ...
}
```

### Reservation - Create Reservation

```rust
// ❌ BEFORE
pub async fn create_reservation(...) -> Result<...> {
    let reservation_id = ReservationId::new();
    let store = state.create_reservation_store();  // 🔴 MISSING ID
    // ...
}

// ✅ AFTER
pub async fn create_reservation(...) -> Result<...> {
    let reservation_id = ReservationId::new();
    let store = state.create_reservation_store(reservation_id);  // ✅ PASS ID
    // ...
}
```

### Event - Create Event

```rust
// ❌ BEFORE
pub async fn create_event(...) -> Result<...> {
    let event_id = EventId::new();
    let store = state.create_event_store();  // 🔴 MISSING ID
    // ...
}

// ✅ AFTER
pub async fn create_event(...) -> Result<...> {
    let event_id = EventId::new();
    let store = state.create_event_store(event_id);  // ✅ PASS ID
    // ...
}
```

---

## Pattern 3: READ Endpoints (Existing Entity)

### Payment - Get Payment

```rust
// ❌ BEFORE
pub async fn get_payment(
    Path(payment_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<PaymentResponse>, AppError> {
    let payment_id_typed = PaymentId::from_uuid(payment_id);
    let store = state.create_payment_store();  // 🔴 WRONG STREAM

    let result = store.send_and_wait_for(
        PaymentAction::GetPayment { payment_id: payment_id_typed },
        |action| matches!(action, PaymentAction::PaymentQueried { .. }),
        Duration::from_secs(5),
    ).await?;
    // ...
}

// ✅ AFTER
pub async fn get_payment(
    Path(payment_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<PaymentResponse>, AppError> {
    let payment_id_typed = PaymentId::from_uuid(payment_id);
    let store = state.create_payment_store(payment_id_typed);  // ✅ CORRECT STREAM

    let result = store.send_and_wait_for(
        PaymentAction::GetPayment { payment_id: payment_id_typed },
        |action| matches!(action, PaymentAction::PaymentQueried { .. }),
        Duration::from_secs(5),
    ).await?;
    // ...
}
```

### Reservation - Get Reservation

```rust
// ❌ BEFORE
pub async fn get_reservation(Path(reservation_id): Path<Uuid>, ...) -> Result<...> {
    let reservation_id_typed = ReservationId::from_uuid(reservation_id);
    let store = state.create_reservation_store();  // 🔴 WRONG STREAM
    // ...
}

// ✅ AFTER
pub async fn get_reservation(Path(reservation_id): Path<Uuid>, ...) -> Result<...> {
    let reservation_id_typed = ReservationId::from_uuid(reservation_id);
    let store = state.create_reservation_store(reservation_id_typed);  // ✅ CORRECT STREAM
    // ...
}
```

### Event - Get Event

```rust
// ❌ BEFORE
pub async fn get_event(Path(event_id): Path<Uuid>, ...) -> Result<...> {
    let event_id_typed = EventId::from_uuid(event_id);
    let store = state.create_event_store();  // 🔴 WRONG STREAM
    // ...
}

// ✅ AFTER
pub async fn get_event(Path(event_id): Path<Uuid>, ...) -> Result<...> {
    let event_id_typed = EventId::from_uuid(event_id);
    let store = state.create_event_store(event_id_typed);  // ✅ CORRECT STREAM
    // ...
}
```

### Inventory - Get Availability

```rust
// ❌ BEFORE
pub async fn get_event_availability(Path(event_id): Path<Uuid>, ...) -> Result<...> {
    let event_id_typed = EventId::from_uuid(event_id);
    let inventory_store = state.create_inventory_store();  // 🔴 WRONG STREAM

    inventory_store.send_and_wait_for(
        InventoryAction::GetAllSections { event_id: event_id_typed },
        // ...
    ).await?;
}

// ✅ AFTER
pub async fn get_event_availability(Path(event_id): Path<Uuid>, ...) -> Result<...> {
    let event_id_typed = EventId::from_uuid(event_id);
    let inventory_store = state.create_inventory_store(event_id_typed);  // ✅ CORRECT STREAM

    inventory_store.send_and_wait_for(
        InventoryAction::GetAllSections { event_id: event_id_typed },
        // ...
    ).await?;
}
```

---

## Pattern 4: UPDATE/DELETE Endpoints

### Payment - Refund Payment

```rust
// ❌ BEFORE - Creates TWO stores (query + command)
pub async fn refund_payment(
    ownership: RequireOwnership<PaymentId>,
    Path(payment_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<RefundPaymentRequest>,
) -> Result<Json<RefundPaymentResponse>, AppError> {
    let payment_id_typed = PaymentId::from_uuid(payment_id);

    // Query store to get current payment state
    let query_store = state.create_payment_store();  // 🔴 WRONG STREAM
    let result = query_store.send_and_wait_for(
        PaymentAction::GetPayment { payment_id: payment_id_typed },
        // ...
    ).await?;

    // ... validate payment status ...

    // Command store to process refund
    let command_store = state.create_payment_store();  // 🔴 WRONG STREAM
    command_store.send(PaymentAction::RefundPayment { ... }).await?;
    // ...
}

// ✅ AFTER - Both stores use correct stream
pub async fn refund_payment(
    ownership: RequireOwnership<PaymentId>,
    Path(payment_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<RefundPaymentRequest>,
) -> Result<Json<RefundPaymentResponse>, AppError> {
    let payment_id_typed = PaymentId::from_uuid(payment_id);

    // Query store to get current payment state
    let query_store = state.create_payment_store(payment_id_typed);  // ✅ CORRECT STREAM
    let result = query_store.send_and_wait_for(
        PaymentAction::GetPayment { payment_id: payment_id_typed },
        // ...
    ).await?;

    // ... validate payment status ...

    // Command store to process refund
    let command_store = state.create_payment_store(payment_id_typed);  // ✅ CORRECT STREAM
    command_store.send(PaymentAction::RefundPayment { ... }).await?;
    // ...
}
```

### Reservation - Cancel Reservation

```rust
// ❌ BEFORE
pub async fn cancel_reservation(Path(reservation_id): Path<Uuid>, ...) -> Result<...> {
    let reservation_id_typed = ReservationId::from_uuid(reservation_id);
    let store = state.create_reservation_store();  // 🔴 WRONG STREAM
    // ...
}

// ✅ AFTER
pub async fn cancel_reservation(Path(reservation_id): Path<Uuid>, ...) -> Result<...> {
    let reservation_id_typed = ReservationId::from_uuid(reservation_id);
    let store = state.create_reservation_store(reservation_id_typed);  // ✅ CORRECT STREAM
    // ...
}
```

### Event - Update/Delete Event

```rust
// ❌ BEFORE
pub async fn update_event(Path(event_id): Path<Uuid>, ...) -> Result<...> {
    let event_id_typed = EventId::from_uuid(event_id);
    let store = state.create_event_store();  // 🔴 WRONG STREAM
    // ...
}

pub async fn delete_event(Path(event_id): Path<Uuid>, ...) -> Result<...> {
    let event_id_typed = EventId::from_uuid(event_id);
    let store = state.create_event_store();  // 🔴 WRONG STREAM
    // ...
}

// ✅ AFTER
pub async fn update_event(Path(event_id): Path<Uuid>, ...) -> Result<...> {
    let event_id_typed = EventId::from_uuid(event_id);
    let store = state.create_event_store(event_id_typed);  // ✅ CORRECT STREAM
    // ...
}

pub async fn delete_event(Path(event_id): Path<Uuid>, ...) -> Result<...> {
    let event_id_typed = EventId::from_uuid(event_id);
    let store = state.create_event_store(event_id_typed);  // ✅ CORRECT STREAM
    // ...
}
```

---

## Pattern 5: LIST Endpoints (Query-Only Operations)

**Rule**: List endpoints query **projections** (not event streams), so use **nil UUID** as placeholder.

**Why**: These operations route through stores for consistency, but reducers query projections directly. The stream ID parameter is required by signature but unused in execution.

### Payment - List User Payments

```rust
// ❌ BEFORE
pub async fn list_user_payments(
    session: SessionUser,
    State(state): State<AppState>,
) -> Result<Json<ListPaymentsResponse>, AppError> {
    let customer_id = CustomerId::from_uuid(session.user_id.0);
    let store = state.create_payment_store();  // 🔴 MISSING ID!

    let result = store.send_and_wait_for(
        PaymentAction::ListCustomerPayments {
            customer_id,
            limit: 100,
            offset: 0,
        },
        // ...
    ).await?;
    // ...
}

// ✅ AFTER
pub async fn list_user_payments(
    session: SessionUser,
    State(state): State<AppState>,
) -> Result<Json<ListPaymentsResponse>, AppError> {
    let customer_id = CustomerId::from_uuid(session.user_id.0);

    // Use nil UUID for query-only operations (no event store access)
    let store = state.create_payment_store(PaymentId::from_uuid(Uuid::nil()));  // ✅

    let result = store.send_and_wait_for(
        PaymentAction::ListCustomerPayments {
            customer_id,
            limit: 100,
            offset: 0,
        },
        // ...
    ).await?;
    // ...
}
```

### Reservation - List User Reservations

```rust
// ❌ BEFORE
pub async fn list_user_reservations(...) -> Result<...> {
    let store = state.create_reservation_store();  // 🔴 MISSING ID!
    // ...
}

// ✅ AFTER
pub async fn list_user_reservations(...) -> Result<...> {
    // Use nil UUID for query-only operations
    let store = state.create_reservation_store(ReservationId::from_uuid(Uuid::nil()));  // ✅
    // ...
}
```

### How It Works

**Reducer Implementation** (example from `aggregates/payment.rs:520`):
```rust
PaymentAction::ListCustomerPayments { customer_id, limit, offset } => {
    let projection = env.projection.clone();  // ← Queries PROJECTION, not event store!
    Effect::Future(async move {
        // Never accesses stream ID - goes straight to projection
        let payments = projection.load_customer_payments(&customer_id, limit, offset).await?;
        Some(PaymentAction::CustomerPaymentsListed { payments })
    })
}
```

**Key Points**:
- Reducer uses `env.projection` (read model), not `env.event_store` (write model)
- Stream ID parameter exists but is never accessed
- Nil UUID signals "query-only operation, no specific instance"

### Alternative: Direct Projection Query (Future Optimization)

If you want to skip the store entirely:
```rust
pub async fn list_user_payments(...) -> Result<...> {
    let customer_id = CustomerId::from_uuid(session.user_id.0);

    // Query projection directly (no store needed)
    let payments = state.payment_query
        .load_customer_payments(&customer_id, 100, 0)
        .await?;

    // Format and return...
}
```

**Trade-offs**:
- ✅ Cleaner, more direct
- ✅ No dummy UUID needed
- ❌ Breaks consistency (other operations go through stores)
- ❌ Bypasses reducer logic (if any validation/transformation needed)

**Recommendation**: Use nil UUID pattern for consistency with existing architecture.

---

## Pattern 6: Unit Tests

### Test Store Creation

```rust
// ❌ BEFORE
#[tokio::test]
async fn test_payment_processing() {
    // Setup
    let clock = Arc::new(SystemClock);
    let event_store = Arc::new(InMemoryEventStore::new());
    let payment_query = Arc::new(MockPaymentQuery::new());

    let env = PaymentEnvironment::new(
        clock,
        event_store,
        Arc::new(InMemoryEventBus::new()),
        StreamId::new("payment"),  // 🔴 WRONG
        payment_query,
    );

    let store = Store::new(PaymentState::new(), PaymentReducer::new(), env);

    // Test...
}

// ✅ AFTER
#[tokio::test]
async fn test_payment_processing() {
    // Setup
    let payment_id = PaymentId::new();  // Generate test ID
    let clock = Arc::new(SystemClock);
    let event_store = Arc::new(InMemoryEventStore::new());
    let payment_query = Arc::new(MockPaymentQuery::new());

    let env = PaymentEnvironment::new(
        clock,
        event_store,
        Arc::new(InMemoryEventBus::new()),
        StreamId::new(&format!("payment-{}", payment_id.as_uuid())),  // ✅ CORRECT
        payment_query,
    );

    let store = Store::new(PaymentState::new(), PaymentReducer::new(), env);

    // Test...
}
```

### Test Helper Functions

```rust
// ❌ BEFORE
fn create_payment_store_for_test() -> Store<...> {
    let env = PaymentEnvironment::new(
        // ...
        StreamId::new("payment"),  // 🔴 WRONG
        // ...
    );
    Store::new(PaymentState::new(), PaymentReducer::new(), env)
}

// ✅ AFTER
fn create_payment_store_for_test(payment_id: PaymentId) -> Store<...> {
    let env = PaymentEnvironment::new(
        // ...
        StreamId::new(&format!("payment-{}", payment_id.as_uuid())),  // ✅ CORRECT
        // ...
    );
    Store::new(PaymentState::new(), PaymentReducer::new(), env)
}
```

---

## Compilation Error Reference

### Expected Error 1: Missing Argument

```
error[E0061]: this function takes 2 arguments but 1 was supplied
  --> src/api/payments.rs:272:21
   |
272|     let store = state.create_payment_store();
   |                       ^^^^^^^^^^^^^^^^^^^^^ expected 2 arguments
   |
note: function defined here
  --> src/server/state.rs:245:12
   |
245|     pub fn create_payment_store(&self, payment_id: PaymentId) -> Store<...> {
   |            ^^^^^^^^^^^^^^^^^^^^ --------------------
```

**Fix**: Add the required payment_id argument:
```rust
let store = state.create_payment_store(payment_id);
```

### Expected Error 2: Type Mismatch

```
error[E0308]: mismatched types
  --> src/api/payments.rs:272:45
   |
272|     let store = state.create_payment_store(payment_id);
   |                                             ^^^^^^^^^^ expected `PaymentId`, found `Uuid`
```

**Fix**: Convert Uuid to PaymentId:
```rust
let payment_id_typed = PaymentId::from_uuid(payment_id);
let store = state.create_payment_store(payment_id_typed);
```

---

## Search Commands for Implementation

### Find All Store Creation Calls

```bash
# Payment stores
grep -rn "create_payment_store()" examples/ticketing/src/ examples/ticketing/tests/

# Reservation stores
grep -rn "create_reservation_store()" examples/ticketing/src/ examples/ticketing/tests/

# Event stores
grep -rn "create_event_store()" examples/ticketing/src/ examples/ticketing/tests/

# Inventory stores
grep -rn "create_inventory_store()" examples/ticketing/src/ examples/ticketing/tests/

# All stores (comprehensive)
grep -rn "\.create_\w*_store()" examples/ticketing/
```

### Verify Fixes

```bash
# After fixes, search for remaining calls without arguments
# (Should return 0 matches)
grep -rn "create_payment_store()" examples/ticketing/src/ examples/ticketing/tests/
grep -rn "create_reservation_store()" examples/ticketing/src/ examples/ticketing/tests/
grep -rn "create_event_store()" examples/ticketing/src/ examples/ticketing/tests/
grep -rn "create_inventory_store()" examples/ticketing/src/ examples/ticketing/tests/
```

---

## Implementation Workflow

1. **Update server/state.rs**:
   - Add parameters to all `create_*_store()` methods
   - Update stream ID construction
   - Compile: `cargo build --all-features`
   - **Expect**: Many compilation errors (missing arguments)

2. **Fix API endpoints** (use compiler errors as guide):
   - Start with `api/payments.rs`
   - Then `api/reservations.rs`
   - Then `api/events.rs`
   - Then `api/availability.rs` (inventory)
   - Compile after each file: `cargo build --all-features`

3. **Fix tests**:
   - Unit tests in `src/aggregates/*/tests.rs`
   - Integration tests in `tests/*.rs`
   - Compile: `cargo test --all-features`

4. **Final verification**:
   ```bash
   cargo build --all-features
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test --all-features
   ```

5. **Clean database and run deployment tests**:
   ```bash
   docker compose down -v
   docker compose up -d
   ./scripts/run-deployment-tests.sh
   ```

---

**Last Updated**: 2025-11-23
**Status**: 📘 Ready for Implementation
