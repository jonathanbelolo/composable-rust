# Observability Guide

This guide covers observability features in Composable Rust: tracing, metrics, and monitoring.

## Table of Contents

1. [Overview](#overview)
2. [Tracing](#tracing)
3. [Metrics](#metrics)
4. [Production Setup](#production-setup)
5. [Troubleshooting](#troubleshooting)

---

## Overview

Composable Rust provides comprehensive observability through:

- **Tracing**: Structured logging with span context propagation
- **Metrics**: Performance and health metrics exportable to Prometheus
- **Health Checks**: System health status reporting

### Observability Philosophy

- **Instrumentation by Default**: All critical paths are instrumented
- **Zero-Cost When Disabled**: Tracing/metrics have minimal overhead when subscribers aren't configured
- **Production-Ready**: Designed for high-throughput systems (1000+ commands/sec)

---

## Tracing

### Setup

Add `tracing-subscriber` to your dependencies:

```toml
[dependencies]
composable-rust-runtime = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
```

### Basic Configuration

```rust
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn main() {
    // Initialize tracing subscriber
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,composable_rust=debug".into())
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Your application code...
}
```

### Environment Variables

Control tracing output with the `RUST_LOG` environment variable:

```bash
# Show all debug logs from composable_rust crates
RUST_LOG=composable_rust=debug cargo run

# Show trace-level logs for the runtime
RUST_LOG=composable_rust_runtime=trace cargo run

# Multiple filters
RUST_LOG=info,composable_rust=debug,composable_rust_runtime=trace cargo run
```

### Instrumented Operations

The framework automatically instruments:

#### Store Operations

- `store_send`: Action processing (includes action metadata)
- `store_send_internal`: Internal send implementation
- `execute_effect`: Effect execution (all variants)

```rust
// Trace output example:
// DEBUG store_send{}: Processing action
// TRACE execute_effect{}: Executing Effect::Future
// TRACE execute_effect{}: Effect::Future produced an action
```

#### Event Store Operations

- `append_events`: Event appending with stream_id, version, and event count
- `load_events`: Event loading with stream_id and version range
- `save_snapshot`: Snapshot persistence
- `load_snapshot`: Snapshot retrieval

```rust
// Trace output example:
// DEBUG append_events: stream_id="order-123" expected_version=Some(5) event_count=3
// DEBUG load_events: stream_id="order-123" from_version=0 to_version=None
```

### Span Context Propagation

Spans automatically nest to show the full execution path:

```
INFO  store_send: Processing command
  DEBUG execute_effect: Executing Effect::Sequential
    TRACE execute_effect: Executing sequential effect 1 of 3
      DEBUG append_events: Appending 2 events to stream
    TRACE execute_effect: Executing sequential effect 2 of 3
      DEBUG publish_event: Publishing event to topic
```

### Custom Tracing in Reducers

Add tracing to your reducers for domain-specific insights:

```rust
use tracing::instrument;

#[derive(Clone)]
struct OrderReducer;

impl Reducer for OrderReducer {
    type State = OrderState;
    type Action = OrderAction;
    type Environment = OrderEnv;

    #[instrument(skip(self, state, env), fields(order_id = %state.id))]
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            OrderAction::PlaceOrder { customer_id, items } => {
                tracing::info!(
                    customer_id = %customer_id,
                    item_count = items.len(),
                    "Placing order"
                );

                // Business logic...
                smallvec![/* effects */]
            }
            // ...
        }
    }
}
```

### Structured Fields

Add structured data to spans and events:

```rust
tracing::info!(
    order_id = %order.id,
    customer_id = %customer.id,
    total_amount = order.total,
    "Order completed successfully"
);
```

---

## Metrics

### Setup

Metrics are collected using the `metrics` crate and can be exported to Prometheus:

```toml
[dependencies]
metrics = "0.23"
metrics-exporter-prometheus = "0.15"
```

### Configuration

```rust
use metrics_exporter_prometheus::PrometheusBuilder;

#[tokio::main]
async fn main() {
    // Install Prometheus exporter
    let builder = PrometheusBuilder::new();
    builder
        .with_http_listener(([0, 0, 0, 0], 9000))
        .install()
        .expect("Failed to install Prometheus exporter");

    println!("Metrics available at http://localhost:9000/metrics");

    // Your application code...
}
```

### Available Metrics

#### Store Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `store.commands.total` | Counter | Total commands processed |
| `store.effects.executed` | Counter | Effects executed by type (`type` label) |
| `store.shutdown.initiated` | Counter | Shutdown operations started |
| `store.shutdown.completed` | Counter | Successful shutdowns |
| `store.shutdown.timeout` | Counter | Shutdown timeouts |
| `store.shutdown.rejected_actions` | Counter | Actions rejected during shutdown |

#### Event Store Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `event_store.append.event_count` | Histogram | Events per append operation |
| `event_store.append.duration` | Histogram | Append operation duration (seconds) |
| `event_store.append.success` | Counter | Successful appends |
| `event_store.append.error` | Counter | Failed appends |
| `event_store.load.duration` | Histogram | Load operation duration (seconds) |
| `event_store.concurrency_conflict` | Counter | Optimistic concurrency conflicts |

#### Circuit Breaker Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `circuit_breaker.state_change` | Counter | State transitions (`from` and `to` labels) |
| `circuit_breaker.call.success` | Counter | Successful calls |
| `circuit_breaker.call.failure` | Counter | Failed calls |
| `circuit_breaker.call.rejected` | Counter | Calls rejected (circuit open) |

#### Dead Letter Queue Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `dlq.push` | Counter | Items added to DLQ |
| `dlq.size` | Gauge | Current DLQ size |
| `dlq.capacity` | Gauge | DLQ capacity |

### Querying Metrics

#### Prometheus Queries

```promql
# Command rate (commands per second)
rate(store_commands_total[1m])

# Effect execution by type
sum(rate(store_effects_executed[5m])) by (type)

# P95 event append latency
histogram_quantile(0.95, rate(event_store_append_duration_bucket[5m]))

# Circuit breaker state changes
rate(circuit_breaker_state_change[5m])

# DLQ usage percentage
(dlq_size / dlq_capacity) * 100
```

### Grafana Dashboard

Example Grafana dashboard panels:

```json
{
  "title": "Command Throughput",
  "targets": [{
    "expr": "rate(store_commands_total[1m])"
  }]
},
{
  "title": "Effect Latency (p50, p95, p99)",
  "targets": [
    {
      "expr": "histogram_quantile(0.50, rate(event_store_append_duration_bucket[5m]))",
      "legendFormat": "p50"
    },
    {
      "expr": "histogram_quantile(0.95, rate(event_store_append_duration_bucket[5m]))",
      "legendFormat": "p95"
    },
    {
      "expr": "histogram_quantile(0.99, rate(event_store_append_duration_bucket[5m]))",
      "legendFormat": "p99"
    }
  ]
}
```

---

## Production Setup

### Complete Example

```rust
use composable_rust_runtime::{Store, StoreConfig};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,composable_rust=debug".into())
        )
        .with(tracing_subscriber::fmt::layer().json())  // JSON for prod
        .init();

    // 2. Initialize metrics exporter
    PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], 9000))
        .install()
        .expect("Failed to install Prometheus exporter");

    tracing::info!("Metrics available at http://0.0.0.0:9000/metrics");

    // 3. Create store with configuration
    let config = StoreConfig::default()
        .with_dlq_max_size(5000)
        .with_shutdown_timeout(Duration::from_secs(30));

    let store = Store::with_config(
        MyState::default(),
        MyReducer,
        production_environment(),
        config,
    );

    // 4. Check health
    let health = store.health();
    tracing::info!(?health, "Store initialized");

    // 5. Run your application...
    run_app(store).await?;

    // 6. Graceful shutdown
    tracing::info!("Shutting down...");
    store.shutdown(Duration::from_secs(30)).await?;

    Ok(())
}
```

### Docker Compose Setup

```yaml
version: '3.8'

services:
  app:
    build: .
    ports:
      - "8080:8080"
      - "9000:9000"  # Metrics endpoint
    environment:
      - RUST_LOG=info,composable_rust=debug
      - DATABASE_URL=postgresql://user:pass@postgres:5432/db

  prometheus:
    image: prom/prometheus:latest
    ports:
      - "9090:9090"
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'

  grafana:
    image: grafana/grafana:latest
    ports:
      - "3000:3000"
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=admin
```

### Prometheus Configuration

```yaml
# prometheus.yml
global:
  scrape_interval: 15s

scrape_configs:
  - job_name: 'composable-rust'
    static_configs:
      - targets: ['app:9000']
```

### Log Aggregation

For production, use structured JSON logging with a log aggregator:

```rust
// JSON output for Elasticsearch, CloudWatch, etc.
tracing_subscriber::fmt()
    .json()
    .with_current_span(true)
    .with_span_list(true)
    .init();
```

---

## Troubleshooting

### High Latency

**Check**: Effect execution metrics

```bash
# Query Prometheus
histogram_quantile(0.95, rate(event_store_append_duration_bucket[5m]))
```

**Look for**:
- Slow database queries
- High circuit breaker rejection rates
- DLQ filling up

### Missing Traces

**Check**: Environment variable

```bash
echo $RUST_LOG
# Should output: info,composable_rust=debug (or similar)
```

**Verify**: Subscriber is initialized

```rust
// Must be called before any tracing occurs
tracing_subscriber::fmt::init();
```

### Memory Growth

**Check**: DLQ size

```rust
let health = store.health();
println!("{:?}", health);
```

**Monitor**: `dlq_size` metric in Prometheus

### Circuit Breaker Opening

**Check**: Failure rate

```promql
# Failure rate over 5 minutes
rate(circuit_breaker_call_failure[5m]) / rate(circuit_breaker_call_success[5m] + circuit_breaker_call_failure[5m])
```

**Investigate**:
- Database connection issues
- Event bus unavailability
- External service timeouts

### Performance Tuning

**Reduce Overhead**:

```rust
// Use EnvFilter to limit tracing scope
tracing_subscriber::EnvFilter::new("warn,composable_rust=info")
```

**Sampling** (for high-throughput systems):

```rust
// Sample 1 in 100 requests
use tracing_subscriber::layer::SubscriberExt;

tracing_subscriber::registry()
    .with(tracing_subscriber::fmt::layer().with_filter(
        tracing_subscriber::filter::dynamic_filter_fn(|metadata, _ctx| {
            rand::random::<u8>() == 0  // ~0.4% sample rate
        })
    ))
    .init();
```

---

## Best Practices

1. **Use Structured Logging**: Always add context fields to spans
   ```rust
   tracing::info!(user_id = %user.id, action = "login", "User logged in");
   ```

2. **Avoid PII in Logs**: Don't log sensitive data
   ```rust
   // ❌ Bad
   tracing::info!(email = %user.email, "User created");

   // ✅ Good
   tracing::info!(user_id = %user.id, "User created");
   ```

3. **Use Appropriate Log Levels**:
   - `error`: Critical failures requiring immediate attention
   - `warn`: Issues that should be investigated
   - `info`: Important business events
   - `debug`: Detailed diagnostic information
   - `trace`: Very verbose, only for debugging

4. **Monitor Key Metrics**:
   - Command throughput
   - Effect latency (p95, p99)
   - Error rates
   - DLQ size
   - Circuit breaker state

5. **Set Up Alerts**:
   - DLQ > 80% capacity
   - Error rate > 5%
   - Circuit breaker opens
   - Effect latency p99 > threshold

---

## Next Steps

- [Health Checks](./health-checks.md) - System health monitoring
- [Error Handling](./error-handling.md) - Retry policies and circuit breakers
- [Production Deployment](./deployment.md) - Deployment best practices

