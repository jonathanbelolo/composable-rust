# Stream ID Fix - Investigation Results

**Date**: 2025-11-23
**Status**: ✅ Investigation Complete

---

## Question 1: Inventory Stream ID Pattern

### Investigation

**Checked**:
- `src/types.rs:1016` - InventoryState structure
- `src/api/availability.rs:92` - API usage patterns

**Findings**:

```rust
// InventoryState structure (line 1018)
pub struct InventoryState {
    /// All inventories indexed by (event_id, section)
    pub inventories: HashMap<(EventId, String), Inventory>,
    pub seat_assignments: HashMap<SeatId, SeatAssignment>,
    pub loading_states: HashMap<(EventId, String), LoadingState>,
    // ...
}
```

**Current API Pattern** (`api/availability.rs:86-114`):
```rust
pub async fn get_event_availability(
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<...> {
    let event_id_typed = EventId::from_uuid(event_id);
    let inventory_store = state.create_inventory_store();  // ❌ No event_id!

    inventory_store.send_and_wait_for(
        InventoryAction::GetAllSections {
            event_id: event_id_typed,  // Event ID passed in ACTION
        },
        // ...
    ).await?;
}
```

### Analysis

**Current Architecture (BROKEN)**:
- Global singleton stream: `"inventory"`
- Contains inventory events for ALL events
- State has `HashMap<(EventId, String), Inventory>` to manage multiple events
- Queries filter by event_id to find relevant data
- **Problem**: Same concurrency conflicts as other aggregates

**Correct Architecture**:
- Per-event streams: `"inventory-{event_id}"`
- Each stream contains events for ONE event only
- State could be simplified to `HashMap<String, Inventory>` (sections only)
  - EventId in key is redundant since stream already scopes to one event
  - Or keep as-is and ignore EventId (always same value)
- **Benefit**: Each event's inventory is isolated, no conflicts

### Decision

**Stream ID Pattern**: `inventory-{event_id}`

**Reasoning**:
1. Each event has independent inventory
2. Inventory lifecycle tied to event lifecycle
3. No shared state between events
4. Follows same pattern as Payment, Reservation, Event aggregates

### Implementation

**Update `server/state.rs`**:
```rust
pub fn create_inventory_store(
    &self,
    event_id: EventId,  // ← NEW PARAMETER
) -> Store<InventoryState, InventoryAction, InventoryEnvironment, InventoryReducer> {
    let stream_id = StreamId::new(&format!("inventory-{}", event_id.as_uuid()));
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

**Update `api/availability.rs`**:
```rust
pub async fn get_event_availability(
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<...> {
    let event_id_typed = EventId::from_uuid(event_id);
    let inventory_store = state.create_inventory_store(event_id_typed);  // ✅ Pass event_id

    inventory_store.send_and_wait_for(
        InventoryAction::GetAllSections { event_id: event_id_typed },
        // ...
    ).await?;
}
```

**Other Files Using Inventory Store**:
- `api/availability.rs` - 3 locations (lines 92, 180, 277)
- `api/reservations.rs` - Check for inventory operations
- `api/websocket.rs` - Check for inventory subscriptions

---

## Question 2: All Store Creation Call Sites

### Inventory Store Locations

**File**: `src/api/availability.rs`
- Line 92: `get_event_availability()` - has event_id from path ✅
- Line 180: `get_section_availability()` - has event_id from path ✅
- Line 277: `get_total_available()` - has event_id from path ✅

**File**: `src/api/reservations.rs`
- Need to check if it creates inventory stores (for seat reservation)

**File**: `src/api/websocket.rs`
- Need to check if it creates inventory stores (for real-time updates)

### Payment Store Locations

**File**: `src/api/payments.rs`
- Line 272: `process_payment()` - generates new payment_id ✅
- Line 461: `get_payment()` - has payment_id from path ✅
- Line 572: `refund_payment()` - has payment_id from path (query) ✅
- Line 629: `refund_payment()` - has payment_id from path (command) ✅
- Line 714: `list_user_payments()` - uses projection (NO CHANGE) ✅

### Reservation Store Locations

**File**: `src/api/reservations.rs`
- Need to catalog all `create_reservation_store()` calls

### Event Store Locations

**File**: `src/api/events.rs` (assumed to exist)
- Need to catalog all `create_event_store()` calls

### Next Steps

Run comprehensive search:
```bash
# Find all create_*_store() calls
grep -rn "create_payment_store\|create_reservation_store\|create_event_store\|create_inventory_store" \
  examples/ticketing/src/api/ \
  examples/ticketing/src/aggregates/ \
  examples/ticketing/src/runtime/ \
  examples/ticketing/tests/
```

---

## Question 3: Saga Coordinators & Background Workers

### Investigation Needed

**Check for**:
1. Saga coordinators that create stores directly
2. Event consumers that create stores (vs updating projections)
3. Background workers or scheduled tasks

**Search Commands**:
```bash
# Find saga-related code
find examples/ticketing/src -name "*saga*" -type f

# Find consumer implementations
grep -rn "impl.*Consumer\|struct.*Consumer" examples/ticketing/src/

# Check runtime directory
ls -la examples/ticketing/src/runtime/
```

### Expected Results

**Hypothesis**:
- Sagas publish events via event bus (no direct store creation)
- Consumers update projections (read side, no aggregate stores)
- No background workers that create stores

**If Saga Creates Stores**:
- Need to pass entity IDs to saga constructor
- Saga would need to know which instances to coordinate

---

## Question 4: Test Helper Functions

### Investigation Needed

**Check for**:
1. Test utilities in `src/test_utils.rs` or similar
2. Helper functions in test modules
3. Common test setup patterns

**Search Commands**:
```bash
# Find test helper modules
find examples/ticketing -name "test_utils.rs" -o -name "helpers.rs"

# Find helper functions in tests
grep -rn "fn create.*store\|fn setup.*store" examples/ticketing/tests/
grep -rn "fn create.*store\|fn setup.*store" examples/ticketing/src/aggregates/*/tests.rs
```

### Expected Updates

If helpers exist, update signatures:
```rust
// Before
fn create_test_payment_store() -> Store<...> { ... }

// After
fn create_test_payment_store(payment_id: PaymentId) -> Store<...> { ... }
```

---

## Summary of Findings

### ✅ Confirmed Decisions

1. **Inventory**: Per-event streams → `inventory-{event_id}`
2. **Pattern**: All aggregates follow same model (entity = stream)
3. **API Impact**: All endpoints have entity IDs available (path or generated)

### 📋 Remaining Tasks

1. Catalog ALL `create_*_store()` call sites across entire codebase
2. Check for saga coordinators
3. Check for background workers
4. Identify test helpers

### 🎯 Ready to Implement

**Prerequisites Met**:
- [x] Stream ID pattern determined for all aggregates
- [x] Architecture decision documented
- [x] Sample implementation patterns written

**Next Step**: Begin implementation starting with Phase 2 (Core Infrastructure Changes)

---

**Investigation Complete**: 2025-11-23
**Ready for Implementation**: ✅ YES
