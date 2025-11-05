# Implementation Decisions - Phase 1

This document captures key architectural and implementation decisions made during Phase 1, their rationale, trade-offs, and alternatives considered.

## Core Architecture Decisions

### Decision 1: `&mut State` in Reducers

**Choice**: Reducers take `&mut Self::State` instead of consuming and returning state.

**Rationale**:
- **Performance**: Zero-copy, in-place mutations (no allocation/clone overhead)
- **Ergonomics**: Direct field access (`state.count += 1` vs `State { count: state.count + 1, .. }`)
- **Still pure**: Mutation is an implementation detail; reducer is pure from caller's perspective

**Trade-offs**:
- ✅ **Pro**: 10-100x faster for large state structs
- ✅ **Pro**: More natural imperative style for business logic
- ❌ **Con**: Slightly less "functional" looking
- ❌ **Con**: Can't use structural sharing (not needed for our use case)

**Alternatives considered**:
1. **Consume and return**: `fn reduce(self, state: State, ...) -> (State, Vec<Effect>)`
   - Rejected: Unnecessary allocations, verbose updates
2. **Immutable + Copy-on-Write**: Using persistent data structures
   - Rejected: Significant complexity, performance overhead
3. **Builder pattern**: `state.with_count(state.count + 1)`
   - Rejected: Verbose, doesn't scale to complex updates

**Status**: ✅ Validated by benchmarks (< 1μs reducer execution)

---

### Decision 2: Effects as Values, Not Execution

**Choice**: `Effect<Action>` enum represents side effect *descriptions*, not execution.

**Rationale**:
- **Testability**: Can assert on effects without executing them
- **Purity**: Keeps reducers pure (no hidden I/O)
- **Flexibility**: Effects can be composed, transformed (via `map()`), or cancelled
- **Replay**: Can replay reducers without re-executing side effects

**Trade-offs**:
- ✅ **Pro**: Tests don't need mocks (effects are data)
- ✅ **Pro**: Time-travel debugging (replay without side effects)
- ✅ **Pro**: Effect composition (`Effect::Sequential`, `Effect::Parallel`)
- ❌ **Con**: Two-phase execution (describe, then execute)
- ❌ **Con**: Type complexity (`Pin<Box<dyn Future>>` for async)

**Alternatives considered**:
1. **Direct async execution**: `async fn reduce(...) { database.save().await; }`
   - Rejected: Hidden side effects, hard to test, no replay
2. **Callback-based**: `fn reduce(..., on_save: impl Fn())`
   - Rejected: Callback hell, hard to compose
3. **Command pattern with traits**: `trait Effect { fn execute(&self); }`
   - Rejected: More complex, harder to serialize (Phase 2 requirement)

**Status**: ✅ Validated by TestStore (deterministic effect testing)

---

### Decision 3: EffectHandle with Direct/Cascading Modes

**Choice**: `Store::send()` returns `EffectHandle` with two tracking modes:
- **Direct**: Tracks only immediate effects (default)
- **Cascading**: Tracks entire effect tree (opt-in)

**Rationale**:
- **Feedback loop support**: Effects can produce actions that trigger more effects
- **Await-able**: `handle.wait()` enables deterministic testing
- **Flexible**: Direct mode for most cases, Cascading for integration tests
- **Non-blocking**: `send()` returns immediately, `wait()` is optional

**Trade-offs**:
- ✅ **Pro**: Enables TestStore to work with effect chains
- ✅ **Pro**: Similar to JavaScript Promises (familiar model)
- ✅ **Pro**: Zero overhead if not awaited
- ❌ **Con**: Additional complexity (tracking counter, watch channel)
- ❌ **Con**: Must remember to call `.await` on `send()` (tokio lint helps)

**Alternatives considered**:
1. **No handle**: `async fn send(...) -> ()`
   - Rejected: Can't wait for effects in tests
2. **Always cascading**: Track everything by default
   - Rejected: Overhead for simple cases
3. **Callback on completion**: `send(..., on_complete: impl Fn())`
   - Rejected: Doesn't compose well with async

**Status**: ✅ Validated by 37 tests including effect tracking tests

---

### Decision 4: TestStore with Action Queue

**Choice**: Separate `TestStore` that queues actions instead of auto-feeding them back.

**Rationale**:
- **Determinism**: Tests control when actions are processed
- **Clarity**: Explicit `receive()` calls show expected behavior
- **Debugging**: Can inspect queue state at any point
- **Zero production overhead**: TestStore is test-only code

**Trade-offs**:
- ✅ **Pro**: Deterministic effect testing
- ✅ **Pro**: No production code changes needed
- ✅ **Pro**: Drop guard catches unprocessed actions (test hygiene)
- ❌ **Con**: Separate API to learn (vs just using Store)
- ❌ **Con**: Must remember to call `receive()` or `assert_no_pending_actions()`

**Alternatives considered**:
1. **Mock effects**: Stub out effect execution
   - Rejected: Doesn't test effect-to-action feedback
2. **Time-based waiting**: `tokio::time::sleep()` until queue empty
   - Rejected: Flaky, slow tests
3. **Polling with timeout**: Check queue periodically
   - Rejected: Non-deterministic, harder to debug

**Status**: ✅ Validated by 15 TestStore self-tests

---

### Decision 5: Thread-Safe Interior Mutability for FixedClock

**Choice**: `FixedClock` uses `Arc<RwLock<DateTime<Utc>>>` instead of `Cell<DateTime<Utc>>`.

**Rationale**:
- **Sync requirement**: `Clock` trait requires `Send + Sync`
- **Cross-thread testing**: Store is `Clone` and can be shared across threads
- **Read-heavy**: RwLock allows concurrent reads (common case)

**Trade-offs**:
- ✅ **Pro**: Works with concurrent tests
- ✅ **Pro**: Minimal overhead (RwLock optimized for reads)
- ❌ **Con**: Requires `expect()` calls (lock poisoning is unrecoverable)
- ❌ **Con**: Slightly more complex than Cell

**Alternatives considered**:
1. **`Cell<DateTime<Utc>>`**: Single-threaded interior mutability
   - Rejected: Doesn't impl `Sync`, fails trait bounds
2. **`Mutex<DateTime<Utc>>`**: Simpler lock
   - Rejected: No concurrent reads (RwLock better for read-heavy)
3. **`AtomicU64`** with timestamp encoding
   - Rejected: Overly complex for marginal benefit

**Status**: ✅ Validated by 17 tests including concurrent access

---

### Decision 6: Static Dispatch for Environment Traits

**Choice**: Environment is generic parameter `E: Environment`, not `Box<dyn Environment>`.

**Rationale**:
- **Zero-cost abstractions**: Compiler monomorphizes each implementation
- **No vtable overhead**: Direct function calls, not virtual dispatch
- **Better optimization**: Inlining, dead code elimination
- **Type safety**: Compile-time verification of trait bounds

**Trade-offs**:
- ✅ **Pro**: Zero runtime cost
- ✅ **Pro**: Better compiler optimizations
- ✅ **Pro**: Explicit dependencies (visible in type signature)
- ❌ **Con**: More verbose type signatures
- ❌ **Con**: Can't swap implementations at runtime (not a requirement)

**Alternatives considered**:
1. **Trait objects**: `env: Box<dyn Clock>`
   - Rejected: Runtime cost, no inlining, heap allocation
2. **Enum dispatch**: `enum Clock { System(SystemClock), Fixed(FixedClock) }`
   - Rejected: Closed set, can't add implementations externally

**Status**: ✅ Validated by benchmarks (reducer execution < 1μs)

---

### Decision 7: Explicit Effect Composition Methods

**Choice**: `Effect::map()`, `Effect::merge()`, `Effect::chain()` as explicit methods.

**Rationale**:
- **Discoverability**: IDE autocomplete shows available operations
- **Type safety**: Compiler checks closures and types
- **Clarity**: Intent is obvious (`merge` vs `chain`)

**Trade-offs**:
- ✅ **Pro**: Clear, self-documenting code
- ✅ **Pro**: Type-safe transformations
- ✅ **Pro**: Easy to add more combinators later
- ❌ **Con**: Not as terse as operator overloading
- ❌ **Con**: Requires learning the API

**Alternatives considered**:
1. **Operator overloading**: `effect1 + effect2` for parallel
   - Rejected: Ambiguous meaning, not idiomatic Rust
2. **Builder pattern**: `Effect::builder().add(...).merge()`
   - Rejected: Verbose, unnecessary allocations
3. **Macros**: `effects![e1, e2, e3]`
   - Rejected: Magic syntax, harder to debug

**Status**: ✅ Validated by 9 effect composition tests

---

### Decision 8: Error Handling Strategy

**Choice**: Three-tier error handling:
1. **Reducer panics** → Halt store (lock poison)
2. **Effect panics** → Isolate, log, continue
3. **Lock poisoning** → Unrecoverable, propagate panic

**Rationale**:
- **Fail fast for bugs**: Reducer panics indicate logic errors
- **Resilient for runtime**: Effect failures are expected (network, etc.)
- **Clear boundaries**: Pure core (panics = bugs) vs imperative shell (errors = reality)

**Trade-offs**:
- ✅ **Pro**: Clear error model (panics vs errors vs log)
- ✅ **Pro**: Production system stays up despite effect failures
- ✅ **Pro**: Forces reducer correctness (comprehensive tests)
- ❌ **Con**: Must restart application if reducer panics
- ❌ **Con**: Effect failures only visible in logs (Phase 4: metrics)

**Alternatives considered**:
1. **Result everywhere**: `fn reduce(...) -> Result<...>`
   - Rejected: Reduces can't meaningfully fail (they're pure)
2. **Catch all panics**: Catch reducer panics, log, continue
   - Rejected: Hides bugs, corrupted state risk
3. **Circuit breaker**: Stop processing after N failures
   - Deferred to Phase 4 (production hardening)

**Status**: ✅ Documented in docs/error-handling.md + test_effect_panic_isolation

---

## Implementation Details

### Tokio Runtime Configuration

**Choice**: `tokio::spawn` for effect execution, no custom executor.

**Rationale**: Tokio is battle-tested, well-optimized, and widely used. No need to reinvent.

**Trade-offs**: Depends on Tokio, but that's acceptable (industry standard).

### Pin<Box<dyn Future>> for Effect::Future

**Choice**: Type-erased boxed futures in `Effect::Future` variant.

**Rationale**:
- Enums need sized types
- Different futures have different types
- Boxing enables heterogeneous collections

**Trade-offs**: Heap allocation per future, but negligible vs I/O cost.

### Arc<AtomicUsize> for Effect Counter

**Choice**: Atomic counter for pending effects tracking.

**Rationale**:
- Lock-free reads/writes
- Shared across effect spawns
- Minimal overhead

**Trade-offs**: None significant (atomics are fast).

### DecrementGuard RAII Pattern

**Choice**: Guard ensures counter decrements even on panic.

**Rationale**: Effect panics shouldn't leak counter increments.

**Trade-offs**: Tiny overhead (struct + Drop), huge safety gain.

---

## Performance Targets & Results

| Target | Actual | Status |
|--------|--------|--------|
| Reducer < 1μs | ✅ < 1μs | Met |
| Tests < 100ms | ✅ ~0.1s | Met |
| Store throughput > 100k/sec | ⏸️ TBD (benchmarks ready) | Pending |

Run benchmarks: `cargo bench -p composable-rust-runtime`

---

## Future Decisions (Noted for Later Phases)

### Phase 2: Serialization
- **Pending**: bincode vs serde_json for event store
- **Leaning**: bincode (5-10x faster, smaller)

### Phase 3: Event Bus
- **Pending**: Redpanda vs native Kafka
- **Leaning**: Redpanda (self-hostable, compatible)

### Phase 4: Observability
- **Pending**: Metrics backend (Prometheus vs StatsD)
- **Leaning**: Prometheus (pull model, PromQL)

---

## Lessons Learned

1. **Start simple**: Counter example validated architecture before complexity
2. **Type-driven development**: Types caught errors at compile time
3. **Benchmarks early**: Identified `&mut State` perf benefit immediately
4. **Tests before features**: TestStore needed before complex effects
5. **Document as you go**: Counter README captured architecture clearly

## References

- Architecture spec: `specs/architecture.md`
- Error handling: `docs/error-handling.md`
- Counter README: `examples/counter/README.md`
- Phase 1 TODO: `plans/phase-1/TODO.md`
