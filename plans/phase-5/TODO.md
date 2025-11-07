# Phase 5: Developer Experience - TODO List

**Goal**: Make the framework easy, delightful, and productive to use.

**Duration**: 4-5 weeks (revised from initial 1.5-2 week estimate due to critical projection system)

**Status**: 🔄 IN PLANNING

**Philosophy**: A framework is only as good as its developer experience. Phase 5 transforms Composable Rust from "production-ready" to "joy to use" by adding documentation, tooling, examples, and utilities that eliminate friction and accelerate development.

---

## Strategic Context: Developer Joy

From the roadmap:

**Goal**: New developer can build their first aggregate in < 1 hour, first saga in < 2 hours.

**Key Requirements**:
1. **Discoverability**: Easy to find how to do common tasks
2. **Learnability**: Gentle learning curve with clear examples
3. **Productivity**: Reduce boilerplate, increase signal-to-noise
4. **Debuggability**: Easy to understand what's happening and why

**Investment**: ~2 weeks for comprehensive developer experience improvements
**Return**: Framework adoption accelerates, developers are productive immediately

---

## Prerequisites

Before starting Phase 5:
- [ ] Phase 4 complete (Production hardening)
- [ ] All 171 tests passing (156 library + 15 integration)
- [ ] Review developer feedback from Phases 1-4
- [ ] Identify pain points in current examples (Counter, Order, Checkout Saga)
- [ ] Review ergonomics of existing APIs

---

## 1. Documentation Overhaul

### 1.1 Getting Started Guide

**Scope**: Comprehensive tutorial from zero to working application

**Content**:
- Installation and setup (< 5 minutes)
- "Your First Aggregate" walkthrough (< 30 minutes)
  - Simple Todo aggregate with full explanation
  - State, Action, Reducer, Effects explained with code
  - Testing the reducer
  - Running with Store
- "Your First Event-Sourced Aggregate" (< 30 minutes)
  - Connecting to PostgreSQL event store
  - Event replay and state reconstruction
  - Snapshots for performance
- "Your First Saga" (< 1 hour)
  - Multi-aggregate coordination
  - Compensation on failure
  - Testing saga flows

**Tasks**:
- [ ] Create `docs/getting-started.md` (comprehensive tutorial)
- [ ] Add "Todo" example in `examples/todo/` (simpler than Counter)
- [ ] Add code snippets with detailed explanations
- [ ] Add troubleshooting section for common setup issues
- [ ] Add "Next Steps" roadmap at end

**Success Criteria**:
- New developer can complete tutorial in < 1 hour
- Working Todo aggregate with tests at the end
- Clear mental model of the five core types
- Know where to go next (cookbook, examples)

---

### 1.2 Pattern Cookbook

**Scope**: Solutions to common scenarios and questions

**Patterns to Document**:
1. **Basic Patterns**
   - Simple CRUD aggregate (create, read, update, delete)
   - Aggregate with validation rules
   - Aggregate with side effects (email, notifications)
   - Querying read models (projections)

2. **Intermediate Patterns**
   - Multi-aggregate coordination (saga)
   - Event versioning and schema migration
   - Soft delete vs hard delete
   - Idempotency handling
   - Optimistic concurrency with retries

3. **Advanced Patterns**
   - Process managers (long-running workflows)
   - Snapshot strategies (when and how)
   - Event upcasting (migrating old events)
   - Cross-aggregate queries (eventual consistency)
   - Temporal queries ("what was state at time T?")

4. **Testing Patterns**
   - Testing reducers in isolation
   - Testing with mock dependencies
   - Testing saga compensation flows
   - Property-based testing for invariants
   - Integration testing with testcontainers

5. **Performance Patterns**
   - When to use snapshots
   - When to use batching
   - Connection pool sizing
   - Event replay optimization
   - Read model maintenance

**Tasks**:
- [ ] Create `docs/cookbook.md` with 20+ patterns
- [ ] Each pattern includes:
  - [ ] Problem statement
  - [ ] Solution with code
  - [ ] Explanation of trade-offs
  - [ ] Links to full examples
- [ ] Add code examples to `examples/patterns/`
- [ ] Cross-link with API reference

**Success Criteria**:
- Covers 80% of common scenarios
- Each pattern has working code
- Clear when to use each pattern
- Links to deeper resources

---

### 1.3 API Reference Enhancement

**Scope**: Improve documentation in code and generated docs

**Tasks**:
- [ ] Audit all public APIs for documentation completeness
- [ ] Add "Examples" section to all major traits
- [ ] Add "Common Pitfalls" sections where relevant
- [ ] Add module-level documentation with architecture diagrams
- [ ] Create `docs/api-guide.md` (narrative API tour)
- [ ] Add links between related items
- [ ] Add "See Also" sections
- [ ] Generate complete API docs with `cargo doc`

**Success Criteria**:
- Every public API has documentation
- Examples for all major traits
- Module docs explain purpose and relationships
- Generated docs are navigable and clear

---

### 1.4 Troubleshooting Guide

**Scope**: Help developers debug common issues

**Content**:
- Common compilation errors and fixes
- Runtime error scenarios and solutions
- Performance troubleshooting (slow replays, high memory)
- Database connection issues
- Event bus configuration problems
- Saga debugging (stuck sagas, compensation failures)
- Tracing and debugging techniques

**Tasks**:
- [ ] Create `docs/troubleshooting.md`
- [ ] Document 15+ common issues with solutions
- [ ] Add error code reference
- [ ] Add debugging checklist
- [ ] Add "How to get help" section
- [ ] Link from error messages where possible

**Success Criteria**:
- Covers most common issues from Phases 1-4
- Clear diagnostic steps
- Links to relevant documentation
- Fast time-to-resolution for common problems

---

### 1.5 Migration Guides

**Scope**: Help developers adopt Composable Rust

**Guides**:
1. **From Traditional CRUD**
   - Mindset shift to event sourcing
   - Converting entities to aggregates
   - Modeling commands vs events
   - Handling reads (projections)

2. **From Other Event Sourcing Frameworks**
   - Comparing to EventStore, Axon, etc.
   - Mapping concepts
   - Migration strategies

3. **Event Schema Evolution**
   - Adding fields to events
   - Removing fields
   - Renaming events
   - Event upcasting techniques

**Tasks**:
- [ ] Create `docs/migration-from-crud.md`
- [ ] Create `docs/event-versioning.md`
- [ ] Add examples of schema evolution
- [ ] Document migration best practices

**Success Criteria**:
- Clear path from CRUD to event sourcing
- Event versioning strategies documented
- Real examples of migrations

---

### 1.6 Consistency Patterns & Architectural Guidelines

**Scope**: Critical documentation on when to use projections vs event store, and how to design for eventual consistency

**Priority**: 🔥 CRITICAL - Essential architectural guidance to prevent misuse

**Philosophy**: Projections are eventually consistent. Sagas and critical workflows must NOT depend on projections. Events should carry all data needed downstream.

**Content**:

**1. When to Use Projections vs Event Store**

Document the clear separation:

```
✅ Use Projections For:
- UI display (customer lists, order history)
- Search interfaces (product catalog)
- Reports and analytics
- Non-critical queries where 10-100ms lag is acceptable
- Read-heavy operations (10K+ reads/sec)

❌ DON'T Use Projections For:
- Saga decision-making (use carried state)
- Commands that depend on previous writes
- Critical workflows requiring immediate consistency
- Anything where correctness depends on up-to-date data

✅ Use Event Store For:
- Rebuilding aggregate state (with snapshots)
- Saga state checks (when you need current state)
- Commands that read-then-write
- Anything requiring strong consistency
```

**2. Saga Patterns (Avoiding Projection Queries)**

Document the correct saga pattern:

```rust
// ✅ GOOD: Carry state through saga
struct CheckoutSagaState {
    order_id: String,
    order_total: Decimal,        // Carried from order creation
    items: Vec<Item>,             // Carried
    customer_id: String,          // Carried
    shipping_address: Address,    // Carried

    // Don't query projections - have everything we need
}

impl CheckoutSaga {
    fn handle_order_placed(&mut self, event: OrderPlacedEvent) {
        // Event carries all data needed downstream
        self.state.order_total = event.total;
        self.state.items = event.items;

        // Next step: charge payment (no projection query!)
        self.charge_payment(self.state.order_total)
    }
}

// ❌ BAD: Query projection in saga
impl CheckoutSaga {
    async fn handle_order_placed(&mut self, event: OrderPlacedEvent) {
        // ❌ Projection might not be updated yet!
        let order = projection.get_order(event.order_id).await?;
        self.state.order_total = order.total;  // Race condition!
    }
}
```

**Key Principles**:
- Sagas should never query projections
- Carry all needed state in saga state
- Events should be "fat" (include all downstream needs)
- If saga needs current state, read from event store (not projection)

**3. Event Design Guidelines**

Events should carry all data needed by downstream consumers:

```rust
// ✅ GOOD: Fat event with all needed data
#[derive(Clone, Serialize, Deserialize)]
pub struct OrderPlacedEvent {
    order_id: String,
    customer_id: String,
    items: Vec<Item>,              // ✅ Full items, not just IDs
    total: Decimal,                // ✅ Calculated total
    shipping_address: Address,     // ✅ Full address
    payment_method: PaymentMethod, // ✅ Everything downstream needs
    created_at: DateTime<Utc>,
}

// ❌ BAD: Thin event, forces consumers to query
#[derive(Clone, Serialize, Deserialize)]
pub struct OrderPlacedEvent {
    order_id: String,  // Only ID - consumers must query!
}

// Guideline: If a saga will need it, include it in the event
```

**Trade-offs**:
- Fat events = larger storage, but eliminates queries
- Thin events = smaller storage, but forces projection queries (dangerous)
- **Recommendation**: Err on the side of fat events for critical workflows

**4. Read-After-Write Patterns**

Document the safe patterns:

```rust
// Pattern 1: Return data from command (recommended)
let order = order_store
    .send(PlaceOrder { customer_id, items })
    .await?;  // Returns the created order

// Use the returned data, no query needed
saga.process_order(order);

// Pattern 2: Read from event store (strong consistency)
let events = event_store.load_events(order_stream_id).await?;
let order = Order::from_events(events);  // Always consistent

// Pattern 3: Accept eventual consistency (UI only)
// After write, projection updates in 10-100ms
// UI shows loading state or optimistic update
```

**5. Testing Patterns for Eventual Consistency**

Document how to test systems with eventual consistency:

```rust
// Test saga without projections
#[tokio::test]
async fn test_checkout_saga_flow() {
    let saga = CheckoutSaga::new();

    // Event carries all needed data
    let event = OrderPlacedEvent {
        order_id: "order-1",
        total: 99.99,
        items: vec![/* ... */],
        // ... all needed data
    };

    // Saga processes without querying
    saga.handle(event);

    // Assert saga state (no projection queries)
    assert_eq!(saga.state.order_total, 99.99);
}

// Test projection separately (eventual consistency is OK)
#[tokio::test]
async fn test_order_projection() {
    let projection = OrderProjection::new(store);

    // Apply event
    projection.apply_event(&order_placed_event).await?;

    // Query projection (test the view)
    let orders = projection.get_customer_orders("customer-1").await?;
    assert_eq!(orders.len(), 1);
}

// Integration test: verify eventual consistency
#[tokio::test]
async fn test_projection_catches_up() {
    // Write events
    event_store.append_events(stream_id, events).await?;

    // Wait for projection to catch up (test-only)
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Now query should work
    let orders = projection.get_customer_orders("customer-1").await?;
    assert_eq!(orders.len(), 5);
}
```

**6. Architecture Decision Tree**

Provide a decision tree:

```
Need to query data?
├─ Is this in a saga or critical workflow?
│  ├─ Yes → DON'T use projection
│  │       ├─ Option 1: Carry state through saga
│  │       ├─ Option 2: Read from event store
│  │       └─ Option 3: Return data from command
│  └─ No → Is eventual consistency acceptable?
│         ├─ Yes → Use projection (UI, reports, search)
│         └─ No → Read from event store
```

**7. Common Pitfalls**

Document what NOT to do:

```
❌ Pitfall 1: Saga queries projection immediately after write
   Problem: Projection not updated yet (race condition)
   Fix: Carry data through saga state

❌ Pitfall 2: Command reads from projection, then writes
   Problem: Projection might be stale
   Fix: Read from event store (current state)

❌ Pitfall 3: Events only contain IDs
   Problem: Forces all consumers to query
   Fix: Include full data in events

❌ Pitfall 4: Testing with real projections in saga tests
   Problem: Tests become slow and flaky
   Fix: Sagas shouldn't depend on projections at all
```

**Tasks**:
- [ ] Create `docs/consistency-patterns.md` (comprehensive guide)
  - [ ] When to use projections vs event store
  - [ ] Clear guidelines with examples
  - [ ] Decision tree diagram
  - [ ] Common pitfalls and fixes
- [ ] Create `docs/saga-patterns.md` (saga-specific guidance)
  - [ ] How to design sagas that don't query projections
  - [ ] Carrying state through saga
  - [ ] Event design for sagas
  - [ ] Multiple working examples
- [ ] Create `docs/event-design-guidelines.md`
  - [ ] Fat vs thin events
  - [ ] What data to include in events
  - [ ] Versioning and schema evolution
  - [ ] Performance considerations
- [ ] Add section to Pattern Cookbook (section 1.2)
  - [ ] "Read-After-Write Patterns"
  - [ ] "Saga State Management"
  - [ ] "Event Design for Workflows"
- [ ] Update all example aggregates with proper event design
  - [ ] Order events carry full data
  - [ ] Payment events include all details
  - [ ] Inventory events self-contained
- [ ] Add testing guide section
  - [ ] Testing sagas without projections
  - [ ] Testing projections separately
  - [ ] Integration testing with eventual consistency
  - [ ] Handling timing issues in tests
- [ ] Add architecture decision records (ADRs)
  - [ ] ADR: Why sagas don't query projections
  - [ ] ADR: Event design philosophy (fat events)
  - [ ] ADR: When to use event store vs projections

**Success Criteria**:
- ✅ Clear, comprehensive documentation on consistency patterns
- ✅ Developers understand when to use projections vs event store
- ✅ Saga patterns are well-documented with examples
- ✅ Event design guidelines prevent common mistakes
- ✅ Testing patterns handle eventual consistency correctly
- ✅ Decision tree helps developers make right choices
- ✅ Common pitfalls documented with fixes
- ✅ All examples follow best practices

**Estimated Time**: 2 days
- Day 1: Write core documentation (consistency-patterns, saga-patterns, event-design)
- Day 2: Add to cookbook, update examples, write testing guide

**Priority**: Must be completed BEFORE developers start building sagas and projections

---

## 2. Projection/Read Model System

**Priority**: 🔥 CRITICAL - Must be implemented early (Week 1)

**Scope**: Build a complete projection system for maintaining denormalized read models that are updated from events. This is the query side of CQRS - essential for real applications.

**Philosophy**: Events are the source of truth (write model). Projections are optimized views for queries (read model). True CQRS separation means different databases for writes and reads.

---

### 2.1 Core Projection Abstraction

**Scope**: Define the core `Projection` trait and infrastructure

**Design**:
```rust
/// A projection builds and maintains a read model from events.
pub trait Projection: Send + Sync {
    /// The event type this projection listens to
    type Event: DeserializeOwned + Send;

    /// Apply an event to update the projection
    fn apply_event(&self, event: &Self::Event) -> impl Future<Output = Result<()>> + Send;

    /// Get the projection name (for checkpointing)
    fn name(&self) -> &str;

    /// Rebuild projection from scratch (drop and recreate)
    fn rebuild(&self) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }  // Optional: default no-op
    }
}

/// Checkpoint tracking: where is this projection in the event stream?
pub trait ProjectionCheckpoint: Send + Sync {
    async fn save_position(&self, projection_name: &str, position: EventPosition) -> Result<()>;
    async fn load_position(&self, projection_name: &str) -> Result<Option<EventPosition>>;
}

/// Event position in the stream (for catch-up)
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EventPosition {
    pub offset: u64,        // Kafka offset or event number
    pub timestamp: DateTime<Utc>,
}
```

**Tasks**:
- [ ] Create `core/src/projection.rs` module
- [ ] Define `Projection` trait with async methods
- [ ] Define `ProjectionCheckpoint` trait
- [ ] Define `EventPosition` type
- [ ] Add to `core/src/lib.rs` exports

**Success Criteria**:
- Core abstractions defined
- Trait is async-native (no BoxFuture)
- Clear separation of concerns

---

### 2.2 Projection Store Abstraction

**Scope**: Abstract storage backends for projections

**Design**:
```rust
/// Storage backend for projection data.
///
/// Different from EventStore - projections can use any database
/// that's optimized for queries (Postgres, Redis, Elasticsearch).
pub trait ProjectionStore: Send + Sync {
    /// Save projection data
    async fn save(&self, key: &str, data: &[u8]) -> Result<()>;

    /// Get projection data by key
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;

    /// Delete projection data
    async fn delete(&self, key: &str) -> Result<()>;

    /// Check if projection exists
    async fn exists(&self, key: &str) -> Result<bool> {
        Ok(self.get(key).await?.is_some())
    }
}

/// Extended trait for stores that support querying
pub trait QueryableProjectionStore: ProjectionStore {
    type Query;
    type QueryResult;

    async fn query(&self, query: Self::Query) -> Result<Self::QueryResult>;
}
```

**Tasks**:
- [ ] Define `ProjectionStore` trait in `core/src/projection.rs`
- [ ] Define `QueryableProjectionStore` for complex queries
- [ ] Add projection-specific error types
- [ ] Document trait design patterns

**Success Criteria**:
- Storage abstraction is backend-agnostic
- Simple key-value interface
- Optional query interface for complex backends

---

### 2.3 PostgreSQL Projection Store

**Scope**: PostgreSQL backend for projections with SEPARATE database support

**Key Feature**: Support separate read database (true CQRS)

**Architecture**:
```
Write Side:                  Read Side:
┌─────────────────┐         ┌─────────────────┐
│  Event Store    │         │  Projections    │
│  (Postgres 1)   │         │  (Postgres 2)   │
│                 │         │                 │
│  events table   │         │  customer_view  │
│  snapshots      │         │  order_summary  │
└─────────────────┘         │  product_search │
                            └─────────────────┘
        │                            ▲
        │ Events published           │ Updated by
        │ to Redpanda               │ projections
        ▼                            │
┌──────────────────────────────────────┐
│         Redpanda Event Bus           │
└──────────────────────────────────────┘
```

**Implementation**:
```rust
pub struct PostgresProjectionStore {
    pool: PgPool,
    table_name: String,
}

impl PostgresProjectionStore {
    pub fn new(pool: PgPool, table_name: String) -> Self {
        Self { pool, table_name }
    }

    /// Create with separate database URL (CQRS separation)
    pub async fn new_with_separate_db(
        database_url: &str,
        table_name: String,
    ) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        Ok(Self::new(pool, table_name))
    }
}

impl ProjectionStore for PostgresProjectionStore {
    async fn save(&self, key: &str, data: &[u8]) -> Result<()> {
        sqlx::query(&format!(
            "INSERT INTO {} (key, data, updated_at)
             VALUES ($1, $2, now())
             ON CONFLICT (key) DO UPDATE
             SET data = EXCLUDED.data, updated_at = now()",
            self.table_name
        ))
        .bind(key)
        .bind(data)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    // ... other methods
}
```

**Schema Design**:
```sql
-- Generic projection table
CREATE TABLE projection_data (
    key TEXT PRIMARY KEY,
    data BYTEA NOT NULL,      -- bincode or JSON
    updated_at TIMESTAMPTZ NOT NULL
);

-- Or use JSONB for queryable projections
CREATE TABLE order_projections (
    id TEXT PRIMARY KEY,
    customer_id TEXT NOT NULL,
    data JSONB NOT NULL,
    total DECIMAL(10,2),
    status TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_customer ON order_projections(customer_id);
CREATE INDEX idx_status ON order_projections(status);
CREATE INDEX idx_data_gin ON order_projections USING gin(data);

-- Checkpoint table (tracks projection progress)
CREATE TABLE projection_checkpoints (
    projection_name TEXT PRIMARY KEY,
    event_offset BIGINT NOT NULL,
    event_timestamp TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

**Tasks**:
- [ ] Create `composable-rust-projections` crate
- [ ] Implement `PostgresProjectionStore`
- [ ] Support separate database configuration
- [ ] Implement `PostgresProjectionCheckpoint`
- [ ] Add migration files for projection tables
- [ ] Add connection pooling configuration
- [ ] Document CQRS database separation pattern
- [ ] Add integration tests with separate databases

**Success Criteria**:
- Can use same Postgres as event store (simple case)
- Can use SEPARATE Postgres database (CQRS best practice)
- JSONB support for flexible schemas
- Indexed queries are fast (< 10ms)
- Connection pooling works correctly

---

### 2.4 Redis Projection Store

**Scope**: Redis backend for caching hot projections

**Use Cases**:
- Session data (every HTTP request)
- Recently viewed items
- Shopping cart state
- Real-time leaderboards
- Cache layer on top of Postgres

**Implementation**:
```rust
pub struct RedisProjectionStore {
    client: redis::Client,
    key_prefix: String,
    default_ttl: Option<Duration>,
}

impl RedisProjectionStore {
    pub fn new(redis_url: &str, key_prefix: String) -> Result<Self> {
        let client = redis::Client::open(redis_url)?;
        Ok(Self {
            client,
            key_prefix,
            default_ttl: None,
        })
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.default_ttl = Some(ttl);
        self
    }
}

impl ProjectionStore for RedisProjectionStore {
    async fn save(&self, key: &str, data: &[u8]) -> Result<()> {
        let full_key = format!("{}:{}", self.key_prefix, key);
        let mut conn = self.client.get_async_connection().await?;

        if let Some(ttl) = self.default_ttl {
            conn.set_ex(&full_key, data, ttl.as_secs()).await?;
        } else {
            conn.set(&full_key, data).await?;
        }
        Ok(())
    }
    // ... other methods
}
```

**Advanced Redis Features**:
```rust
/// Redis-specific extensions
impl RedisProjectionStore {
    /// Increment counter (for analytics, rate limiting)
    pub async fn increment(&self, key: &str) -> Result<i64> {
        let full_key = format!("{}:{}", self.key_prefix, key);
        let mut conn = self.client.get_async_connection().await?;
        Ok(conn.incr(&full_key, 1).await?)
    }

    /// Add to sorted set (for leaderboards)
    pub async fn zadd(&self, set_key: &str, score: f64, member: &str) -> Result<()> {
        let full_key = format!("{}:{}", self.key_prefix, set_key);
        let mut conn = self.client.get_async_connection().await?;
        conn.zadd(&full_key, member, score).await?;
        Ok(())
    }

    /// Get top N from sorted set
    pub async fn zrevrange(&self, set_key: &str, start: isize, stop: isize) -> Result<Vec<String>> {
        let full_key = format!("{}:{}", self.key_prefix, set_key);
        let mut conn = self.client.get_async_connection().await?;
        Ok(conn.zrevrange(&full_key, start, stop).await?)
    }
}
```

**Tasks**:
- [ ] Implement `RedisProjectionStore` in projections crate
- [ ] Support TTL configuration (auto-expiring data)
- [ ] Add Redis-specific features (sorted sets, counters)
- [ ] Implement `RedisProjectionCheckpoint`
- [ ] Document Redis use cases
- [ ] Add integration tests with testcontainers
- [ ] Add examples: session cache, leaderboard

**Success Criteria**:
- Sub-millisecond read/write latency
- TTL support for auto-expiring data
- Sorted set support for leaderboards
- Works as cache layer over Postgres

---

### 2.5 Cached Projection Store (Postgres + Redis)

**Scope**: Transparent caching layer combining Postgres and Redis

**Pattern**: Write-through cache

**Implementation**:
```rust
pub struct CachedProjectionStore<P, C>
where
    P: ProjectionStore,
    C: ProjectionStore,
{
    primary: P,      // Postgres (source of truth)
    cache: C,        // Redis (fast cache)
    ttl: Duration,
}

impl<P, C> ProjectionStore for CachedProjectionStore<P, C>
where
    P: ProjectionStore,
    C: ProjectionStore,
{
    async fn save(&self, key: &str, data: &[u8]) -> Result<()> {
        // Write to both (write-through)
        self.primary.save(key, data).await?;
        self.cache.save(key, data).await?;  // Ignore cache errors
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        // Try cache first
        if let Some(data) = self.cache.get(key).await.ok().flatten() {
            tracing::debug!(key, "Cache hit");
            return Ok(Some(data));
        }

        // Cache miss - read from primary
        tracing::debug!(key, "Cache miss");
        let data = self.primary.get(key).await?;

        // Populate cache
        if let Some(ref d) = data {
            let _ = self.cache.save(key, d).await;  // Best effort
        }

        Ok(data)
    }
}
```

**Tasks**:
- [ ] Implement `CachedProjectionStore` wrapper
- [ ] Support write-through caching
- [ ] Support cache-aside pattern
- [ ] Add cache hit/miss metrics
- [ ] Document caching strategies
- [ ] Add example showing performance improvement

**Success Criteria**:
- Transparent caching (drop-in replacement)
- Cache hit = sub-millisecond
- Cache miss = fallback to Postgres
- Metrics show cache effectiveness

---

### 2.6 Projection Manager

**Scope**: Orchestrate projection updates from event bus

**Design**:
```rust
pub struct ProjectionManager<P: Projection> {
    projection: P,
    event_bus: Arc<dyn EventBus>,
    checkpoint: Arc<dyn ProjectionCheckpoint>,
    topic: String,
    consumer_group: String,
}

impl<P: Projection> ProjectionManager<P> {
    pub async fn start(&self) -> Result<()> {
        // 1. Load checkpoint (where did we leave off?)
        let last_position = self.checkpoint
            .load_position(self.projection.name())
            .await?;

        // 2. Subscribe to events from last position
        let subscription = self.event_bus
            .subscribe(&self.topic, &self.consumer_group)
            .await?;

        // 3. Process events continuously
        loop {
            match subscription.next_event().await {
                Ok(event) => {
                    // Apply event to projection
                    self.projection.apply_event(&event).await?;

                    // Save checkpoint every N events
                    if event.offset % 100 == 0 {
                        self.checkpoint.save_position(
                            self.projection.name(),
                            EventPosition {
                                offset: event.offset,
                                timestamp: event.timestamp,
                            }
                        ).await?;
                    }
                },
                Err(e) => {
                    tracing::error!(error = ?e, "Projection error");
                    // Retry logic, DLQ, etc.
                }
            }
        }
    }

    pub async fn rebuild(&self) -> Result<()> {
        tracing::info!(projection = self.projection.name(), "Rebuilding projection");

        // 1. Drop current projection data
        self.projection.rebuild().await?;

        // 2. Reset checkpoint to beginning
        self.checkpoint.save_position(
            self.projection.name(),
            EventPosition { offset: 0, timestamp: Utc::now() }
        ).await?;

        // 3. Replay all events (catch-up mode)
        // ... implementation

        Ok(())
    }
}
```

**Tasks**:
- [ ] Implement `ProjectionManager` in projections crate
- [ ] Support checkpoint-based resumption
- [ ] Implement rebuild/catch-up mechanism
- [ ] Add error handling and retries
- [ ] Add metrics (lag, throughput, errors)
- [ ] Document projection lifecycle
- [ ] Add graceful shutdown

**Success Criteria**:
- Projections resume from last checkpoint
- Can rebuild from scratch
- Handles errors gracefully
- Metrics show projection lag

---

### 2.7 Example: Customer Order History Projection

**Scope**: Concrete example for Banking/E-commerce

**Projection**:
```rust
pub struct CustomerOrderHistoryProjection {
    store: Arc<PostgresProjectionStore>,
}

impl Projection for CustomerOrderHistoryProjection {
    type Event = OrderEvent;

    fn name(&self) -> &str {
        "customer_order_history"
    }

    async fn apply_event(&self, event: &Self::Event) -> Result<()> {
        match event {
            OrderEvent::OrderPlaced { order_id, customer_id, items, total, timestamp } => {
                let summary = OrderSummary {
                    id: order_id.clone(),
                    customer_id: customer_id.clone(),
                    item_count: items.len(),
                    total: *total,
                    status: "pending".to_string(),
                    created_at: *timestamp,
                };

                // Store in queryable format
                sqlx::query(
                    "INSERT INTO order_projections
                     (id, customer_id, data, total, status, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, now())"
                )
                .bind(&summary.id)
                .bind(&summary.customer_id)
                .bind(serde_json::to_value(&summary)?)  // JSONB
                .bind(summary.total)
                .bind(&summary.status)
                .bind(summary.created_at)
                .execute(&self.store.pool)
                .await?;

                Ok(())
            },
            OrderEvent::OrderShipped { order_id, .. } => {
                sqlx::query(
                    "UPDATE order_projections
                     SET status = 'shipped', updated_at = now()
                     WHERE id = $1"
                )
                .bind(order_id)
                .execute(&self.store.pool)
                .await?;

                Ok(())
            },
            _ => Ok(()),
        }
    }

    async fn rebuild(&self) -> Result<()> {
        // Drop and recreate table
        sqlx::query("TRUNCATE order_projections").execute(&self.store.pool).await?;
        Ok(())
    }
}

// Query API (separate from projection)
impl CustomerOrderHistoryProjection {
    pub async fn get_customer_orders(&self, customer_id: &str) -> Result<Vec<OrderSummary>> {
        Ok(sqlx::query_as::<_, OrderSummary>(
            "SELECT data FROM order_projections
             WHERE customer_id = $1
             ORDER BY created_at DESC
             LIMIT 100"
        )
        .bind(customer_id)
        .fetch_all(&self.store.pool)
        .await?)
    }
}
```

**Tasks**:
- [ ] Create full example in `examples/projections/`
- [ ] Show order history projection
- [ ] Show query API separate from projection updates
- [ ] Add integration test with Redpanda
- [ ] Document the pattern

**Success Criteria**:
- Working end-to-end example
- Events → Projection → Query
- Shows CQRS separation clearly

---

### 2.8 Testing Utilities for Projections

**Scope**: Make projection testing fast and deterministic with in-memory infrastructure

**Priority**: Complete the testing trinity alongside `InMemoryEventStore` and `InMemoryEventBus`

**Philosophy**: Just like we test aggregates without real Postgres and sagas without real Redpanda, we should test projections without real databases. This keeps tests fast, deterministic, and parallel.

---

#### **InMemoryProjectionStore Implementation**

**Core infrastructure for testing** (goes in `testing` crate):

```rust
/// In-memory projection store for fast, deterministic testing.
///
/// Complements InMemoryEventStore and InMemoryEventBus to provide
/// a complete in-memory testing infrastructure.
pub struct InMemoryProjectionStore {
    data: Arc<RwLock<HashMap<String, Vec<u8>>>>,
}

impl InMemoryProjectionStore {
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Clear all data (for test isolation)
    pub async fn clear(&self) {
        self.data.write().await.clear();
    }

    /// Get current size (for assertions)
    pub async fn len(&self) -> usize {
        self.data.read().await.len()
    }

    /// Check if key exists (for assertions)
    pub async fn contains_key(&self, key: &str) -> bool {
        self.data.read().await.contains_key(key)
    }
}

impl ProjectionStore for InMemoryProjectionStore {
    async fn save(&self, key: &str, data: &[u8]) -> Result<()> {
        self.data.write().await.insert(key.to_string(), data.to_vec());
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.data.read().await.get(key).cloned())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.data.write().await.remove(key);
        Ok(())
    }
}

impl Default for InMemoryProjectionStore {
    fn default() -> Self {
        Self::new()
    }
}
```

**Usage in tests**:
```rust
#[tokio::test]
async fn test_customer_order_projection() {
    // Fast, in-memory infrastructure (no Docker, no Postgres)
    let store = Arc::new(InMemoryProjectionStore::new());
    let projection = CustomerOrderProjection::new(store.clone());

    // Apply events
    projection.apply_event(&OrderPlacedEvent { ... }).await?;
    projection.apply_event(&OrderShippedEvent { ... }).await?;

    // Query and assert
    let data = store.get("customer:123:orders").await?;
    assert!(data.is_some());
}
```

---

#### **InMemoryProjectionCheckpoint Implementation**

For testing projection catch-up and resumption:

```rust
/// In-memory checkpoint tracking for testing projection resumption.
pub struct InMemoryProjectionCheckpoint {
    positions: Arc<RwLock<HashMap<String, EventPosition>>>,
}

impl InMemoryProjectionCheckpoint {
    pub fn new() -> Self {
        Self {
            positions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn clear(&self) {
        self.positions.write().await.clear();
    }
}

impl ProjectionCheckpoint for InMemoryProjectionCheckpoint {
    async fn save_position(&self, projection_name: &str, position: EventPosition) -> Result<()> {
        self.positions.write().await.insert(projection_name.to_string(), position);
        Ok(())
    }

    async fn load_position(&self, projection_name: &str) -> Result<Option<EventPosition>> {
        Ok(self.positions.read().await.get(projection_name).copied())
    }
}
```

---

#### **ProjectionTestHarness (High-Level Helper)**

Simplify projection testing with a fluent API:

```rust
pub struct ProjectionTestHarness<P: Projection> {
    projection: P,
    store: Arc<InMemoryProjectionStore>,
}

impl<P: Projection> ProjectionTestHarness<P> {
    pub fn new(projection: P) -> Self {
        Self {
            projection,
            store: Arc::new(InMemoryProjectionStore::new()),
        }
    }

    /// Apply events to projection
    pub async fn given_events(&mut self, events: Vec<P::Event>) -> &mut Self {
        for event in events {
            self.projection.apply_event(&event).await
                .expect("Failed to apply event");
        }
        self
    }

    /// Assert projection state
    pub async fn then_contains(&self, key: &str) -> &Self {
        assert!(self.store.contains_key(key).await, "Projection missing key: {}", key);
        self
    }

    /// Get raw data for assertions
    pub async fn get_data(&self, key: &str) -> Option<Vec<u8>> {
        self.store.get(key).await.unwrap()
    }

    /// Get store reference
    pub fn store(&self) -> &Arc<InMemoryProjectionStore> {
        &self.store
    }
}
```

**Usage**:
```rust
#[tokio::test]
async fn test_order_projection_with_harness() {
    let mut harness = ProjectionTestHarness::new(OrderProjection::new());

    harness
        .given_events(vec![
            OrderPlacedEvent { order_id: "1", total: 99.99 },
            OrderShippedEvent { order_id: "1" },
        ])
        .await
        .then_contains("order:1")
        .await;

    // Clean, readable, fast
}
```

---

**Tasks**:
- [ ] Add `InMemoryProjectionStore` to `testing` crate
  - [ ] Implement `ProjectionStore` trait
  - [ ] Add helper methods (clear, len, contains_key)
  - [ ] Add documentation
- [ ] Add `InMemoryProjectionCheckpoint` to `testing` crate
  - [ ] Implement `ProjectionCheckpoint` trait
  - [ ] Add clear method for test isolation
- [ ] Add `ProjectionTestHarness` to `testing` crate
  - [ ] Fluent API for readability
  - [ ] Assertion helpers
  - [ ] Documentation with examples
- [ ] Update `testing/src/lib.rs` exports
  - [ ] Export all projection testing utilities
  - [ ] Maintain consistency with InMemoryEventStore/InMemoryEventBus
- [ ] Add comprehensive tests for test utilities
  - [ ] Test InMemoryProjectionStore
  - [ ] Test InMemoryProjectionCheckpoint
  - [ ] Test ProjectionTestHarness
- [ ] Document testing patterns in `docs/testing-projections.md`
  - [ ] In-memory vs real database testing
  - [ ] When to use each approach
  - [ ] Examples for common scenarios

**Success Criteria**:
- ✅ `InMemoryProjectionStore` works just like `InMemoryEventStore`
- ✅ Projection tests run at memory speed (< 10ms per test)
- ✅ No Docker/Postgres/Redis needed for projection tests
- ✅ Tests are deterministic and can run in parallel
- ✅ Clear, readable test code with harness
- ✅ Complete testing trinity: EventStore + EventBus + ProjectionStore

**Estimated Time**: 0.5 day (same session as other projection testing utilities)

---

### 2.9 Documentation

**Scope**: Comprehensive projection documentation

**Documents to Create**:
- [ ] `docs/projections.md`: Complete guide
  - [ ] What are projections vs snapshots
  - [ ] CQRS pattern explanation
  - [ ] When to use which storage backend
  - [ ] Separate database strategy
  - [ ] Checkpoint mechanism
  - [ ] Rebuild/catch-up process
  - [ ] Error handling and retries
- [ ] `docs/projection-patterns.md`: Common patterns
  - [ ] List views (customer list, order list)
  - [ ] Detail views (customer profile, order details)
  - [ ] Search indexes (product catalog)
  - [ ] Analytics aggregations (sales reports)
  - [ ] Caching strategies
- [ ] API documentation for all projection types

**Success Criteria**:
- Clear explanation of projections
- Multiple working examples
- Performance guidance
- Operations runbook

---

### 2.10 Success Criteria

Projection system is complete when:

**Core Features**:
- [ ] `Projection` trait abstraction complete
- [ ] `ProjectionStore` trait with multiple backends
- [ ] PostgreSQL projection store with separate DB support
- [ ] Redis projection store for caching
- [ ] Cached projection store (Postgres + Redis)
- [ ] `ProjectionManager` for orchestration
- [ ] Checkpoint mechanism working
- [ ] Rebuild/catch-up functionality

**Implementation**:
- [ ] Can create projections that update from events
- [ ] Can use separate Postgres database (CQRS)
- [ ] Can cache hot projections in Redis
- [ ] Projections resume from checkpoint on restart
- [ ] Can rebuild projections from scratch
- [ ] Error handling with retries and DLQ

**Examples**:
- [ ] Customer order history projection (Postgres)
- [ ] Session cache projection (Redis)
- [ ] Leaderboard projection (Redis sorted sets)
- [ ] All examples fully documented

**Testing**:
- [ ] Test utilities for projections
- [ ] Integration tests with real databases
- [ ] Performance benchmarks (Postgres vs Redis)

**Documentation**:
- [ ] Complete projection guide
- [ ] Pattern cookbook
- [ ] Operations documentation
- [ ] API reference

---

### 2.11 Estimated Time

**Week 1** (5 days):
- Day 1: Core abstractions (Projection, ProjectionStore traits)
- Day 2-3: PostgreSQL projection store + separate DB support
- Day 4: Checkpoint mechanism
- Day 5: ProjectionManager + basic example

**Week 2** (3 days):
- Day 1: Redis projection store
- Day 2: Cached store (Postgres + Redis)
- Day 3: Testing utilities + documentation

**Total**: 8 days (~1.5 weeks)

**Priority**: This should be Week 1-2 of Phase 5, before macros and tooling.

---

## 3. Developer Tooling & Macros

### 3.1 Derive Macros for Boilerplate Reduction

**Scope**: Reduce boilerplate in common cases

**Macros to Create**:

1. **`#[derive(Action)]`** - Generate action boilerplate
   ```rust
   #[derive(Action, Clone, Debug)]
   enum OrderAction {
       #[command]
       PlaceOrder { customer_id: CustomerId, items: Vec<LineItem> },

       #[event]
       OrderPlaced { order_id: OrderId, timestamp: DateTime<Utc> },
   }
   ```
   - Generates `is_command()`, `is_event()` helpers
   - Generates type-safe constructors
   - Generates event metadata handling

2. **`#[derive(State)]`** - Generate state boilerplate
   ```rust
   #[derive(State, Clone, Debug)]
   struct OrderState {
       orders: HashMap<OrderId, Order>,
       #[version]
       version: Version,
   }
   ```
   - Implements common traits
   - Generates default() if appropriate
   - Handles version tracking

3. **`#[derive(Aggregate)]`** - Wire up full aggregate
   ```rust
   #[derive(Aggregate)]
   struct Order {
       state: OrderState,
       reducer: OrderReducer,
   }
   ```
   - Generates builder pattern
   - Wires state + reducer + environment
   - Generates test helpers

**Tasks**:
- [ ] Create `composable-rust-macros` crate
- [ ] Implement `#[derive(Action)]` macro
- [ ] Implement `#[derive(State)]` macro
- [ ] Implement `#[derive(Aggregate)]` macro (optional)
- [ ] Add comprehensive macro documentation
- [ ] Add macro expansion tests
- [ ] Document when NOT to use macros

**Success Criteria**:
- Reduces boilerplate by 30-50% for simple aggregates
- Macros are optional (manual implementation always works)
- Clear error messages when macros fail
- Examples show both macro and manual approaches

---

### 3.2 Builder Pattern Helpers

**Scope**: Make testing easier with builders

**Content**:
```rust
// Instead of:
let order = Order {
    id: OrderId::new(),
    customer_id: CustomerId::new(),
    items: vec![],
    status: OrderStatus::Pending,
    created_at: Utc::now(),
};

// Use:
let order = OrderBuilder::new()
    .customer_id(customer_id)
    .add_item(item1)
    .add_item(item2)
    .build();
```

**Tasks**:
- [ ] Create `#[derive(Builder)]` macro or helper
- [ ] Add builders for all example aggregates
- [ ] Document builder pattern best practices
- [ ] Add builders to testing crate

**Success Criteria**:
- Test data creation is concise and readable
- Builders handle common defaults
- Optional fields handled elegantly

---

### 3.3 Event Serialization Helpers

**Scope**: Make event versioning easier

**Content**:
```rust
#[event(version = 2)]
struct OrderPlacedV2 {
    order_id: OrderId,
    customer_id: CustomerId,
    items: Vec<LineItem>,
    #[added(version = 2)]
    discount_code: Option<String>,
}

impl From<OrderPlacedV1> for OrderPlacedV2 {
    fn from(v1: OrderPlacedV1) -> Self {
        Self {
            order_id: v1.order_id,
            customer_id: v1.customer_id,
            items: v1.items,
            discount_code: None,  // Default for missing field
        }
    }
}
```

**Tasks**:
- [ ] Create event versioning helper traits
- [ ] Add `#[event(version = N)]` attribute macro
- [ ] Document event evolution patterns
- [ ] Add examples of upcasting

**Success Criteria**:
- Event versioning is explicit and type-safe
- Old events can be read and upcasted
- Schema evolution is documented

---

## 4. Testing Utilities Enhancement

### 4.1 Reducer Assertion Helpers

**Scope**: Make reducer testing more ergonomic

**Content**:
```rust
// Instead of:
let mut state = OrderState::default();
let effects = reducer.reduce(&mut state, action, &env);
assert_eq!(state.orders.len(), 1);
assert!(matches!(effects[0], Effect::PublishEvent(_)));

// Use:
ReducerTest::new(OrderReducer, env)
    .given_state(OrderState::default())
    .when_action(OrderAction::PlaceOrder { ... })
    .then_state(|state| {
        assert_eq!(state.orders.len(), 1);
    })
    .then_effects(|effects| {
        assert_event_published(effects, "OrderPlaced");
    })
    .run();
```

**Tasks**:
- [ ] Create `ReducerTest` builder in testing crate
- [ ] Add assertion helpers:
  - [ ] `assert_state_changed()`
  - [ ] `assert_effects_count()`
  - [ ] `assert_effect_type()`
  - [ ] `assert_event_published()`
  - [ ] `assert_no_effects()`
- [ ] Add property-based testing helpers
- [ ] Document testing patterns

**Success Criteria**:
- Reducer tests are readable and concise
- Common assertions are one-liners
- Clear error messages on failures

---

### 4.2 Snapshot Testing Support

**Scope**: Test state changes over time

**Content**:
```rust
#[test]
fn test_order_lifecycle() {
    let test = SnapshotTest::new("order_lifecycle")
        .record_action(PlaceOrder { ... })
        .record_action(AddItem { ... })
        .record_action(ConfirmOrder { ... })
        .verify();

    // First run: saves snapshot
    // Future runs: compares against snapshot
}
```

**Tasks**:
- [ ] Create snapshot testing utilities
- [ ] Add snapshot serialization (JSON for readability)
- [ ] Add snapshot update workflow
- [ ] Document snapshot testing patterns

**Success Criteria**:
- Can snapshot reducer state over actions
- Snapshots are readable (JSON)
- Easy to update snapshots when intentional

---

### 4.3 Property-Based Testing Helpers

**Scope**: Test invariants with proptest

**Content**:
```rust
proptest! {
    #[test]
    fn order_total_never_negative(actions in vec(any::<OrderAction>(), 0..100)) {
        let mut state = OrderState::default();
        for action in actions {
            reducer.reduce(&mut state, action, &env);
            assert!(state.total >= 0.0);
        }
    }
}
```

**Tasks**:
- [ ] Add proptest strategies for common types
- [ ] Add invariant testing helpers
- [ ] Document property-based testing patterns
- [ ] Add examples to cookbook

**Success Criteria**:
- Easy to generate random actions
- Common invariants easy to test
- Examples show how to find bugs

---

### 4.4 Saga Testing Utilities

**Scope**: Make saga testing easier

**Content**:
```rust
SagaTest::new(CheckoutSaga, env)
    .when_success("payment")
    .when_failure("inventory")
    .then_compensates(["payment"])
    .then_saga_failed()
    .run();
```

**Tasks**:
- [ ] Create `SagaTest` builder
- [ ] Add compensation assertion helpers
- [ ] Add timeout simulation helpers
- [ ] Document saga testing patterns

**Success Criteria**:
- Saga happy path tests are simple
- Compensation flows are testable
- Timeout scenarios are easy to simulate

---

## 5. Example Applications

### 5.1 Todo Application (Simplest)

**Scope**: Minimal example for learning

**Features**:
- Create todo
- Mark complete
- Delete todo
- List todos (read model)

**Purpose**: Gentlest introduction, simpler than Counter

**Tasks**:
- [ ] Create `examples/todo/`
- [ ] Implement Todo aggregate
- [ ] Add comprehensive README
- [ ] Add tests demonstrating patterns
- [ ] Link from getting started guide

**Success Criteria**:
- < 200 lines of code
- Complete in < 30 minutes following guide
- Clear learning path to more complex examples

---

### 5.2 Banking Application (Intermediate)

**Scope**: Real-world domain with complexity

**Features**:
- Open account
- Deposit/withdraw funds
- Transfer between accounts (saga)
- Account balance constraints
- Transaction history
- Overdraft protection

**Purpose**: Demonstrates intermediate patterns, multi-aggregate saga

**Tasks**:
- [ ] Create `examples/banking/`
- [ ] Implement Account aggregate
- [ ] Implement Transfer saga
- [ ] Add balance constraints and validation
- [ ] Add read models for transactions
- [ ] Add comprehensive README
- [ ] Add tests for happy path + failures

**Success Criteria**:
- Demonstrates saga pattern clearly
- Shows constraint validation
- Shows read model projection
- Complete, realistic example

---

### 5.3 Inventory Management (Advanced)

**Scope**: Complex domain with multiple aggregates

**Features**:
- Product catalog
- Stock tracking
- Purchase orders
- Stock replenishment (saga)
- Low stock alerts
- Reservation system (hold stock during checkout)

**Purpose**: Demonstrates advanced patterns, process managers

**Tasks**:
- [ ] Create `examples/inventory/`
- [ ] Implement Product aggregate
- [ ] Implement StockLevel aggregate
- [ ] Implement Replenishment saga
- [ ] Add reservation system
- [ ] Add alerting
- [ ] Add comprehensive README

**Success Criteria**:
- Demonstrates complex coordination
- Shows process manager pattern
- Shows multiple sagas interacting
- Production-quality example

---

### 5.4 E-commerce Platform (Complete Reference)

**Scope**: Full-featured application combining all patterns

**Combines**:
- Orders (from Phase 2)
- Payments (from Phase 3)
- Inventory (new)
- Shipping (new)
- Customers (new)

**Purpose**: Show how everything fits together in production

**Tasks**:
- [ ] Create `examples/ecommerce/` combining all aggregates
- [ ] Add docker-compose for full stack
- [ ] Add Grafana dashboards
- [ ] Add load testing scripts
- [ ] Add deployment guide
- [ ] Add operations runbook

**Success Criteria**:
- Can run entire e-commerce flow
- Full observability
- Production deployment example
- Load tested to targets

---

## 6. Project Templates & Scaffolding

### 6.1 Project Template

**Scope**: Quick-start template for new projects

**Content**:
```
composable-rust-template/
├── src/
│   ├── aggregates/
│   │   └── example.rs
│   ├── sagas/
│   ├── lib.rs
│   └── main.rs
├── tests/
├── Cargo.toml
├── docker-compose.yml
└── README.md
```

**Tasks**:
- [ ] Create template repository
- [ ] Add cargo-generate template
- [ ] Include basic aggregate scaffold
- [ ] Include docker-compose for Postgres + Redpanda
- [ ] Add CI/CD template (GitHub Actions)
- [ ] Document template usage

**Success Criteria**:
- `cargo generate` creates working project
- Includes best practices by default
- Developer can start coding immediately

---

### 6.2 Aggregate Scaffold Generator

**Scope**: CLI tool to generate aggregate boilerplate

**Usage**:
```bash
composable-rust new aggregate Order
# Generates:
# - src/aggregates/order/mod.rs (state, action, reducer)
# - tests/order_tests.rs
```

**Tasks**:
- [ ] Create `composable-rust-cli` crate
- [ ] Implement `new aggregate` command
- [ ] Implement `new saga` command
- [ ] Add interactive prompts for options
- [ ] Document CLI usage

**Success Criteria**:
- Generates idiomatic code
- Includes test scaffolding
- Saves 10-15 minutes per aggregate

---

## 7. Performance & Optimization Guide

### 7.1 Performance Tuning Documentation

**Scope**: Help developers optimize their systems

**Content**:
1. **Snapshot Strategies**
   - When to snapshot (every N events)
   - Snapshot serialization formats
   - Snapshot storage and cleanup

2. **Event Replay Optimization**
   - Batch loading events
   - Parallel replay for independent aggregates
   - Caching strategies

3. **Database Optimization**
   - Connection pool sizing (formula: 2x CPU cores + effective_spindle_count)
   - Index strategies
   - Partition strategies for large event stores
   - Archiving old events

4. **Event Bus Optimization**
   - Consumer group sizing
   - Batching strategies
   - Partition key selection
   - Lag monitoring

5. **Common Bottlenecks**
   - N+1 queries
   - Unbounded result sets
   - Missing indexes
   - Over-snapshotting
   - Under-snapshotting

**Tasks**:
- [ ] Create `docs/performance-tuning.md`
- [ ] Add concrete examples and benchmarks
- [ ] Add profiling guide (flamegraphs, perf)
- [ ] Add optimization checklist
- [ ] Document trade-offs

**Success Criteria**:
- Developers can identify bottlenecks
- Clear guidance on optimization
- Realistic benchmarks included

---

### 7.2 Profiling and Benchmarking Guide

**Scope**: Teach developers to measure and optimize

**Content**:
- How to use cargo-flamegraph
- How to read flamegraphs
- How to use criterion benchmarks
- How to use perf/valgrind
- When to optimize (measure first!)

**Tasks**:
- [ ] Create `docs/profiling.md`
- [ ] Add flamegraph examples
- [ ] Add benchmark examples
- [ ] Document profiling workflow

**Success Criteria**:
- Developers can profile their code
- Know what to optimize
- Can measure improvements

---

## 8. Debugging & Observability Tools

### 8.1 Event Replay Debugger

**Scope**: Visualize event replay for debugging

**Content**:
```bash
composable-rust replay order-123 --from-version 0 --to-version 50 --show-state
# Shows state evolution with each event
```

**Tasks**:
- [ ] Add replay command to CLI
- [ ] Add state visualization
- [ ] Add event filtering
- [ ] Add time-travel debugging

**Success Criteria**:
- Can see state at any point in time
- Easy to identify when bug was introduced
- Helpful for debugging production issues

---

### 8.2 Saga Visualization

**Scope**: Visualize saga execution flow

**Content**:
```bash
composable-rust saga checkout-123 --visualize
# Generates diagram showing saga flow, compensations
```

**Tasks**:
- [ ] Add saga visualization to CLI
- [ ] Generate mermaid diagrams
- [ ] Show compensation flows
- [ ] Show current saga state

**Success Criteria**:
- Visual diagram of saga execution
- Helpful for debugging stuck sagas
- Shows compensation chains

---

## 9. API Stability & Versioning

### 9.1 API Audit for 1.0

**Scope**: Prepare for 1.0 stable release

**Tasks**:
- [ ] Audit all public APIs
- [ ] Mark experimental APIs with `#[experimental]`
- [ ] Document breaking change policy
- [ ] Create deprecation strategy
- [ ] Version all crates to 0.9.0 (pre-1.0)

**Success Criteria**:
- Public API surface is stable
- Deprecation policy documented
- Ready for 1.0 commitment

---

### 9.2 Semantic Versioning Documentation

**Scope**: Document versioning policy

**Tasks**:
- [ ] Create `docs/versioning.md`
- [ ] Document semver policy
- [ ] Document upgrade guide process
- [ ] Document LTS support (if applicable)

**Success Criteria**:
- Clear versioning commitments
- Upgrade path documented
- Breaking change policy clear

---

## 10. Community & Ecosystem

### 10.1 Contribution Guide

**Scope**: Make contributing easy

**Tasks**:
- [ ] Create `CONTRIBUTING.md`
- [ ] Document development setup
- [ ] Document PR process
- [ ] Add code of conduct
- [ ] Add issue templates

**Success Criteria**:
- Clear contribution process
- Welcoming to new contributors

---

### 10.2 Plugin/Extension System (Optional)

**Scope**: Allow community extensions

**Content**:
- Effect trait extensions
- Custom event store implementations
- Custom event bus implementations
- Custom metrics exporters

**Tasks**:
- [ ] Document extension points
- [ ] Create extension guide
- [ ] Add example extensions

**Success Criteria**:
- Community can extend framework
- Extension points are stable

---

## 11. Success Criteria

Phase 5 is complete when:

**Developer Productivity**:
- [ ] New developer builds first aggregate in < 1 hour
- [ ] New developer builds first saga in < 2 hours
- [ ] Getting started guide complete and tested
- [ ] Pattern cookbook has 20+ solutions
- [ ] API reference is comprehensive

**Tooling**:
- [ ] Macros reduce boilerplate by 30-50%
- [ ] Project template generates working scaffold
- [ ] CLI tool generates aggregate boilerplate
- [ ] Testing utilities make tests 50% more concise

**Examples**:
- [ ] Todo example (simple)
- [ ] Banking example (intermediate)
- [ ] Inventory example (advanced)
- [ ] E-commerce platform (complete reference)
- [ ] Each example has comprehensive README

**Documentation**:
- [ ] Getting started guide (1 hour tutorial)
- [ ] Pattern cookbook (20+ patterns)
- [ ] API reference (all public APIs documented)
- [ ] Troubleshooting guide (15+ common issues)
- [ ] Migration guides (CRUD, event versioning)
- [ ] Performance tuning guide
- [ ] Profiling guide

**Quality Checks**:
- [ ] All tests still passing (170+ tests)
- [ ] Documentation builds without warnings
- [ ] Examples all run successfully
- [ ] Macros have expansion tests
- [ ] API stability audit complete

---

## Estimated Time Breakdown

Based on comprehensive scope including critical projection system:

1. **Projections/Read Models** (8 days) **[CRITICAL - Week 1-2]**:
   - Core abstractions: 1 day
   - PostgreSQL projection store + separate DB: 2 days
   - Checkpoint mechanism: 1 day
   - ProjectionManager + basic example: 1 day
   - Redis projection store: 1 day
   - Cached store (Postgres + Redis): 1 day
   - Testing utilities + documentation: 1 day

2. **Documentation** (7-8 days):
   - Consistency patterns (CRITICAL): 2 days
   - Getting started guide: 1 day
   - Pattern cookbook: 2 days
   - API reference enhancement: 1 day
   - Troubleshooting + migration guides: 1 day
   - Performance tuning guide: 1 day

3. **Tooling & Macros** (3-4 days):
   - Derive macros: 2 days
   - CLI scaffolding: 1 day
   - Testing utilities: 1 day

4. **Examples** (3-4 days):
   - Todo example: 0.5 day
   - Banking example: 1 day
   - Inventory example: 1.5 days
   - E-commerce integration: 1 day

5. **Templates & Polish** (1-2 days):
   - Project template: 0.5 day
   - Debugging tools: 0.5 day
   - API stability audit: 0.5 day
   - Final polish: 0.5 day

**Total**: 22-26 days (~4-5 weeks of full-time work)

**Note**: This is significantly longer than the initial roadmap estimate of 1.5-2 weeks because:
1. Projection system is CRITICAL and wasn't fully scoped initially (8 days)
2. Consistency patterns documentation is essential architectural guidance (2 days)
3. Multiple storage backends (Postgres + Redis) add complexity
4. Separate database support for true CQRS requires additional work

**Prioritization Strategy**:
- **Week 1-2**: Projections (critical for real applications)
- **Week 3**: Consistency patterns + documentation overhaul
- **Week 4**: Examples + tooling
- **Week 5**: Templates, debugging tools, polish

---

## Notes

### Developer Experience Principles

1. **Discoverability**: Easy to find answers (good docs, examples)
2. **Learnability**: Gentle learning curve (tutorial → cookbook → advanced)
3. **Productivity**: Reduce friction (macros, templates, tools)
4. **Debuggability**: Easy to understand problems (good errors, debugging tools)

### Documentation Philosophy

- **Show, don't tell**: Code examples > prose
- **Progressive disclosure**: Simple first, complexity later
- **Cross-linking**: Connect related concepts
- **Real-world scenarios**: Not toy examples

### Tooling Philosophy

- **Optional, not required**: Macros are shortcuts, not necessities
- **Transparent**: Clear what macros generate
- **Helpful errors**: Guide to solution, not just error

### Example Philosophy

- **Realistic**: Real domains, real complexity
- **Complete**: Not just happy path
- **Documented**: Explain why, not just what
- **Tested**: Show testing patterns

---

## Conclusion

Phase 5 transforms Composable Rust from a powerful framework to a delightful one. By focusing on documentation, tooling, examples, and developer experience, we ensure that developers can be productive immediately and grow their expertise over time.

**Philosophy**: The best framework is the one developers want to use.

Let's make Composable Rust a joy to work with! 🚀
