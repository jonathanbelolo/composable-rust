# Redpanda Setup Guide

## Table of Contents
- [Overview](#overview)
- [Why Redpanda?](#why-redpanda)
- [Local Development Setup](#local-development-setup)
- [Topic Management](#topic-management)
- [Consumer Group Configuration](#consumer-group-configuration)
- [Monitoring and Debugging](#monitoring-and-debugging)
- [Production Deployment](#production-deployment)
- [Kafka Compatibility](#kafka-compatibility)
- [Troubleshooting](#troubleshooting)

## Overview

Redpanda is a **Kafka-compatible streaming platform** designed for simplicity and performance. Composable Rust uses Redpanda as the default event bus for cross-aggregate communication.

### Key Benefits

- **Kafka-compatible**: Uses standard Kafka protocol (works with rdkafka, Kafka tools)
- **Simpler operations**: No JVM, no Zookeeper, easier to deploy
- **Self-hostable**: Docker, Kubernetes, bare metal deployment
- **Vendor swappable**: Can use Redpanda, Apache Kafka, AWS MSK, Azure Event Hubs
- **BSL 1.1 license**: Permits internal use, becomes Apache 2.0 after 4 years

## Why Redpanda?

### vs. Apache Kafka

| Feature | Redpanda | Apache Kafka |
|---------|----------|-------------|
| **Language** | C++ | Java (JVM) |
| **Dependencies** | None | Zookeeper (< 3.0) |
| **Memory usage** | Lower | Higher (JVM heap) |
| **Startup time** | Seconds | Minutes |
| **Ops complexity** | Simpler | More complex |
| **API** | Kafka-compatible | Kafka protocol |

**Use Redpanda when**: You want Kafka semantics with simpler operations.

**Use Kafka when**: You need a specific Kafka feature not yet in Redpanda.

### vs. EventStoreDB / Kurrent

| Feature | Redpanda | EventStoreDB |
|---------|----------|-------------|
| **Protocol** | Kafka (standard) | Proprietary |
| **Ecosystem** | Massive | Smaller |
| **Vendor lock-in** | None | High |
| **Migration** | Easy | Difficult |

**Decision**: Redpanda provides vendor independence and industry-standard protocol.

## Local Development Setup

### Option 1: Docker (Recommended)

#### Single Node

```bash
# Start Redpanda
docker run -d \
  --name redpanda \
  -p 9092:9092 \
  -p 9644:9644 \
  docker.redpanda.com/redpandadata/redpanda:latest \
  redpanda start \
  --overprovisioned \
  --smp 1 \
  --memory 1G \
  --reserve-memory 0M \
  --node-id 0 \
  --check=false \
  --kafka-addr PLAINTEXT://0.0.0.0:9092 \
  --advertise-kafka-addr PLAINTEXT://127.0.0.1:9092
```

**Ports**:
- `9092`: Kafka API (your application connects here)
- `9644`: Admin API (for management)

#### Docker Compose

Create `docker-compose.yml`:

```yaml
version: '3.9'

services:
  redpanda:
    image: docker.redpanda.com/redpandadata/redpanda:latest
    container_name: redpanda
    ports:
      - "9092:9092"
      - "9644:9644"
    command:
      - redpanda
      - start
      - --overprovisioned
      - --smp 1
      - --memory 1G
      - --reserve-memory 0M
      - --node-id 0
      - --check=false
      - --kafka-addr PLAINTEXT://0.0.0.0:9092
      - --advertise-kafka-addr PLAINTEXT://localhost:9092
    volumes:
      - redpanda-data:/var/lib/redpanda/data

volumes:
  redpanda-data:
```

Start with:

```bash
docker-compose up -d
```

### Option 2: Binary Installation

#### macOS (Homebrew)

```bash
brew install redpanda-data/tap/redpanda
```

#### Linux

```bash
curl -1sLf 'https://dl.redpanda.com/nzc4ZYQK3WRGd9sy/redpanda/cfg/setup/bash.deb.sh' | \
  sudo -E bash

sudo apt-get install redpanda
```

Start Redpanda:

```bash
sudo rpk redpanda start
```

### Verify Installation

```bash
# Install rpk (Redpanda CLI)
# On macOS: brew install redpanda-data/tap/redpanda
# On Linux: included with redpanda package

# Check cluster status
rpk cluster info --brokers localhost:9092

# Should show:
# BROKERS
# =======
# ID    HOST       PORT
# 0     127.0.0.1  9092
```

### Connect from Rust

```rust
use composable_rust_redpanda::RedpandaEventBus;

// Connect to local Redpanda
let event_bus = RedpandaEventBus::new("localhost:9092")?;
```

## Topic Management

### Creating Topics

#### Using rpk

```bash
# Create topic
rpk topic create order-events \
  --brokers localhost:9092 \
  --partitions 3 \
  --replicas 1

# Create multiple topics
rpk topic create \
  order-events \
  payment-events \
  inventory-events \
  --brokers localhost:9092 \
  --partitions 3
```

#### Auto-Create (Development)

Enable auto-creation for development:

```bash
rpk cluster config set auto_create_topics_enabled true
```

**Not recommended for production** (explicit topic creation preferred).

### Listing Topics

```bash
# List all topics
rpk topic list --brokers localhost:9092

# Topic details
rpk topic describe order-events --brokers localhost:9092
```

### Topic Configuration

#### Retention

```bash
# Set retention to 7 days
rpk topic alter-config order-events \
  --set retention.ms=604800000 \
  --brokers localhost:9092

# Set retention to 100GB
rpk topic alter-config order-events \
  --set retention.bytes=107374182400 \
  --brokers localhost:9092
```

#### Partitions

```bash
# Increase partitions (can't decrease!)
rpk topic add-partitions order-events \
  --num 6 \
  --brokers localhost:9092
```

### Topic Naming Best Practices

Use: `{aggregate-type}-events`

Examples:
- `order-events`
- `payment-events`
- `inventory-events`
- `customer-events`

**Rationale**:
- Clear separation by aggregate type
- Easy to subscribe to specific aggregates
- Scales independently

## Consumer Group Configuration

### Understanding Consumer Groups

Consumer groups enable **load balancing** across multiple consumers:

```
Topic: order-events (3 partitions)

Group: order-processor
┌──────────┐
│ Consumer │──── P0 ────┐
└──────────┘            │
┌──────────┐            ├─── Load balanced
│ Consumer │──── P1 ────┤
└──────────┘            │
┌──────────┐            │
│ Consumer │──── P2 ────┘
└──────────┘
```

### Listing Consumer Groups

```bash
# List all groups
rpk group list --brokers localhost:9092

# Group details
rpk group describe payment-saga-coordinator \
  --brokers localhost:9092
```

### Viewing Lag

```bash
# Check consumer lag
rpk group describe payment-saga-coordinator \
  --brokers localhost:9092

# Output shows:
# PARTITION  CURRENT-OFFSET  LOG-END-OFFSET  LAG
# 0          100             100             0
# 1          150             152             2  ← 2 messages behind
# 2          200             200             0
```

### Resetting Offsets

```bash
# Reset to earliest (reprocess all events)
rpk group seek payment-saga-coordinator \
  --to start \
  --topics order-events \
  --brokers localhost:9092

# Reset to specific offset
rpk group seek payment-saga-coordinator \
  --to 100 \
  --topics order-events \
  --brokers localhost:9092
```

### Configuring in Code

```rust
// Explicit consumer group (recommended)
let event_bus = RedpandaEventBus::builder()
    .brokers("localhost:9092")
    .consumer_group("payment-saga-coordinator")
    .auto_offset_reset("earliest")  // or "latest"
    .build()?;

// Auto-generated consumer group
let event_bus = RedpandaEventBus::new("localhost:9092")?;
// Group: composable-rust-{sorted-topics}
```

## Monitoring and Debugging

### Monitoring Tools

#### 1. rpk

```bash
# Cluster health
rpk cluster health --brokers localhost:9092

# Topic stats
rpk topic describe order-events \
  --brokers localhost:9092

# Consumer lag
rpk group describe payment-saga \
  --brokers localhost:9092
```

#### 2. Redpanda Console

Web UI for Redpanda:

```yaml
# docker-compose.yml
services:
  console:
    image: docker.redpanda.com/redpandadata/console:latest
    ports:
      - "8080:8080"
    environment:
      - KAFKA_BROKERS=redpanda:9092
    depends_on:
      - redpanda
```

Access at: http://localhost:8080

Features:
- Topic browsing
- Message inspection
- Consumer group monitoring
- Schema registry (optional)

### Debugging Techniques

#### 1. Tail Events

```bash
# Consume from beginning
rpk topic consume order-events \
  --brokers localhost:9092 \
  --offset start

# Consume latest
rpk topic consume order-events \
  --brokers localhost:9092 \
  --offset end
```

#### 2. Inspect Messages

```bash
# Show message keys and values
rpk topic consume order-events \
  --format '%k %v\n' \
  --brokers localhost:9092
```

#### 3. Check Partition Distribution

```bash
rpk topic describe order-events \
  --brokers localhost:9092

# Shows:
# PARTITION  LEADER  REPLICAS  IN-SYNC-REPLICAS
# 0          0       [0]       [0]
# 1          0       [0]       [0]
# 2          0       [0]       [0]
```

### Application Logging

Enable debug logging in your application:

```rust
use tracing_subscriber;

tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .init();
```

Look for:
- `RedpandaEventBus created successfully`
- `Event published successfully` (with topic, partition, offset)
- `Subscribed to topics` (with consumer group)
- `Received event` (with event type)

## Production Deployment

### Deployment Options

#### 1. Self-Hosted (Docker/K8s)

**Docker Swarm**:
```bash
docker service create \
  --name redpanda \
  --replicas 3 \
  --publish 9092:9092 \
  docker.redpanda.com/redpandadata/redpanda:latest \
  redpanda start \
  --smp 2 \
  --memory 4G
```

**Kubernetes (Helm)**:
```bash
helm repo add redpanda https://charts.redpanda.com
helm install redpanda redpanda/redpanda \
  --namespace redpanda \
  --create-namespace \
  --set replicas=3
```

#### 2. Redpanda Cloud

Managed service: https://redpanda.com/redpanda-cloud

Connect with:
```rust
let event_bus = RedpandaEventBus::builder()
    .brokers("my-cluster.cloud.redpanda.com:9092")
    // Add SASL/TLS config if needed
    .build()?;
```

#### 3. Apache Kafka

Swap to Kafka (Kafka-compatible!):

```rust
// Code doesn't change!
let event_bus = RedpandaEventBus::new("my-kafka-cluster:9092")?;
```

Works with:
- Apache Kafka
- AWS MSK (Managed Streaming for Kafka)
- Azure Event Hubs (Kafka protocol)
- Confluent Cloud

### Production Configuration

```rust
let event_bus = RedpandaEventBus::builder()
    .brokers("broker1:9092,broker2:9092,broker3:9092")
    .producer_acks("all")  // Wait for all replicas (durability)
    .compression("lz4")    // Compress messages
    .timeout(Duration::from_secs(30))
    .consumer_group("payment-saga-prod")
    .auto_offset_reset("latest")  // Only new events
    .buffer_size(10000)   // Larger buffer for high throughput
    .build()?;
```

### High Availability

**Multi-broker setup**:
```bash
# Create topic with replication
rpk topic create order-events \
  --partitions 6 \
  --replicas 3 \  # Survive 2 broker failures
  --brokers broker1:9092
```

**Producer config**:
```rust
.producer_acks("all")  // All replicas acknowledge
```

**Consumer config**:
```rust
.consumer_group("unique-group-name")  // Explicit group name
.auto_offset_reset("latest")  // Don't reprocess on restart
```

### Monitoring in Production

1. **Metrics**: Redpanda exposes Prometheus metrics at `:9644/metrics`
2. **Alerts**: Set up alerts for:
   - Consumer lag > threshold
   - Broker down
   - Disk usage > 80%
3. **Logs**: Ship Redpanda logs to centralized logging
4. **Tracing**: Enable distributed tracing in your application

## Kafka Compatibility

Redpanda implements the **Kafka protocol**, so Kafka clients work out-of-box.

### Compatible Features

✅ **Producer API**: Synchronous and asynchronous produce
✅ **Consumer API**: Consumer groups, manual offset commits
✅ **Admin API**: Topic/partition management
✅ **Kafka Streams**: Works with Kafka Streams library
✅ **Kafka Connect**: Compatible with Kafka Connect

### Incompatible Features

❌ **Kafka Transactions**: Not yet supported (coming soon)
❌ **Exactly-once semantics**: Use at-least-once + idempotency
❌ **KRaft mode**: Redpanda uses Raft (not KRaft)

### Migration from Kafka

**No code changes needed**:
```rust
// Before (Kafka)
let event_bus = RedpandaEventBus::new("kafka:9092")?;

// After (Redpanda)
let event_bus = RedpandaEventBus::new("redpanda:9092")?;
```

**Data migration**:
1. Set up Redpanda cluster
2. Use MirrorMaker 2 to replicate topics
3. Switch consumers to Redpanda
4. Switch producers to Redpanda
5. Decommission Kafka

## Troubleshooting

### Connection Issues

**Problem**: `ConnectionFailed: Failed to create producer`

**Solutions**:
```bash
# 1. Check Redpanda is running
docker ps | grep redpanda

# 2. Test connection
rpk cluster info --brokers localhost:9092

# 3. Check firewall
telnet localhost 9092

# 4. Check Docker network
docker network inspect bridge
```

### Performance Issues

**Problem**: High latency, slow throughput

**Solutions**:
```rust
// 1. Enable compression
.compression("lz4")

// 2. Batch messages (rdkafka does this automatically)

// 3. Increase buffer size
.buffer_size(10000)

// 4. Use more partitions
rpk topic add-partitions order-events --num 12
```

### Consumer Lag

**Problem**: Consumers can't keep up with producers

**Solutions**:
```bash
# 1. Check lag
rpk group describe my-group --brokers localhost:9092

# 2. Add more consumers (horizontal scaling)
# Deploy more instances with same consumer group

# 3. Optimize processing
# - Make event handlers faster
# - Process events in parallel where possible

# 4. Increase partitions (allows more parallel consumers)
rpk topic add-partitions order-events --num 12
```

### Disk Space

**Problem**: Redpanda disk full

**Solutions**:
```bash
# 1. Check disk usage
docker exec redpanda df -h

# 2. Set retention policy
rpk topic alter-config order-events \
  --set retention.ms=86400000 \  # 1 day
  --brokers localhost:9092

# 3. Compact topics (if using log compaction)
rpk topic alter-config order-events \
  --set cleanup.policy=compact
```

## Next Steps

- Read [Event Bus Guide](event-bus.md) for usage patterns
- Read [Saga Pattern Guide](sagas.md) for workflow coordination
- Study `examples/checkout-saga/` for complete example
- Review `redpanda/tests/integration_tests.rs` for testcontainers setup
- Explore Redpanda docs: https://docs.redpanda.com
