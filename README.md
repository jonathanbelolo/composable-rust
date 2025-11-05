# Composable Rust

A functional architecture framework for building event-driven backend systems in Rust.

## Vision

**Composable Rust** brings the principles of functional architecture—popularized by frameworks like Swift's Composable Architecture (TCA)—to the Rust backend ecosystem. By combining Rust's unparalleled type safety and zero-cost abstractions with functional programming patterns and CQRS/Event Sourcing principles, we create a framework for building **battle-tested, industrial-grade business process management systems**.

## Core Principles

1. **Correctness First**: Leverage Rust's type system to make invalid states unrepresentable
2. **Fearless Refactoring**: Changes ripple through the type system, making large-scale refactoring safe
3. **Lightning-Fast Tests**: Business logic tests run at memory speed with zero I/O
4. **Production Performance**: Static dispatch and zero-cost abstractions ensure no runtime overhead
5. **Self-Documenting**: The type system and structure serve as living documentation
6. **Composability**: Complex systems emerge from the composition of simple, isolated components

## Architecture Overview

The framework is built on five fundamental types:

- **State**: Domain state for a feature
- **Action**: All possible inputs (commands, events, cross-aggregate events)
- **Reducer**: Pure function `(State, Action, Environment) → (State, Effects)`
- **Effect**: Side effect descriptions (not execution)
- **Environment**: Injected dependencies via traits

```rust
// Define your state
#[derive(Clone, Debug)]
struct OrderState {
    orders: HashMap<OrderId, Order>,
}

// Define your actions
#[derive(Clone, Debug)]
enum OrderAction {
    PlaceOrder { customer_id: CustomerId, items: Vec<LineItem> },
    OrderPlaced { order_id: OrderId, timestamp: DateTime<Utc> },
}

// Implement the reducer
impl Reducer for OrderReducer {
    type State = OrderState;
    type Action = OrderAction;
    type Environment = OrderEnvironment;

    fn reduce(
        &self,
        state: &mut OrderState,
        action: OrderAction,
        env: &OrderEnvironment,
    ) -> Vec<Effect<OrderAction>> {
        match action {
            OrderAction::PlaceOrder { customer_id, items } => {
                // Business logic here
                vec![Effect::Database(SaveOrder), Effect::PublishEvent(OrderPlaced)]
            }
            _ => vec![Effect::None],
        }
    }
}
```

## Current Status: Phase 1 - Core Abstractions ✅ COMPLETE

✅ **Phase 0**: Foundation & Tooling
✅ **Phase 1**: Core Abstractions
  - Reducer trait (pure functions for business logic)
  - Effect system (5 variants: None, Future, Delay, Parallel, Sequential)
  - Store runtime (state management + effect execution)
  - Environment traits (Clock with production/test implementations)
  - TestStore (deterministic effect testing)
  - Counter example (validates entire architecture)
  - 47 comprehensive tests (all passing)
  - 3,486 lines of documentation

**Next**: Phase 2 - Event Sourcing & Persistence (PostgreSQL event store)

## Project Structure

```
composable-rust/
├── core/           # Core traits and types
├── runtime/        # Store and effect execution
├── testing/        # Test utilities and mocks
├── examples/       # Reference implementations
├── docs/           # Documentation and guides
├── specs/          # Architecture specification
└── plans/          # Implementation roadmap
```

## Crates

- **`composable-rust-core`**: Core traits (Reducer, Effect, Environment)
- **`composable-rust-runtime`**: Store runtime and effect execution
- **`composable-rust-testing`**: Testing utilities and mock implementations

## Quick Start

> **Note**: Phase 1 is complete! Core abstractions are ready. See the Counter example for a working reference implementation.

```toml
[dependencies]
composable-rust-core = { path = "core" }
composable-rust-runtime = { path = "runtime" }

[dev-dependencies]
composable-rust-testing = { path = "testing" }
```

### Run the Counter Example

```bash
# Run the example
cargo run -p counter

# Run tests
cargo test -p counter

# See the architecture reference
cat examples/counter/README.md
```

## Documentation

### Phase 1 Documentation (Complete)

- **[Getting Started Guide](docs/getting-started.md)**: Tutorial walkthrough with Counter example
- **[Core Concepts](docs/concepts.md)**: Deep dive into the five fundamental types
- **[API Reference](docs/api-reference.md)**: Complete API documentation
- **[Error Handling](docs/error-handling.md)**: Three-tier error model
- **[Implementation Decisions](docs/implementation-decisions.md)**: Architectural choices and trade-offs
- **[Counter Example](examples/counter/README.md)**: Architecture reference using Counter

### Architecture & Planning

- **[Architecture Specification](specs/architecture.md)**: Comprehensive architectural design (2,800+ lines)
- **[Implementation Roadmap](plans/implementation-roadmap.md)**: Development plan and timeline
- **[Phase 1 Review](plans/phase-1/PHASE1_REVIEW.md)**: Completion assessment and readiness for Phase 2
- **[Phase 1 TODO](plans/phase-1/TODO.md)**: Phase 1 checklist (complete)

## Development

### Prerequisites

- Rust 1.85.0 or later (required for edition 2024)
- Cargo

### Building

```bash
cargo build --all-features
```

### Testing

```bash
cargo test --all-features
```

### Linting

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

### Formatting

```bash
cargo fmt --all --check
```

### Documentation

```bash
cargo doc --no-deps --all-features --open
```

### Quality Checks

Run all checks locally:

```bash
./scripts/check.sh
```

## Contributing

This project is in active development. Contribution guidelines will be published in Phase 1.

For now, see:
- [Architecture Specification](specs/architecture.md) for design principles
- [Implementation Roadmap](plans/implementation-roadmap.md) for development plan

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Roadmap

### Phase 0: Foundation & Tooling ✅ COMPLETE
- Project structure and workspace setup
- Development tooling configuration
- CI/CD pipeline

### Phase 1: Core Abstractions ✅ COMPLETE
- ✅ Reducer trait implementation
- ✅ Effect system with 5 variants (None, Future, Delay, Parallel, Sequential)
- ✅ Environment traits (Clock for Phase 1)
- ✅ Store runtime with effect execution
- ✅ TestStore for deterministic testing
- ✅ Counter example validating architecture
- ✅ 47 comprehensive tests (all passing)
- ✅ 3,486 lines of documentation

### Phase 2: Event Sourcing & Persistence (Next)
- PostgreSQL event store
- Event replay and state reconstruction
- Snapshot support
- Database traits and implementations

### Phase 3: Composition & Coordination
- Reducer composition utilities
- Saga pattern implementation
- Redpanda event bus integration
- Multi-aggregate workflows
- EventPublisher trait

### Phase 4: Production Hardening
- Performance optimization
- Comprehensive error handling
- Observability (tracing, metrics)
- Battle-tested production implementations

### Phase 5: Developer Experience
- Macros and code generation
- Additional testing utilities
- More example applications
- Enhanced documentation

## Acknowledgments

Inspired by:
- [Swift Composable Architecture (TCA)](https://github.com/pointfreeco/swift-composable-architecture)
- Redux and unidirectional data flow patterns
- CQRS and Event Sourcing architectural patterns

Built for developers who need correctness, performance, and maintainability in production backend systems.
