# Event Bus Architecture: In-Memory vs. Durable Event Distribution

**Status**: Specification
**Created**: 2025-11-23
**Motivation**: Eliminate per-request consumer group overhead, align with event-sourced system best practices

---

## Problem Statement

### Current Architecture Issues

**Problem 1: Expensive Per-Request Consumer Groups**

We currently create a new Redpanda consumer group for every HTTP request that uses a Store:

```rust
// ❌ ANTI-PATTERN: Creates expensive resources per request
let unique_consumer_group = format!("ticketing-payment-store-{}", payment_id.as_uuid());
let store_event_bus: Arc<dyn EventBus> = Arc::new(
    RedpandaEventBus::builder()
        .brokers(&self.config.redpanda.brokers)
        .consumer_group(&unique_consumer_group)
        .build()?
);
```

**Cost per consumer group**:
- TCP connection to brokers (~5-10ms)
- Consumer group coordination protocol
- Offset management (read/write to broker)
- Memory overhead on client and broker
- Connection pool management

**At scale**:
- 100 concurrent requests = 100 consumer groups
- 1000 concurrent requests = 1000 consumer groups
- Each living for ~1-2 seconds

**Problem 2: Wrong Tool for the Job**

Redpanda is designed for:
- ✅ **Long-lived consumers** (hours/days/forever)
- ✅ **Stable topics** (handful to dozens)
- ✅ **Cross-process communication**
- ✅ **Durable event log** (replay, audit)

We're using it for:
- ❌ **Request-scoped coordination** (~1 second lifetime)
- ❌ **Intra-process communication** (same server process)
- ❌ **Ephemeral saga coordination** (no replay needed)

**Problem 3: ProjectionCompletionTracker Inconsistency**

We correctly implemented a singleton pattern for projection completion tracking:
- **ONE** consumer subscribes to `projection.completed` topic
- Multiple handlers `register_interest()` cheaply (in-memory map)
- Avoided the per-request consumer group problem

But we didn't generalize this pattern to all event coordination.

---

## Architectural Philosophy

### Principle: Separate Mechanisms for Different Guarantees

Event-driven systems have **three distinct communication patterns**, each requiring different infrastructure:

| Pattern | Lifetime | Scope | Guarantees | Right Tool |
|---------|----------|-------|------------|------------|
| **Saga Coordination** | Request duration (~100ms-2s) | Intra-process | Speed, low latency | In-memory channels |
| **Event Sourcing** | Forever | Single aggregate | ACID, durability, replay | PostgreSQL EventStore |
| **External Distribution** | Server lifetime | Cross-process | At-least-once, replayable | Redpanda topics |

### Anti-Pattern: Overloading One Tool

**Don't use Redpanda for everything**:
- ❌ Request-scoped saga coordination (use in-memory)
- ✅ Projection updates (correct use)
- ✅ External service notifications (correct use)
- ✅ Cross-process event distribution (correct use)

**Don't use in-memory channels for everything**:
- ✅ Saga coordination within a request (correct use)
- ❌ Durable event log (use PostgreSQL)
- ❌ Cross-process events (use Redpanda)

---

## Target Architecture

### High-Level Data Flow

```
┌─────────────────────────────────────────────────────────────────┐
│  HTTP Request Scope (Single Process, ~1 second)                 │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────┐  In-Memory    ┌──────────────┐               │
│  │   Payment    │  Broadcast    │    Saga      │               │
│  │   Aggregate  │◄─────────────►│  Coordinator │               │
│  │    Store     │   Channel     │    Store     │               │
│  └──────┬───────┘   (Fast)      └──────┬───────┘               │
│         │                               │                       │
│         │ Write (ACID, Synchronous)    │                       │
│         │                               │                       │
│         ▼                               ▼                       │
│  ┌──────────────────────────────────────────────────┐          │
│  │      PostgreSQL Event Store                      │          │
│  │      (Source of Truth, Durable, Replayable)      │          │
│  │                                                   │          │
│  │  - payment-{uuid} streams                        │          │
│  │  - reservation-{uuid} streams                    │          │
│  │  - inventory-{event-id} streams                  │          │
│  └────────────────┬─────────────────────────────────┘          │
│                   │                                             │
│                   │ After ACID commit:                          │
│                   │ Async publish (fire-and-forget)             │
│                   │                                             │
│                   ▼                                             │
│  ┌──────────────────────────────────────────────────┐          │
│  │   Redpanda Topics (Few, Stable, Multiplexed)     │          │
│  │                                                   │          │
│  │   - ticketing-payment-events                     │          │
│  │   - ticketing-reservation-events                 │          │
│  │   - ticketing-inventory-events                   │          │
│  │   - projection.completed                         │          │
│  └────────────────┬─────────────────────────────────┘          │
└───────────────────┼──────────────────────────────────────────────┘
                    │
                    │ Long-lived, stable consumer groups
                    │ (Created once at server startup)
                    │
                    ▼
   ┌─────────────────────────────────────────────────┐
   │  Singleton Consumers (Server Lifetime)          │
   │                                                  │
   │  - Projection Manager (payments_projection)     │
   │  - Projection Manager (reservations_projection) │
   │  - Ownership Index Updater                      │
   │  - WebSocket Broadcaster                        │
   │  - External Billing Service                     │
   │  - Analytics Pipeline                           │
   └──────────────────────────────────────────────────┘
```

### Communication Layers

#### Layer 1: In-Memory Event Bus (Request-Scoped)

**Purpose**: Fast coordination between Stores within a single HTTP request

**Implementation**:
```rust
// Create once per request (cheap: just allocates channel)
let event_bus = Arc::new(InMemoryEventBus::new());

// Pass to all Stores in the request
let payment_store = create_payment_store(payment_id, event_bus.clone());
let saga_store = create_saga_store(saga_id, event_bus.clone());
```

**Characteristics**:
- **Lifetime**: Duration of HTTP request (~100ms-2s)
- **Scope**: Thread-safe, multi-producer multi-consumer
- **Latency**: ~10μs (nanoseconds for broadcast)
- **Durability**: None (ephemeral)
- **Ordering**: Per-topic FIFO
- **Replay**: Not supported (not needed)

**Use Cases**:
- ✅ Saga parent waits for child completion
- ✅ Payment Store waits for PaymentConfirmed
- ✅ Reservation saga coordinates with Inventory
- ✅ Cross-aggregate events within same request
- ❌ NOT for projections (use Redpanda)
- ❌ NOT for external services (use Redpanda)

#### Layer 2: PostgreSQL Event Store (Aggregate-Scoped)

**Purpose**: Durable, ACID-compliant source of truth for aggregate state

**Implementation**:
```rust
// Shared across all requests (connection pool)
let event_store = Arc::new(PostgresEventStore::from_pool(pool));

// Each aggregate has its own stream
append_events! {
    store: event_store,
    stream: "payment-{uuid}",
    expected_version: Some(version),
    events: vec![event],
    // ...
}
```

**Characteristics**:
- **Lifetime**: Forever (until explicitly deleted)
- **Scope**: Per-aggregate stream
- **Latency**: ~1-5ms (disk I/O)
- **Durability**: ACID (PostgreSQL transactions)
- **Ordering**: Per-stream strict ordering via optimistic concurrency
- **Replay**: Full support (load_events, snapshots)

**Use Cases**:
- ✅ Event sourcing (rebuild aggregate state)
- ✅ Audit trail (compliance, debugging)
- ✅ Event replay (projections, new read models)
- ✅ Time travel queries
- ❌ NOT for real-time coordination (too slow)

#### Layer 3: Redpanda Topics (Application-Scoped)

**Purpose**: Durable, cross-process event distribution to external subscribers

**Implementation**:
```rust
// Created ONCE at server startup
let global_event_bus = Arc::new(
    RedpandaEventBus::builder()
        .brokers(&config.redpanda.brokers)
        .consumer_group("ticketing-projections")  // Stable name
        .build()?
);

// After persisting to PostgreSQL, async publish
tokio::spawn(async move {
    global_event_bus.publish("ticketing-payment-events", &event).await
});
```

**Characteristics**:
- **Lifetime**: Server lifetime (or longer)
- **Scope**: Cross-process, cross-service
- **Latency**: ~5-20ms (network + broker)
- **Durability**: At-least-once (configurable retention)
- **Ordering**: Per-partition (partition key = aggregate ID)
- **Replay**: Full support (seek to offset)

**Consumers** (created once at startup):
- `ticketing-projections` - Updates PostgreSQL projections
- `ticketing-ownership-indices` - Updates in-memory ownership maps
- `ticketing-websocket-broadcaster` - Pushes to WebSocket clients
- `external-billing-service` - Charges customers
- `external-analytics-pipeline` - Business intelligence

**Use Cases**:
- ✅ Projection updates (payments_projection, reservations_projection)
- ✅ Cross-service events (billing, analytics)
- ✅ WebSocket real-time notifications
- ✅ Audit log export
- ❌ NOT for request-scoped coordination (too slow)

---

## Concrete Implementation Plan

### Phase 1: Enhance InMemoryEventBus

**Current State**:
```rust
// composable-rust-testing/src/lib.rs
pub struct InMemoryEventBus {
    events: Arc<RwLock<Vec<(String, SerializedEvent)>>>,
}
```

**Problem**: Only stores events, doesn't support subscription/broadcasting.

**Target State**:
```rust
// composable-rust-core/src/event_bus.rs (or new in-memory crate)
pub struct InMemoryEventBus {
    /// Topic -> Broadcast sender
    channels: Arc<RwLock<HashMap<String, broadcast::Sender<SerializedEvent>>>>,
    /// Channel capacity (default: 100)
    capacity: usize,
}

impl InMemoryEventBus {
    pub fn new() -> Self {
        Self::with_capacity(100)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            capacity,
        }
    }
}

#[async_trait]
impl EventBus for InMemoryEventBus {
    async fn publish(&self, topic: &str, event: &SerializedEvent) -> Result<(), EventBusError> {
        let channels = self.channels.read().await;
        if let Some(tx) = channels.get(topic) {
            // Best effort: ignore if no subscribers or channel full
            let _ = tx.send(event.clone());
        }
        Ok(())
    }

    async fn subscribe(&self, topics: &[&str]) -> Result<Pin<Box<dyn Stream<Item = (String, SerializedEvent)> + Send>>, EventBusError> {
        let mut receivers = Vec::new();

        {
            let mut channels = self.channels.write().await;
            for topic in topics {
                let tx = channels.entry(topic.to_string())
                    .or_insert_with(|| broadcast::channel(self.capacity).0);
                receivers.push((topic.to_string(), tx.subscribe()));
            }
        }

        // Merge all receivers into single stream
        Ok(Box::pin(merge_receivers(receivers)))
    }
}
```

**Key Properties**:
- **Thread-safe**: Multiple stores can publish/subscribe concurrently
- **Non-blocking**: Publish doesn't wait for subscribers
- **Automatic cleanup**: Channels drop when last sender/receiver drops
- **Request-scoped**: Created fresh for each HTTP request

### Phase 2: Refactor AppState to Use Layered Event Buses

**Current State**:
```rust
pub struct AppState {
    pub event_bus: Arc<dyn EventBus>,  // Single Redpanda instance
    // ...
}

impl AppState {
    pub fn create_payment_store(&self, payment_id: PaymentId) -> Store<...> {
        // ❌ Creates unique consumer group per request
        let unique_consumer_group = format!("ticketing-payment-store-{}", payment_id);
        let store_event_bus = RedpandaEventBus::builder()
            .consumer_group(&unique_consumer_group)
            .build()?;

        let env = PaymentEnvironment::new(
            self.clock.clone(),
            self.event_store.clone(),
            Arc::new(store_event_bus),  // Unique per request
            stream_id,
            topic,
            query,
        );

        Store::new(state, reducer, env)
    }
}
```

**Target State**:
```rust
pub struct AppState {
    // Layer 3: Redpanda (singleton, stable consumer groups)
    pub global_event_bus: Arc<RedpandaEventBus>,

    // Layer 2: PostgreSQL (source of truth)
    pub event_store: Arc<PostgresEventStore>,

    // Everything else stays the same
    pub config: Arc<Config>,
    pub clock: Arc<dyn Clock>,
    pub projections_pool: Arc<PgPool>,
    // ...
}

impl AppState {
    pub fn create_payment_store(&self, payment_id: PaymentId) -> Store<...> {
        // ✅ Create in-memory event bus (cheap, request-scoped)
        let request_event_bus: Arc<dyn EventBus> = Arc::new(
            InMemoryEventBus::new()
        );

        let env = PaymentEnvironment::new(
            self.clock.clone(),
            self.event_store.clone(),
            request_event_bus,  // In-memory for saga coordination
            stream_id,
            topic,
            query,
        );

        Store::new(state, reducer, env)
    }
}
```

### Phase 3: Update Effect Execution to Publish to Redpanda

**Current State**:
```rust
// runtime/src/lib.rs - Effect::PublishEvent execution
Effect::PublishEvent(EventBusOperation::Publish { event_bus, topic, event, .. }) => {
    // Publishes to the Store's EventBus (in-memory or per-request Redpanda)
    event_bus.publish(&topic, &event).await?;
}
```

**Target State**:
```rust
// PaymentEnvironment gets access to global Redpanda
pub struct PaymentEnvironment {
    pub clock: Arc<dyn Clock>,
    pub event_store: Arc<dyn EventStore>,
    pub event_bus: Arc<dyn EventBus>,  // In-memory for saga coordination
    pub global_event_bus: Arc<RedpandaEventBus>,  // For external distribution
    pub stream_id: StreamId,
    pub payment_topic: String,
    pub projection: Arc<dyn PaymentProjectionQuery>,
}

// PaymentReducer publishes to BOTH buses
fn create_effects(event: PaymentAction, ...) -> SmallVec<[Effect<PaymentAction>; 4]> {
    smallvec![
        // 1. Persist to PostgreSQL (source of truth)
        append_events! { ... },

        // 2. Publish to in-memory bus (saga coordination, FAST)
        publish_event! {
            bus: env.event_bus,  // In-memory
            topic: &env.payment_topic,
            event: serialized.clone(),
            on_success: || None,
            on_error: |e| Some(PaymentAction::ValidationFailed { error: e.to_string() })
        },

        // 3. Async publish to Redpanda (external subscribers, DON'T BLOCK)
        Effect::Future(Box::pin({
            let global_bus = env.global_event_bus.clone();
            let topic = env.payment_topic.clone();
            let event = serialized.clone();
            async move {
                // Fire-and-forget (don't block reducer)
                tokio::spawn(async move {
                    if let Err(e) = global_bus.publish(&topic, &event).await {
                        tracing::warn!(error = %e, topic = %topic, "Failed to publish to Redpanda");
                    }
                });
                None  // Don't produce action
            }
        })),

        // 4. Echo event back as action (for action_broadcast)
        Effect::Future(Box::pin(async move {
            Some(event)
        }))
    ]
}
```

**Alternative Design (Cleaner)**:

Add a new effect variant:

```rust
// core/src/lib.rs
pub enum Effect<Action> {
    // Existing variants...

    /// Publish to both in-memory (fast) and Redpanda (durable, external)
    PublishDual {
        local_bus: Arc<dyn EventBus>,
        global_bus: Arc<RedpandaEventBus>,
        topic: String,
        event: SerializedEvent,
    },
}

// Reducer uses cleaner syntax
publish_dual_event! {
    local_bus: env.event_bus,
    global_bus: env.global_event_bus,
    topic: &env.payment_topic,
    event: serialized
}
```

### Phase 4: Update All Aggregates

Apply the same pattern to:
- `PaymentReducer` ✅
- `ReservationReducer`
- `InventoryReducer`
- `EventReducer` (ticketing event aggregate)
- `AnalyticsReducer`

**Checklist per aggregate**:
- [ ] Environment gets `global_event_bus: Arc<RedpandaEventBus>`
- [ ] `create_effects()` publishes to both buses
- [ ] Tests use `InMemoryEventBus` (fast, no network)
- [ ] Integration tests verify Redpanda receives events

### Phase 5: Update API Handlers

**Current Pattern**:
```rust
pub async fn process_payment(
    State(state): State<AppState>,
    session: SessionUser,
    Json(req): Json<ProcessPaymentRequest>,
) -> Result<Json<ProcessPaymentResponse>, AppError> {
    let store = state.create_payment_store(payment_id);

    // Store uses per-request Redpanda consumer group ❌
    let result = store.send_and_wait_for_with_metadata(...).await?;
}
```

**Target Pattern**:
```rust
pub async fn process_payment(
    State(state): State<AppState>,
    session: SessionUser,
    Json(req): Json<ProcessPaymentRequest>,
) -> Result<Json<ProcessPaymentResponse>, AppError> {
    // Create in-memory event bus for this request's saga coordination
    let request_event_bus = Arc::new(InMemoryEventBus::new());

    // All stores in this request share the same in-memory bus
    let payment_store = state.create_payment_store_with_bus(payment_id, request_event_bus.clone());
    let saga_store = state.create_saga_store_with_bus(saga_id, request_event_bus.clone());

    // Coordination happens via in-memory bus (fast)
    let result = store.send_and_wait_for_with_metadata(...).await?;

    // Events also published to Redpanda asynchronously (external subscribers)
    // No blocking, no per-request consumer groups
}
```

**Or even cleaner** (if all stores created together):

```rust
pub struct RequestContext {
    event_bus: Arc<InMemoryEventBus>,
    correlation_id: CorrelationId,
}

impl AppState {
    pub fn create_request_context(&self) -> RequestContext {
        RequestContext {
            event_bus: Arc::new(InMemoryEventBus::new()),
            correlation_id: CorrelationId::generate(),
        }
    }
}

// In handler
let ctx = state.create_request_context();
let payment_store = state.create_payment_store(payment_id, &ctx);
let saga_store = state.create_saga_store(saga_id, &ctx);
```

---

## Performance Comparison

### Current Architecture (Per-Request Redpanda)

**Single payment request**:
```
1. HTTP request arrives                       T+0ms
2. Create Redpanda consumer group             T+10ms  (TCP handshake, coordination)
3. Store subscribes to topic                  T+15ms  (protocol negotiation)
4. ProcessPayment command                     T+16ms
5. PaymentProcessed → Redpanda               T+20ms  (publish)
6. Projection consumes event                  T+30ms  (poll, deserialize)
7. ProjectionCompleted → Redpanda            T+35ms  (publish)
8. Store polls Redpanda                       T+40ms  (network round-trip)
9. PaymentConfirmed emitted                   T+41ms
10. Response returned                         T+42ms

Total: ~42ms (Redpanda overhead: ~25ms)
Consumer groups created: 1 per request
```

### Target Architecture (In-Memory + Redpanda)

**Single payment request**:
```
1. HTTP request arrives                       T+0ms
2. Create InMemoryEventBus                    T+0.001ms (allocate channel)
3. Store subscribes (in-memory)               T+0.002ms (subscribe to broadcast)
4. ProcessPayment command                     T+1ms
5. PaymentProcessed → PostgreSQL             T+3ms   (disk write)
6. PaymentProcessed → In-memory bus          T+3.01ms (broadcast, μs latency)
7. PaymentProcessed → Redpanda (async)       T+3ms   (fire-and-forget, don't wait)
8. ProjectionCompleted (in-memory)           T+8ms   (projection updates)
9. PaymentConfirmed emitted (in-memory)      T+8.01ms (broadcast)
10. Response returned                         T+9ms

Total: ~9ms (In-memory overhead: ~0.02ms)
Consumer groups created: 0 per request
Redpanda consumers: 1 stable (projection manager)
```

**Speedup**: **4.7x faster** (42ms → 9ms)
**Resource savings**: **Zero** per-request consumer groups

### At Scale (1000 concurrent requests)

| Metric | Current (Redpanda) | Target (In-Memory) | Improvement |
|--------|-------------------|-------------------|-------------|
| Avg latency | 42ms | 9ms | **4.7x faster** |
| Consumer groups | 1000 (ephemeral) | 1 (stable) | **1000x fewer** |
| TCP connections | 1000 | 1 | **1000x fewer** |
| Broker memory | ~100MB | ~1MB | **100x less** |
| Client memory | ~500MB | ~5MB | **100x less** |

---

## Migration Strategy

### Step 1: Preparation (No Breaking Changes)

- [ ] Enhance `InMemoryEventBus` with subscription support
- [ ] Add integration tests for `InMemoryEventBus`
- [ ] Add `global_event_bus` field to `AppState` (keep old `event_bus` for now)
- [ ] Document new pattern in architecture docs

### Step 2: Gradual Migration (One Aggregate at a Time)

- [ ] Migrate `PaymentReducer` to dual-publish pattern
- [ ] Update `create_payment_store()` to use in-memory bus
- [ ] Run deployment tests, verify no regression
- [ ] Migrate `ReservationReducer`
- [ ] Migrate `InventoryReducer`
- [ ] Migrate remaining aggregates

### Step 3: API Handler Updates

- [ ] Update `process_payment()` handler
- [ ] Update reservation handlers
- [ ] Update inventory handlers
- [ ] Run full integration test suite

### Step 4: Cleanup

- [ ] Remove per-request consumer group code
- [ ] Remove old `event_bus` field from `AppState`
- [ ] Update documentation
- [ ] Performance benchmarks (before/after)

### Step 5: Validation

- [ ] Load test: 1000 concurrent requests
- [ ] Monitor Redpanda consumer group count (should be stable ~5-10)
- [ ] Monitor memory usage (should drop significantly)
- [ ] Verify projection updates still work
- [ ] Verify WebSocket notifications still work

---

## Testing Strategy

### Unit Tests (Fast, In-Memory Only)

```rust
#[tokio::test]
async fn test_payment_saga_coordination() {
    let event_bus = Arc::new(InMemoryEventBus::new());
    let event_store = Arc::new(InMemoryEventStore::new());

    let payment_store = create_payment_store(payment_id, event_bus.clone(), event_store.clone());
    let saga_store = create_saga_store(saga_id, event_bus.clone(), event_store.clone());

    // Saga coordinates via in-memory bus (microseconds)
    payment_store.send(ProcessPayment { ... }).await?;

    // Verify saga received PaymentProcessed instantly
    let saga_state = saga_store.state().await;
    assert_eq!(saga_state.payment_status, PaymentStatus::Captured);
}
```

**Benefits**:
- ✅ No network (100x faster)
- ✅ No Docker (CI-friendly)
- ✅ Deterministic (no timing issues)

### Integration Tests (Redpanda Verification)

```rust
#[tokio::test]
async fn test_redpanda_receives_payment_events() {
    // Start testcontainers Redpanda
    let redpanda = RedpandaContainer::default();

    let global_event_bus = Arc::new(RedpandaEventBus::builder()
        .brokers(&redpanda.brokers())
        .consumer_group("test-projections")
        .build()?);

    // Subscribe before publishing
    let mut stream = global_event_bus.subscribe(&["ticketing-payment-events"]).await?;

    // Process payment (publishes to Redpanda asynchronously)
    let payment_store = create_payment_store_with_redpanda(payment_id, global_event_bus.clone());
    payment_store.send(ProcessPayment { ... }).await?;

    // Verify Redpanda received the event
    let (topic, event) = tokio::time::timeout(
        Duration::from_secs(5),
        stream.next()
    ).await?.unwrap();

    assert_eq!(topic, "ticketing-payment-events");
    assert_eq!(event.event_type, "PaymentPaymentProcessed");
}
```

**Verify**:
- ✅ Events reach Redpanda
- ✅ Projections receive events
- ✅ Offset commits work
- ✅ Replay works

### Deployment Tests (End-to-End)

Keep existing deployment tests, verify:
- ✅ Payment processing still works
- ✅ Saga coordination still works
- ✅ Projections update correctly
- ✅ WebSocket notifications arrive
- ✅ **NEW**: Verify faster latency
- ✅ **NEW**: Verify stable consumer group count

---

## Success Criteria

### Performance

- [ ] Payment request latency: **< 10ms** (currently ~42ms)
- [ ] Saga coordination latency: **< 1ms** (in-memory broadcast)
- [ ] Redpanda consumer group count: **≤ 10** (stable, not per-request)

### Resource Efficiency

- [ ] Zero per-request TCP connections to Redpanda
- [ ] Zero per-request consumer group registrations
- [ ] Memory usage: **< 10MB** for 1000 concurrent requests

### Correctness

- [ ] All deployment tests pass
- [ ] Projections receive all events (at-least-once)
- [ ] WebSocket notifications work
- [ ] Event replay works (from PostgreSQL and Redpanda)

### Code Quality

- [ ] Clear separation of concerns (in-memory vs. durable)
- [ ] Consistent pattern across all aggregates
- [ ] Well-documented architecture decision
- [ ] Zero clippy warnings

---

## Open Questions

### Q1: Should InMemoryEventBus go in a new crate?

**Options**:
- A. `composable-rust-core` (always available)
- B. `composable-rust-testing` (current location, but it's not just for testing)
- C. New crate: `composable-rust-memory` (most explicit)

**Recommendation**: **Option C** (`composable-rust-memory`)
- Clear intent (in-memory implementation)
- Doesn't pollute core with implementation details
- Can have its own dependencies (e.g., `tokio::sync::broadcast`)

### Q2: Should we support mixed mode (some stores use Redpanda, others in-memory)?

**Recommendation**: **No, enforce consistency**
- All request-scoped coordination → In-memory
- All cross-process distribution → Redpanda
- Mixing would create confusion and subtle bugs

### Q3: How do we handle event ordering guarantees?

**Current**: Redpanda provides per-partition ordering
**In-Memory**: `broadcast` channel provides FIFO per-subscriber

**Recommendation**: Document that in-memory bus provides:
- ✅ FIFO ordering per subscriber (tokio broadcast guarantee)
- ✅ All subscribers see same order (single broadcast channel per topic)
- ❌ No ordering across topics (not needed for sagas)

### Q4: What about event replay for debugging?

**Current**: Can replay from Redpanda
**In-Memory**: Ephemeral, no replay

**Recommendation**:
- PostgreSQL EventStore = source of truth for replay
- Redpanda = secondary replay source (longer retention)
- In-memory = no replay (not needed, request-scoped)

For debugging specific requests:
- Enable tracing (logs all events in request context)
- Optionally: Add `DebugEventBus` wrapper that logs to file

---

## Summary

### What Changes

| Component | Before | After | Benefit |
|-----------|--------|-------|---------|
| **Saga coordination** | Redpanda per-request | In-memory per-request | 1000x faster, zero consumer groups |
| **Event distribution** | Redpanda per-request | Redpanda singleton | Stable consumer groups |
| **Event storage** | PostgreSQL | PostgreSQL (unchanged) | Source of truth |
| **Request latency** | ~42ms | ~9ms | 4.7x faster |
| **Consumer groups** | 1 per request | 1 per server | 1000x fewer at scale |

### What Stays the Same

- ✅ PostgreSQL EventStore (source of truth)
- ✅ Event sourcing patterns
- ✅ CQRS projections
- ✅ Redpanda for external subscribers
- ✅ EventBus trait interface
- ✅ Reducer patterns (publish effects same way)

### Philosophy

**Separate tools for separate concerns**:
- **In-Memory**: Speed, request-scoped
- **PostgreSQL**: Durability, source of truth
- **Redpanda**: Cross-process, at-least-once delivery

Don't force one tool to do everything.

---

## References

### Similar Patterns in Production Systems

- **Akka**: Actor messages (in-memory) + Event Journal (Cassandra) + Event Bus (Kafka)
- **Axon Framework**: Command Bus (in-memory) + Event Store (RDBMS) + Event Processor (in-memory or Kafka)
- **Eventide**: Message Store (PostgreSQL) + Consumer coordination (in-memory)
- **Lagom**: In-memory for service-local, Kafka for cross-service

### Further Reading

- [Martin Fowler - Event Sourcing](https://martinfowler.com/eaaDev/EventSourcing.html)
- [Chris Richardson - Transactional Outbox Pattern](https://microservices.io/patterns/data/transactional-outbox.html)
- [Confluent - When NOT to use Kafka](https://www.confluent.io/blog/when-not-to-use-apache-kafka/)
- [Vaughn Vernon - Reactive Messaging Patterns](https://www.reactivemanifesto.org/)
