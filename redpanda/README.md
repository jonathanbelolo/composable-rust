# composable-rust-redpanda

**Redpanda/Kafka event bus implementation for Composable Rust.**

## Overview

Production-ready Kafka-compatible event bus using Redpanda with at-least-once delivery semantics.

## Installation

```toml
[dependencies]
composable-rust-redpanda = { path = "../redpanda" }
rdkafka = "0.36"
```

## Quick Start

```rust
use composable_rust_redpanda::RedpandaEventBus;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_bus = RedpandaEventBus::builder()
        .broker("localhost:9092")
        .group_id("my-service")
        .build()
        .await?;

    // Use with Store
    let environment = SagaEnvironment {
        event_bus,
        event_store,
    };

    Ok(())
}
```

## Features

- ✅ **Kafka protocol** - Compatible with Kafka and Redpanda
- ✅ **At-least-once delivery** - Manual offset commits
- ✅ **Consumer groups** - Horizontal scaling
- ✅ **Topic management** - Auto-create topics
- ✅ **Testcontainers** - Integration testing

## Configuration

### Builder API

```rust
let event_bus = RedpandaEventBus::builder()
    .broker("localhost:9092")
    .group_id("order-service")
    .client_id("order-service-1")
    .auto_offset_reset("earliest")
    .session_timeout_ms(10000)
    .build()
    .await?;
```

### Environment Variables

```bash
REDPANDA_BROKERS=localhost:9092
REDPANDA_GROUP_ID=my-service
REDPANDA_CLIENT_ID=my-service-instance-1
```

## Pub/Sub Pattern

### Publishing

```rust
use composable_rust_core::event_bus::EventBus;

event_bus.publish(
    "order-events",
    serialize(&OrderPlacedEvent { order_id: "123" }),
).await?;
```

### Subscribing

```rust
let mut rx = event_bus.subscribe("order-events").await?;

tokio::spawn(async move {
    while let Ok(event) = rx.recv().await {
        let order_event = deserialize::<OrderPlacedEvent>(&event)?;
        process_order(order_event).await?;
    }
});
```

## Deployment

### Docker Compose (Development)

```yaml
services:
  redpanda:
    image: redpandadata/redpanda:latest
    ports:
      - "9092:9092"
      - "9644:9644"  # Admin API
    command:
      - redpanda
      - start
      - --smp 1
      - --overprovisioned
      - --kafka-addr internal://0.0.0.0:9092,external://0.0.0.0:19092
      - --advertise-kafka-addr internal://redpanda:9092,external://localhost:19092
```

### Production

See [Redpanda Setup Guide](../docs/redpanda-setup.md) for production configuration.

## Further Reading

- [Redpanda Setup Guide](../docs/redpanda-setup.md) - Complete setup instructions
- [Event Bus Guide](../docs/event-bus.md) - Cross-aggregate communication
- [Saga Patterns](../docs/saga-patterns.md) - Multi-aggregate coordination

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
