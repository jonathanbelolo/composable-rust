# Event Bus Guide

## Table of Contents
- [Overview](#overview)
- [EventBus Trait](#eventbus-trait)
- [Topic Naming Conventions](#topic-naming-conventions)
- [Implementations](#implementations)
- [Publishing Events](#publishing-events)
- [Subscribing to Events](#subscribing-to-events)
- [Consumer Groups](#consumer-groups)
- [Delivery Semantics](#delivery-semantics)
- [Testing Patterns](#testing-patterns)
- [Troubleshooting](#troubleshooting)

## Overview

The Event Bus provides **cross-aggregate communication** in Composable Rust. Events flow from one aggregate (source of truth in Postgres) through the event bus (Redpanda) to other aggregates and sagas.

### Event Flow Architecture

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
│    Redpanda     │◄─── Distribution
│  (event bus)    │
└────────┬────────┘
         │
    ┌────┴────┐
    │         │
    ▼         ▼
┌───────┐ ┌───────┐
│ Saga  │ │ Other │
└───────┘ └───────┘
```

## EventBus Trait

The `EventBus` trait abstracts event publishing and subscription:

```rust
/// Event bus for cross-aggregate communication
pub trait EventBus: Send + Sync {
    /// Publish an event to a topic
    fn publish(
        &self,
        topic: &str,
        event: &SerializedEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), EventBusError>> + Send + '_>>;

    /// Subscribe to topics and receive event stream
    fn subscribe(
        &self,
        topics: &[&str],
    ) -> Pin<Box<dyn Future<Output = Result<EventStream, EventBusError>> + Send + '_>>;
}

/// Stream of events from subscriptions
pub type EventStream = Pin<Box<dyn Stream<Item = Result<SerializedEvent, EventBusError>> + Send>>;
```

### Key Design Decisions

1. **Async API**: All operations are async (compatible with async reducers)
2. **Topic-based routing**: Events published to topics, subscribers filter
3. **Serialized events**: Uses `SerializedEvent` (already bincode-encoded)
4. **Error handling**: Explicit error types via `EventBusError`

## Topic Naming Conventions

### Standard Pattern

Use the pattern: `{aggregate-type}-events`

Examples:
- `order-events` - All events from Order aggregate
- `payment-events` - All events from Payment aggregate
- `inventory-events` - All events from Inventory aggregate

### Rationale

1. **Clear separation**: Each aggregate type has its own topic
2. **Easy subscription**: Subscribers know which topics to watch
3. **Scalability**: Topics can be partitioned independently
4. **Ordering guarantees**: Events of the same type maintain order

### Partitioning Strategy

Events are partitioned by `event_type` (the message key):

```rust
// In RedpandaEventBus::publish()
let key = event.event_type.as_bytes();
let record = FutureRecord::to(&topic)
    .payload(&payload)
    .key(key);  // Events with same type go to same partition
```

**Ordering guarantee**: Events with the same `event_type` maintain order within a partition.

## Implementations

### InMemoryEventBus (Testing)

Fast, synchronous event bus for testing:

```rust
use composable_rust_testing::InMemoryEventBus;

let event_bus = InMemoryEventBus::new();

// Publish
event_bus.publish("order-events", &event).await?;

// Subscribe
let mut stream = event_bus.subscribe(&["order-events"]).await?;
```

**Features**:
- Synchronous delivery (no network latency)
- Multiple subscribers per topic
- Fast test execution
- No external dependencies

**Use for**:
- Unit tests
- Integration tests without Redpanda
- Local development

### RedpandaEventBus (Production)

Production-ready Kafka-compatible event bus:

```rust
use composable_rust_redpanda::RedpandaEventBus;

// Basic usage
let event_bus = RedpandaEventBus::new("localhost:9092")?;

// Advanced configuration
let event_bus = RedpandaEventBus::builder()
    .brokers("localhost:9092,localhost:9093")
    .producer_acks("all")  // Wait for all replicas
    .compression("lz4")
    .consumer_group("my-service")
    .auto_offset_reset("earliest")
    .buffer_size(5000)
    .build()?;
```

**Features**:
- At-least-once delivery
- Consumer groups for load balancing
- Fault tolerance and reconnection
- Production-grade performance

**Use for**:
- Production deployments
- Distributed systems
- High-throughput applications

See [Redpanda Setup Guide](redpanda-setup.md) for deployment instructions.

## Publishing Events

### Basic Publishing

```rust
let event = SerializedEvent::new(
    "OrderPlaced.v1".to_string(),
    bincode::serialize(&order)?,
    Some(metadata),
);

event_bus.publish("order-events", &event).await?;
```

### Publishing from Reducers

Use `Effect::PublishEvent` to publish after Postgres commit:

```rust
fn reduce(&self, state: &mut State, action: Action, env: &Env) -> SmallVec<[Effect<Action>; 4]> {
    match action {
        OrderAction::PlaceOrder { cart } => {
            // 1. Update state
            state.status = OrderStatus::Placed;

            // 2. Return effects
            smallvec![
                // First: persist to Postgres (source of truth)
                Effect::EventStore(AppendEvents {
                    events: vec![OrderPlaced { order_id: state.id, cart }],
                }),
                // Second: publish to event bus (distribution)
                Effect::PublishEvent {
                    topic: "order-events".to_string(),
                    event: SerializedEvent::new("OrderPlaced.v1", ...),
                    event_bus: env.event_bus.clone(),
                    on_success: None,
                    on_error: None,
                },
            ]
        }
    }
}
```

**Important**: Always persist to Postgres before publishing to Redpanda. Postgres is the source of truth.

### Error Handling

```rust
match event_bus.publish("order-events", &event).await {
    Ok(()) => {
        tracing::info!("Event published successfully");
    }
    Err(EventBusError::PublishFailed { topic, reason }) => {
        tracing::error!(
            topic = %topic,
            reason = %reason,
            "Failed to publish event"
        );
        // Retry or log for manual intervention
    }
    Err(e) => {
        tracing::error!(error = %e, "Event bus error");
    }
}
```

## Subscribing to Events

### Basic Subscription

```rust
use futures::StreamExt;

let mut stream = event_bus.subscribe(&["order-events"]).await?;

while let Some(result) = stream.next().await {
    match result {
        Ok(event) => {
            println!("Received: {}", event.event_type);
            // Process event...
        }
        Err(e) => {
            eprintln!("Error: {}", e);
        }
    }
}
```

### Multi-Topic Subscription

```rust
// Subscribe to multiple topics
let mut stream = event_bus
    .subscribe(&["order-events", "payment-events"])
    .await?;

while let Some(result) = stream.next().await {
    let event = result?;

    match event.event_type.as_str() {
        "OrderPlaced" => handle_order_placed(event),
        "PaymentCompleted" => handle_payment_completed(event),
        _ => {} // Ignore unknown events
    }
}
```

### Saga Subscription Pattern

Sagas subscribe to events from multiple aggregates:

```rust
pub async fn run_checkout_saga(
    event_bus: Arc<dyn EventBus>,
) -> Result<()> {
    let mut stream = event_bus
        .subscribe(&[
            "order-events",
            "payment-events",
            "inventory-events",
        ])
        .await?;

    while let Some(result) = stream.next().await {
        let event = result?;

        // Dispatch to saga reducer
        let action = match event.event_type.as_str() {
            "OrderPlaced" => deserialize_order_placed(&event.data)?,
            "PaymentCompleted" => deserialize_payment_completed(&event.data)?,
            "InventoryReserved" => deserialize_inventory_reserved(&event.data)?,
            _ => continue,
        };

        let effects = saga.reduce(&mut state, action, &env);
        store.execute_effects(effects).await?;
    }

    Ok(())
}
```

## Consumer Groups

Consumer groups enable **load balancing** across multiple instances of a subscriber.

### How Consumer Groups Work

```
Topic: order-events (3 partitions)

Consumer Group: order-processor
┌─────────────┐
│ Instance 1  │──── Partition 0 ────┐
└─────────────┘                      │
┌─────────────┐                      ├─── Events distributed
│ Instance 2  │──── Partition 1 ────┤
└─────────────┘                      │
┌─────────────┐                      │
│ Instance 3  │──── Partition 2 ────┘
└─────────────┘
```

Each instance in the group receives a subset of events (no duplicates).

### Configuring Consumer Groups

```rust
// Explicit consumer group
let event_bus = RedpandaEventBus::builder()
    .brokers("localhost:9092")
    .consumer_group("payment-saga-coordinator")
    .build()?;

// Auto-generated consumer group (from topics)
let event_bus = RedpandaEventBus::new("localhost:9092")?;
// Group ID: "composable-rust-order-events-payment-events" (sorted, deterministic)
```

### Consumer Group Best Practices

1. **One group per saga type**: Each saga type gets its own consumer group
2. **Explicit names**: Use `consumer_group()` for production clarity
3. **Scale horizontally**: Add instances to handle more load
4. **Idempotent processing**: Handle duplicate events (rebalancing may cause duplicates)

## Delivery Semantics

### At-Least-Once Delivery

Redpanda provides **at-least-once delivery** with manual offset commits:

```
┌────────────────────────────────────────────────────────┐
│ 1. Consume message from Kafka                         │
│ 2. Deserialize event                                  │
│ 3. Send to subscriber's channel                       │
│ 4. Commit offset (ONLY after successful send)         │
└────────────────────────────────────────────────────────┘
```

**If the process crashes before step 4**: Event will be redelivered on restart.

### Handling Duplicates

Make your reducers **idempotent**:

```rust
fn reduce(&self, state: &mut State, action: Action, env: &Env) -> SmallVec<[Effect<Action>; 4]> {
    match action {
        SagaAction::OrderPlaced { order_id } => {
            // Check if already processed
            if state.order_id.is_some() {
                return smallvec![Effect::None]; // Duplicate, ignore
            }

            // First time, process it
            state.order_id = Some(order_id);
            smallvec![Effect::DispatchCommand(ProcessPayment { order_id })]
        }
    }
}
```

### Correlation IDs

Use correlation IDs to track causality:

```rust
let metadata = serde_json::json!({
    "correlation_id": "checkout-12345",
    "saga_id": "saga-67890",
    "timestamp": Utc::now(),
});

let event = SerializedEvent::new(
    "OrderPlaced.v1".to_string(),
    data,
    Some(metadata),
);
```

## Testing Patterns

### Unit Tests with InMemoryEventBus

```rust
#[tokio::test]
async fn test_saga_coordination() {
    let event_bus = Arc::new(InMemoryEventBus::new());

    // Publish event
    let event = SerializedEvent::new("OrderPlaced.v1", data, None);
    event_bus.publish("order-events", &event).await.unwrap();

    // Subscribe and verify
    let mut stream = event_bus.subscribe(&["order-events"]).await.unwrap();
    let received = stream.next().await.unwrap().unwrap();

    assert_eq!(received.event_type, "OrderPlaced.v1");
}
```

### Integration Tests with Testcontainers

```rust
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::kafka::{Kafka, KAFKA_PORT};

#[tokio::test]
async fn test_redpanda_round_trip() {
    // Start Kafka container
    let kafka = Kafka::default().start().await.unwrap();

    let host = kafka.get_host().await.unwrap();
    let port = kafka.get_host_port_ipv4(KAFKA_PORT).await.unwrap();
    let brokers = format!("{host}:{port}");

    // Create event bus
    let event_bus = RedpandaEventBus::new(&brokers).unwrap();

    // Test publish/subscribe...
}
```

See `redpanda/tests/integration_tests.rs` for complete examples.

## Troubleshooting

### Common Issues

#### 1. Events Not Received

**Symptoms**: Subscriber doesn't receive events

**Causes**:
- Wrong topic name
- Subscriber started after events published (with `auto_offset_reset = "latest"`)
- Consumer group already has committed offsets

**Solutions**:
```rust
// For testing, start from beginning
let event_bus = RedpandaEventBus::builder()
    .brokers("localhost:9092")
    .auto_offset_reset("earliest")  // Start from beginning
    .build()?;

// For production, use unique consumer group per subscriber type
.consumer_group("my-unique-group-name")
```

#### 2. Duplicate Events

**Symptoms**: Same event processed multiple times

**Causes**:
- At-least-once delivery semantics
- Consumer rebalancing
- Process crash before offset commit

**Solutions**:
- Make reducers idempotent
- Use correlation IDs to detect duplicates
- Track processed event IDs in state

#### 3. Slow Consumption

**Symptoms**: Events piling up, lag increasing

**Causes**:
- Slow event processing
- Single consumer can't keep up
- Small buffer size

**Solutions**:
```rust
// Increase buffer size
.buffer_size(10000)

// Add more consumer instances (same consumer group)
// Scale horizontally

// Optimize event processing (async operations)
```

#### 4. Connection Failures

**Symptoms**: `ConnectionFailed` errors

**Causes**:
- Redpanda not running
- Wrong broker addresses
- Network issues

**Solutions**:
```bash
# Check Redpanda is running
docker ps | grep redpanda

# Test connection
rpk cluster info --brokers localhost:9092
```

### Debug Logging

Enable tracing for event bus operations:

```rust
tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .init();
```

Look for:
- `RedpandaEventBus created successfully`
- `Event published successfully`
- `Subscribed to topics`
- `Received event`

## Next Steps

- Read [Saga Pattern Guide](sagas.md) for workflow coordination
- Read [Redpanda Setup](redpanda-setup.md) for production deployment
- Study `examples/checkout-saga/` for complete event-driven workflow
- Review Phase 3 TODO for advanced patterns
