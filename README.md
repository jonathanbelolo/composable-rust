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

## Current Status: Phase 0 - Foundation & Tooling

✅ Project structure established
✅ Cargo workspace configured (Rust 2024 edition)
✅ Core dependencies added
✅ Development tooling configured (rustfmt, clippy)
✅ CI/CD pipeline created
✅ Initial code scaffolding complete
🚧 Documentation in progress

**Next**: Phase 1 - Core Abstractions (full Reducer, Effect, Environment implementation)

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

> **Note**: The framework is in early development (Phase 0). Full functionality will be available in Phase 1.

```toml
[dependencies]
composable-rust-core = { path = "core" }
composable-rust-runtime = { path = "runtime" }

[dev-dependencies]
composable-rust-testing = { path = "testing" }
```

## Documentation

- **[Architecture Specification](specs/architecture.md)**: Comprehensive architectural design
- **[Implementation Roadmap](plans/implementation-roadmap.md)**: Development plan and timeline
- **[Phase 0 TODO](plans/phase-0/TODO.md)**: Current phase checklist

Additional documentation coming in Phase 1:
- Getting Started Guide
- Core Concepts
- API Reference
- Examples and Patterns

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

### Phase 0: Foundation & Tooling ✅ (Current)
- Project structure and workspace setup
- Development tooling configuration
- CI/CD pipeline

### Phase 1: Core Abstractions (Next)
- Complete Reducer trait implementation
- Effect system with all variants
- Environment traits (Database, Clock, EventPublisher, etc.)
- Basic Store runtime

### Phase 2: CQRS/Event Sourcing
- PostgreSQL event store
- Event replay and state reconstruction
- Snapshot support

### Phase 3: Composition & Coordination
- Reducer composition utilities
- Saga pattern implementation
- Redpanda event bus integration
- Multi-aggregate workflows

### Phase 4: Production Hardening
- Performance optimization
- Comprehensive error handling
- Observability (tracing, metrics)
- Battle-tested production implementations

### Phase 5: Developer Experience
- Macros and code generation
- Testing utilities
- Example applications
- Comprehensive documentation

## Acknowledgments

Inspired by:
- [Swift Composable Architecture (TCA)](https://github.com/pointfreeco/swift-composable-architecture)
- Redux and unidirectional data flow patterns
- CQRS and Event Sourcing architectural patterns

Built for developers who need correctness, performance, and maintainability in production backend systems.
