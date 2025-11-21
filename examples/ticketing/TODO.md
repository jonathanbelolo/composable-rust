# TODO Tracking

**Last Updated**: 2025-11-20 (Phase A.5)
**Total TODOs**: 36 items
**Status**: All categorized and prioritized

---

## Phase A: Completed ✅

These TODOs were already addressed during Phase A implementation:

- [x] **Health check implementation** (A.1) - Real health checks implemented
- [x] **Event API endpoints** (A.2) - GET, PUT, DELETE complete
- [x] **Payment API endpoints** (A.3) - GET, refund complete
- [x] **Persist in-memory projections** (A.4) - All projections use PostgreSQL

---

## Phase B: Production Hardening (Deferred)

These TODOs require production-grade features and should be addressed in Phase B:

### B.1: Payment Gateway Integration

**Priority**: HIGH (required for production)
**Effort**: 8-16 hours
**Files**: `src/api/payments.rs`

- [ ] **Line 246**: Verify reservation ownership via query layer
  ```rust
  // TODO (Phase 12.4): Verify reservation ownership via query layer
  ```

- [ ] **Line 255**: Replace placeholder amount with actual reservation data
  ```rust
  // TODO (Phase 12.5): Replace placeholder amount with actual amount from reservation
  ```

- [ ] **Line 259**: Integrate real payment gateway (Stripe, PayPal, etc.)
  ```rust
  // TODO (Phase 12.5): Payment gateway integration (Stripe, PayPal, etc.)
  ```

- [ ] **Line 425**: Get actual amount from reservation
  ```rust
  amount: 200.0, // TODO: Get from reservation in Phase 12.4
  ```

- [ ] **Line 509**: Add transaction_id to Payment domain model
  ```rust
  transaction_id: None, // TODO: Add transaction_id to Payment domain model
  ```

- [ ] **Line 624**: Implement refund policy eligibility checks
  ```rust
  // TODO (Phase 12.5): Check refund policy eligibility
  ```

**Implementation Notes**:
- Requires Stripe/PayPal SDK integration
- Needs refund policy rules (time-based, event-based)
- Requires transaction_id in domain model

### B.2: WebSocket Connection Management

**Priority**: HIGH (required for production scalability)
**Effort**: 4-8 hours
**Files**: `src/api/websocket.rs`

- [ ] **Line 270**: Check if user already has active connection (rate limiting)
  ```rust
  // TODO: Check if user already has an active connection (rate limiting)
  ```

- [ ] **Line 271**: Store connection in registry (DashMap<UserId, WebSocket>)
  ```rust
  // TODO: Store connection in a connection registry (DashMap<UserId, WebSocket>)
  ```

- [ ] **Line 272**: Close existing connection if present
  ```rust
  // TODO: Close existing connection if present
  ```

- [ ] **Line 843**: Remove connection from registry on disconnect
  ```rust
  // TODO: Remove connection from registry (for rate limiting)
  ```

**Implementation Notes**:
- Use `DashMap<CustomerId, Arc<Mutex<WebSocket>>>` for concurrent access
- Implement per-user rate limiting (1 connection per user)
- Add connection lifecycle tracking (connected_at, last_ping, etc.)

### B.3: Request Lifecycle Enhancements

**Priority**: MEDIUM
**Effort**: 2-4 hours
**Files**: `src/request_lifecycle/store.rs`

- [ ] **Line 39**: Execute effects (Delay for timeout)
  ```rust
  // TODO: Execute effects (Delay for timeout)
  ```

**Implementation Notes**:
- Implement Effect::Delay for request timeout handling
- Requires timeout configuration per endpoint

### B.4: Auth Security Enhancements

**Priority**: MEDIUM
**Effort**: 2-4 hours
**Files**: `src/auth/handlers.rs`, `src/auth/middleware.rs`

- [ ] **handlers.rs:211**: Extract fingerprint from request header
  ```rust
  fingerprint: None, // TODO: Extract from request header if available
  ```

- [ ] **middleware.rs:504**: Implement admin override for customer profiles
  ```rust
  // TODO: Add admin override check - admins should be able to view any customer profile
  ```

**Implementation Notes**:
- Fingerprint helps detect session hijacking
- Admin override requires role-based access control (RBAC)
- See also: `src/api/analytics.rs:567` and `:617` (customer profile admin access)

---

## Phase C: Feature Expansion (Deferred)

These TODOs extend the domain model with new features:

### C.1: Event Domain Model Extensions

**Priority**: LOW (nice-to-have)
**Effort**: 4-8 hours
**Files**: `src/api/events.rs`, `src/types.rs`

- [ ] **events.rs:271**: Support both start_time and end_time (currently only has date)
  ```rust
  // - date -> both start_time and end_time (TODO: extend domain model)
  ```

- [ ] **events.rs:273**: Add description field to Event type
  ```rust
  // - description is not in domain model yet (TODO: add to Event type)
  ```

- [ ] **events.rs:277**: Return description from domain model (currently stub)
  ```rust
  description: String::from("Event description not yet available"),
  ```

- [ ] **events.rs:279**: Return actual end_time from domain model
  ```rust
  end_time: event.date.inner(), // TODO: Add separate end_time to Event domain model
  ```

- [ ] **events.rs:344**: GET endpoint - description stub
- [ ] **events.rs:346**: GET endpoint - end_time stub
- [ ] **events.rs:423**: PUT endpoint - support for description/start_time/end_time
- [ ] **events.rs:478**: PUT response - description stub
- [ ] **events.rs:480**: PUT response - end_time stub

**Implementation Plan**:
1. Extend `Event` type in `src/types.rs`:
   ```rust
   pub struct Event {
       pub id: EventId,
       pub name: EventName,
       pub start_time: DateTime<Utc>,
       pub end_time: DateTime<Utc>,
       pub description: Option<String>,
       pub venue: Venue,
       pub sections: HashMap<String, u32>,
   }
   ```

2. Update event creation/update commands in `src/aggregates/event.rs`
3. Migrate existing events (schema migration in PostgreSQL)
4. Update API endpoints to accept new fields

### C.2: Reservation Enhancements

**Priority**: MEDIUM
**Effort**: 4-8 hours
**Files**: `src/api/reservations.rs`

- [ ] **Line 178**: Support specific seat selection
  ```rust
  let specific_seats = None; // TODO: Convert request.specific_seats properly
  ```

- [ ] **Line 302**: Extract section from domain model
  ```rust
  section: String::from("General Admission"), // TODO: Extract from domain model when available
  ```

- [ ] **Line 308**: Extract completed_at timestamp from JSONB
  ```rust
  completed_at: None, // TODO: Extract from JSONB if needed
  ```

- [ ] **Line 489**: Same as line 302

**Implementation Notes**:
- Specific seat selection requires seat map in domain model
- Section should come from reservation aggregate state

### C.3: Analytics Enhancements

**Priority**: LOW
**Effort**: 2-4 hours
**Files**: `src/api/analytics.rs`

- [ ] **Line 455**: Add method to count all events with sales
  ```rust
  // TODO: Add method to count all events with sales
  ```

- [ ] **Line 461**: Implement proper event counting
  ```rust
  events_with_sales, // TODO: Implement proper counting
  ```

- [ ] **Line 569**: Implement pagination for customer purchase history
  ```rust
  /// Returns last 10 purchases by default. For full history, use paginated endpoint (TODO).
  ```

- [ ] **Line 617**: Implement admin check for customer profile override
  ```rust
  // TODO: Also check if user is admin for override capability
  ```

**Implementation Notes**:
- Events with sales: Add `count_events_with_sales()` to sales projection
- Pagination: Add `offset` and `limit` query params to customer history endpoint
- Admin check: Requires RBAC (see B.4)

### C.4: Payment Enhancements

**Priority**: LOW
**Effort**: 2 hours
**Files**: `src/api/payments.rs`

- [ ] **Line 718**: Add pagination for payment list
  ```rust
  // Get all payments (limit 100 for now, TODO: add pagination query params)
  ```

**Implementation Notes**:
- Add `offset` and `limit` query params
- Add pagination metadata to response (total_count, has_more)

---

## Intentional Documentation TODOs (Keep As-Is)

These TODOs serve as documentation for intentional design decisions:

### Testing Stubs (Keep)

**Files**: `src/api/availability.rs`

- [x] **Line 215**: Stub data for missing projections (helpful for testing)
  ```rust
  // TODO: Remove this stub once event creation is fully implemented
  ```

- [x] **Line 304**: Same as above

**Rationale**: These stubs provide sensible defaults when projections are empty (e.g., during initial setup or testing). They don't hurt production (once events exist, real data is returned) and make the system more robust during development.

**Decision**: Keep these TODOs as documentation of intentional behavior.

---

## Summary by Priority

### CRITICAL (Phase B)
- 6 items: Payment gateway integration
- 4 items: WebSocket connection management
- **Total**: 10 items

### HIGH (Phase B)
- 2 items: Auth security enhancements
- 1 item: Request lifecycle
- **Total**: 3 items

### MEDIUM (Phase C)
- 4 items: Reservation enhancements
- **Total**: 4 items

### LOW (Phase C)
- 9 items: Event domain model extensions
- 3 items: Analytics enhancements
- 1 item: Payment pagination
- **Total**: 13 items

### Documentation (Keep)
- 2 items: Testing stubs in availability.rs
- **Total**: 2 items

---

## Next Steps

### Phase B: Production Hardening

1. **B.1: Payment Gateway** (highest priority)
   - Integrate Stripe SDK
   - Implement refund policies
   - Add transaction tracking

2. **B.2: WebSocket Management** (scalability)
   - Connection registry with DashMap
   - Per-user rate limiting
   - Connection lifecycle tracking

3. **B.3: Request Lifecycle** (resilience)
   - Effect::Delay execution
   - Timeout handling

4. **B.4: Auth Enhancements** (security)
   - Fingerprint tracking
   - Admin RBAC

### Phase C: Feature Expansion

1. **C.1: Event Extensions** (UX improvements)
   - Description, start_time, end_time
   - Schema migration

2. **C.2: Reservation Features** (advanced booking)
   - Specific seat selection
   - Proper section tracking

3. **C.3: Analytics Features** (insights)
   - Event counting
   - Pagination

4. **C.4: Payment Features** (usability)
   - Payment list pagination

---

## Audit Trail

**Phase A.5 Audit** (2025-11-20):
- Scanned: 36 TODO comments
- Categorized: 36 items (10 CRITICAL, 3 HIGH, 17 MEDIUM/LOW, 2 Documentation)
- Removed: 0 items (all TODOs are intentional)
- Documented: All items tracked in this file

**Files Scanned**:
```bash
grep -r "TODO" src/ --include="*.rs" -n
```

**Total Lines**: 36 matches across 8 files:
- `src/request_lifecycle/store.rs`: 1 TODO
- `src/auth/handlers.rs`: 1 TODO
- `src/auth/middleware.rs`: 1 TODO
- `src/api/events.rs`: 9 TODOs
- `src/api/payments.rs`: 7 TODOs
- `src/api/websocket.rs`: 4 TODOs
- `src/api/reservations.rs`: 4 TODOs
- `src/api/analytics.rs`: 5 TODOs
- `src/api/availability.rs`: 2 TODOs

---

## Maintenance

This file should be updated:
- After each phase completion (to mark items as done)
- When new TODOs are added (to track them properly)
- When priorities change (based on user feedback)

To regenerate the audit:
```bash
grep -r "TODO" src/ --include="*.rs" -n | tee /tmp/todo_audit.txt
```
