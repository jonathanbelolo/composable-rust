# Phase 1: Core Abstractions - TODO List

**Goal**: Implement the fundamental types and traits that everything else builds on.

**Duration**: 1.5-2 weeks

**Status**: 🚧 **IN PROGRESS**

**Philosophy**: Validate core abstractions with the simplest possible example. Defer advanced features to later phases.

---

## Prerequisites

Before starting Phase 1:
- [x] Phase 0 complete
- [x] All quality checks passing
- [x] Modern Rust Expert skill created
- [x] Repository set up and pushed to GitHub

---

## 1. Core Traits Enhancement (`composable-rust-core`)

### 1.1 Reducer Trait
- [x] Basic Reducer trait defined in `core/src/lib.rs`
- [ ] Add comprehensive examples to Reducer documentation
- [ ] Document trait bounds (`State`, `Action`, `Environment` requirements)
- [ ] Add usage patterns and best practices to module docs
- [ ] Document the `&mut State` performance choice

### 1.2 Effect Type (Core Variants Only)
**Scope**: None, Future, Delay, Parallel, Sequential

Current state: All 5 variants defined ✅

Tasks:
- [x] `Effect::None` - No-op effect
- [x] `Effect::Future` - Arbitrary async computation returning `Option<Action>`
- [x] `Effect::Delay` - Delayed action dispatch
- [x] `Effect::Parallel` - Concurrent effect execution
- [x] `Effect::Sequential` - Sequential effect execution
- [ ] Add comprehensive documentation for each variant with examples
- [ ] Document effect composition patterns
- [ ] Add usage guidelines (when to use which variant)

**Deferred to later phases**:
- ❌ Database effects (Phase 2)
- ❌ HTTP effects (Phase 4, if needed)
- ❌ Event publishing (Phase 3)
- ❌ Cancellable effects (Phase 3/4)
- ❌ Command dispatch (Phase 3)

### 1.3 Effect Composition Utilities
- [x] Basic `merge()` and `chain()` methods exist
- [ ] Implement `Effect::map()` (transform action type)
- [ ] Add comprehensive tests for effect composition
- [ ] Document composition patterns with examples
- [ ] Consider adding helper constructors (e.g., `Effect::delay_action()`)

### 1.4 Environment Traits (Minimal - Clock Only)
**Scope**: Only traits needed for Counter example

- [x] **Clock Trait** (already defined)
  - [x] `now()` method
  - [ ] Add comprehensive documentation with examples
  - [ ] Document production vs test implementations

**Deferred to later phases**:
- ❌ Database trait (Phase 2)
- ❌ EventPublisher trait (Phase 3)
- ❌ HttpClient trait (Phase 4, if needed)
- ❌ IdGenerator trait (Phase 2)

**Note**: Counter is a pure state machine with NO side effects. Clock is only needed for demonstration purposes.

---

## 2. Store Implementation (`composable-rust-runtime`)

### 2.1 Store Core
Current: Basic Store with `send()` and `state()` methods ✅

Enhancement tasks:
- [x] Basic Store structure defined
- [x] Generic over State, Action, Environment, Reducer
- [ ] Add comprehensive documentation with examples
- [ ] Document Store lifecycle and threading model
- [ ] Document concurrency guarantees and lock behavior
- [ ] Add usage examples to module docs

### 2.2 Effect Executor Implementation
**Scope**: "Basic effect executor (just Future support initially)" - per roadmap

**Minimal viable implementation**:
- [x] `Effect::None` - No-op (already handled)
- [ ] **`Effect::Future`** - Execute future, feed resulting action back to Store
  - [ ] Spawn future execution
  - [ ] Handle `Some(action)` - send action back to Store
  - [ ] Handle `None` - no-op
  - [ ] Error handling (log failures)
- [ ] **`Effect::Delay`** - Use `tokio::time::sleep()`, dispatch action after delay
- [ ] **`Effect::Parallel`** - Execute effects concurrently
  - [ ] Spawn all effects concurrently
  - [ ] Collect all resulting actions
  - [ ] Feed actions back to Store
- [ ] **`Effect::Sequential`** - Execute effects in order
  - [ ] Execute each effect sequentially
  - [ ] Wait for completion before next
  - [ ] Feed resulting actions back to Store in order

**Implementation notes**:
- Start with Future only, add others incrementally
- All effects should feed actions back through `self.send()`
- Use `tokio::spawn` for concurrency
- Add tracing/logging for debugging

### 2.3 Action Feedback Loop
- [x] Basic feedback loop exists (effects can produce actions)
- [ ] Document feedback loop architecture
- [ ] Add example showing action → effect → action cycle
- [ ] Consider adding queue to prevent stack overflow (if needed)
- [ ] Document potential issues (cycles, infinite loops)

### 2.4 Error Handling
- [ ] Define `StoreError` type using `thiserror`
- [ ] Decide: What happens if reducer panics?
  - [ ] Document decision and rationale
  - [ ] Implement chosen strategy
- [ ] Handle effect execution failures gracefully
- [ ] Add error logging via `tracing`
- [ ] Document error handling patterns

### 2.5 Concurrency Management
- [x] Basic `RwLock` for state
- [ ] Document locking strategy (when writes block reads)
- [ ] Add concurrency tests:
  - [ ] Concurrent `send()` calls
  - [ ] Read state during effect execution
  - [ ] Multiple effects producing concurrent actions
- [ ] Performance benchmark: action throughput

---

## 3. Testing Utilities (`composable-rust-testing`)

### 3.1 Mock Implementations (Minimal - Clock Only)
**Scope**: Only mocks needed for Counter example

- [x] **FixedClock** (already implemented)
  - [x] Basic implementation
  - [ ] Add `advance()` method to simulate time passage
  - [ ] Add comprehensive tests
  - [ ] Add documentation with examples

**Deferred to later phases**:
- ❌ MockDatabase (Phase 2)
- ❌ MockEventPublisher (Phase 3)
- ❌ MockHttpClient (Phase 4, if needed)
- ❌ SequentialIdGenerator (Phase 2)

### 3.2 Test Helpers (Basic)
- [ ] **State assertion helpers**:
  - [ ] Simple assertion macros for state verification
  - [ ] Examples in documentation
- [ ] **Effect assertion helpers**:
  - [ ] `assert_no_effects!()` helper
  - [ ] Pattern matching helpers for effect types
- [ ] **Test environment factory**:
  - [ ] `test_environment()` helper with FixedClock

**Deferred**:
- Action/State builders (add as needed)
- Complex fixture utilities (Phase 2+)

### 3.3 Property-Based Testing (Optional)
- [ ] Consider `Arbitrary` implementations for Counter types
- [ ] Add property test example if valuable
- [ ] Document property testing patterns

**Note**: May defer to Phase 2 if not immediately valuable for Counter.

---

## 4. Example: Counter Aggregate

**Goal**: Simplest possible example to validate abstractions. Pure state machine, NO side effects.

### 4.1 Counter Implementation
Location: `examples/counter/`

- [ ] **State**:
  ```rust
  #[derive(Clone, Debug, Default)]
  struct CounterState {
      count: i64,
  }
  ```

- [ ] **Actions**:
  ```rust
  #[derive(Clone, Debug)]
  enum CounterAction {
      Increment,
      Decrement,
      Reset,
  }
  ```

- [ ] **Reducer**:
  - [ ] Implement `Reducer` trait
  - [ ] Pure state mutations (no I/O)
  - [ ] Return `Effect::None` for all actions (pure state machine)
  - [ ] Full documentation with examples

- [ ] **Environment**:
  ```rust
  struct CounterEnvironment<C: Clock> {
      clock: C,  // For demonstration, not actually used
  }
  ```

- [ ] **Example binary** (`examples/counter/main.rs`):
  - [ ] Create Store with Counter reducer
  - [ ] Send several actions
  - [ ] Print state after each action
  - [ ] Demonstrate the complete flow

### 4.2 Counter Tests
Location: `examples/counter/tests/`

- [ ] **Unit tests** for reducer logic:
  - [ ] Test Increment action
  - [ ] Test Decrement action
  - [ ] Test Reset action
  - [ ] Test starting from different initial states
- [ ] **Property tests** (optional):
  - [ ] Increment then decrement = identity
  - [ ] Multiple increments = sum
- [ ] **Integration test** with Store:
  - [ ] Create Store
  - [ ] Send actions
  - [ ] Verify state via `store.state()`
- [ ] **Concurrency test**:
  - [ ] Multiple concurrent increments
  - [ ] Verify final count is correct
- [ ] **Benchmark**:
  - [ ] Reducer execution time (target: < 1μs)
  - [ ] Store throughput (target: > 100k actions/sec)

### 4.3 Counter Documentation
- [ ] Comprehensive README in `examples/counter/README.md`
- [ ] Explain the architecture using Counter as reference
- [ ] Document all concepts (State, Action, Reducer, Effect, Store)
- [ ] Add diagrams (optional but helpful)
- [ ] Link from main documentation

---

## 5. Documentation

### 5.1 API Documentation
- [ ] Complete all `///` doc comments with examples
- [ ] Add `# Examples` sections to all public APIs
- [ ] Add `# Panics` and `# Errors` sections where applicable
- [ ] Document all type parameters and bounds
- [ ] Add links between related types
- [ ] Verify `cargo doc --no-deps --all-features --open` looks good

### 5.2 Module Documentation
- [ ] Enhance `core/src/lib.rs` module docs:
  - [ ] Architecture overview
  - [ ] Usage patterns
  - [ ] Examples
  - [ ] Links to Counter example
- [ ] Enhance `runtime/src/lib.rs` module docs:
  - [ ] Store lifecycle
  - [ ] Effect execution model
  - [ ] Concurrency guarantees
  - [ ] Examples
- [ ] Enhance `testing/src/lib.rs` module docs:
  - [ ] Testing philosophy
  - [ ] Mock usage patterns
  - [ ] Examples

### 5.3 Guide Documentation
- [ ] Update `docs/getting-started.md`:
  - [ ] Add Counter example walkthrough
  - [ ] Explain core concepts
  - [ ] Show how to run the example
- [ ] Update `docs/concepts.md`:
  - [ ] Phase 1 concepts (Reducer, Effect, Store)
  - [ ] Architecture principles
  - [ ] Effect-as-value pattern
- [ ] Update `docs/api-reference.md`:
  - [ ] Document all new APIs
  - [ ] Organize by module
  - [ ] Add usage examples

### 5.4 Architecture Documentation
- [ ] Review `specs/architecture.md` for alignment
- [ ] Document any deviations from original plan
- [ ] Update if implementation differs from spec
- [ ] Add "Implementation Notes" section documenting Phase 1 decisions

---

## 6. Validation & Testing

### 6.1 Unit Tests
- [ ] Reducer trait tests (if testable patterns exist)
- [ ] Effect composition tests (`map`, etc.)
- [ ] Store tests:
  - [ ] `send()` basic functionality
  - [ ] `state()` accessor
  - [ ] Concurrent access
- [ ] FixedClock tests (advance time, etc.)

### 6.2 Integration Tests
- [ ] Full Store + Reducer + Effects integration
- [ ] Counter example end-to-end test
- [ ] Effect execution tests (all 5 effect types)
- [ ] Error handling integration tests
- [ ] Feedback loop test (effect produces action produces effect)

### 6.3 Performance Benchmarks
Location: `benches/phase1_benchmarks.rs`

- [ ] Reducer execution (target: < 1μs for Counter)
- [ ] Store `send()` throughput (target: > 100k actions/sec)
- [ ] Effect execution overhead (measure each effect type)
- [ ] Concurrent action processing (measure scalability)
- [ ] Document results in `docs/performance.md`

### 6.4 Quality Checks
- [ ] `cargo build --all-features` succeeds
- [ ] `cargo test --all-features` passes
  - [ ] All tests run in < 100ms (memory speed target)
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes
- [ ] `cargo fmt --all --check` passes
- [ ] `cargo doc --no-deps --all-features` builds successfully
- [ ] CI pipeline passes on GitHub

---

## 7. Key Implementation Decisions

Document decisions as they're made:

### 7.1 Effect Execution Model
- [ ] **Decision**: Store owns environment, effects access it via Store ✅ (current)
- [ ] **Rationale**: (Document why this is best)
- [ ] **Trade-offs**: (Document what we gave up)
- [ ] **Alternatives considered**: (List other options)

### 7.2 State Mutation Strategy
- [x] **Decision**: `&mut State` for performance (already chosen)
- [x] **Rationale**: Zero-copy, in-place updates
- [ ] **Guidelines**: Document when to use `&mut` vs return new state
- [ ] **Constraints**: Document requirements this places on State types

### 7.3 Action Requirements
- [ ] **Decision**: What bounds are required? (`Clone`? `Send`? `'static`?)
- [ ] **Rationale**: Why these bounds are necessary
- [ ] **Impact**: How this affects user code
- [ ] **Recommendation**: Best practices for Action types

### 7.4 Error Handling in Store
- [ ] **Decision**: What happens if reducer panics?
  - Option A: Propagate panic (fail fast)
  - Option B: Catch and log, continue processing
  - Option C: Poison lock and halt store
- [ ] **Decision**: What happens if effect execution fails?
  - Log and continue? Retry? Halt?
- [ ] **Rationale**: Document chosen strategy and why
- [ ] **Implementation**: Ensure code matches documented behavior

---

## 8. Phase 1 Scope Reminder

**IN SCOPE** (Phase 1):
- ✅ Effect enum: None, Future, Delay, Parallel, Sequential
- ✅ Effect executor: Just Future support initially (others added incrementally)
- ✅ Clock trait (for Counter environment)
- ✅ FixedClock (testing utility)
- ✅ Counter example (pure state machine)
- ✅ Core abstractions validation

**OUT OF SCOPE** (Later phases):
- ❌ Database effects/traits → Phase 2
- ❌ Event publishing → Phase 3
- ❌ HTTP effects → Phase 4 (if needed)
- ❌ Saga coordination → Phase 3
- ❌ Event sourcing → Phase 2
- ❌ Advanced effect patterns → Phase 3/4

**Remember**: "Make it work, make it right, make it fast—in that order."

---

## 9. Validation Checklist

Phase 1 is complete when (from roadmap):

- [ ] ✅ Can implement a simple reducer (Counter works)
- [ ] ✅ Can create and run a Store
- [ ] ✅ Effects execute and produce new actions
- [ ] ✅ Tests run in < 100ms (memory speed)
- [ ] ✅ Counter example works end-to-end
- [ ] ✅ All public APIs are documented

**Success Criteria**: "Can explain the entire architecture using just the counter example."

---

## 10. Transition to Phase 2

### 10.1 Phase 2 Preparation
- [ ] Review Phase 2 goals (Event Sourcing & Persistence)
- [ ] Identify dependencies needed (sqlx, bincode)
- [ ] Spike PostgreSQL schema design if needed
- [ ] Create `plans/phase-2/TODO.md`

### 10.2 Final Phase 1 Review
- [ ] All validation criteria met
- [ ] Counter example demonstrates architecture completely
- [ ] Performance targets met
- [ ] Documentation complete
- [ ] Ready to add persistence layer

---

## Success Criteria

Phase 1 is complete when:

- ✅ Reducer, Effect, Store abstractions work correctly
- ✅ Counter example demonstrates entire flow
- ✅ Can explain architecture using only Counter
- ✅ Tests run at memory speed (< 100ms)
- ✅ Performance targets met (> 100k actions/sec)
- ✅ All public APIs documented
- ✅ All quality checks pass

**Key Quote from Roadmap**: "Success: Can explain the entire architecture using just the counter example."

---

## Notes & Decisions

_Use this section to capture important decisions during Phase 1:_

- **Effect Execution**: (TBD)
- **Error Strategy**: (TBD)
- **Performance Results**: (TBD)
- **Deviations from Plan**: (TBD)

---

## Estimated Time Breakdown

Based on roadmap estimate of 1.5-2 weeks:

1. Effect documentation & composition utilities: 1-2 days
2. Store effect executor implementation: 2-3 days
3. Error handling & concurrency: 1-2 days
4. FixedClock enhancement: 0.5 days
5. Counter example implementation: 1-2 days
6. Testing (unit + integration + benchmarks): 2-3 days
7. Documentation: 2-3 days
8. Validation & polish: 1 day
9. Buffer for unknowns: 1-2 days

**Total**: 11-18 days (1.5-2.5 weeks of full-time work)

---

## References

- **Architecture Spec**: `specs/architecture.md` (sections 3-5)
- **Roadmap**: `plans/implementation-roadmap.md` (Phase 1 section)
- **Modern Rust Expert**: `.claude/skills/modern-rust-expert.md`
- **Phase 0 TODO**: `plans/phase-0/TODO.md` (completed example)

---

## Quick Start

**First task**: Implement `Effect::Future` execution in Store

**Order of implementation**:
1. Effect executor (start with Future, add others incrementally)
2. Error handling infrastructure
3. Counter example
4. Testing & benchmarks
5. Documentation
6. Validation

**Next**: Begin with Store effect executor implementation!
