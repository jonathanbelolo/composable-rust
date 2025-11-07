# Examples

Complete, working examples demonstrating Composable Rust patterns.

## Available Examples

### ✅ Order Processing System

**Location**: `examples/order-processing/`

**Features Demonstrated**:
- Event sourcing with PostgreSQL event store
- CQRS pattern (commands vs events)
- `#[derive(Action)]` and `#[derive(State)]` macros
- Event store operations with `append_events!` macro
- Version tracking for optimistic concurrency
- State reconstruction from events
- ReducerTest for testing

**Key Files**:
- `src/types.rs` - Action and State definitions with derive macros
- `src/reducer.rs` - Business logic with event sourcing
- `tests/integration_tests.rs` - Full integration tests

**Run the example**:
```bash
# Start PostgreSQL (via Docker)
docker run -d --name postgres -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:15

# Run migrations
cargo run --example order-processing --bin setup-db

# Run the example
cargo run --example order-processing
```

---

## Upcoming Examples

### Counter (Simple)

**Status**: Planned for Phase 5

Basic counter demonstrating core concepts without event sourcing.

### Multi-Aggregate Saga

**Status**: Planned for Phase 3

Distributed workflow across multiple aggregates using saga pattern.

### Production Deployment

**Status**: Planned for Phase 4

Complete production setup with observability, metrics, and monitoring.

---

## See Also

- **Tutorial**: [Getting Started](getting-started.md) - Step-by-step guide
- **Concepts**: [Core Concepts](concepts.md) - Architecture deep dive
- **Architecture**: [Architecture Specification](../specs/architecture.md) - Complete design
- **Order Processing Code**: [`examples/order-processing/`](../examples/order-processing/)
