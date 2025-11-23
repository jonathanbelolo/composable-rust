# Stream ID Architecture Fix: From Singleton to Per-Instance Streams

**Status**: 🔴 Critical Bug - Blocking Deployment Tests
**Priority**: P0 - Must Fix Before Production
**Estimated Effort**: 4-6 hours
**Breaking Change**: Yes - Requires Database Cleanup

---

## Executive Summary

### The Problem

All deployment tests are failing with optimistic concurrency conflicts because **all instances of each aggregate type share a single event stream**:

- All payments → stream `"payment"`
- All reservations → stream `"reservation"`
- All events → stream `"event"`
- All inventory → stream `"inventory"`

This violates fundamental event sourcing principles where **each aggregate instance must have its own stream**.

### Why This Is Wrong

In event sourcing:
- **Aggregate Instance** = **Event Stream** (one-to-one mapping)
- Each instance has its own event history
- Stream versions track instance-specific changes

With singleton streams:
- Multiple instances compete for the same stream
- Optimistic concurrency control breaks (all expect version 0)
- First write succeeds, all others fail permanently
- Retries don't help - the conflict is architectural

### Impact

**Current Behavior** (5/6 tests passing):
```
Test 1: create payment A → writes to stream "payment" v0→v1 ✅
Test 2: create payment B → tries stream "payment" v0, finds v1 ❌ CONFLICT
Test 3: create payment C → tries stream "payment" v0, finds v2 ❌ CONFLICT
...
Result: 45 optimistic concurrency conflicts, tests timeout/fail
```

**Correct Behavior** (after fix):
```
Test 1: create payment A → writes to stream "payment-{uuid-A}" v0→v1 ✅
Test 2: create payment B → writes to stream "payment-{uuid-B}" v0→v1 ✅
Test 3: create payment C → writes to stream "payment-{uuid-C}" v0→v1 ✅
...
Result: 0 conflicts, all tests pass
```

---

## Current Architecture Analysis

### How Stores Are Currently Created

**File**: `src/server/state.rs`

```rust
// ❌ WRONG - Creates singleton stream for ALL payments
pub fn create_payment_store(&self) -> Store<...> {
    let env = PaymentEnvironment::new(
        self.clock.clone(),
        self.event_store.clone(),
        self.event_bus.clone(),
        StreamId::new("payment"),  // 🔴 ALL payments share this stream!
        self.payment_query.clone(),
    );
    Store::new(PaymentState::new(), PaymentReducer::new(), env)
}

// Same pattern for all other aggregates:
// - create_reservation_store() → StreamId::new("reservation")
// - create_event_store() → StreamId::new("event")
// - create_inventory_store() → StreamId::new("inventory")
```

### How This Breaks Optimistic Concurrency

**Scenario**: Two concurrent payment requests in deployment tests

```rust
// Request 1: Process payment for reservation A
let payment_id_A = PaymentId::new();
let store_A = state.create_payment_store();  // stream = "payment"
store_A.send(ProcessPayment { payment_id: payment_id_A, ... }).await;
// Appends to stream "payment" at version 0 → succeeds, now v1

// Request 2: Process payment for reservation B (concurrent or sequential)
let payment_id_B = PaymentId::new();
let store_B = state.create_payment_store();  // stream = "payment" (SAME!)
store_B.send(ProcessPayment { payment_id: payment_id_B, ... }).await;
// Tries to append to stream "payment" at version 0 → CONFLICT! (actual v1)
```

**PostgreSQL Event Store Behavior**:
```sql
-- Request 1 succeeds
INSERT INTO events (stream_id, version, ...)
VALUES ('payment', 0, ...);  -- ✅ Stream version 0→1

-- Request 2 fails
INSERT INTO events (stream_id, version, ...)
VALUES ('payment', 0, ...);  -- ❌ CONFLICT: expected v0, found v1
```

### Affected Components

| Component | Current Stream ID | Correct Stream ID | Status |
|-----------|-------------------|-------------------|--------|
| Payment | `"payment"` | `"payment-{payment_id}"` | 🔴 Broken |
| Reservation | `"reservation"` | `"reservation-{reservation_id}"` | 🔴 Broken |
| Event | `"event"` | `"event-{event_id}"` | 🔴 Broken |
| Inventory | `"inventory"` | `"inventory-{event_id}"` or `"inventory-{event_id}-{section}"` | 🟡 Investigate |

---

## Target Architecture

### Stream ID Pattern

**Principle**: **One stream per aggregate instance**

```rust
// Payment aggregate (entity)
payment-550e8400-e29b-41d4-a716-446655440000
payment-660e8400-e29b-41d4-a716-446655440001
payment-770e8400-e29b-41d4-a716-446655440002

// Reservation aggregate (entity)
reservation-880e8400-e29b-41d4-a716-446655440000
reservation-990e8400-e29b-41d4-a716-446655440001

// Event aggregate (entity)
event-aa0e8400-e29b-41d4-a716-446655440000
event-bb0e8400-e29b-41d4-a716-446655440001

// Inventory aggregate (per-event OR per-event-section - TBD)
inventory-{event_id}                    # Option A: One stream per event
inventory-{event_id}-{section}          # Option B: One stream per section
```

### Correct Store Creation Pattern

**File**: `src/server/state.rs`

```rust
// ✅ CORRECT - Each payment instance gets its own stream
pub fn create_payment_store(&self, payment_id: PaymentId) -> Store<...> {
    let stream_id = StreamId::new(&format!("payment-{}", payment_id.as_uuid()));

    let env = PaymentEnvironment::new(
        self.clock.clone(),
        self.event_store.clone(),
        self.event_bus.clone(),
        stream_id,  // ✅ Instance-specific stream!
        self.payment_query.clone(),
    );

    Store::new(PaymentState::new(), PaymentReducer::new(), env)
}

// Same pattern for other aggregates:
pub fn create_reservation_store(&self, reservation_id: ReservationId) -> Store<...> {
    let stream_id = StreamId::new(&format!("reservation-{}", reservation_id.as_uuid()));
    // ...
}

pub fn create_event_store(&self, event_id: EventId) -> Store<...> {
    let stream_id = StreamId::new(&format!("event-{}", event_id.as_uuid()));
    // ...
}

// Inventory needs investigation - see "Open Questions" below
pub fn create_inventory_store(&self, event_id: EventId) -> Store<...> {
    let stream_id = StreamId::new(&format!("inventory-{}", event_id.as_uuid()));
    // OR: StreamId::new(&format!("inventory-{}-{}", event_id, section))
    // ...
}
```

### API Endpoint Pattern Changes

**Before (Broken)**:
```rust
// src/api/payments.rs
pub async fn process_payment(...) -> Result<...> {
    let payment_id = PaymentId::new();
    let store = state.create_payment_store();  // ❌ Wrong stream

    store.send(PaymentAction::ProcessPayment { payment_id, ... }).await?;
}

pub async fn get_payment(Path(payment_id): Path<Uuid>, ...) -> Result<...> {
    let payment_id_typed = PaymentId::from_uuid(payment_id);
    let store = state.create_payment_store();  // ❌ Wrong stream - loads nothing!

    store.send(PaymentAction::GetPayment { payment_id: payment_id_typed }).await?;
}
```

**After (Correct)**:
```rust
// src/api/payments.rs
pub async fn process_payment(...) -> Result<...> {
    let payment_id = PaymentId::new();
    let store = state.create_payment_store(payment_id);  // ✅ Correct stream

    store.send(PaymentAction::ProcessPayment { payment_id, ... }).await?;
}

pub async fn get_payment(Path(payment_id): Path<Uuid>, ...) -> Result<...> {
    let payment_id_typed = PaymentId::from_uuid(payment_id);
    let store = state.create_payment_store(payment_id_typed);  // ✅ Loads correct instance

    store.send(PaymentAction::GetPayment { payment_id: payment_id_typed }).await?;
}
```

### State Loading Behavior

**Current (Broken)**:
```rust
// Create store with generic "payment" stream
let store = state.create_payment_store();

// Send GetPayment for payment_id=123
store.send(GetPayment { payment_id: 123 }).await;

// Store loads events from stream "payment"
// This contains ALL payments (A, B, C, ..., 123, ...)
// State rebuilds from ALL payment events (WRONG!)
// Query for payment 123 might find it, but inefficient and semantically wrong
```

**After Fix (Correct)**:
```rust
// Create store for specific payment instance
let store = state.create_payment_store(payment_id: 123);

// Send GetPayment for payment_id=123
store.send(GetPayment { payment_id: 123 }).await;

// Store loads events from stream "payment-123"
// This contains ONLY payment 123's events
// State rebuilds from that payment's history (CORRECT!)
```

---

## Implementation Strategy

### Phase 1: Investigation & Design Decisions

**Tasks**:
1. ✅ Understand inventory aggregate semantics
   - Is InventoryState per-event or per-event-section?
   - Check if one event has multiple inventory instances
   - Determine correct stream ID pattern

2. ✅ Identify all store creation call sites
   - Search for `create_payment_store()`, `create_reservation_store()`, etc.
   - Catalog all locations that need ID parameters

3. ✅ Check for saga coordinators or background workers
   - Do any sagas create stores directly?
   - Or do they only publish events?

### Phase 2: Core Infrastructure Changes

**File**: `src/server/state.rs`

**Changes**:
```rust
// Update all store creation methods to accept entity IDs

// 1. Payment Store
pub fn create_payment_store(
    &self,
    payment_id: PaymentId,  // ← NEW PARAMETER
) -> Store<PaymentState, PaymentAction, PaymentEnvironment, PaymentReducer> {
    let stream_id = StreamId::new(&format!("payment-{}", payment_id.as_uuid()));
    let env = PaymentEnvironment::new(
        self.clock.clone(),
        self.event_store.clone(),
        self.event_bus.clone(),
        stream_id,
        self.payment_query.clone(),
    );
    Store::new(PaymentState::new(), PaymentReducer::new(), env)
}

// 2. Reservation Store
pub fn create_reservation_store(
    &self,
    reservation_id: ReservationId,  // ← NEW PARAMETER
) -> Store<ReservationState, ReservationAction, ReservationEnvironment, ReservationReducer> {
    let stream_id = StreamId::new(&format!("reservation-{}", reservation_id.as_uuid()));
    let env = ReservationEnvironment::new(
        self.clock.clone(),
        self.event_store.clone(),
        self.event_bus.clone(),
        stream_id,
        self.reservation_query.clone(),
    );
    Store::new(ReservationState::new(), ReservationReducer::new(), env)
}

// 3. Event Store
pub fn create_event_store(
    &self,
    event_id: EventId,  // ← NEW PARAMETER
) -> Store<EventState, EventAction, EventEnvironment, EventReducer> {
    let stream_id = StreamId::new(&format!("event-{}", event_id.as_uuid()));
    let env = EventEnvironment::new(
        self.clock.clone(),
        self.event_store.clone(),
        self.event_bus.clone(),
        stream_id,
        self.events_projection.clone(),
    );
    Store::new(EventState::new(), EventReducer::new(), env)
}

// 4. Inventory Store (depends on investigation)
pub fn create_inventory_store(
    &self,
    event_id: EventId,  // ← NEW PARAMETER
    // section: Option<String>,  // ← If per-section
) -> Store<InventoryState, InventoryAction, InventoryEnvironment, InventoryReducer> {
    let stream_id = StreamId::new(&format!("inventory-{}", event_id.as_uuid()));
    // OR: let stream_id = StreamId::new(&format!("inventory-{}-{}", event_id, section));
    let env = InventoryEnvironment::new(
        self.clock.clone(),
        self.event_store.clone(),
        self.event_bus.clone(),
        stream_id,
        self.inventory_query.clone(),
    );
    Store::new(InventoryState::new(), InventoryReducer::new(), env)
}
```

### Phase 3: API Endpoint Updates

**File**: `src/api/payments.rs`

**Changes**:
```rust
// 1. process_payment - NEW payment, generate ID first
pub async fn process_payment(...) -> Result<...> {
    let payment_id = PaymentId::new();  // Generate ID
    let reservation_id = ReservationId::from_uuid(request.reservation_id);
    let customer_id = CustomerId::from_uuid(session.user_id.0);

    // ... validation ...

    let store = state.create_payment_store(payment_id);  // ✅ Pass ID

    let action = PaymentAction::ProcessPayment {
        payment_id,
        reservation_id,
        amount,
        payment_method,
    };

    store.send_with_metadata(action, Some(metadata)).await?;
    // ... rest of logic ...
}

// 2. get_payment - EXISTING payment, use ID from path
pub async fn get_payment(
    Path(payment_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<PaymentResponse>, AppError> {
    let payment_id_typed = PaymentId::from_uuid(payment_id);
    let store = state.create_payment_store(payment_id_typed);  // ✅ Pass ID

    let result = store.send_and_wait_for(
        PaymentAction::GetPayment { payment_id: payment_id_typed },
        |action| matches!(action, PaymentAction::PaymentQueried { .. }),
        Duration::from_secs(5),
    ).await?;
    // ... rest of logic ...
}

// 3. refund_payment - EXISTING payment, use ID from path
pub async fn refund_payment(
    ownership: RequireOwnership<PaymentId>,
    Path(payment_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<RefundPaymentRequest>,
) -> Result<Json<RefundPaymentResponse>, AppError> {
    let payment_id_typed = PaymentId::from_uuid(payment_id);

    // First query to get current payment state
    let query_store = state.create_payment_store(payment_id_typed);  // ✅ Pass ID
    let result = query_store.send_and_wait_for(
        PaymentAction::GetPayment { payment_id: payment_id_typed },
        |action| matches!(action, PaymentAction::PaymentQueried { .. }),
        Duration::from_secs(5),
    ).await?;

    // ... extract payment, validate ...

    // Then create command store for refund
    let command_store = state.create_payment_store(payment_id_typed);  // ✅ Pass ID
    command_store.send(PaymentAction::RefundPayment { ... }).await?;
    // ... rest of logic ...
}

// 4. list_user_payments - NO CHANGE (uses projection, not store)
pub async fn list_user_payments(...) -> Result<...> {
    // This uses projection queries, not stores
    // No changes needed
}
```

**File**: `src/api/reservations.rs`

**Changes** (same pattern):
```rust
// 1. create_reservation
pub async fn create_reservation(...) -> Result<...> {
    let reservation_id = ReservationId::new();  // Generate ID
    let store = state.create_reservation_store(reservation_id);  // ✅ Pass ID
    // ...
}

// 2. get_reservation
pub async fn get_reservation(Path(reservation_id): Path<Uuid>, ...) -> Result<...> {
    let reservation_id_typed = ReservationId::from_uuid(reservation_id);
    let store = state.create_reservation_store(reservation_id_typed);  // ✅ Pass ID
    // ...
}

// 3. cancel_reservation
pub async fn cancel_reservation(Path(reservation_id): Path<Uuid>, ...) -> Result<...> {
    let reservation_id_typed = ReservationId::from_uuid(reservation_id);
    let store = state.create_reservation_store(reservation_id_typed);  // ✅ Pass ID
    // ...
}

// 4. list_user_reservations - NO CHANGE (projection)
```

**File**: `src/api/events.rs`

**Changes** (same pattern):
```rust
// 1. create_event
pub async fn create_event(...) -> Result<...> {
    let event_id = EventId::new();  // Generate ID
    let store = state.create_event_store(event_id);  // ✅ Pass ID
    // ...
}

// 2. get_event
pub async fn get_event(Path(event_id): Path<Uuid>, ...) -> Result<...> {
    let event_id_typed = EventId::from_uuid(event_id);
    let store = state.create_event_store(event_id_typed);  // ✅ Pass ID
    // ...
}

// 3. update_event
pub async fn update_event(Path(event_id): Path<Uuid>, ...) -> Result<...> {
    let event_id_typed = EventId::from_uuid(event_id);
    let store = state.create_event_store(event_id_typed);  // ✅ Pass ID
    // ...
}

// 4. delete_event
pub async fn delete_event(Path(event_id): Path<Uuid>, ...) -> Result<...> {
    let event_id_typed = EventId::from_uuid(event_id);
    let store = state.create_event_store(event_id_typed);  // ✅ Pass ID
    // ...
}

// 5. list_events - NO CHANGE (projection)
```

**File**: `src/api/inventory.rs` (if exists)

**Changes** (TBD based on investigation):
```rust
// Need to determine:
// - How is inventory queried? By event_id? By event_id + section?
// - What operations create/modify inventory?
// - Pattern will follow same principle: pass entity ID(s) to create_inventory_store()
```

### Phase 4: Saga & Background Worker Updates

**Investigation Needed**:
1. Check `src/aggregates/sagas/` for any saga coordinators
2. Check `src/runtime/consumer.rs` for projection consumers
3. Check if any background workers create stores

**Expected**:
- Sagas probably only publish events (no store creation)
- Consumers update projections (no store creation)
- If any create stores, they need the entity ID

### Phase 5: Test Updates

**Unit Tests** (`src/aggregates/*/tests.rs`):
```rust
// Before
#[tokio::test]
async fn test_payment_processing() {
    let env = create_test_environment();
    let reducer = PaymentReducer::new();
    let mut state = PaymentState::new();

    let store = Store::new(state, reducer, env);  // ❌ No stream ID
    // ...
}

// After
#[tokio::test]
async fn test_payment_processing() {
    let payment_id = PaymentId::new();  // Generate test ID
    let env = create_test_environment(payment_id);  // ✅ Pass ID to env
    let reducer = PaymentReducer::new();
    let mut state = PaymentState::new();

    let store = Store::new(state, reducer, env);
    // ...
}
```

**Integration Tests** (`tests/cqrs_integration.rs`, etc.):
```rust
// Before
let store = create_payment_store_for_test();

// After
let payment_id = PaymentId::new();
let store = create_payment_store_for_test(payment_id);
```

**Deployment Tests** (`tests/full_deployment_test.rs`):
- No changes needed - tests hit HTTP API
- API endpoints already handle ID generation/extraction

### Phase 6: Compilation & Error Fixing

**Expected Errors**:
```
error[E0061]: this function takes 2 arguments but 1 was supplied
  --> src/some_file.rs:123:45
   |
123|     let store = state.create_payment_store();
   |                       ^^^^^^^^^^^^^^^^^^^^ expected 2 arguments
```

**Strategy**:
1. Run `cargo build --all-features`
2. Fix each compilation error by passing required entity ID
3. Repeat until clean build

### Phase 7: Database Migration

**Required**:
- Clear all existing event streams (they use old singleton pattern)
- Old streams: `"payment"`, `"reservation"`, `"event"`, `"inventory"`
- New streams: `"payment-{uuid}"`, `"reservation-{uuid}"`, etc.

**Script** (already exists):
```bash
./scripts/run-deployment-tests.sh
# Already includes PostgreSQL database cleanup (truncate events table)
```

**Manual Cleanup** (if needed):
```bash
# Clear event store
docker exec ticketing-events psql -U postgres -d ticketing_events -c "TRUNCATE events CASCADE;"

# Clear projections
docker exec ticketing-projections psql -U postgres -d ticketing_projections -c "TRUNCATE available_seats, payments, reservations, ownership_indices CASCADE;"

# Clear auth
docker exec ticketing-auth psql -U postgres -d ticketing_auth -c "TRUNCATE users, sessions, magic_links, oauth_states CASCADE;"

# Clear analytics
docker exec ticketing-analytics psql -U postgres -d ticketing_analytics -c "TRUNCATE event_sales CASCADE;"

# Clear Redis
docker exec ticketing-redis redis-cli FLUSHALL
```

---

## Open Questions

### 1. Inventory Stream ID Pattern

**Question**: Is inventory per-event or per-event-section?

**Investigation Needed**:
- Check `src/types.rs` for `InventoryState` structure
- Check if state contains `HashMap<Section, SectionState>` (per-event)
- Or if state is for one section only (per-event-section)

**Hypothesis**: Based on events like `InventoryInitialized { event_id, section, ... }`, inventory is probably:
- **One stream per event** with state managing multiple sections internally
- Stream ID: `inventory-{event_id}`

**Decision Required**: Once confirmed, update `create_inventory_store()` accordingly.

### 2. API Operations That Don't Match This Pattern

**Question**: Are there any operations that query across multiple instances?

**Answer**: Yes, but they should use **projections**, not stores:
- `list_user_payments` - uses `payments_projection` ✅
- `list_user_reservations` - uses `reservations_projection` ✅
- `list_events` - uses `events_projection` ✅
- Analytics queries - use `sales_analytics_projection` ✅

**Principle**:
- **Stores** = write side + single instance queries (CQRS write model)
- **Projections** = read side + multi-instance queries (CQRS read model)

### 3. Saga Coordination

**Question**: Do sagas create stores or only publish events?

**Investigation Needed**:
- Check `src/aggregates/sagas/` directory
- Check for `EventInventorySaga` or other saga coordinators
- Determine if they instantiate stores

**Expected**: Sagas publish events to event bus, they don't create stores directly.

### 4. Test Helper Functions

**Question**: Are there test helper functions that create stores?

**Investigation Needed**:
- Check for `create_*_store_for_test()` helper functions
- Check test utilities in `src/test_utils.rs` or similar

**Action**: Update helpers to accept entity IDs as parameters.

---

## Success Criteria

### Functional Requirements

- ✅ All deployment tests pass (6/6)
- ✅ Zero optimistic concurrency conflicts
- ✅ Each aggregate instance has its own event stream
- ✅ Stream IDs follow pattern: `{aggregate_type}-{instance_id}`

### Technical Requirements

- ✅ `cargo build --all-features` succeeds
- ✅ `cargo test --all-features` passes (all unit + integration tests)
- ✅ `cargo clippy --all-targets --all-features -- -D warnings` clean
- ✅ Deployment test script runs successfully

### Verification Steps

1. **Database Cleanup**:
   ```bash
   docker compose down -v  # Clear all volumes
   docker compose up -d     # Fresh start
   ./scripts/run-deployment-tests.sh
   ```

2. **Check Event Store**:
   ```bash
   docker exec ticketing-events psql -U postgres -d ticketing_events -c \
     "SELECT stream_id, version FROM events ORDER BY created_at;"
   ```

   **Expected Output**:
   ```
   stream_id                              | version
   --------------------------------------+---------
   payment-550e8400-...                  | 0
   payment-660e8400-...                  | 0
   reservation-770e8400-...              | 0
   event-880e8400-...                    | 0
   ...
   ```

   **Should NOT see**: Singleton streams like `"payment"`, `"reservation"`, `"event"`

3. **Check Logs for Conflicts**:
   ```bash
   grep -i "concurrency conflict" /tmp/ticketing-deployment-test.log
   ```

   **Expected**: No matches (exit code 1)

4. **Verify Test Results**:
   ```
   🧪 Test 1: Health Check - PASSED
   🧪 Test 2: Event CRUD Operations - PASSED
   🧪 Test 3: Availability Queries - PASSED
   🧪 Test 4: Reservation Flow - PASSED
   🧪 Test 5: Payment Processing - PASSED  ← This should now pass!
   🧪 Test 6: Analytics Queries - PASSED

   ✅ All deployment tests passed! (6/6)
   ```

---

## Risk Assessment

### High Risk

**Database Cleanup Required**:
- All existing event data uses old singleton streams
- Cannot coexist with new per-instance streams
- **Mitigation**: This is example/test code, full cleanup is acceptable

### Medium Risk

**Large Surface Area**:
- Changes touch many files (server/state.rs, all API endpoints, tests)
- High chance of missing a call site
- **Mitigation**: Compiler will catch all missing parameters (build errors)

**Type Signature Changes**:
- Breaking API change for store creation methods
- **Mitigation**: All internal code, no external API consumers

### Low Risk

**Logic Changes**:
- No business logic changes, only infrastructure
- Reducers, aggregates remain unchanged
- **Mitigation**: Existing tests validate business logic

---

## Implementation Checklist

### Pre-Implementation

- [ ] Investigate inventory stream ID pattern (per-event vs per-section)
- [ ] Catalog all `create_*_store()` call sites
- [ ] Check for saga coordinators that create stores
- [ ] Review test helper functions

### Core Changes

- [ ] Update `server/state.rs`:
  - [ ] `create_payment_store(payment_id)`
  - [ ] `create_reservation_store(reservation_id)`
  - [ ] `create_event_store(event_id)`
  - [ ] `create_inventory_store(event_id)` (or event_id + section)

### API Endpoints

- [ ] Update `api/payments.rs`:
  - [ ] `process_payment` - pass payment_id
  - [ ] `get_payment` - pass payment_id
  - [ ] `refund_payment` - pass payment_id (2 stores: query + command)

- [ ] Update `api/reservations.rs`:
  - [ ] `create_reservation` - pass reservation_id
  - [ ] `get_reservation` - pass reservation_id
  - [ ] `cancel_reservation` - pass reservation_id

- [ ] Update `api/events.rs`:
  - [ ] `create_event` - pass event_id
  - [ ] `get_event` - pass event_id
  - [ ] `update_event` - pass event_id
  - [ ] `delete_event` - pass event_id

- [ ] Update `api/inventory.rs` (if exists):
  - [ ] Any inventory operations

### Tests

- [ ] Update unit tests in `src/aggregates/*/tests.rs`
- [ ] Update integration tests in `tests/*.rs`
- [ ] Update test helper functions (if any)

### Verification

- [ ] `cargo build --all-features` succeeds
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` clean
- [ ] `cargo test --all-features` passes
- [ ] Clear database volumes
- [ ] Run deployment tests: `./scripts/run-deployment-tests.sh`
- [ ] Verify 6/6 tests pass
- [ ] Check logs for zero concurrency conflicts
- [ ] Inspect event store for per-instance streams

---

## Appendix: Event Sourcing Fundamentals

### The Aggregate-Stream Relationship

In event sourcing, this relationship is **fundamental**:

```
Aggregate Instance  ←→  Event Stream  (1:1 mapping)
```

**Why This Matters**:
- Each aggregate instance has its own lifecycle
- Each lifecycle is recorded as an ordered sequence of events
- The sequence is stored in one stream
- Stream version = aggregate version (optimistic locking)

**Example - Payment Lifecycle**:
```
Payment Instance: payment-550e8400-...
├─ Event 0: PaymentInitiated { amount: $200, ... }
├─ Event 1: PaymentProcessed { transaction_id: "txn_123", ... }
└─ Event 2: PaymentConfirmed { ... }

Stream: payment-550e8400-...
Version: 2 (3 events, versions 0-2)
```

**Multiple Instances**:
```
payment-550e8400-...  →  [PaymentInitiated, PaymentProcessed, PaymentConfirmed]  v2
payment-660e8400-...  →  [PaymentInitiated, PaymentProcessed, PaymentFailed]    v2
payment-770e8400-...  →  [PaymentInitiated]                                     v0
```

**Singleton Stream (WRONG)**:
```
payment  →  [
    PaymentInitiated(550e8400),   v0
    PaymentInitiated(660e8400),   v1  ← Different payment!
    PaymentProcessed(550e8400),   v2
    PaymentProcessed(660e8400),   v3
    PaymentConfirmed(550e8400),   v4
    PaymentFailed(660e8400),      v5
    PaymentInitiated(770e8400),   v6  ← Another different payment!
]

Problems:
- Multiple aggregates share one stream (violates 1:1 rule)
- Version numbers span multiple instances (concurrency control breaks)
- Event order mixes unrelated instances (replay produces garbage state)
- Optimistic concurrency expects version 0 for new payment (fails after first)
```

### Optimistic Concurrency Control

**Purpose**: Prevent lost updates in concurrent scenarios

**Mechanism**:
1. Load aggregate state from event stream
2. Note current version (e.g., v5)
3. Process command, generate new events
4. Append events with expected version (v5)
5. Database checks: current version == expected version
6. If match: append succeeds, version increments
7. If mismatch: append fails with concurrency conflict

**Per-Instance Streams (CORRECT)**:
```
Thread A: Load payment-123 (v2) → append event (expect v2) → SUCCESS (now v3)
Thread B: Load payment-456 (v1) → append event (expect v1) → SUCCESS (now v2)
           ↑ Different streams, no conflict!
```

**Singleton Stream (BROKEN)**:
```
Thread A: Load payment (v0) → append PaymentInitiated(123) expect v0 → SUCCESS (now v1)
Thread B: Load payment (v0) → append PaymentInitiated(456) expect v0 → CONFLICT! (actual v1)
           ↑ Same stream, both expect v0, second fails!
```

---

## References

- **Event Sourcing Pattern**: https://martinfowler.com/eaaDev/EventSourcing.html
- **CQRS Journey**: https://docs.microsoft.com/en-us/previous-versions/msp-n-p/jj554200(v=pandp.10)
- **Composable Rust Docs**: `docs/event-design-guidelines.md`, `docs/consistency-patterns.md`
- **PostgreSQL Event Store**: `postgres/src/lib.rs` (optimistic concurrency implementation)

---

**Last Updated**: 2025-11-23
**Author**: Claude Code
**Review Status**: 🟡 Awaiting User Approval
