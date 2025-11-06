# Phase 3: Sagas & Coordination - TODO List

**Goal**: Multi-aggregate workflows, event routing with Redpanda integration, saga pattern.

**Duration**: 1.5-2 weeks

**Status**: 🚧 **NOT STARTED**

**Philosophy**: Events flow from Postgres (source of truth) → Redpanda (event bus) → Subscribers (sagas, projections). Build on Phase 2's event sourcing foundation to enable cross-aggregate coordination.

---

## Prerequisites

Before starting Phase 3:
- [x] Phase 1 complete (Core abstractions validated)
- [x] Phase 2 complete (Event sourcing with Postgres)
- [ ] Order Processing example works with PostgreSQL
- [ ] Understand saga pattern (compensating transactions)
- [ ] Understand Kafka/Redpanda concepts (topics, partitions, consumer groups)
- [ ] Review Phase 3 goals in roadmap

---

## Strategic Decision: Why Redpanda?

From the roadmap:

**Decision**: Use Redpanda (Kafka-compatible) for event bus, not specialized vendors.

**Rationale**:
- **Industry standard**: Kafka API is ubiquitous, massive ecosystem
- **Vendor swappability**: Can use Kafka, AWS MSK, Azure Event Hubs (all Kafka-compatible)
- **Simpler operations**: Redpanda is easier to operate than Kafka
- **Self-hostable**: Docker, Kubernetes, bare metal deployment
- **BSL 1.1 license**: Permits internal use, becomes Apache 2.0 after 4 years
- **Client flexibility**: Every client can choose their Kafka-compatible infrastructure

**Why NOT Kurrent/EventStoreDB?**
- Vendor lock-in risk with proprietary systems
- Migration nightmare with years of event history
- With Redpanda: clients choose infrastructure, can swap vendors

**Investment**: ~1-2 weeks to build abstraction and Redpanda integration
**Return**: Strategic flexibility and industry-standard event streaming

---

## Event Flow Architecture

```
┌─────────────┐
│   Command   │
└──────┬──────┘
       │
       ▼
┌─────────────────┐
│    Reducer      │
│   (validates)   │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  1. Save Event  │
│   to Postgres   │◄─── Source of truth
│  (event store)  │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ 2. Publish to   │
│    Redpanda     │◄─── At-least-once delivery
│  (event bus)    │
└────────┬────────┘
         │
    ┌────┴────┐
    │         │
    ▼         ▼
┌───────┐ ┌───────┐
│ Saga  │ │ Other │
│       │ │ Aggr. │
└───────┘ └───────┘
```

**Key Principle**: Postgres first (durability), then Redpanda (distribution).

---

## 1. Event Bus Abstraction (`composable-rust-core`)

### 1.1 EventBus Trait
**Scope**: Abstract event publishing and subscription

```rust
/// Event bus for cross-aggregate communication
pub trait EventBus: Send + Sync {
    /// Publish an event to a topic
    async fn publish(&self, topic: &str, event: &SerializedEvent) -> Result<(), EventBusError>;

    /// Subscribe to topics and receive event stream
    async fn subscribe(&self, topics: &[&str]) -> Result<EventStream, EventBusError>;
}

/// Stream of events from subscriptions
pub type EventStream = Pin<Box<dyn Stream<Item = Result<SerializedEvent, EventBusError>> + Send>>;
```

**Tasks**:
- [ ] Define EventBus trait in `core/src/event_bus.rs`
- [ ] Define EventBusError type using `thiserror`
- [ ] Define EventStream type alias
- [ ] Document publish semantics (at-least-once)
- [ ] Document subscribe semantics (consumer groups)
- [ ] Add comprehensive doc comments

### 1.2 Effect Extensions for EventBus
**Scope**: Add PublishEvent effect variant

```rust
pub enum Effect<Action> {
    // ... existing variants
    PublishEvent {
        topic: String,
        event: SerializedEvent,
        event_bus: Arc<dyn EventBus>,
        // Optional callback action
        on_success: Option<Action>,
        on_error: Option<Box<dyn Fn(EventBusError) -> Action + Send + Sync>>,
    },
}
```

**Tasks**:
- [ ] Add `Effect::PublishEvent` variant
- [ ] Update Effect::map() to handle PublishEvent
- [ ] Update merge() and chain() to handle PublishEvent
- [ ] Add tests for PublishEvent composition
- [ ] Document PublishEvent usage patterns

### 1.3 Topic Naming Conventions
**Scope**: Standard topic naming strategy

**Pattern**: `{aggregate-type}-events` (e.g., "order-events", "payment-events")

**Tasks**:
- [ ] Document topic naming conventions
- [ ] Add helper for generating topic names
- [ ] Document partitioning strategy (by aggregate ID)
- [ ] Add examples in documentation

---

## 2. In-Memory Event Bus (`composable-rust-testing`)

### 2.1 InMemoryEventBus Implementation
**Scope**: HashMap-based event bus for testing

```rust
pub struct InMemoryEventBus {
    subscribers: Arc<RwLock<HashMap<String, Vec<Sender<SerializedEvent>>>>>,
}

impl InMemoryEventBus {
    pub fn new() -> Self;

    // Test helpers
    pub fn topic_count(&self) -> usize;
    pub fn subscriber_count(&self, topic: &str) -> usize;
}
```

**Tasks**:
- [ ] Implement EventBus trait for InMemoryEventBus
- [ ] Use tokio channels for pub/sub
- [ ] Synchronous delivery (no network delay)
- [ ] Support multiple subscribers per topic
- [ ] Add inspection methods for tests
- [ ] Add comprehensive tests
- [ ] Document usage in testing

### 2.2 Test Helpers for Event Bus
**Scope**: Utilities for testing event-driven workflows

**Tasks**:
- [ ] Event spy (capture published events)
- [ ] Event builder helpers
- [ ] Assertion helpers (assert_event_published, etc.)
- [ ] Subscription test helpers
- [ ] Document test patterns

---

## 3. Redpanda Integration (`composable-rust-redpanda`)

### 3.1 New Crate Setup
**Scope**: Create dedicated crate for Redpanda

**Tasks**:
- [ ] Create `redpanda/` directory in workspace
- [ ] Add to workspace Cargo.toml
- [ ] Add dependencies: `rdkafka`, `tokio`, `futures`
- [ ] Create `redpanda/src/lib.rs` with module structure
- [ ] Configure crate metadata in Cargo.toml
- [ ] Add README explaining Redpanda setup

### 3.2 RedpandaEventBus Implementation
**Scope**: Implement EventBus trait using rdkafka

```rust
pub struct RedpandaEventBus {
    producer: FutureProducer,
    brokers: String,
}

impl RedpandaEventBus {
    pub async fn new(brokers: &str) -> Result<Self, EventBusError>;
    pub async fn from_config(config: ClientConfig) -> Result<Self, EventBusError>;
}
```

**Tasks**:
- [ ] Implement EventBus trait for RedpandaEventBus
- [ ] Configure rdkafka producer (at-least-once semantics)
- [ ] Configure rdkafka consumer (consumer groups)
- [ ] Handle serialization (bincode to bytes)
- [ ] Add connection pooling/management
- [ ] Handle errors gracefully
- [ ] Add comprehensive tests with testcontainers

### 3.3 Event Publishing
**Scope**: Publish events to Redpanda after Postgres commit

**Flow**:
1. Reducer emits Effect::EventStore(AppendEvents)
2. Store executes: save to Postgres
3. On success, emit Effect::PublishEvent
4. Store executes: publish to Redpanda

**Tasks**:
- [ ] Implement publish() with rdkafka FutureProducer
- [ ] Set message key to aggregate ID (for partitioning)
- [ ] Serialize event with bincode
- [ ] Add metadata (correlation IDs, timestamps)
- [ ] Handle publish failures (log, retry?)
- [ ] Add tracing for observability
- [ ] Document publish semantics

### 3.4 Event Subscription
**Scope**: Subscribe to Redpanda topics

**Tasks**:
- [ ] Implement subscribe() with rdkafka StreamConsumer
- [ ] Configure consumer group ID
- [ ] Deserialize events from bincode
- [ ] Handle offset commits (at-least-once)
- [ ] Handle rebalancing gracefully
- [ ] Add error handling (deserialization, network)
- [ ] Document subscription patterns

### 3.5 Testing with Testcontainers
**Scope**: Integration tests with real Redpanda

**Tasks**:
- [ ] Add testcontainers dependency (redpanda)
- [ ] Create test helpers for Redpanda setup
- [ ] Write integration tests for publish
- [ ] Write integration tests for subscribe
- [ ] Test pub/sub round-trip
- [ ] Test consumer groups
- [ ] Document testing approach

---

## 4. Event Publishing Flow (`composable-rust-runtime`)

### 4.1 Effect Executor for PublishEvent
**Scope**: Execute PublishEvent effects in Store

**Tasks**:
- [ ] Add PublishEvent handling to Store effect executor
- [ ] Execute event bus publish asynchronously
- [ ] Handle publish errors (log, callback action)
- [ ] Feed callback actions back to Store
- [ ] Add tests for PublishEvent execution
- [ ] Document error handling strategy

### 4.2 Two-Phase Event Persistence
**Scope**: Postgres first, then Redpanda

**Pattern**:
```rust
// Reducer returns both effects
vec![
    Effect::EventStore(AppendEvents { ... }),
    Effect::PublishEvent { ... },  // Only if AppendEvents succeeds
]
```

**Tasks**:
- [ ] Document two-phase pattern
- [ ] Show examples of conditional publishing
- [ ] Add tests for persistence + publish flow
- [ ] Document failure scenarios
- [ ] Document idempotency strategy

### 4.3 Idempotency Support
**Scope**: Handle duplicate event delivery

**Strategy**:
- Events include correlation IDs
- Subscribers check correlation ID before processing
- Reducers are idempotent (same event twice = same result)

**Tasks**:
- [ ] Add correlation_id to SerializedEvent metadata
- [ ] Document idempotency patterns
- [ ] Add examples of idempotent reducers
- [ ] Add tests for duplicate handling

---

## 5. Saga Support

### 5.1 Saga Pattern Basics
**Scope**: Sagas are event-sourced aggregates that coordinate other aggregates

**Key Insight**: Sagas don't need special framework support—they're just reducers with state machines.

```rust
pub struct SagaState {
    saga_id: String,
    current_step: Step,
    completed_steps: Vec<Step>,
    compensation_steps: Vec<Step>,
    // IDs for compensation
    order_id: Option<OrderId>,
    payment_id: Option<PaymentId>,
}

pub enum SagaAction {
    // Commands
    StartSaga { ... },
    // Events from other aggregates
    OrderPlaced { order_id: OrderId, ... },
    PaymentCompleted { payment_id: PaymentId, ... },
    PaymentFailed { error: String, ... },
    // Internal saga events
    SagaCompleted,
    SagaFailed { reason: String },
}
```

**Tasks**:
- [ ] Document saga pattern (state machine approach)
- [ ] Show saga as reducer example
- [ ] Document compensation pattern
- [ ] Show timeout handling (via Delay effect)
- [ ] Add comprehensive saga tests
- [ ] Document saga best practices

### 5.2 Saga State Persistence
**Scope**: Sagas use event sourcing (like any aggregate)

**Tasks**:
- [ ] Document saga event persistence
- [ ] Show saga using EventStore
- [ ] Add saga state reconstruction example
- [ ] Document saga versioning

### 5.3 Compensation Pattern
**Scope**: Rolling back partial workflow

**Pattern**:
```rust
match (state.current_step, action) {
    (Step::PaymentProcessing, PaymentFailed { error }) => {
        // Start compensation
        state.current_step = Step::Compensating;
        vec![
            Effect::DispatchCommand(CancelOrder { order_id }),
        ]
    }
}
```

**Tasks**:
- [ ] Document compensation strategies
- [ ] Show examples of compensating actions
- [ ] Add tests for compensation flows
- [ ] Document when NOT to compensate

### 5.4 Timeout Handling
**Scope**: Handle delayed responses

**Pattern**:
```rust
// Start operation with timeout
vec![
    Effect::DispatchCommand(ReserveInventory { ... }),
    Effect::Delay {
        duration: Duration::from_secs(30),
        action: Some(Box::new(InventoryTimeout)),
    },
]

// Cancel timeout on success
match action {
    InventoryReserved { ... } => {
        // Cancel delay effect (implementation detail)
        vec![Effect::None]
    }
}
```

**Tasks**:
- [ ] Document timeout patterns
- [ ] Show cancellable delays
- [ ] Add timeout tests
- [ ] Document timeout best practices

---

## 6. Cross-Aggregate Communication

### 6.1 DispatchCommand Effect
**Scope**: Send commands to other aggregates

```rust
pub enum Effect<Action> {
    // ... existing variants
    DispatchCommand {
        target: String,  // Aggregate ID or service name
        command: Box<dyn Any + Send + Sync>,  // Type-erased command
        // In-process: store reference
        // Distributed: via Redpanda (Phase 4)
    },
}
```

**Tasks**:
- [ ] Add Effect::DispatchCommand variant
- [ ] Implement in-process dispatch (store reference)
- [ ] Document command routing
- [ ] Add tests for cross-aggregate commands
- [ ] Document distributed dispatch (Phase 4 consideration)

### 6.2 Event Routing
**Scope**: Route events to multiple subscribers

**Pattern**: Subscribers filter events by correlation ID or saga ID

**Tasks**:
- [ ] Document event routing patterns
- [ ] Show subscriber filtering examples
- [ ] Add multi-subscriber tests
- [ ] Document fan-out patterns

### 6.3 Correlation ID Propagation
**Scope**: Track causality across aggregates

**Tasks**:
- [ ] Add saga_id/correlation_id to event metadata
- [ ] Document propagation patterns
- [ ] Show examples in saga
- [ ] Add tests for correlation tracking

---

## 7. Reducer Composition Utilities

### 7.1 combine_reducers Helper
**Scope**: Compose multiple reducers

```rust
pub fn combine_reducers<S, A, E>(
    reducers: Vec<Box<dyn Reducer<State = S, Action = A, Environment = E>>>,
) -> impl Reducer<State = S, Action = A, Environment = E>
```

**Tasks**:
- [ ] Implement combine_reducers in `core/src/composition.rs`
- [ ] Document composition semantics
- [ ] Add examples
- [ ] Add tests

### 7.2 scope_reducer Helper
**Scope**: Scope a reducer to a sub-state

```rust
pub fn scope_reducer<S, SubS, A, E>(
    reducer: impl Reducer<State = SubS, Action = A, Environment = E>,
    get_state: fn(&S) -> &SubS,
    set_state: fn(&mut S, SubS),
) -> impl Reducer<State = S, Action = A, Environment = E>
```

**Tasks**:
- [ ] Implement scope_reducer
- [ ] Document scoping patterns
- [ ] Add examples
- [ ] Add tests

### 7.3 Documentation and Patterns
**Scope**: Composition best practices

**Tasks**:
- [ ] Document when to use composition
- [ ] Show real-world examples
- [ ] Add anti-patterns section
- [ ] Document performance considerations

---

## 8. Example: Checkout Saga

### 8.1 Checkout Saga Implementation
**Location**: `examples/checkout-workflow/`

**Aggregates Involved**:
- **Order** (from Phase 2)
- **Payment** (new)
- **Inventory** (new)
- **CheckoutSaga** (new)

**Workflow Steps**:
1. Customer initiates checkout
2. Saga creates order (PlaceOrder command)
3. On OrderPlaced event → Process payment (ProcessPayment command)
4. On PaymentCompleted → Reserve inventory (ReserveInventory command)
5. On InventoryReserved → Complete checkout

**Compensation Flows**:
- Payment fails → Cancel order
- Inventory reservation fails → Refund payment, cancel order
- Timeout on any step → Full compensation

**Tasks**:
- [ ] Define CheckoutSaga state and actions
- [ ] Implement Payment aggregate (simplified)
- [ ] Implement Inventory aggregate (simplified)
- [ ] Implement saga reducer with all steps
- [ ] Add happy path test (all steps succeed)
- [ ] Add payment failure test (with compensation)
- [ ] Add inventory timeout test
- [ ] Add full compensation test
- [ ] Wire up Redpanda event bus
- [ ] Document workflow in README

### 8.2 Payment Aggregate
**Scope**: Simple payment processing aggregate

**Commands**: ProcessPayment, RefundPayment
**Events**: PaymentCompleted, PaymentFailed, PaymentRefunded

**Tasks**:
- [ ] Define PaymentState and PaymentAction
- [ ] Implement PaymentReducer
- [ ] Add payment validation
- [ ] Add payment tests
- [ ] Document payment aggregate

### 8.3 Inventory Aggregate
**Scope**: Simple inventory management

**Commands**: ReserveInventory, ReleaseInventory
**Events**: InventoryReserved, InventoryReleased, InsufficientInventory

**Tasks**:
- [ ] Define InventoryState and InventoryAction
- [ ] Implement InventoryReducer
- [ ] Add inventory validation
- [ ] Add inventory tests
- [ ] Document inventory aggregate

### 8.4 Integration Tests
**Scope**: End-to-end saga testing

**Tests**:
- [ ] Happy path: checkout → order → payment → inventory → success
- [ ] Payment fails: checkout → order → payment fails → cancel order
- [ ] Inventory fails: checkout → order → payment → inventory fails → refund + cancel
- [ ] Timeout: checkout → order → payment → inventory timeout → compensation
- [ ] All events published to Redpanda
- [ ] Saga state persisted to Postgres

**Tasks**:
- [ ] Set up test environment (Postgres + Redpanda)
- [ ] Write integration tests
- [ ] Use testcontainers
- [ ] Verify event flow
- [ ] Verify compensation

### 8.5 Checkout Documentation
**Scope**: Comprehensive example documentation

**Tasks**:
- [ ] README with workflow diagram
- [ ] Document each aggregate
- [ ] Document saga coordination
- [ ] Document compensation logic
- [ ] Document event flow
- [ ] Add usage examples

---

## 9. Documentation

### 9.1 API Documentation
- [ ] Complete EventBus trait documentation
- [ ] Document Effect::PublishEvent with examples
- [ ] Document Effect::DispatchCommand with examples
- [ ] Document RedpandaEventBus usage
- [ ] Document InMemoryEventBus usage
- [ ] Add `# Examples` sections to all APIs
- [ ] Add `# Errors` sections where applicable

### 9.2 Guide Documentation
- [ ] Create `docs/sagas.md`:
  - [ ] Saga pattern explanation
  - [ ] State machine approach
  - [ ] Compensation strategies
  - [ ] Timeout handling
  - [ ] Best practices
- [ ] Create `docs/event-bus.md`:
  - [ ] EventBus abstraction
  - [ ] Redpanda setup guide
  - [ ] Topic naming conventions
  - [ ] Consumer groups
  - [ ] Troubleshooting
- [ ] Update `docs/getting-started.md`:
  - [ ] Add multi-aggregate section
  - [ ] Add saga example walkthrough

### 9.3 Redpanda Setup Guide
- [ ] Create `docs/redpanda-setup.md`:
  - [ ] Local Redpanda with Docker
  - [ ] Topic creation
  - [ ] Consumer group configuration
  - [ ] Monitoring and debugging
  - [ ] Production deployment options
  - [ ] Kafka compatibility notes

---

## 10. Validation & Testing

### 10.1 Unit Tests
- [ ] EventBus trait implementations
- [ ] InMemoryEventBus functionality
- [ ] RedpandaEventBus (with mocks)
- [ ] Effect::PublishEvent composition
- [ ] Saga reducer logic
- [ ] Reducer composition utilities

### 10.2 Integration Tests
- [ ] RedpandaEventBus with testcontainers
- [ ] Pub/sub round-trip
- [ ] Consumer groups
- [ ] Checkout saga end-to-end
- [ ] Compensation flows
- [ ] Timeout scenarios

### 10.3 Performance Tests
- [ ] Event publishing throughput
- [ ] Event consumption rate
- [ ] Saga coordination latency
- [ ] End-to-end checkout workflow time

### 10.4 Quality Checks
- [ ] `cargo build --all-features` succeeds
- [ ] `cargo test --all-features` passes
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes
- [ ] `cargo fmt --all --check` passes
- [ ] `cargo doc --no-deps --all-features` builds successfully

---

## 11. Key Implementation Decisions

### 11.1 Event Bus: Redpanda ✅
- **Decision**: Redpanda (Kafka-compatible) for event bus
- **Rationale**: Industry standard, vendor swappable, simpler ops than Kafka
- **Alternatives**: AWS SNS/SQS (rejected: vendor lock-in), NATS (considered but less adoption)

### 11.2 Event Publishing Order
- [ ] **Decision**: Postgres first, then Redpanda
- [ ] **Rationale**: Postgres is source of truth, Redpanda for distribution
- [ ] **Trade-offs**: Potential delay between persist and publish, handled via idempotency

### 11.3 Topic Strategy
- [ ] **Decision**: One topic per aggregate type (e.g., "order-events")
- [ ] **Rationale**: Clear separation, easy to subscribe to specific aggregate types
- [ ] **Partitioning**: By aggregate ID for ordering guarantees

### 11.4 Consumer Groups
- [ ] **Decision**: Each saga type gets its own consumer group
- [ ] **Rationale**: Independent processing, scaling per saga type
- [ ] **Configuration**: Consumer group ID = saga type name

### 11.5 Idempotency Strategy
- [ ] **Decision**: Correlation IDs in event metadata + idempotent reducers
- [ ] **Rationale**: Handle at-least-once delivery, duplicate events safe
- [ ] **Implementation**: Subscribers check correlation ID, skip duplicates

### 11.6 Command Dispatching
- [ ] **Decision**: In-process via store reference (Phase 3), distributed via Redpanda (Phase 4)
- [ ] **Rationale**: Start simple, add distribution when needed
- [ ] **Future**: Commands can be published to Redpanda for distributed systems

---

## 12. Phase 3 Scope Reminder

**IN SCOPE** (Phase 3):
- ✅ EventBus trait abstraction
- ✅ InMemoryEventBus for testing
- ✅ RedpandaEventBus implementation
- ✅ Event publishing after Postgres commit
- ✅ Saga pattern (as reducers with state machines)
- ✅ Cross-aggregate communication (events)
- ✅ Reducer composition utilities
- ✅ Checkout Saga example (Order + Payment + Inventory)

**OUT OF SCOPE** (Later phases):
- ❌ Distributed command dispatching → Phase 4
- ❌ Dead letter queues → Phase 4
- ❌ Advanced error handling (retries, circuit breakers) → Phase 4
- ❌ Production observability → Phase 4
- ❌ Performance optimization → Phase 4

---

## 13. Transition to Phase 4

### 13.1 Phase 4 Preparation
- [ ] Review Phase 4 goals (Production Hardening)
- [ ] Identify Redpanda production features needed
- [ ] Plan observability integration
- [ ] Create `plans/phase-4/TODO.md`

### 13.2 Final Phase 3 Review
- [ ] All validation criteria met
- [ ] Checkout Saga demonstrates full workflow
- [ ] Events flow through Redpanda correctly
- [ ] Compensation and timeouts work
- [ ] Documentation complete
- [ ] Ready for production hardening

---

## 14. Success Criteria

Phase 3 is complete when:

- ✅ Events can be published to Redpanda after Postgres commit
- ✅ Subscribers can receive events from Redpanda
- ✅ Saga coordinates multiple aggregates
- ✅ Compensation works when steps fail
- ✅ Timeouts are handled correctly
- ✅ Checkout example demonstrates complete workflow
- ✅ Tests run fast (unit < 100ms, integration < 5s)
- ✅ Can implement 5-step workflow with compensation in < 200 LOC
- ✅ All public APIs documented
- ✅ All quality checks pass

**Key Quote from Roadmap**: "Can implement a 5-step workflow with compensation in < 200 LOC."

---

## Estimated Time Breakdown

Based on roadmap estimate of 1.5-2 weeks:

1. **EventBus trait & core types**: 1 day
2. **InMemoryEventBus**: 1 day
3. **Redpanda crate setup**: 0.5 day
4. **RedpandaEventBus implementation**: 2-3 days
5. **Effect::PublishEvent integration**: 1 day
6. **Saga pattern documentation**: 1 day
7. **Payment aggregate**: 1 day
8. **Inventory aggregate**: 1 day
9. **Checkout Saga**: 2-3 days
10. **Integration tests**: 2 days
11. **Documentation**: 2 days
12. **Validation & polish**: 1 day

**Total**: 15-18 days (2-2.5 weeks of full-time work)

**Note**: Roadmap estimates 1.5-2 weeks. Budget 2-2.5 weeks for safety, especially for Redpanda learning curve.

---

## Notes

### Phase 3 Focus
This phase adds **distribution** to the framework. Phase 2 had single-aggregate workflows. Phase 3 enables multi-aggregate coordination via events.

### Redpanda Learning Resources
- Redpanda Quickstart: https://docs.redpanda.com/current/get-started/quick-start/
- Redpanda Docker: https://docs.redpanda.com/current/get-started/quick-start-docker/
- rdkafka crate: https://docs.rs/rdkafka/

### Testing Strategy
- **Unit tests**: Use InMemoryEventBus (fast, deterministic)
- **Integration tests**: Use real Redpanda via testcontainers
- **Saga tests**: Mock aggregates, test state machine logic

---

## Conclusion

Phase 3 builds on Phase 2's event sourcing to enable distributed, multi-aggregate workflows. The saga pattern (implemented as regular reducers with state machines) provides coordination and compensation without framework magic.

**Philosophy**: Events-first architecture. Everything flows through the event bus. Sagas are just subscribers that dispatch commands.

Let's build distributed workflows! 🚀
