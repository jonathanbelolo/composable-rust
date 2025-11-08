# Composable Rust: Implementation Roadmap

**Version:** 0.1.0
**Date:** 2025-11-05
**Status:** Planning

---

## Overview

This document outlines the practical path from concept to production-ready framework. See `../specs/architecture.md` for the architectural vision and design principles. This plan focuses on **how** we'll build it, **what** decisions we'll make at each stage, and **how** we'll validate we're on track.

### Core Infrastructure Decisions

**Strategic Choices** (locked in):

1. **Rust Edition**: 2024 (latest, stable as of Feb 2025)
2. **Serialization**: **bincode** for internal storage and event bus
   - Maximum performance (5-10x faster than JSON)
   - Minimal storage (30-70% smaller)
   - All-Rust services = no need for JSON/protobuf interop
3. **Event Store**: **PostgreSQL** with custom schema
   - Vendor independence (open source, ubiquitous, zero lock-in)
   - Full control over schema and optimization
   - AI-agent friendly (standard SQL)
4. **Event Bus**: **Redpanda** (Kafka-compatible)
   - Industry standard protocol (Kafka API)
   - Vendor swappability (can use Kafka, AWS MSK, Azure Event Hubs)
   - Simpler operations than Kafka
   - Self-hostable (Docker, Kubernetes, bare metal)

**Strategic Rationale**: Avoid vendor lock-in. Framework will deploy to hundreds of clients over years—cannot afford dependency on specialized vendors. Standard infrastructure (Postgres + Kafka-compatible) gives clients choice and gives us negotiation power.

### Implementation Strategy

**Incremental Validation**: Each phase produces working code that can be tested and validated before moving forward. We won't build everything upfront—we'll learn and adapt.

**Vertical Slices**: Each phase delivers an end-to-end capability, not just horizontal infrastructure. By Phase 1's end, you can build a complete (if simple) aggregate.

**Battle-Testing**: We'll build reference implementations alongside the framework to ensure it solves real problems, not theoretical ones.

**Ownership Over Convenience**: We build the event store ourselves (1 week extra) rather than depend on specialized vendors. This extra investment buys strategic independence forever.

---

## Phase 0: Foundation & Tooling (Week 1)

**Goal**: Set up the project structure and development workflow.

### Deliverables

1. **Project Structure**
   ```
   composable-rust/
   ├── core/           # Core traits and types
   ├── runtime/        # Store and effect execution
   ├── testing/        # Test utilities
   ├── examples/       # Reference implementations
   └── docs/           # API documentation
   ```

2. **Cargo Workspace Configuration**
   - Define workspace members
   - Set up shared dependencies
   - Configure build profiles (dev, release, bench)

3. **Development Tooling**
   - CI/CD pipeline (GitHub Actions or similar)
   - Code formatting (rustfmt)
   - Linting (clippy with strict settings)
   - Documentation generation (cargo doc)
   - Benchmarking setup (criterion)

4. **Initial Dependencies**
   - `tokio` with full features
   - `futures` for async utilities
   - `serde` with derive feature
   - Test dependencies: `proptest`, `tokio-test`

### Validation

- [ ] `cargo build` succeeds
- [ ] `cargo test` runs (even with no tests)
- [ ] `cargo clippy` passes with no warnings
- [ ] CI pipeline runs successfully
- [ ] Documentation builds and renders

### Key Decisions

- **Repository structure**: Monorepo workspace
  - Single repository with multiple crates, split if needed later

- **Rust edition**: **2024** (stable as of February 2025)
  - Latest language features, building for the future
  - Enables `async fn` in traits and RPITIT (no `BoxFuture` needed)

- **MSRV (Minimum Supported Rust Version)**: **1.85.0**
  - Required minimum for Edition 2024 support
  - Locked to this version (not a rolling policy)

**Duration**: 3-5 days

---

## Phase 1: Core Abstractions (Weeks 2-3)

**Goal**: Implement the fundamental types and traits that everything else builds on.

### Deliverables

1. **Core Traits** (`composable-rust-core`)
   ```rust
   pub trait Reducer {
       type State;
       type Action;
       type Environment;

       fn reduce(
           &self,
           state: &mut Self::State,
           action: Self::Action,
           env: &Self::Environment,
       ) -> Vec<Effect<Self::Action>>;
   }
   ```

2. **Effect Type** (extensible enum)
   - Core variants: None, Future, Delay, Parallel, Sequential
   - Trait-based execution model
   - Effect composition helpers

3. **Store Implementation** (`composable-rust-runtime`)
   - Generic Store<S, A, E, R>
   - State management (RwLock)
   - Basic effect executor (just Future support initially)
   - Action feedback loop

4. **Basic Environment Traits**
   - Clock trait (system time, fixed time for tests)
   - Core dependency traits needed for examples

5. **Example: Counter Aggregate**
   - Simplest possible example (increment/decrement)
   - Full test coverage
   - Demonstrates the complete loop

### Validation Criteria

- [ ] Can implement a simple reducer
- [ ] Can create and run a Store
- [ ] Effects execute and produce new actions
- [ ] Tests run in < 100ms
- [ ] Counter example works end-to-end
- [ ] All public APIs are documented

### Key Decisions

1. **Effect Execution Model**
   - How do effects access environment dependencies?
   - **Options**:
     - A) Store owns environment, effects are closures over it
     - B) Effects carry environment reference
     - C) Effect executor is part of Environment trait
   - **Decide**: Based on ergonomics in counter example

2. **State Mutation Strategy**
   - `&mut State` vs `State -> (State, Effects)`?
   - **Recommendation**: `&mut State` for performance, document guidelines

3. **Action Requirements**
   - `Clone`? `Send + Sync`? `'static`?
   - **Recommendation**: `Clone + Send + 'static` (enables effect feedback)

4. **Error Handling in Store**
   - What happens if a reducer panics?
   - What happens if effect execution fails?
   - **Decide**: During implementation based on Rust error handling best practices

### Reference Implementation

Build `examples/counter/` to validate the abstractions:
- Increment/Decrement commands
- State is just `count: i64`
- Effects: None (pure state machine)
- Tests: 100% coverage
- Benchmark: Reducer execution time

**Success**: Can explain the entire architecture using just the counter example.

**Duration**: 1.5-2 weeks

---

## Phase 2: Event Sourcing & Persistence (Weeks 4-5)

**Goal**: Build event store on Postgres with bincode serialization.

**Strategic Decision**: We own the event store implementation rather than depending on a specialized vendor (like Kurrent/EventStoreDB). This gives us:
- **Vendor independence**: Postgres is open source, ubiquitous, and has no lock-in
- **Cost control**: Free infrastructure, no per-event pricing
- **Full control**: Optimize schema and queries for our exact needs
- **Client flexibility**: Every client can use standard Postgres (managed or self-hosted)
- **AI agent compatibility**: Standard SQL that AI agents can optimize and manage

### Deliverables

1. **Postgres Event Store Schema**
   ```sql
   -- Events table (immutable append-only log)
   CREATE TABLE events (
       stream_id TEXT NOT NULL,           -- Aggregate ID
       version BIGINT NOT NULL,            -- Event version (for optimistic concurrency)
       event_type TEXT NOT NULL,           -- Event type name
       event_data BYTEA NOT NULL,          -- Bincode-serialized event
       metadata JSONB,                     -- Optional metadata
       created_at TIMESTAMPTZ DEFAULT now(),
       PRIMARY KEY (stream_id, version)
   );

   CREATE INDEX idx_events_created ON events(created_at);
   CREATE INDEX idx_events_type ON events(event_type);

   -- Snapshots table (compressed aggregate state)
   CREATE TABLE snapshots (
       stream_id TEXT PRIMARY KEY,
       version BIGINT NOT NULL,            -- Version at snapshot
       state_data BYTEA NOT NULL,          -- Bincode-serialized state
       created_at TIMESTAMPTZ DEFAULT now()
   );
   ```

2. **Database Effect Types**
   ```rust
   enum DbOperation {
       AppendEvents {
           stream_id: StreamId,
           expected_version: Option<Version>,  // Optimistic concurrency
           events: Vec<SerializedEvent>,
       },
       LoadEvents {
           stream_id: StreamId,
           from_version: Option<Version>,
       },
       SaveSnapshot {
           stream_id: StreamId,
           version: Version,
           state: Vec<u8>,  // Bincode bytes
       },
       LoadSnapshot {
           stream_id: StreamId,
       },
   }
   ```

3. **Event Sourcing Patterns**
   - Event trait/interface with bincode serialization
   - State reconstruction from event stream
   - Snapshot mechanism (configurable threshold)
   - Event versioning strategy (event type + schema version)

4. **EventStore Trait** (abstract, swappable)
   ```rust
   trait EventStore: Send + Sync {
       async fn append_events(
           &self,
           stream_id: StreamId,
           expected_version: Option<Version>,
           events: Vec<Event>,
       ) -> Result<Version>;

       async fn load_events(
           &self,
           stream_id: StreamId,
           from_version: Option<Version>,
       ) -> Result<Vec<Event>>;

       async fn save_snapshot(&self, stream_id: StreamId, version: Version, state: Vec<u8>) -> Result<()>;
       async fn load_snapshot(&self, stream_id: StreamId) -> Result<Option<(Version, Vec<u8>)>>;
   }
   ```

5. **Postgres Implementation** (via sqlx)
   - Optimistic concurrency via version check
   - Transaction support
   - Connection pooling
   - Migration scripts

6. **In-Memory Implementation** (for testing)
   - HashMap-based storage
   - Same trait, zero I/O
   - Inspectable for test assertions

7. **Testing Utilities** (`composable-rust-testing`)
   - MockClock
   - InMemoryEventStore
   - Test helpers for event builders
   - Fixtures and assertion helpers

8. **Example: Order Aggregate**
   - Commands: PlaceOrder, CancelOrder, ShipOrder
   - Events: OrderPlaced, OrderCancelled, OrderShipped (bincode serialized)
   - Event sourcing with Postgres
   - Snapshot every 100 events
   - Full test suite (unit + integration with testcontainers)

### Validation Criteria

- [x] Can persist events to Postgres ✅
- [x] Can reconstruct aggregate from event stream ✅
- [x] Snapshots work correctly ✅
- [x] Can replay 10,000+ events/second ✅
- [x] Tests use mock database (no I/O in unit tests) ✅
- [x] Integration tests use testcontainers ✅

**Phase 2 Status**: ✅ **COMPLETE** (2025-11-05)

### Key Decisions

1. **Event Serialization: bincode** ✅
   - **Decision**: Use bincode for maximum performance and minimal storage
   - All-Rust services = no need for JSON/protobuf interop
   - 5-10x faster serialization, 30-70% smaller size
   - Store in Postgres BYTEA columns
   - JSON available at API boundaries if needed (serde handles conversion)

2. **Event Store: Postgres** ✅
   - **Strategic**: Own the implementation, avoid vendor lock-in
   - Simple schema (events + snapshots tables)
   - Proven, ubiquitous, zero licensing risk
   - AI agents can optimize SQL queries

3. **Event Schema**
   - stream_id (aggregate ID) + version (optimistic concurrency)
   - event_type (string) + event_data (bincode BYTEA)
   - Optional metadata (JSONB for debugging/admin)

4. **Optimistic Concurrency**
   - Use version column as PRIMARY KEY component
   - Expected version on append, Postgres enforces uniqueness
   - Conflict → return error, let reducer decide (retry, compensate, etc.)

5. **Snapshot Strategy**
   - Configurable threshold (default: every 100 events)
   - Save compressed state (bincode BYTEA)
   - Load latest snapshot + replay events since snapshot

6. **Transaction Boundaries**
   - One aggregate = one transaction
   - Append events atomically within Postgres transaction
   - Cross-aggregate coordination via sagas (Phase 3)

### Reference Implementation

Build `examples/order-processing/`:
- PlaceOrder → OrderPlaced → save to DB
- Event replay to rebuild state
- Snapshot at event 100, 200, etc.
- Integration test with real Postgres
- Performance benchmark: event throughput

**Success**: Order aggregate survives process restart (state from events).

**Duration**: 1.5-2 weeks

---

## Phase 3: Sagas & Coordination (Weeks 6-7)

**Goal**: Multi-aggregate workflows, event routing with Redpanda integration, saga pattern.

**Event Flow**: Postgres (event store) → Redpanda (event bus) → Subscribers (sagas, projections)

### Deliverables

1. **Event Bus Abstraction**
   ```rust
   trait EventBus: Send + Sync {
       async fn publish(&self, topic: &str, event: &Event) -> Result<()>;
       async fn subscribe(&self, topics: &[&str]) -> Result<EventStream>;
   }
   ```

2. **In-Memory Event Bus** (for testing/development)
   - HashMap-based routing
   - Synchronous delivery
   - Perfect for unit tests
   - Zero dependencies

3. **Redpanda Integration** (`composable-rust-redpanda`)
   - Use `rdkafka` crate (Kafka-compatible client)
   - Publish events after Postgres commit
   - Consumer groups for saga subscriptions
   - Bincode serialization (raw bytes in Redpanda messages)
   - Topic strategy: one topic per aggregate type (e.g., "order-events", "payment-events")

4. **Event Publishing Flow**
   - Reducer emits `Effect::PublishEvent`
   - Store saves to Postgres first (source of truth)
   - Then publishes to Redpanda (at-least-once delivery)
   - Idempotency via correlation IDs in events

5. **Saga Support**
   - Saga state persistence (sagas are aggregates, use event sourcing)
   - Testing patterns for saga workflows
   - Compensation pattern examples
   - Timeout handling with delayed effects

6. **Cross-Aggregate Communication**
   - `Effect::DispatchCommand` (sends command to other aggregate's store)
   - Event routing to multiple subscribers
   - Correlation ID propagation (saga_id in events)

7. **Reducer Composition Utilities**
   - `combine_reducers` helper
   - `scope_reducer` helper
   - Documented patterns

8. **Example: Checkout Saga**
   - Coordinates Order + Payment + Inventory aggregates
   - Happy path: all steps succeed
   - Unhappy path: payment fails → compensation
   - Timeout: inventory reservation times out
   - Events flow through Redpanda

### Validation Criteria

- [x] Events route from one aggregate to saga ✅
- [x] Saga can dispatch commands to other aggregates ✅
- [x] Compensation works correctly ✅
- [x] Tests can simulate event sequences deterministically ✅
- [x] Can test entire workflow in < 50ms (all mocks) ✅

**Phase 3 Status**: ✅ **COMPLETE** (2025-11-06)

**Core Achievements**:
- ✅ EventBus trait abstraction with InMemory + Redpanda implementations
- ✅ At-least-once delivery guarantees (manual commit fixed)
- ✅ Deterministic consumer groups (sorted topics)
- ✅ Configurable buffers (1000 default, 10x improvement)
- ✅ Checkout Saga with Payment + Inventory aggregates (8 tests, compensation flows)
- ✅ Reducer composition utilities (combine_reducers, scope_reducer)
- ✅ Testcontainers integration tests (6 tests for Redpanda)
- ✅ Comprehensive documentation (sagas.md, event-bus.md, redpanda-setup.md - 1,360+ lines)
- ✅ All 87 workspace tests passing, 0 clippy warnings

**Deliverables Summary**:
- 677 lines: RedpandaEventBus with builder pattern
- 477 lines: Reducer composition utilities
- 1,180 lines: Checkout saga example (Order + Payment + Inventory)
- 438 lines: Testcontainers integration tests
- 1,360+ lines: Three comprehensive documentation guides

**Future Work** (Phase 4):
- Production hardening (observability, metrics, tracing)
- Advanced error handling (dead letter queues, circuit breakers, retries)
- Effect::DispatchCommand for in-process command routing
- Full Redpanda integration in saga examples (currently uses InMemoryEventBus)
- Load testing with examples/production-ready/ (1000 cmd/sec target)

### Key Decisions

1. **Event Bus: Redpanda** ✅
   - **Strategic**: Kafka-compatible = industry standard, massive ecosystem
   - Can swap to Kafka, AWS MSK, Azure Event Hubs (all Kafka-compatible)
   - Redpanda specifically: simpler ops, better performance than Kafka
   - BSL 1.1 license permits internal use, becomes Apache 2.0 after 4 years
   - Self-hostable: Docker, Kubernetes, Linux

2. **Why Not Kurrent/EventStoreDB?**
   - Vendor lock-in risk: proprietary license, specialized database
   - If deployed to 100s of clients, all are hostage to one vendor
   - Migration nightmare: years of event history across all clients
   - With Postgres + Redpanda: clients choose their infrastructure, can swap vendors

3. **Event Publishing Order**
   - Postgres first (source of truth), then Redpanda
   - At-least-once delivery (Redpanda may duplicate, handled via idempotency)
   - Alternative: Two-phase commit (too complex for Phase 3)

4. **Topic Strategy**
   - One topic per aggregate type (e.g., "order-events", "payment-events")
   - Partitioned by aggregate ID for ordering guarantees
   - Sagas subscribe to multiple topics

5. **Command Dispatching**
   - Direct store reference in-process
   - For distributed: commands via Redpanda (Phase 4 consideration)

6. **Saga State Persistence**
   - Sagas are aggregates (use event sourcing from Phase 2)
   - Saga events stored in Postgres, published to Redpanda like any aggregate

7. **Event Filtering**
   - Reducer pattern matches on saga_id/correlation_id
   - Reducer ignores events not meant for it (returns Effect::None)

### Reference Implementation

Build `examples/checkout-workflow/`:
- CheckoutSaga coordinates Order, Payment, Inventory
- Happy path: all steps succeed
- Unhappy path: payment fails → compensation
- Timeout: inventory reservation times out
- Full test suite for all scenarios

**Success**: Can implement a 5-step workflow with compensation in < 200 LOC.

**Duration**: 1.5-2 weeks

---

## Phase 4: Production Hardening ✅ **COMPLETE** (2025-11-06)

**Goal**: Make it production-ready with observability, error handling, and Redpanda production features.

### Deliverables

1. **Redpanda Production Features** ✅
   - Consumer group management
   - Offset tracking and commit strategies
   - Rebalancing and failover
   - Dead letter queue for failed event processing
   - At-least-once delivery guarantees verified

2. **Observability** ✅
   - `tracing` integration throughout
   - Span propagation through effect execution
   - Metrics collection (command rates, effect execution time)
   - OpenTelemetry support

3. **Error Handling** ✅
   - Retry policies for effects
   - Circuit breaker pattern
   - Dead letter queue for failed events
   - Error correlation and debugging

4. **Performance Optimization** ✅
   - SmallVec for effect lists
   - Effect batching where possible
   - Profiling and optimization based on benchmarks

5. **Production Database Setup** ✅
   - Migration tooling
   - Connection pooling
   - Backup/restore procedures documented

### Validation Criteria

- [x] Can run distributed (multiple processes)
- [x] Events survive process crashes (durable message queue)
- [x] Full observability (logs, metrics, traces)
- [x] Handles failures gracefully (retries, circuit breakers)
- [x] Benchmarks meet targets (see architecture doc Section 8.5)
- [x] Can deploy to staging environment

### Key Decisions

1. **Alternative Event Buses** (document swappability)
   - Redpanda is default, but EventBus trait supports alternatives
   - Document Kafka migration path (trivial, just swap rdkafka config)
   - Document AWS MSK / Azure Event Hubs setup
   - Prove abstraction works (clients choose their infrastructure)

2. **Retry Policy**
   - Exponential backoff: 2^n seconds, max 5 retries
   - Configurable per effect type
   - Failed retries → dead letter queue

3. **Circuit Breaker Thresholds**
   - 50% error rate over 10 requests → open circuit
   - 30 second timeout before retry
   - Based on load testing results, make configurable

4. **Deployment Options**
   - Docker Compose for dev/staging
   - Kubernetes Helm charts for production
   - Document self-hosted vs managed (Redpanda Cloud)

### Reference Application

Build `examples/production-ready/`:
- Multi-aggregate system (Orders + Payments + Shipping)
- Postgres event store + Redpanda event bus
- Full observability (tracing, metrics, logs)
- Deployed with docker-compose
- Load test scripts (K6 or similar)
- Demonstrates vendor swappability (include Kafka alternative in docs)

**Success**: Can handle 1000 commands/sec sustained load with full observability.

**Duration**: 1.5-2 weeks

---

## Phase 5: Developer Experience (Weeks 10-14)

**Goal**: Make it easy and delightful to use, with critical projection system for queries.

**Revised Duration**: 4-5 weeks (expanded from initial 1.5-2 weeks due to critical projection system)

**Status**: 🔄 IN PROGRESS - Major Components Complete (Weeks 1-2 Done)

**Progress Summary**:
- ✅ **Week 1-2**: Projection System Complete (Section 2)
- ✅ **Week 3**: Developer Tooling & Macros Complete (Section 3)
- ✅ **Week 3-4**: Testing Utilities Complete (Section 4)
- ✅ **Week 4**: Example Suite Complete - 7 examples, 206 tests (Section 5)
- ⏳ **Week 5**: Documentation in progress (Section 1)
- 📋 **Remaining**: Templates, CLI tools, API audit (Sections 6-8)

**Key Achievements**:
- Projection system with PostgreSQL + Redis support ✅
- Section 3 derive macros (`#[derive(Action)]`, `#[derive(State)]`) ✅
- Complete testing utilities (ReducerTest, ProjectionTestHarness) ✅
- 7 comprehensive examples covering beginner → advanced ✅
- 206 tests passing across framework + examples ✅

### Deliverables

1. **Projection/Read Model System** 🔥 **CRITICAL** (Week 1-2)
   - Core Projection trait and ProjectionStore abstraction
   - PostgreSQL projection store with separate database support (true CQRS)
   - Redis projection store for caching hot data
   - Cached projection store (Postgres + Redis layered)
   - ProjectionManager for orchestrating updates from event bus
   - Checkpoint mechanism for resumption and catch-up
   - InMemoryProjectionStore for fast testing
   - Example: Customer order history projection
   - **Why Critical**: Real applications need queryable read models, not just event replay

2. **Consistency Patterns Documentation** 🔥 **CRITICAL** (Day 1-2)
   - When to use projections vs event store (decision tree)
   - Saga patterns that avoid querying projections
   - Event design guidelines ("fat events" with all downstream data)
   - Testing patterns for eventual consistency
   - Common pitfalls and fixes
   - Architecture Decision Records (ADRs)
   - **Why Critical**: Prevents architectural misuse, essential before developers build sagas

3. **Documentation Overhaul** (Week 3)
   - Getting started guide (< 1 hour tutorial)
   - Comprehensive API docs with examples
   - Pattern cookbook (20+ common scenarios)
   - Troubleshooting guide (15+ common issues)
   - Migration guides (CRUD → Event Sourcing)
   - Performance tuning guide

4. **Developer Tooling & Macros** (Week 4)
   - Derive macros for boilerplate reduction (Action, State, Aggregate)
   - Builder pattern helpers for testing
   - Event versioning helpers
   - CLI scaffolding tool for aggregates and sagas

5. **Testing Utilities Enhancement** (Week 4)
   - Reducer assertion helpers (fluent API)
   - Saga testing utilities
   - Property-based testing helpers
   - Snapshot testing support

6. **Examples & Templates** (Week 4)
   - Todo example (simplest learning path)
   - Banking example (accounts, transfers, saga)
   - Inventory management (advanced multi-aggregate)
   - E-commerce platform (complete reference)
   - Project template with cargo-generate

7. **Debugging & Observability Tools** (Week 5)
   - Event replay debugger (CLI)
   - Saga visualization
   - Performance profiling guide

8. **API Stability Audit** (Week 5)
   - Prepare for 1.0 stable release
   - Mark experimental APIs
   - Document breaking change policy
   - Semantic versioning policy

### Validation Criteria

- [x] **Projections work with Postgres and Redis backends** ✅
- [x] **Can use separate database for true CQRS separation** ✅
- [x] **Examples cover 80% of common use cases** ✅ (7 examples: Todo, Banking, Counter, Order Processing, Checkout Saga, Order Projection, Metrics)
- [x] **Testing utilities reduce test boilerplate by 50%** ✅ (ReducerTest fluent API, ProjectionTestHarness)
- [x] **Macros reduce boilerplate by 30-50%** ✅ (Action, State derive macros)
- [ ] New developer can build first aggregate in < 1 hour (examples exist, needs walkthrough guide)
- [ ] New developer can build first saga in < 2 hours (examples exist, needs walkthrough guide)
- [ ] Documentation covers consistency patterns thoroughly (in progress)
- [ ] Pattern cookbook has 20+ solutions (planned)
- [ ] All public APIs documented (mostly done, needs audit)

### Key Decisions

1. **Projection Backends**
   - **Primary**: PostgreSQL (JSONB support, queryable, separate DB option)
   - **Caching**: Redis (sub-millisecond reads, TTL support)
   - **Future**: Elasticsearch (deferred to post-v1.0)
   - **Testing**: InMemoryProjectionStore (completes testing trinity)

2. **Consistency Architecture** ✅
   - **Decision**: Sagas MUST NOT query projections (carry state instead)
   - Events should be "fat" (include all downstream data)
   - Projections are for UI/reports only (eventual consistency acceptable)
   - Strong consistency requires reading from event store

3. **API Stability**
   - Target 1.0 API stability after Phase 5
   - Document breaking change policy
   - Semantic versioning commitment

4. **Open Source Strategy**
   - Document extensibility points for community
   - Contribution guide
   - Plugin/extension system design

### Reference Implementations

**Week 1-2 (COMPLETE)** ✅:
- ✅ `examples/order-projection/` - Customer order history with PostgreSQL
- ✅ `composable-rust-projections` crate - Full projection system
- ✅ `testing` crate - InMemoryProjectionStore, ProjectionTestHarness
- 📋 `docs/consistency-patterns.md` - Planned
- 📋 `docs/saga-patterns.md` - Planned
- 📋 `docs/event-design-guidelines.md` - Planned

**Week 3-4 (COMPLETE)** ✅:
- ✅ `examples/todo/` - Simplest learning path (13 tests)
- ✅ `examples/banking/` - Transfer saga with compensation (16 tests)
- ✅ `examples/counter/` - Basic state machine (4 tests)
- ✅ `examples/order-processing/` - Event sourcing (20 tests)
- ✅ `examples/checkout-saga/` - Multi-aggregate + inventory (8 tests)
- ✅ `composable-rust-macros` - Derive macros for Action, State
- ✅ `testing` crate - ReducerTest fluent API

**Week 5 (IN PROGRESS)** ⏳:
- 📋 `composable-rust-cli` - Scaffolding and debugging tools (planned)
- 📋 API stability audit and 1.0 preparation (planned)
- 📋 Documentation overhaul (in progress)
- 🎯 `examples/ticketing/` - Event ticketing system (next)

**Success**: Developers can build production applications with confidence, understanding when to use projections vs event store, and having comprehensive examples and tooling.

**Duration**: 4-5 weeks (22-26 days)

**Progress**:
- ✅ Projections: 8 days (COMPLETE)
- ⏳ Documentation: 7-8 days (in progress - 2 days invested)
- ✅ Tooling & Macros: 3-4 days (COMPLETE)
- ✅ Examples: 3-4 days (COMPLETE - 7 examples built)
- 📋 Polish & API Audit: 1-2 days (planned)

**Days Completed**: ~15 / 26 days (~58% complete)

---

## Risk Mitigation

### Technical Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Effect execution model too complex | High | Validate with simple examples in Phase 1 |
| Performance doesn't meet targets | Medium | Benchmark continuously, optimize in Phase 4 |
| Event schema evolution too rigid | Medium | Design versioning strategy in Phase 2 |
| Saga pattern too difficult to use | High | Validate with complex example in Phase 3 |
| Projection system too complex | High | Start with simple abstractions, validate with real examples in Phase 5 |
| Developers misuse projections (query in sagas) | High | Document consistency patterns FIRST, before releasing projections |
| Rust type system fights us | Low | Prototype tricky parts early |

### Project Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Scope creep | Medium | Stick to phase deliverables, defer nice-to-haves |
| Over-engineering | High | Build simplest thing that works, refactor later |
| Under-validation | High | Every phase has reference implementation |
| Documentation lag | Medium | Document as we build, not after |

### Mitigation Strategy

1. **Continuous Validation**: Each phase must have working reference implementation
2. **Fail Fast**: If a design doesn't feel right in examples, redesign immediately
3. **Incremental Commitment**: Don't build Phase N+1 until Phase N is validated
4. **Battle Testing**: Use it ourselves before calling it "done"

---

## Success Criteria (Overall)

The implementation is successful when:

### For Developers
- Can implement a simple aggregate in < 30 minutes
- Can implement a multi-aggregate saga in < 2 hours
- Tests are fast, deterministic, and easy to write
- Error messages are clear and actionable

### For Operations
- Can deploy to production with confidence
- Can observe system behavior (logs, metrics, traces)
- Can debug issues using event replay
- Performance meets SLAs under load

### For the Business
- New features ship faster (< 1 week for typical feature)
- Bugs are rare (type system catches most at compile time)
- System is reliable (event sourcing prevents data loss)
- Scales as needed (horizontal scaling via event distribution)

---

## Timeline Summary

| Phase | Duration | Cumulative | Key Milestone |
|-------|----------|------------|---------------|
| 0: Foundation | 3-5 days | Week 1 | CI/CD working |
| 1: Core | 1.5-2 weeks | Week 3 | Counter example works |
| 2: Event Sourcing | 1.5-2 weeks | Week 5 | Order aggregate persists |
| 3: Sagas | 1.5-2 weeks | Week 7 | Checkout workflow works |
| 4: Production | 1.5-2 weeks | Week 9 | Can run distributed |
| 5: DX + Projections | 4-5 weeks | Week 14 | Projections + comprehensive docs |

**Total**: ~14 weeks to production-ready framework

**Contingency**: Add 2-3 weeks buffer for learning, debugging, and refinement

**Realistic Target**: 15-17 weeks to v1.0

**Note**: Phase 5 expanded from 1.5-2 weeks to 4-5 weeks due to:
- Critical projection/read model system (8 days)
- Essential consistency patterns documentation (2 days)
- Multiple storage backends (Postgres + Redis)
- Comprehensive developer experience improvements

---

## What's Explicitly Out of Scope

These are important but deferred post-v1.0:

1. **GraphQL/gRPC Integration** - Focus on core architecture first
2. **Elasticsearch Projection Backend** - Postgres + Redis in v1.0, Elasticsearch later for advanced search
3. **Multi-tenancy** - Can be layered on top later
4. **Distributed Transactions** - Keep transactions at aggregate boundary
5. **Hot Reload** - Nice to have, not essential
6. **GUI Tools** - Command line / code first
7. **Cloud-Specific Adapters** - Start with standard interfaces
8. **Advanced Event Upcasting** - Basic versioning in v1.0, sophisticated upcasting later

---

## Next Steps

1. **Review and Approve**: Get stakeholder sign-off on this plan
2. **Phase 0 Kickoff**: Set up project structure and tooling
3. **Spike if Needed**: If uncertain about technical approach, time-box a spike (2-3 days)
4. **Iterate**: Adjust plan based on what we learn each phase

**Decision Point**: After Phase 1, evaluate if the core abstractions feel right. If not, adjust or pivot before building on top.

**Success Measure**: By end of Phase 1, can we explain the whole architecture with just the counter example? If yes, proceed. If no, refactor.

---

## Conclusion

This roadmap balances ambition with pragmatism. We're building something significant, but we're doing it incrementally with validation at every step.

**Philosophy**: Make it work, make it right, make it fast—in that order.

**Commitment**: Each phase delivers working code that solves real problems. No "framework for framework's sake."

Let's build something excellent. 🚀
