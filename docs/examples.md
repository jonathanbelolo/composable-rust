# Examples

**Complete, working examples demonstrating Composable Rust patterns.**

## Overview

The `examples/` directory contains production-quality examples showcasing different aspects of the framework. Each example is fully functional and includes tests.

---

## Phase 1: Core Abstractions

### ✅ Counter (`examples/counter/`)

**Status**: Complete

Basic counter demonstrating core concepts without I/O dependencies.

**Features**:
- Pure reducer logic (Increment, Decrement, Reset)
- `FixedClock` for deterministic testing
- `Store` with effect execution
- Unit and integration tests

**Key Concepts**:
- Reducer trait implementation
- Effect::None for pure state machines
- Environment with Clock dependency injection

**Run**:
```bash
cargo run --example counter
```

**See**: [Counter README](../examples/counter/README.md)

---

## Phase 2: Event Sourcing & Persistence

### ✅ Order Processing (`examples/order-processing/`)

**Status**: Complete (**now with HTTP/WebSocket support**)

Event-sourced order management system with PostgreSQL event store, HTTP API, and WebSocket real-time updates.

**Features**:
- Event sourcing with PostgreSQL
- CQRS pattern (commands vs events)
- HTTP REST API (`http` feature)
- WebSocket real-time updates (`http` feature)
- State reconstruction from events
- Optimistic concurrency with version tracking
- Integration tests

**Key Concepts**:
- `EventStore` trait with PostgresEventStore
- Event serialization with bincode
- `send_and_wait_for()` request-response pattern
- WebSocket action broadcasting

**Run**:
```bash
# With in-memory store
cargo run --bin order-processing

# With PostgreSQL
cargo run --bin order-processing --features postgres

# With HTTP API and WebSocket
cargo run --bin order-processing --features http
```

**See**: [Order Processing README](../examples/order-processing/README.md)

---

### ✅ Order Projection (`examples/order-projection/`)

**Status**: Complete

Read model projection from order events.

**Features**:
- Projection trait implementation
- Idempotent event handling with timestamps
- Denormalized views for queries
- Eventual consistency patterns

**Key Concepts**:
- Projections for read models
- Handling out-of-order events
- Query optimization

**Run**:
```bash
cargo run --example order-projection
```

---

### ✅ Todo Application (`examples/todo/`)

**Status**: Complete

Simple todo list with event sourcing.

**Features**:
- CRUD operations
- Event sourcing basics
- Simple domain model

**Run**:
```bash
cargo run --example todo
```

---

## Phase 3: Multi-Aggregate Coordination

### ✅ Checkout Saga (`examples/checkout-saga/`)

**Status**: Complete

Distributed saga across Order, Payment, and Inventory aggregates.

**Features**:
- Saga pattern with compensation
- Cross-aggregate coordination via EventBus
- Redpanda/Kafka integration
- Compensation on failure
- InMemoryEventBus for testing

**Key Concepts**:
- EventBus trait (publish/subscribe)
- Saga state machine
- Compensation logic (rollback)
- At-least-once delivery

**Run**:
```bash
# With in-memory event bus
cargo run --example checkout-saga

# With Redpanda
docker compose up -d redpanda
cargo run --example checkout-saga --features redpanda
```

**See**: [`docs/sagas.md`](./sagas.md) and [`docs/saga-patterns.md`](./saga-patterns.md)

---

## Phase 4: Production Hardening

### ✅ Metrics Demo (`examples/metrics-demo/`)

**Status**: Complete

Observability demo with tracing, metrics, and OpenTelemetry.

**Features**:
- Prometheus metrics
- Distributed tracing with OpenTelemetry
- Structured logging
- Performance benchmarking

**Key Concepts**:
- Store metrics collection
- Effect execution tracing
- Prometheus endpoint

**Run**:
```bash
cargo run --example metrics-demo

# View metrics
curl http://localhost:9090/metrics
```

**See**: [`docs/observability.md`](./observability.md)

---

### ✅ Banking Example (`examples/banking/`)

**Status**: Complete

Production-grade banking system demonstrating advanced patterns.

**Features**:
- Account management
- Transaction processing
- Audit logging
- Retry policies and circuit breakers

**Run**:
```bash
cargo run --example banking
```

---

## Phase 5: Developer Experience

### ✅ Ticketing System (`examples/ticketing/`)

**Status**: Complete

Full-featured ticketing system with HTTP API, WebSocket updates, and authentication.

**Features**:
- Complete CRUD operations
- HTTP REST API
- WebSocket real-time updates
- Authentication (magic link, OAuth, passkeys)
- Email notifications
- Role-based access control

**Key Concepts**:
- Composable Rust Web integration
- Real-time event broadcasting
- Authentication framework
- Email providers (SMTP, Console)

**Run**:
```bash
# Start with all features
cargo run --bin ticketing --features http,auth,email

# Access API
curl http://localhost:3000/health
```

**See**: [Ticketing README](../examples/ticketing/README.md)

---

## Running All Examples

### Prerequisites

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Start infrastructure (if needed)
docker compose up -d postgres redpanda
```

### Run Individual Examples

```bash
# Counter (no dependencies)
cargo run --example counter

# Order Processing (with HTTP)
cargo run --bin order-processing --features http

# Checkout Saga (with Redpanda)
cargo run --example checkout-saga --features redpanda

# Ticketing (with all features)
cargo run --bin ticketing --features http,auth,email
```

### Run All Tests

```bash
# Unit tests only
cargo test

# Integration tests (requires PostgreSQL + Redpanda)
docker compose up -d
cargo test --all-features
```

---

## Example Comparison

| Example | Phase | Event Store | Event Bus | HTTP/WebSocket | Auth | Complexity |
|---------|-------|-------------|-----------|----------------|------|------------|
| Counter | 1 | ❌ | ❌ | ❌ | ❌ | ⭐ Simple |
| Todo | 2 | ✅ | ❌ | ❌ | ❌ | ⭐⭐ Basic |
| Order Processing | 2 | ✅ | ❌ | ✅ | ❌ | ⭐⭐⭐ Moderate |
| Order Projection | 2 | ✅ | ❌ | ❌ | ❌ | ⭐⭐ Basic |
| Checkout Saga | 3 | ✅ | ✅ | ❌ | ❌ | ⭐⭐⭐⭐ Advanced |
| Banking | 4 | ✅ | ✅ | ❌ | ❌ | ⭐⭐⭐⭐ Advanced |
| Metrics Demo | 4 | ✅ | ❌ | ✅ | ❌ | ⭐⭐⭐ Moderate |
| Ticketing | 5 | ✅ | ✅ | ✅ | ✅ | ⭐⭐⭐⭐⭐ Production |

---

## Learning Path

### 1. Start with Counter (5 minutes)

Learn the basics: State, Action, Reducer, Effect, Store.

```bash
cd examples/counter
cargo run
```

### 2. Add Event Sourcing (15 minutes)

Learn event sourcing and PostgreSQL integration.

```bash
cd examples/order-processing
docker compose up -d postgres
cargo run --features postgres
```

### 3. Multi-Aggregate Coordination (30 minutes)

Learn sagas and event bus communication.

```bash
cd examples/checkout-saga
docker compose up -d redpanda
cargo run --features redpanda
```

### 4. HTTP/WebSocket (20 minutes)

Learn HTTP APIs and real-time updates.

```bash
cd examples/order-processing
cargo run --features http

# In another terminal
curl -X POST http://localhost:3000/api/v1/orders \
  -H "Content-Type: application/json" \
  -d '{"customer_id": "cust-1", "items": [...]}'
```

### 5. Production System (60 minutes)

Study the complete ticketing system with all features.

```bash
cd examples/ticketing
cargo run --features http,auth,email
```

---

## Contributing Examples

Want to contribute an example? Great! Here's what we're looking for:

### Desired Examples (Future)

- **E-Commerce Platform** - Shopping cart, checkout, inventory management
- **Chat Application** - Real-time messaging with WebSocket
- **Blog Platform** - Posts, comments, authentication
- **Workflow Engine** - Long-running processes with compensation
- **API Gateway** - Request routing and aggregation

### Requirements for New Examples

1. **Complete**: Fully working, not a skeleton
2. **Tested**: Unit and integration tests included
3. **Documented**: README.md with explanation
4. **Clean**: Follow framework patterns and conventions
5. **Realistic**: Solves real-world problems

**See**: [Contributing Guide](../CONTRIBUTING.md)

---

## See Also

- **Tutorial**: [Getting Started](./getting-started.md) - Step-by-step guide
- **Concepts**: [Core Concepts](./concepts.md) - Architecture deep dive
- **API Reference**: [API Documentation](./api-reference.md) - Complete API
- **Patterns**: [Pattern Cookbook](./cookbook.md) - Common recipes
- **Architecture**: [Architecture Spec](../specs/architecture.md) - Complete design

---

## License

All examples are licensed under MIT OR Apache-2.0.
