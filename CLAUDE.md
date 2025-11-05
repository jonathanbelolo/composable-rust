# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

---

## ⚡ **Critical: Modern Rust Expert Skill**

**Before writing any code, consult:** `.claude/skills/modern-rust-expert.md`

This skill is the **authoritative source** for all Rust coding standards in this project. It embeds:
- Rust Edition 2024 features and patterns
- Clippy compliance patterns (pedantic + strict denies)
- Documentation standards
- Modern async patterns (async fn in traits, RPITIT)
- Functional-but-pragmatic philosophy
- Common gotchas and fixes

**Everything in this file defers to the Modern Rust Expert skill for coding standards.**

---

## Project Overview

**Composable Rust** is a functional architecture framework for building event-driven backend systems in Rust, inspired by Swift's Composable Architecture (TCA). The framework combines Rust's type safety with functional programming patterns, CQRS, and Event Sourcing to create battle-tested, industrial-grade business process management systems.

**Current Status**: Phase 0 complete. Phase 1 (Core Abstractions) is next.

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
├── testing/           # Test utilities and mock implementations (FixedClock, etc.)
├── examples/          # Reference implementations (Phase 5)
├── docs/              # Documentation (placeholders until Phase 1)
├── specs/             # Architecture specification and design docs
├── plans/             # Implementation roadmap and phase TODOs
└── .claude/skills/    # Claude Code skills (modern-rust-expert.md)
```

### Crate Dependencies

- **core**: No dependencies on runtime (pure traits and types)
- **runtime**: Depends on core (implements Store and effect execution)
- **testing**: Depends on core + runtime (provides mocks for testing)

### Critical Files

- `specs/architecture.md`: Complete architectural design (2800+ lines)
- `plans/implementation-roadmap.md`: Phase-by-phase development plan
- `.claude/skills/modern-rust-expert.md`: Rust 2024 + clippy patterns

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

### Phase 0 (Complete)
- Workspace structure established
- Development tooling configured
- CI/CD pipeline in place
- Initial scaffolding with placeholders

### Phase 1 (Next)
Focus: Implement core abstractions
- Complete Reducer trait
- Full Effect enum (Database, Http, PublishEvent, Delay, Future, etc.)
- Environment traits (Database, Clock, EventPublisher, HttpClient, IdGenerator)
- Basic Store with effect execution

**Key files to modify**:
- `core/src/lib.rs`: Effect enum, Environment traits
- `runtime/src/lib.rs`: Store implementation, effect executor
- `testing/src/lib.rs`: Mock implementations

### Future Phases
- Phase 2: PostgreSQL event store (bincode serialization)
- Phase 3: Redpanda event bus, Saga pattern
- Phase 4: Production hardening, observability
- Phase 5: Developer experience, macros, examples

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

**Start here and work down:**

1. **Rust/Clippy errors?** → `.claude/skills/modern-rust-expert.md` (patterns, fixes, examples)
2. **Architecture questions?** → `specs/architecture.md` (2800+ lines, comprehensive)
3. **Phase-specific guidance?** → `plans/implementation-roadmap.md` (what to build when)
4. **Catching issues early?** → `./scripts/check.sh` (runs all quality checks)

### Common Issues Quick Reference

| Issue | Solution Location |
|-------|-------------------|
| Clippy warning/error | Modern Rust Expert skill → search for the lint name |
| Async trait patterns | Modern Rust Expert skill → "Async Functions in Traits" |
| Documentation format | Modern Rust Expert skill → "Documentation Standards" |
| Architectural pattern | `specs/architecture.md` → search for pattern (saga, effect, etc.) |
| Phase requirements | `plans/implementation-roadmap.md` → find current phase |
| Build/test failure | Run `./scripts/check.sh` → see which check fails |

## Architecture Deep Dive

For detailed understanding:
- **State machine patterns**: See saga examples in `specs/architecture.md` section 6.3
- **Effect system**: Section 5 of architecture spec
- **CQRS/Event Sourcing**: Section 4 of architecture spec
- **Dependency injection**: Section 3 of architecture spec

The architecture document is comprehensive (2800+ lines) and should be referenced for any non-trivial implementation decisions.

---

## 🎯 Before Starting Any Work

**Checklist for every coding session:**

1. ✅ **Read**: `.claude/skills/modern-rust-expert.md` (if writing Rust code)
2. ✅ **Review**: Current phase in `plans/implementation-roadmap.md`
3. ✅ **Check**: `specs/architecture.md` for architectural patterns
4. ✅ **Run**: `./scripts/check.sh` after changes
5. ✅ **Document**: Update CLAUDE.md if adding new patterns

**The Modern Rust Expert skill is your first and most important reference for all Rust code.**

---

## Summary

This project follows a **documentation-driven, type-safe, functional architecture**. The three key documents are:

1. **`.claude/skills/modern-rust-expert.md`** → How to write Rust code correctly
2. **`specs/architecture.md`** → What to build and why (architecture)
3. **`plans/implementation-roadmap.md`** → When to build it (phases)

Everything else supports these three core references.
