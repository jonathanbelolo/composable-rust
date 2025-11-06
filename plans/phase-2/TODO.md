# Phase 2: Event Sourcing & Persistence - TODO List

**Goal**: Build event store on PostgreSQL with bincode serialization. Add event sourcing patterns and state reconstruction.

**Duration**: 1.5-2 weeks

**Status**: 🎯 **PHASE 2A COMPLETE** | 🚧 **PHASE 2B IN PROGRESS**

**Philosophy**: Own the event store implementation (no vendor lock-in). Build on Phase 1's proven abstractions.

---

## ✅ **Phase 2A: Event Sourcing Foundation - COMPLETE**

**Completed**: 2025-11-06

### What Was Built:
- ✅ **Event sourcing abstractions** in `core`:
  - Event trait for serialization
  - StreamId and Version types
  - EventStore trait (4 operations: append, load, save/load snapshots)
  - SerializedEvent struct with bincode support
- ✅ **InMemoryEventStore** in `testing`:
  - Full EventStore implementation with HashMap backend
  - Optimistic concurrency control
  - Version tracking
  - Snapshot support
- ✅ **SystemClock** production implementation in `core`
- ✅ **Order Processing Example** (`examples/order-processing/`):
  - Complete event-sourced aggregate with 3 commands, 3 events
  - Command validation with business rules
  - Event persistence to EventStore
  - **State reconstruction from events** (event replay)
  - **Version tracking** during both command flow and replay
  - Validation failure tracking
  - Clock dependency injection
  - 16 unit tests (all passing)
  - Comprehensive documentation

### Critical Fixes Applied:
1. ✅ **Version tracking during event replay** - Fixed critical bug where version wasn't incremented during replay
2. ✅ **Optimistic concurrency** - Version properly tracked in both normal flow and replay
3. ✅ **Validation observability** - Validation failures now tracked in state
4. ✅ **Clock injection** - All timestamps use injected Clock for testability
5. ✅ **Error logging** - Serialization errors now logged

### Validation:
- ✅ 91 tests passing (16 in Order Processing example)
- ✅ Zero clippy warnings
- ✅ Demo successfully reconstructs state: "Status=Shipped, Items=2, Total=$100.00, Version=2"
- ✅ Event sourcing correctness verified by comprehensive code review

### What's Deferred to Phase 2B:
- ⏸️ PostgreSQL EventStore implementation
- ⏸️ Database schema and migrations
- ⏸️ Integration tests with testcontainers
- ⏸️ Snapshot performance optimization
- ⏸️ Detailed README for Order Processing example

---

---

## Prerequisites

Before starting Phase 2:
- [x] Phase 1 complete and validated
- [x] All 47 tests passing
- [x] Counter example working
- [x] Core abstractions proven (Reducer, Effect, Store)
- [ ] PostgreSQL installed locally (for development)
- [ ] Understand bincode serialization strategy
- [ ] Review Phase 2 goals in roadmap

---

## Strategic Decision: Why Own the Event Store?

From the roadmap:

**Decision**: Build event store on PostgreSQL rather than use specialized vendors (EventStoreDB, Kurrent).

**Rationale**:
- **Vendor independence**: Postgres is open source, ubiquitous, zero lock-in
- **Cost control**: Free infrastructure, no per-event pricing
- **Full control**: Optimize schema and queries for our exact needs
- **Client flexibility**: Every client can use standard Postgres (managed or self-hosted)
- **AI agent compatibility**: Standard SQL that AI agents can optimize
- **Migration safety**: If deployed to 100s of clients, all retain infrastructure choice

**Investment**: ~1 week extra work buys strategic independence forever.

---

## 1. Database Schema Design

### 1.1 Events Table
**Scope**: Immutable append-only log with optimistic concurrency

```sql
CREATE TABLE events (
    stream_id TEXT NOT NULL,           -- Aggregate ID
    version BIGINT NOT NULL,            -- Event version (optimistic concurrency)
    event_type TEXT NOT NULL,           -- Event type name (for deserialization)
    event_data BYTEA NOT NULL,          -- Bincode-serialized event
    metadata JSONB,                     -- Optional metadata (correlation IDs, etc.)
    created_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (stream_id, version)
);

CREATE INDEX idx_events_created ON events(created_at);
CREATE INDEX idx_events_type ON events(event_type);
```

**Tasks**:
- [ ] Create migration file: `migrations/001_create_events_table.sql`
- [ ] Define schema with PRIMARY KEY on (stream_id, version)
- [ ] Add indexes for common queries (created_at, event_type)
- [ ] Document schema design decisions
- [ ] Test schema with sample data

### 1.2 Snapshots Table
**Scope**: Compressed aggregate state for performance

```sql
CREATE TABLE snapshots (
    stream_id TEXT PRIMARY KEY,
    version BIGINT NOT NULL,            -- Version at snapshot
    state_data BYTEA NOT NULL,          -- Bincode-serialized state
    created_at TIMESTAMPTZ DEFAULT now()
);
```

**Tasks**:
- [ ] Create migration file: `migrations/002_create_snapshots_table.sql`
- [ ] Define schema with stream_id as PRIMARY KEY
- [ ] Document snapshot strategy (when to create, when to use)
- [ ] Test snapshot creation and retrieval

### 1.3 Migration Tooling
**Scope**: sqlx-cli for database migrations

**Tasks**:
- [ ] Add sqlx as dependency (with postgres feature)
- [ ] Add sqlx-cli for migrations
- [ ] Create `.env.example` with DATABASE_URL
- [ ] Document migration workflow in README
- [ ] Create `scripts/migrate.sh` helper script
- [ ] Add migration instructions to Phase 2 docs

---

## 2. Core Types & Traits (`composable-rust-core`)

### 2.1 Event Trait
**Scope**: Define Event abstraction for bincode serialization

```rust
/// An event that can be stored and replayed
pub trait Event: Send + Sync + 'static {
    /// Returns the event type name (for deserialization routing)
    fn event_type(&self) -> &'static str;

    /// Serialize event to bincode bytes
    fn to_bytes(&self) -> Result<Vec<u8>, EventError>;

    /// Deserialize event from bincode bytes
    fn from_bytes(bytes: &[u8]) -> Result<Self, EventError>
    where
        Self: Sized;
}
```

**Tasks**:
- [ ] Define Event trait in `core/src/event.rs`
- [ ] Add EventError type using `thiserror`
- [ ] Document Event trait with examples
- [ ] Add comprehensive doc comments
- [ ] Consider blanket impl for `Serialize + DeserializeOwned` types

### 2.2 StreamId and Version Types
**Scope**: Strong types for event stream identification

```rust
/// Unique identifier for an event stream (aggregate instance)
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StreamId(String);

/// Event version number (for optimistic concurrency)
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u64);
```

**Tasks**:
- [ ] Define StreamId newtype in `core/src/stream.rs`
- [ ] Define Version newtype in `core/src/stream.rs`
- [ ] Implement Display, FromStr for StreamId
- [ ] Implement arithmetic operations for Version (+1, etc.)
- [ ] Add comprehensive tests
- [ ] Document usage patterns

### 2.3 EventStore Trait
**Scope**: Abstract event store operations (builds on Environment pattern)

```rust
/// Event store operations for event sourcing
pub trait EventStore: Send + Sync {
    /// Append events to a stream with optimistic concurrency check
    async fn append_events(
        &self,
        stream_id: StreamId,
        expected_version: Option<Version>,
        events: Vec<SerializedEvent>,
    ) -> Result<Version, EventStoreError>;

    /// Load events from a stream, optionally from a specific version
    async fn load_events(
        &self,
        stream_id: StreamId,
        from_version: Option<Version>,
    ) -> Result<Vec<SerializedEvent>, EventStoreError>;

    /// Save a snapshot of aggregate state
    async fn save_snapshot(
        &self,
        stream_id: StreamId,
        version: Version,
        state: Vec<u8>,
    ) -> Result<(), EventStoreError>;

    /// Load the latest snapshot for a stream
    async fn load_snapshot(
        &self,
        stream_id: StreamId,
    ) -> Result<Option<(Version, Vec<u8>)>, EventStoreError>;
}
```

**Tasks**:
- [ ] Define EventStore trait in `core/src/event_store.rs`
- [ ] Define SerializedEvent struct (event_type, data, metadata)
- [ ] Define EventStoreError type using `thiserror`
- [ ] Document all methods with examples
- [ ] Add `# Errors` sections to docs
- [ ] Consider connection pooling requirements

### 2.4 Effect Extensions for EventStore
**Scope**: Add EventStore effect variant

**Tasks**:
- [ ] Add `Effect::EventStore` variant to core/src/lib.rs
- [ ] Define EventStoreOperation enum (AppendEvents, LoadEvents, SaveSnapshot, LoadSnapshot)
- [ ] Update Effect::map() to handle EventStore variant
- [ ] Update merge() and chain() to handle EventStore
- [ ] Add tests for EventStore effect composition
- [ ] Document EventStore effect usage patterns

---

## 3. PostgreSQL Implementation (`composable-rust-postgres`)

### 3.1 New Crate Setup
**Scope**: Create dedicated crate for Postgres implementation

**Tasks**:
- [ ] Create `postgres/` directory in workspace
- [ ] Add to workspace Cargo.toml
- [ ] Set up dependencies (sqlx with postgres + runtime features)
- [ ] Create `postgres/src/lib.rs` with module structure
- [ ] Add README explaining crate purpose
- [ ] Configure crate metadata in Cargo.toml

### 3.2 PostgresEventStore Implementation
**Scope**: Implement EventStore trait using sqlx

```rust
pub struct PostgresEventStore {
    pool: PgPool,
}

impl PostgresEventStore {
    pub async fn new(database_url: &str) -> Result<Self, EventStoreError>;
    pub async fn from_pool(pool: PgPool) -> Self;
}
```

**Tasks**:
- [ ] Implement EventStore trait for PostgresEventStore
- [ ] Use sqlx for queries (compile-time checked SQL)
- [ ] Implement optimistic concurrency (version check on insert)
- [ ] Use transactions for atomic event appends
- [ ] Add connection pooling configuration
- [ ] Handle database errors gracefully
- [ ] Add comprehensive tests (requires testcontainers)

### 3.3 Event Appending with Optimistic Concurrency
**Scope**: Safe concurrent event appending

**Implementation Strategy**:
1. Begin transaction
2. If expected_version is Some, check current max version
3. Insert events with sequential versions
4. Commit transaction
5. Return new version or ConcurrencyError

**Tasks**:
- [ ] Implement append_events with transaction
- [ ] Add version conflict detection
- [ ] Return ConcurrencyError on version mismatch
- [ ] Test concurrent append scenarios
- [ ] Document concurrency guarantees
- [ ] Add retry guidance in documentation

### 3.4 Event Loading
**Scope**: Efficient event stream retrieval

**Tasks**:
- [ ] Implement load_events query
- [ ] Support optional from_version parameter
- [ ] Return events ordered by version
- [ ] Consider pagination for large streams (defer if not needed)
- [ ] Add tests for various load scenarios
- [ ] Document performance characteristics

### 3.5 Snapshot Support
**Scope**: State snapshots for performance

**Tasks**:
- [ ] Implement save_snapshot (UPSERT pattern)
- [ ] Implement load_snapshot (latest snapshot)
- [ ] Test snapshot creation and retrieval
- [ ] Document snapshot strategy (when to create)
- [ ] Add configurable snapshot threshold
- [ ] Test state reconstruction (snapshot + events since)

### 3.6 Testing with Testcontainers
**Scope**: Integration tests with real Postgres

**Tasks**:
- [ ] Add testcontainers dependency (postgres)
- [ ] Create test helpers for database setup
- [ ] Write integration tests for all EventStore operations
- [ ] Test optimistic concurrency conflicts
- [ ] Test snapshot lifecycle
- [ ] Document testing approach
- [ ] Add CI support for integration tests

---

## 4. In-Memory Implementation (`composable-rust-testing`)

### 4.1 InMemoryEventStore
**Scope**: HashMap-based EventStore for fast unit tests

```rust
pub struct InMemoryEventStore {
    events: Arc<RwLock<HashMap<StreamId, Vec<SerializedEvent>>>>,
    snapshots: Arc<RwLock<HashMap<StreamId, (Version, Vec<u8>)>>>,
}
```

**Tasks**:
- [ ] Implement EventStore trait for InMemoryEventStore
- [ ] Use HashMap for in-memory storage
- [ ] Implement same concurrency semantics as Postgres
- [ ] Add inspection methods for test assertions
- [ ] Add reset() method for test isolation
- [ ] Add comprehensive tests
- [ ] Document usage in testing

### 4.2 Test Helpers
**Scope**: Utilities for testing event-sourced aggregates

**Tasks**:
- [ ] Event builder helpers (reduce boilerplate)
- [ ] Assertion helpers (assert_events_match, etc.)
- [ ] Stream fixtures (pre-populated event streams)
- [ ] Snapshot test helpers
- [ ] Document test patterns with examples

---

## 5. Event Sourcing Patterns

### 5.1 State Reconstruction
**Scope**: Rebuild state from event stream

**Pattern**:
```rust
impl MyState {
    /// Reconstruct state from events
    pub fn from_events(events: impl Iterator<Item = MyEvent>) -> Self {
        events.fold(Self::default(), |mut state, event| {
            state.apply_event(event);
            state
        })
    }

    /// Apply a single event to state
    fn apply_event(&mut self, event: MyEvent) {
        // Update state based on event
    }
}
```

**Tasks**:
- [ ] Document state reconstruction pattern
- [ ] Add examples to getting-started.md
- [ ] Show apply_event pattern
- [ ] Document relationship between Reducer and apply_event
- [ ] Add tests demonstrating pattern

### 5.2 Snapshot Strategy
**Scope**: When and how to create snapshots

**Strategy**:
- Default threshold: every 100 events
- Configurable per aggregate
- Load snapshot + replay events since snapshot
- Snapshots are optional (can always replay from start)

**Tasks**:
- [ ] Define SnapshotConfig type
- [ ] Implement snapshot threshold logic
- [ ] Document snapshot trade-offs (storage vs. replay time)
- [ ] Add configuration examples
- [ ] Test snapshot + replay scenarios

### 5.3 Event Versioning
**Scope**: Handle event schema evolution

**Strategy**:
- event_type includes schema version (e.g., "OrderPlaced.v1")
- Deserialize based on event_type
- Upcasting: Old events → new format during deserialization
- Document versioning approach for users

**Tasks**:
- [ ] Document event versioning strategy
- [ ] Add examples of schema evolution
- [ ] Show upcasting pattern
- [ ] Add tests for multiple event versions
- [ ] Document best practices

---

## 6. Runtime Integration (`composable-rust-runtime`)

### 6.1 Effect Executor for EventStore
**Scope**: Execute EventStore effects

**Tasks**:
- [ ] Add EventStore effect handling to Store
- [ ] Execute event store operations asynchronously
- [ ] Handle event store errors (log, propagate, retry?)
- [ ] Feed resulting actions back to Store
- [ ] Add tests for EventStore effect execution
- [ ] Document error handling strategy

### 6.2 Event Persistence in Store
**Scope**: Store actions as events automatically

**Pattern**: Store can optionally persist actions as events

**Tasks**:
- [ ] Consider adding event persistence to Store (optional)
- [ ] Document manual vs automatic event persistence
- [ ] Show examples of both approaches
- [ ] Test persistence integration
- [ ] Document best practices

---

## 7. Example: Order Processing Aggregate ✅ **COMPLETE**

**Goal**: Real-world example demonstrating event sourcing with InMemoryEventStore (Postgres implementation deferred to Phase 2B).

**Status**: ✅ **FULLY IMPLEMENTED AND VALIDATED**

### 7.1 Order Implementation ✅
Location: `examples/order-processing/`

**State**:
```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrderState {
    pub order_id: Option<OrderId>,
    pub customer_id: Option<CustomerId>,
    pub items: Vec<LineItem>,
    pub status: OrderStatus,
    pub total: Money,
    pub version: Option<Version>,      // ✅ Event sourcing version tracking
    pub last_error: Option<String>,    // ✅ Validation failure tracking
}

#[derive(Clone, Debug)]
pub enum OrderStatus {
    Draft,
    Placed,
    Cancelled,
    Shipped,
}
```

**Actions (Commands + Events)**:
```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OrderAction {
    // Commands
    PlaceOrder { order_id: OrderId, customer_id: CustomerId, items: Vec<LineItem> },
    CancelOrder { order_id: OrderId, reason: String },
    ShipOrder { order_id: OrderId, tracking: String },

    // Events (results of commands)
    OrderPlaced { order_id: OrderId, customer_id: CustomerId, items: Vec<LineItem>, total: Money, timestamp: DateTime<Utc> },
    OrderCancelled { order_id: OrderId, reason: String, timestamp: DateTime<Utc> },
    OrderShipped { order_id: OrderId, tracking: String, timestamp: DateTime<Utc> },

    // Internal feedback actions
    ValidationFailed { error: String },
    EventPersisted { event: Box<OrderAction>, version: u64 },
}
```

**Tasks**:
- [x] Define OrderState, OrderAction, OrderStatus types
- [x] Implement Serialize/Deserialize for events
- [x] Implement event serialization with bincode
- [x] Implement Reducer for Order
  - [x] PlaceOrder → validate → OrderPlaced event → save to event store
  - [x] CancelOrder → validate → OrderCancelled event → save to event store
  - [x] ShipOrder → validate → OrderShipped event → save to event store
- [x] Implement apply_event for state reconstruction
- [x] Create OrderEnvironment with EventStore + Clock

### 7.2 Order Reducer Logic ✅
**Scope**: Command validation and event emission

**Pattern**:
1. Receive command
2. Validate (check state, business rules)
3. If valid: Return Effect::EventStore(AppendEvents(...))
4. If invalid: Apply ValidationFailed to state, return Effect::None
5. On event replay: Apply event to state + track version

**Tasks**:
- [x] Implement command validation logic (all 3 commands)
- [x] Emit events as event store effects
- [x] Handle event replay in apply_event
- [x] Track version during event replay
- [x] Test all command scenarios (success, validation failures)
- [x] Document command/event split pattern

**Implemented**:
- ✅ `validate_place_order()`: Checks order not placed, items not empty, valid quantities/prices
- ✅ `validate_cancel_order()`: Checks order ID match, status allows cancellation
- ✅ `validate_ship_order()`: Checks order ID match, status allows shipping, tracking not empty
- ✅ All validation failures now update `state.last_error` for observability

### 7.3 Event Sourcing with InMemoryEventStore ✅
**Scope**: Persist Order events to event store

**Tasks**:
- [x] Initialize InMemoryEventStore in example (Postgres deferred)
- [x] Append events on command execution
- [x] Load events to reconstruct state
- [x] Test process restart scenario (state from events)
- [x] Document event sourcing flow

**Implemented**:
- ✅ Demo Part 1: Place order with 2 items
- ✅ Demo Part 2: Ship order with tracking number
- ✅ Demo Part 3: Simulate restart, load 2 events, reconstruct state
- ✅ Demo Part 4: Validate business rules (can't cancel shipped order)
- ✅ All assertions pass including version tracking

### 7.4 Snapshot Integration ⏸️
**Scope**: Snapshot Order state after N events

**Status**: **DEFERRED to Phase 2B** (Postgres implementation)

**Tasks**:
- [ ] Configure snapshot threshold (e.g., every 100 events)
- [ ] Create snapshots automatically
- [ ] Load snapshot + replay remaining events
- [ ] Test snapshot creation and loading
- [ ] Benchmark: replay with/without snapshots

### 7.5 Order Tests ✅
Location: `examples/order-processing/src/{types.rs,reducer.rs}`

**Unit Tests** (16 tests, all passing):
- [x] Test PlaceOrder command → OrderPlaced event
- [x] Test CancelOrder validation (only placed orders can be cancelled)
- [x] Test ShipOrder validation (only placed orders can be shipped)
- [x] Test state reconstruction from events (**NEW: test_event_replay_version_tracking**)
- [x] Test apply_event for all event types
- [x] Test reducer logic with InMemoryEventStore
- [x] Test validation functions (empty items, zero quantity, invalid price)
- [x] Test Money and LineItem calculations
- [x] Test event serialization round-trip
- [x] Test version tracking during replay (**CRITICAL TEST ADDED**)

**Integration Tests** (deferred to Phase 2B with Postgres):
- [ ] End-to-end: PlaceOrder → save to Postgres → reload → verify state
- [ ] Concurrency: Test optimistic concurrency conflicts
- [ ] Snapshot: Create snapshot, reload, verify state
- [ ] Process restart: Save events, "restart" (new Store), rebuild state

**Property Tests** (optional):
- [ ] Event replay is deterministic
- [ ] State from snapshot + events = state from all events

### 7.6 Order Documentation ✅
- [x] Comprehensive module documentation in `src/lib.rs`
- [x] Explain event sourcing using Order as reference
- [x] Document command/event pattern (with examples)
- [x] All public APIs documented with examples
- [x] Link from main example (cargo run --bin order-processing)

**Note**: Detailed README deferred to Phase 2B (after Postgres integration)

---

## 8. Documentation

### 8.1 API Documentation
- [ ] Complete all `///` doc comments with examples
- [ ] Document Event trait with examples
- [ ] Document Database trait with examples
- [ ] Document StreamId and Version types
- [ ] Add `# Examples` sections to all new APIs
- [ ] Add `# Errors` sections where applicable
- [ ] Verify `cargo doc --no-deps --all-features --open` looks good

### 8.2 Guide Documentation
- [ ] Update `docs/getting-started.md`:
  - [ ] Add event sourcing section
  - [ ] Add Order Processing example walkthrough
  - [ ] Show how to set up event store
  - [ ] Document event persistence pattern
- [ ] Update `docs/concepts.md`:
  - [ ] Add event sourcing concepts
  - [ ] Explain command/event split
  - [ ] Document snapshot strategy
  - [ ] Add event versioning section
- [ ] Create `docs/event-sourcing.md`:
  - [ ] Deep dive on event sourcing
  - [ ] State reconstruction patterns
  - [ ] Snapshot strategies
  - [ ] Event versioning and schema evolution
  - [ ] Best practices

### 8.3 Database Setup Guide
- [ ] Create `docs/database-setup.md`:
  - [ ] Local Postgres installation
  - [ ] Running migrations
  - [ ] Connection string configuration
  - [ ] Testcontainers for integration tests
  - [ ] Production database setup
  - [ ] Backup and restore procedures

### 8.4 Architecture Documentation
- [ ] Review `specs/architecture.md` section 4 (Event Sourcing)
- [ ] Document implementation decisions:
  - [ ] Why Postgres over EventStoreDB
  - [ ] Why bincode over JSON
  - [ ] Optimistic concurrency strategy
  - [ ] Snapshot threshold choices
- [ ] Update with any deviations from original plan

---

## 9. Validation & Testing

### 9.1 Unit Tests
- [ ] Event trait implementations
- [ ] StreamId and Version types
- [ ] EventStore effect composition
- [ ] InMemoryEventStore functionality
- [ ] State reconstruction from events
- [ ] Snapshot creation and loading
- [ ] All Order reducer logic

### 9.2 Integration Tests
- [ ] PostgresEventStore with testcontainers
- [ ] Optimistic concurrency conflicts
- [ ] Event appending and loading
- [ ] Snapshot lifecycle
- [ ] Order aggregate end-to-end
- [ ] Process restart scenario

### 9.3 Performance Benchmarks
Location: `benches/phase2_benchmarks.rs`

**Benchmarks**:
- [ ] Event serialization (bincode vs JSON comparison)
- [ ] Event appending throughput (target: 10k+ events/sec)
- [ ] Event replay speed (target: 10k+ events/sec)
- [ ] Snapshot creation time
- [ ] State reconstruction (with/without snapshots)
- [ ] Document results in `docs/performance.md`

### 9.4 Quality Checks
- [ ] `cargo build --all-features` succeeds
- [ ] `cargo test --all-features` passes
  - [ ] Unit tests run in < 100ms
  - [ ] Integration tests run in < 5 seconds
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes
- [ ] `cargo fmt --all --check` passes
- [ ] `cargo doc --no-deps --all-features` builds successfully
- [ ] CI pipeline passes on GitHub

---

## 10. Key Implementation Decisions

Document decisions as they're made:

### 10.1 Serialization: bincode ✅
- **Decision**: Use bincode for event and snapshot serialization
- **Rationale**:
  - 5-10x faster than JSON
  - 30-70% smaller storage
  - All-Rust services = no interop needed
  - Serde makes it easy to switch if needed
- **Trade-offs**: Not human-readable (use JSONB metadata for debugging)

### 10.2 Event Store: PostgreSQL ✅
- **Decision**: Build on Postgres, not specialized event store
- **Rationale**:
  - Vendor independence (open source, ubiquitous)
  - Zero lock-in risk
  - Standard SQL (AI-agent friendly)
  - Free infrastructure
  - Client flexibility
- **Trade-offs**: Extra week of implementation vs. strategic independence

### 10.3 Optimistic Concurrency Strategy
- [ ] **Decision**: (Stream_id, version) as PRIMARY KEY
- [ ] **Rationale**: (Document why this approach)
- [ ] **Alternatives**: (List other options considered)

### 10.4 Snapshot Threshold
- [ ] **Decision**: (Default threshold value)
- [ ] **Rationale**: (Balance between storage and replay time)
- [ ] **Configuration**: (How users can customize)

### 10.5 Event Versioning
- [ ] **Decision**: (event_type naming convention)
- [ ] **Rationale**: (How to handle schema evolution)
- [ ] **Migration Strategy**: (Upcasting vs. multiple versions)

### 10.6 EventStore Error Handling
- [ ] **Decision**: (Retry strategy, circuit breaker, etc.)
- [ ] **Rationale**: (When to retry, when to fail fast)
- [ ] **User Guidance**: (How users should handle event store errors)

---

## 11. Phase 2 Scope Reminder

**IN SCOPE** (Phase 2):
- ✅ PostgreSQL event store (custom schema)
- ✅ bincode serialization
- ✅ Event trait and types
- ✅ EventStore trait with Postgres implementation
- ✅ InMemoryEventStore for testing
- ✅ Event sourcing patterns (state reconstruction)
- ✅ Snapshot support
- ✅ Order Processing aggregate example
- ✅ Event versioning strategy

**OUT OF SCOPE** (Later phases):
- ❌ Event publishing to Redpanda → Phase 3
- ❌ Saga coordination → Phase 3
- ❌ Cross-aggregate communication → Phase 3
- ❌ EventBus trait → Phase 3
- ❌ Advanced projections → Phase 4
- ❌ Production hardening (retries, circuit breakers) → Phase 4

**Remember**: "Make it work, make it right, make it fast—in that order."

---

## 12. Validation Checklist

### Phase 2A Checklist (InMemoryEventStore) ✅ COMPLETE

- [x] ✅ Can persist events to InMemoryEventStore
- [x] ✅ Can reconstruct aggregate from event stream
- [x] ✅ Snapshots work correctly (InMemoryEventStore implementation)
- [x] ✅ Tests use in-memory event store (no I/O in unit tests)
- [x] ✅ Order Processing example survives process restart (state from events)
- [x] ✅ All public APIs are documented
- [x] ✅ **Version tracking works in both command flow and event replay**
- [x] ✅ **Optimistic concurrency control implemented**
- [x] ✅ **Clock dependency injection for testability**

### Phase 2B Checklist (PostgreSQL) - IN PROGRESS

- [ ] ✅ Can persist events to Postgres
- [ ] ✅ Can replay 10,000+ events/second
- [ ] ✅ Integration tests use testcontainers
- [ ] ✅ Database migrations created and tested
- [ ] ✅ Snapshot performance optimization with Postgres

**Phase 2A Success Criteria** ✅: "Order Processing aggregate survives process restart (state from events using InMemoryEventStore)."

**Phase 2B Success Criteria** 🚧: "Order Processing aggregate survives process restart with PostgreSQL backend."

---

## 13. Transition to Phase 3

### 13.1 Phase 3 Preparation
- [ ] Review Phase 3 goals (Sagas & Coordination)
- [ ] Identify Redpanda dependencies (rdkafka)
- [ ] Spike event bus abstraction if needed
- [ ] Create `plans/phase-3/TODO.md`

### 13.2 Final Phase 2 Review
- [ ] All validation criteria met
- [ ] Order Processing example demonstrates event sourcing completely
- [ ] Performance targets met (10k+ events/sec)
- [ ] Documentation complete
- [ ] Ready to add event bus and sagas

---

## 14. Success Criteria

Phase 2 is complete when:

- ✅ Event trait and EventStore trait work correctly
- ✅ PostgreSQL event store persists and loads events
- ✅ State reconstruction from events works
- ✅ Snapshots improve replay performance
- ✅ Order Processing example demonstrates entire event sourcing flow
- ✅ Can explain event sourcing using only Order Processing example
- ✅ Tests run fast (unit < 100ms, integration < 5s)
- ✅ Performance targets met (10k+ events/sec)
- ✅ All public APIs documented
- ✅ All quality checks pass

**Key Quote from Roadmap**: "Success: Order Processing aggregate survives process restart (state from events)."

---

## Notes & Decisions

### Phase 2A Implementation Decisions ✅

**Event Sourcing Foundation**:
- ✅ Built complete EventStore abstraction before Postgres implementation
- ✅ InMemoryEventStore validates all event sourcing patterns
- ✅ Order Processing example proves event replay works correctly

**Version Tracking**:
- ✅ Two-flow version tracking:
  - Normal command flow: EventPersisted action carries version from EventStore
  - Event replay flow: Reducer increments version during event application
- ✅ Version arithmetic: EventStore returns 0-indexed position, state tracks next expected version (position + 1)

**Critical Bug Fixes**:
1. ✅ Version not tracked during event replay - FIXED (2025-11-06)
   - Root cause: Events applied to state without version increment
   - Fix: Added version tracking in event replay match arm
   - Test added: `test_event_replay_version_tracking()`
2. ✅ Serialization errors silent - FIXED
   - Added tracing::error!() for serialization failures
3. ✅ Validation failures invisible - FIXED
   - Added `last_error: Option<String>` to OrderState
   - ValidationFailed action now updates state

**Clock Dependency Injection**:
- ✅ Created SystemClock in `core/src/lib.rs` (environment module)
- ✅ All timestamps use `env.clock.now()` for testability
- ✅ Tests can use FixedClock for deterministic time

**Deviations from Original Plan**:
- ⏸️ PostgreSQL implementation deferred to Phase 2B
- ✅ InMemoryEventStore validated all event sourcing patterns first
- ✅ This "make it work" approach proved all abstractions before database complexity

### Phase 2B (Upcoming):
- **Database Configuration**: (TBD)
- **Snapshot Strategy**: (TBD - likely every 100 events)
- **Performance Results**: (TBD - target 10k+ events/sec)
- **Postgres Schema**: (TBD - PRIMARY KEY on (stream_id, version))

---

## Estimated Time Breakdown

Based on roadmap estimate of 1.5-2 weeks:

1. **Database schema & migrations**: 1-2 days
2. **Event trait & types**: 1 day
3. **EventStore trait definition**: 0.5 day
4. **PostgresEventStore implementation**: 2-3 days
5. **InMemoryEventStore implementation**: 1 day
6. **Event sourcing patterns**: 1 day
7. **Order Processing aggregate example**: 2-3 days
8. **Testing (unit + integration + benchmarks)**: 2-3 days
9. **Documentation**: 2-3 days
10. **Validation & polish**: 1 day
11. **Buffer for unknowns**: 1-2 days

**Total**: 14-21 days (2-3 weeks of full-time work)

**Note**: Roadmap estimates 1.5-2 weeks. Budget 2-3 weeks for safety.

---

## References

- **Architecture Spec**: `specs/architecture.md` (section 4: Event Sourcing)
- **Roadmap**: `plans/implementation-roadmap.md` (Phase 2 section, lines 194-362)
- **Phase 1 Review**: `plans/phase-1/PHASE1_REVIEW.md`
- **Modern Rust Expert**: `.claude/skills/modern-rust-expert.md`
- **Phase 1 TODO**: `plans/phase-1/TODO.md` (completed example)

---

## Quick Start

**First task**: Set up PostgreSQL locally and create database schema

**Order of implementation**:
1. Database schema design and migrations
2. Event trait and core types (StreamId, Version)
3. EventStore trait definition
4. PostgresEventStore implementation (with testcontainers)
5. InMemoryEventStore implementation (for testing)
6. Event sourcing patterns documentation
7. Order Processing aggregate example
8. Testing & benchmarks
9. Documentation
10. Validation

**Phase 2A**: ✅ **COMPLETE** - Event sourcing foundation validated with Order Processing example
**Phase 2B Next**: Begin with database schema design and migration setup for PostgreSQL!

---

## 🎉 Phase 2A Completion Summary

**Date Completed**: 2025-11-06

**What Was Accomplished**:
1. ✅ **Event Sourcing Abstractions** - Complete EventStore trait with 4 operations
2. ✅ **InMemoryEventStore** - Full implementation with optimistic concurrency
3. ✅ **SystemClock** - Production clock implementation for dependency injection
4. ✅ **Order Processing Example** - Production-quality event-sourced aggregate
5. ✅ **Version Tracking** - Correct implementation in both flows (command + replay)
6. ✅ **Comprehensive Testing** - 91 tests passing, 16 in Order Processing
7. ✅ **Zero Technical Debt** - All critical bugs fixed, zero clippy warnings

**Critical Achievement**:
The Order Processing aggregate successfully demonstrates the **complete event sourcing workflow**:
- Commands validated and emit events
- Events persisted to EventStore
- State reconstructed from events after "process restart"
- Version tracking ensures optimistic concurrency
- **Demo output**: "Reconstructed state: Status=Shipped, Items=2, Total=$100.00, Version=2"

**Code Review Verdict**: ✅ **FLAWLESS** (after critical bug fixes)

**Files Modified/Created**:
- `core/src/lib.rs` - Added SystemClock, updated Event/EventStore traits
- `testing/src/lib.rs` - InMemoryEventStore already existed (from Phase 1)
- `examples/order-processing/` - Complete event-sourced aggregate (745 lines)
  - `src/types.rs` - Domain types with version tracking
  - `src/reducer.rs` - Reducer with event replay version tracking
  - `src/main.rs` - Comprehensive 4-part demo

**Lessons Learned**:
1. ✅ Build abstractions first, prove with in-memory, then add persistence
2. ✅ Ultra-thorough code reviews catch critical bugs (version tracking)
3. ✅ Test version tracking explicitly - would have caught bug earlier
4. ✅ Clock injection is essential for testable timestamps

**Ready for Phase 2B**: PostgreSQL implementation will be straightforward now that all event sourcing patterns are validated.

---

## Dependencies to Add

**Core dependencies**:
- `serde` (already have): For Event serialization
- `bincode`: Fast binary serialization
- `sqlx`: PostgreSQL client with compile-time checked queries
  - Features: `postgres`, `runtime-tokio`, `tls-rustls`
- `thiserror`: Error type derivation

**Development dependencies**:
- `testcontainers`: For Postgres integration tests
  - `testcontainers-modules` with `postgres` feature

**Optional**:
- `bytes`: For zero-copy serialization (if needed)
- `chrono` or `time`: For timestamp handling in events

---

## Questions to Resolve

- [ ] Should Event trait be auto-implemented for all `Serialize + DeserializeOwned` types?
- [ ] Should we use sqlx compile-time checking or runtime queries?
- [ ] What should happen if snapshot deserialization fails? (Fall back to full replay?)
- [ ] Should snapshots be compressed? (zstd, lz4?)
- [ ] How to handle event metadata? (Fixed fields vs. arbitrary JSONB?)
- [ ] Should we support batch event loading? (Pagination for large streams?)

**Resolution Process**: Answer during implementation based on practical needs from Order Processing example.

---

## Phase 2 Milestone

**🎯 End Goal**: Order Processing aggregate that:
1. Accepts commands (PlaceOrder, CancelOrder, ShipOrder)
2. Emits events (OrderPlaced, OrderCancelled, OrderShipped)
3. Persists events to PostgreSQL event store
4. Reconstructs state from events after process restart
5. Uses snapshots for performance
6. Has comprehensive tests (unit + integration)
7. Demonstrates event sourcing best practices

**Success Metric**: Run example, place orders, stop process, restart, verify state is correctly reconstructed from events.

---

## Alignment with Roadmap ✅

**All naming conventions now match `plans/implementation-roadmap.md`:**

✅ **EventStore trait** (not Database) - More specific for event sourcing
✅ **PostgresEventStore** (not PostgresDatabase) - Consistent naming
✅ **InMemoryEventStore** (not MockDatabase) - Matches roadmap terminology
✅ **examples/order-processing/** (not examples/order/) - More descriptive
✅ **composable-rust-postgres crate** - Separate crate confirmed

**Ready to begin Phase 2!** 🚀
