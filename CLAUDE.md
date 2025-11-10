# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

---

## ⚡ **Critical: Claude Code Skills**

This repository includes **7 expert skills** that provide comprehensive guidance for all aspects of development:

### Core Skills (Use for Every Task)

1. **`modern-rust-expert`** (`.claude/skills/modern-rust-expert/SKILL.md`)
   - **Use for**: All Rust code writing, clippy fixes, async patterns
   - Rust Edition 2024 features and patterns
   - Clippy compliance (pedantic + strict denies)
   - Documentation standards, async fn in traits
   - 1200+ lines of patterns and examples

2. **`composable-rust-architecture`** (`.claude/skills/composable-rust-architecture/SKILL.md`)
   - **Use for**: Implementing reducers, effects, state machines
   - Core architectural patterns (State, Action, Reducer, Effect, Environment)
   - Developer experience macros (#[derive(State)], #[derive(Action)], append_events!)
   - 850+ lines covering 40-60% code reduction techniques

### Domain-Specific Skills

3. **`composable-rust-event-sourcing`** (`.claude/skills/composable-rust-event-sourcing/SKILL.md`)
   - **Use for**: Event-sourced aggregates, EventStore, event design
   - Fat vs thin events with data inclusion checklist
   - Event versioning and schema evolution
   - 650+ lines with comprehensive patterns

4. **`composable-rust-sagas`** (`.claude/skills/composable-rust-sagas/SKILL.md`)
   - **Use for**: Multi-aggregate coordination, compensation logic
   - Saga state machines, nested sagas (parent-child coordination)
   - EventBus integration, timeout handling
   - 1000+ lines covering simple to complex workflows

5. **`composable-rust-web`** (`.claude/skills/composable-rust-web/SKILL.md`)
   - **Use for**: HTTP APIs, WebSocket, authentication
   - Axum patterns, WebSocket JSON protocol
   - Authentication (magic links, OAuth, passkeys)
   - 600+ lines with production-ready patterns

6. **`composable-rust-testing`** (`.claude/skills/composable-rust-testing/SKILL.md`)
   - **Use for**: Unit tests, integration tests, test utilities
   - ReducerTest builder (Given-When-Then API)
   - TestStore, FixedClock, mocks, property-based testing
   - 550+ lines of testing patterns

7. **`composable-rust-production`** (`.claude/skills/composable-rust-production/SKILL.md`)
   - **Use for**: Production operations, migrations, monitoring
   - Database migrations (sqlx), connection pooling, backup/restore
   - Performance tuning, disaster recovery, troubleshooting
   - 400+ lines of operational excellence

**Total**: ~5,250 lines covering the complete development lifecycle.

**These skills are automatically activated based on context.** You don't need to invoke them manually.

---

## Project Overview

**Composable Rust** is a functional architecture framework for building event-driven backend systems in Rust, inspired by Swift's Composable Architecture (TCA). The framework combines Rust's type safety with functional programming patterns, CQRS, and Event Sourcing to create battle-tested, industrial-grade business process management systems.

**Current Status**: Phase 4 complete. Phase 5 (Developer Experience) is next.

**Phase 4 Achievement**: Production-ready framework with full observability (tracing, metrics, OpenTelemetry), advanced error handling (retries, circuit breakers, DLQ), performance optimization (SmallVec, batch operations), and database migrations. 156 library tests + 15 integration tests passing. See `plans/phase-4/TODO.md` for comprehensive completion assessment.

## Core Architecture

The framework is built on **five fundamental types** that compose together:

1. **State**: Domain state for a feature (Clone-able, owned data)
2. **Action**: Unified type for all inputs (commands, events, cross-aggregate events)
3. **Reducer**: Pure function `(State, Action, Environment) → (State, Effects)`
4. **Effect**: Side effect descriptions (values, not execution)
5. **Environment**: Injected dependencies via traits

### Key Architectural Principles

- **Functional Core, Imperative Shell**: Pure business logic, effects as values
- **Unidirectional Data Flow**: `Action → Reducer → (State, Effects) → Effect Execution → More Actions`
- **Explicit Effects**: All side effects are visible, testable, and composable
- **Dependency Injection**: Traits with static dispatch for zero-cost abstractions
- **Functional-but-Pragmatic**: Prefer functional patterns, but be practical (e.g., `&mut self` in reducers for performance)

### The Feedback Loop

Actions flow through the system in a cycle:
1. External command arrives as an Action
2. Reducer processes: `(State, Action, Env) → (New State, Effects)`
3. Store executes effects (database, HTTP, events)
4. Effects can produce new Actions (e.g., `Effect::Future` returns `Option<Action>`)
5. New Actions feed back into step 1

This creates a self-sustaining event loop where everything is an Action.

## Codebase Structure

### Workspace Organization

```
composable-rust/
├── core/              # Core traits: Reducer, Effect, Environment (no I/O)
├── runtime/           # Store implementation and effect execution
├── testing/           # Test utilities and mock implementations
├── postgres/          # PostgreSQL event store
├── redpanda/          # Redpanda/Kafka event bus
├── web/               # HTTP and WebSocket framework (Axum)
├── auth/              # Authentication framework
├── examples/          # Reference implementations (10+ examples)
├── docs/              # Documentation (21 comprehensive guides)
├── specs/             # Architecture specification (2,800+ lines)
├── plans/             # Implementation roadmap and phase TODOs
└── .claude/skills/    # 7 Claude Code expert skills (5,250+ lines)
   ├── modern-rust-expert/
   ├── composable-rust-architecture/
   ├── composable-rust-event-sourcing/
   ├── composable-rust-sagas/
   ├── composable-rust-web/
   ├── composable-rust-testing/
   └── composable-rust-production/
```

### Crate Dependencies

- **core**: No dependencies on runtime (pure traits and types)
- **runtime**: Depends on core (implements Store and effect execution)
- **testing**: Depends on core + runtime (provides mocks for testing)

### Critical Files

- **`.claude/skills/`**: 7 expert skills (5,250+ lines total)
  - Start here for any development task
  - Skills are automatically activated by context
- **`specs/architecture.md`**: Complete architectural design (2,800+ lines)
- **`plans/implementation-roadmap.md`**: Phase-by-phase development plan
- **`docs/`**: 21 comprehensive documentation files
  - `concepts.md`: Core architecture deep dive
  - `consistency-patterns.md`: Critical for multi-aggregate patterns
  - `production-database.md`: Operations guide (800+ lines)

## Development Commands

### Essential Commands

```bash
# Build everything
cargo build --all-features

# Run all tests
cargo test --all-features

# Run specific test
cargo test --test test_name

# Run tests in specific crate
cargo test -p composable-rust-core

# Lint with strict warnings
cargo clippy --all-targets --all-features -- -D warnings

# Format code
cargo fmt --all

# Build documentation
cargo doc --no-deps --all-features --open

# Run ALL quality checks (recommended before commits)
./scripts/check.sh
```

### Quality Check Script

`./scripts/check.sh` runs:
1. Format check (`cargo fmt --all --check`)
2. Clippy (`cargo clippy --all-targets --all-features -- -D warnings`)
3. Build (`cargo build --all-features`)
4. Tests (`cargo test --all-features`)
5. Documentation (`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`)

**Always run this before committing.**

## Code Standards

> **📚 Authoritative Reference**: `.claude/skills/modern-rust-expert.md`
>
> This section is a quick summary. For comprehensive guidance, patterns, and examples, consult the Modern Rust Expert skill.

### Quick Reference

| Standard | Value | Details in Skill |
|----------|-------|------------------|
| **Edition** | 2024 | Modern features: async fn in traits, RPITIT, let-else |
| **MSRV** | 1.85.0 | Minimum required for Edition 2024 |
| **Lints** | Pedantic + strict denies | unwrap/panic/todo/expect all denied |
| **Async** | Native async fn in traits | No `async-trait` crate, no `BoxFuture` |
| **Imports** | Explicit only | No wildcard imports in library code |
| **Docs** | All public APIs | Type names in backticks, # Panics/Errors sections |

### Critical Lint Rules (from workspace Cargo.toml)

```toml
[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }  # Must have lower priority
unwrap_used = "deny"     # Use Result/Option properly
expect_used = "deny"     # Only in tests with #[allow] + # Panics doc
panic = "deny"           # No panics in library code
todo = "deny"            # No TODOs in production code
unimplemented = "deny"   # Use proper error handling
```

### Modern Rust Patterns (Edition 2024)

**Use these patterns** (detailed in Modern Rust Expert skill):

```rust
// ✅ Async fn in traits (no BoxFuture!)
trait Database: Send + Sync {
    async fn save(&self, data: &[u8]) -> Result<()>;
}

// ✅ RPITIT (Return Position Impl Trait In Traits)
trait Clock: Send + Sync {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

// ✅ Let-else for early returns
fn process(input: Option<Value>) -> Result<()> {
    let Some(value) = input else {
        return Err(Error::Missing);
    };
    // Continue with value...
}

// ✅ Const fn where possible
pub const fn new(value: T) -> Self {
    Self { value }
}
```

**Full details, more patterns, and examples**: See `.claude/skills/modern-rust-expert.md`

## Architecture-Specific Patterns

### Writing Reducers

```rust
impl Reducer for MyReducer {
    type State = MyState;
    type Action = MyAction;
    type Environment = MyEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,  // ✅ Mutable for performance
        action: Self::Action,
        env: &Self::Environment,
    ) -> Vec<Effect<Self::Action>> {
        match action {
            MyAction::Command { .. } => {
                // 1. Validate
                // 2. Update state
                // 3. Return effects
                vec![Effect::Database(...), Effect::PublishEvent(...)]
            }
            MyAction::Event { .. } => {
                // Events are often idempotent
                vec![Effect::None]
            }
        }
    }
}
```

### Effects as Values

**Never execute side effects in reducers**:
```rust
// ❌ BAD - executing side effect
fn reduce(...) {
    env.database.save(state).await;  // NO!
}

// ✅ GOOD - returning effect description
fn reduce(...) -> Vec<Effect> {
    vec![Effect::Database(SaveState)]  // YES!
}
```

### Dependency Injection Pattern

Three implementations for every dependency:
1. **Production**: Real implementation (PostgresDatabase, SystemClock)
2. **Test**: Fast, deterministic mocks (MockDatabase, FixedClock)
3. **Development**: Instrumented versions (LoggingDatabase)

Use traits with static dispatch:
```rust
struct MyEnvironment<D, C>
where
    D: Database,
    C: Clock,
{
    database: D,
    clock: C,
}
```

## Testing Philosophy

- **Business logic tests run at memory speed** (no I/O in reducers)
- Use `FixedClock` from `testing` crate for deterministic time
- Property-based testing with `proptest` for invariants
- Integration tests use real dependencies (testcontainers)

Example:
```rust
#[tokio::test]
async fn test_reducer() {
    let env = MyEnvironment {
        clock: FixedClock::new(test_time()),
        database: MockDatabase::new(),
    };

    let mut state = MyState::default();
    let effects = reducer.reduce(&mut state, action, &env);

    assert_eq!(state.field, expected);
    assert!(matches!(effects[0], Effect::Database(_)));
}
```

## Phase-Specific Context

### Phase 0 (Complete) ✅
- Workspace structure established
- Development tooling configured
- CI/CD pipeline in place
- Initial scaffolding with placeholders

### Phase 1 (Complete) ✅
**Achievement**: Core abstractions validated
- ✅ Reducer trait implemented and tested
- ✅ Effect enum with 5 variants (None, Future, Delay, Parallel, Sequential)
- ✅ Environment traits (Clock with SystemClock, FixedClock)
- ✅ Store with complete effect execution
- ✅ TestStore for deterministic testing
- ✅ Counter example validating entire architecture
- ✅ 47 comprehensive tests (all passing)
- ✅ 3,486 lines of documentation

**Key files completed**:
- `core/src/lib.rs`: Reducer trait, Effect enum with composition methods
- `runtime/src/lib.rs`: Store implementation, effect executor, error handling
- `testing/src/lib.rs`: TestStore, FixedClock with advance()
- `examples/counter/`: Complete reference implementation
- `docs/`: 6 comprehensive documentation files

### Phase 2 (Complete) ✅
**Achievement**: Event Sourcing & Persistence
- ✅ PostgreSQL event store with append/load operations
- ✅ Event replay for state reconstruction
- ✅ InMemoryEventStore for fast testing
- ✅ Database trait (EventStore) with PostgreSQL implementation
- ✅ bincode serialization for events
- ✅ Order Processing example with full event sourcing
- ✅ 9 integration tests (all passing)

**Key files completed**:
- `postgres/src/lib.rs`: PostgreSQL event store implementation
- `core/src/event.rs`: SerializedEvent, EventStore trait
- `testing/src/lib.rs`: InMemoryEventStore
- `examples/order-processing/`: Complete event-sourced aggregate

### Phase 3 (Complete) ✅
**Achievement**: Multi-aggregate coordination via event bus
- ✅ EventBus trait abstraction (publish/subscribe)
- ✅ InMemoryEventBus for testing (synchronous, deterministic)
- ✅ RedpandaEventBus with Kafka-compatible integration
- ✅ At-least-once delivery semantics (manual offset commits)
- ✅ Reducer composition utilities (combine_reducers, scope_reducer)
- ✅ Checkout Saga example with compensation (Order + Payment + Inventory)
- ✅ Testcontainers integration tests (6 tests)
- ✅ Comprehensive documentation (sagas.md, event-bus.md, redpanda-setup.md)
- ✅ 87 workspace tests passing, 0 clippy warnings

**Key files completed**:
- `core/src/event_bus.rs`: EventBus trait, Effect::PublishEvent
- `testing/src/lib.rs`: InMemoryEventBus
- `redpanda/src/lib.rs`: RedpandaEventBus with builder pattern
- `core/src/composition.rs`: Reducer composition utilities
- `examples/checkout-saga/`: Complete saga with compensation flows
- `docs/sagas.md`, `docs/event-bus.md`, `docs/redpanda-setup.md`

### Phase 4 (Complete) ✅
**Achievement**: Production-ready framework with comprehensive hardening
- ✅ Observability (tracing, metrics, OpenTelemetry support)
- ✅ Advanced error handling (retry policies, circuit breakers, DLQ)
- ✅ Performance optimization (SmallVec for effects, batch operations)
- ✅ Database migrations with sqlx::migrate!()
- ✅ Connection pooling and production database setup
- ✅ Comprehensive documentation (production-database.md)
- ✅ 156 library tests + 15 integration tests passing
- ✅ Code quality audit: All clippy errors fixed, dead code removed

**Key files completed**:
- `runtime/src/lib.rs`: Tracing, metrics, retry policies, circuit breakers
- `postgres/src/lib.rs`: Migration runner, batch operations, connection pooling
- `core/src/event_store.rs`: append_batch() for efficient bulk operations
- `docs/production-database.md`: Complete production operations guide (800+ lines)
- `runtime/benches/`: Performance benchmarks validating optimization

### Phase 5 (Next)
Focus: Developer Experience
- Macros and code generation
- Additional testing utilities
- More example applications
- Enhanced documentation

## Strategic Decisions (Locked In)

From `plans/implementation-roadmap.md`:

- **Serialization**: bincode (5-10x faster than JSON, 30-70% smaller)
- **Event Store**: PostgreSQL with custom schema (vendor independence)
- **Event Bus**: Redpanda (Kafka-compatible, self-hostable)
- **Philosophy**: Vendor independence over convenience, ownership over speed
- **Performance**: Static dispatch, zero-cost abstractions

## Common Patterns

### Saga as Reducer (Phase 3)
Sagas are just reducers with state machines. No special framework:
```rust
struct SagaState {
    current_step: Step,
    completed_steps: Vec<Step>,
    // IDs for compensation
}

impl Reducer for SagaReducer {
    fn reduce(...) {
        match (state.current_step, action) {
            (Step1, Event1) => { /* transition */ },
            (Step2, Failed) => { self.compensate(state) },
            // ...
        }
    }
}
```

### Event Sourcing (Phase 2)
State is derived from events:
```rust
impl State {
    fn from_events(events: impl Iterator<Item = Event>) -> Self {
        events.fold(Self::default(), |mut state, event| {
            state.apply_event(event);
            state
        })
    }
}
```

## Important Constraints

1. **No `unwrap`/`panic`/`todo`/`expect`** in library code (clippy denies it)
   - Exception: Tests can use `expect` with `#[allow(clippy::expect_used)]` + `# Panics` doc
2. **No wildcard imports** (`use super::*`) in library code (tests/examples OK)
3. **All type names in docs need backticks** or clippy complains
4. **Workspace dependencies cannot be optional** (define in crate Cargo.toml instead)
5. **Manual Debug impl** needed for types containing `Future` (see `core/src/lib.rs` Effect enum)
6. **Edition 2024 requires MSRV 1.85.0+**
7. **Async patterns**:
   - ✅ Use `async fn` in trait definitions (not `BoxFuture` return types)
   - ✅ Use RPITIT: `fn foo() -> impl Future<Output = T> + Send`
   - ✅ `Pin<Box<dyn Future>>` **is OK** in enum variants (sized types needed)
   - ❌ Don't use `BoxFuture` as trait return type (outdated pattern)
8. **Lint priority**: `pedantic` must have `priority = -1` (lower than individual lints)

## When Stuck

### Debugging & Reference Hierarchy

**The 7 skills are your first stop.** They're automatically activated by context, but you can also reference them directly:

**By Task Type:**

| Task | Primary Skill | Backup Resources |
|------|---------------|------------------|
| Writing Rust code | `modern-rust-expert` | - |
| Implementing reducers/effects | `composable-rust-architecture` | `docs/concepts.md` |
| Event-sourced aggregates | `composable-rust-event-sourcing` | `docs/event-design-guidelines.md` |
| Multi-aggregate workflows | `composable-rust-sagas` | `docs/saga-patterns.md` |
| HTTP/WebSocket APIs | `composable-rust-web` | `docs/websocket.md` |
| Writing tests | `composable-rust-testing` | - |
| Production operations | `composable-rust-production` | `docs/production-database.md` |

**By Problem Type:**

| Problem | Solution Location |
|---------|-------------------|
| Clippy warning/error | `modern-rust-expert` skill → search lint name |
| Async trait patterns | `modern-rust-expert` skill → "Async Functions in Traits" |
| Documentation format | `modern-rust-expert` skill → "Documentation Standards" |
| Reducer patterns | `composable-rust-architecture` skill → "Reducer Pattern" |
| Event schema design | `composable-rust-event-sourcing` skill → "Data Inclusion Checklist" |
| Saga compensation | `composable-rust-sagas` skill → "Compensation Pattern" |
| WebSocket protocol | `composable-rust-web` skill → "Message Protocol" |
| Database migrations | `composable-rust-production` skill → "Database Migrations" |
| Build/test failure | Run `./scripts/check.sh` → see which check fails |

**If skills don't answer your question:**

1. Check `specs/architecture.md` (2,800+ lines, comprehensive design)
2. Check `docs/` directory (21 specialized guides)
3. Check `plans/implementation-roadmap.md` (phase-specific context)

## Architecture Deep Dive

For detailed understanding:
- **State machine patterns**: See saga examples in `specs/architecture.md` section 6.3
- **Effect system**: Section 5 of architecture spec
- **CQRS/Event Sourcing**: Section 4 of architecture spec
- **Dependency injection**: Section 3 of architecture spec

The architecture document is comprehensive (2800+ lines) and should be referenced for any non-trivial implementation decisions.

---

## 🎯 Before Starting Any Work

**The skills handle most of this automatically, but for manual reference:**

1. ✅ **Skills are active**: All 7 skills automatically activate based on context
2. ✅ **Rust code?** → `modern-rust-expert` skill (Rust 2024, clippy compliance)
3. ✅ **Architecture?** → `composable-rust-architecture` skill (reducers, effects, macros)
4. ✅ **Domain logic?** → Relevant skill (event-sourcing, sagas, web, testing, production)
5. ✅ **Quality checks**: Run `./scripts/check.sh` after changes

**The 7 skills contain 5,250+ lines of expert knowledge covering everything you need.**

---

## Summary

This project follows a **documentation-driven, type-safe, functional architecture** supported by comprehensive automation:

### Knowledge Layers (In Order of Priority)

1. **`.claude/skills/`** (7 skills, 5,250+ lines)
   - **Primary resource for all development tasks**
   - Automatically activated by context
   - Covers: Rust, architecture, event sourcing, sagas, web, testing, production

2. **`docs/`** (21 comprehensive guides)
   - Deep dives into specific topics
   - Referenced by skills when needed
   - Key: `concepts.md`, `consistency-patterns.md`, `production-database.md`

3. **`specs/architecture.md`** (2,800+ lines)
   - Complete architectural specification
   - For non-trivial design decisions
   - Referenced by skills and docs

4. **`plans/implementation-roadmap.md`**
   - Phase-by-phase development plan
   - Current status and next steps

**The skills are self-sufficient for 95% of tasks. Skills reference docs/specs when needed.**
