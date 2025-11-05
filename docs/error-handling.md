# Error Handling Strategy

This document defines the error handling philosophy and implementation for the Composable Rust framework.

## Philosophy

**Fail Fast for Logic Errors, Resilient for Runtime Errors**

- **Reducers**: Pure functions that should never fail. Panics indicate bugs.
- **Effects**: Side effects that may fail. Failures are logged but don't halt the system.
- **Store**: Coordination layer that guarantees consistency. Lock poisoning halts the store.

## Error Categories

### 1. Reducer Panics (Logic Errors)

**What**: A reducer panics during state transition

**Cause**: Programming bug (e.g., division by zero, index out of bounds, unwrap on None)

**Behavior**:
- Panic propagates through `Store::send()`
- `RwLock` for state is poisoned
- All subsequent Store operations will fail with poison error
- **Store is effectively halted**

**Rationale**:
- Reducers are pure functions tested at memory speed
- Panics indicate bugs that must be fixed, not runtime conditions
- Fail-fast prevents corrupted state
- Poison lock prevents continued operation with potentially inconsistent state

**Recovery**: None. Fix the bug and restart.

**Testing**: All reducer logic should have comprehensive unit tests to prevent panics.

### 2. Effect Execution Failures (Runtime Errors)

**What**: An effect panics or fails during execution

**Cause**:
- Future panics during execution
- Async operation fails (network, database, etc.)
- Resource exhaustion

**Behavior**:
- Panic is **isolated** to the spawned task (via `tokio::spawn`)
- Error is **logged** via `tracing::error!`
- Effect counter is **decremented** (via `DecrementGuard` RAII)
- Other effects and Store operations **continue normally**
- Parent `EffectHandle` will still complete

**Rationale**:
- Effects represent side effects in the real world, which can fail
- Effect failures should not halt the entire system
- Observability via logging enables debugging
- Graceful degradation: system continues with reduced functionality

**Recovery**: Automatic via effect isolation. Monitor logs for recurring failures.

**Testing**: Integration tests should verify Store continues operating after effect failures.

### 3. Lock Poisoning (Catastrophic Errors)

**What**: `RwLock` is poisoned due to panic while holding lock

**Cause**:
- Reducer panics while holding write lock
- State accessor panics while holding read lock (very rare)

**Behavior**:
- All subsequent lock acquisitions return `PoisonError`
- Store operations panic with poison error
- **Store is permanently unusable**

**Rationale**:
- Poisoned lock indicates state may be corrupted
- Continuing with potentially invalid state is dangerous
- Fail-fast prevents cascading failures

**Recovery**: None. Application must be restarted.

**Prevention**: Write panic-free reducers with comprehensive tests.

## API Error Types

### Phase 1: No Result Types

Current Store API is **infallible from caller perspective**:

```rust
pub async fn send(&self, action: A) -> EffectHandle
pub async fn state<F, T>(&self, f: F) -> T
```

**Why**:
- Reducer panics are bugs (fail-fast via panic propagation)
- Effect failures are isolated (don't propagate to caller)
- Lock operations panic on poison (unrecoverable)

**Future**: If Phase 2+ adds operations that can fail gracefully (e.g., database save), we may add:

```rust
#[derive(Error, Debug)]
pub enum StoreError {
    #[error("Lock poisoned: state may be corrupted")]
    Poisoned,

    #[error("Effect execution timed out")]
    Timeout,

    // Phase 2+: Database errors, event publishing errors, etc.
}
```

## Implementation Details

### DecrementGuard Pattern

Ensures effect counter decrements even on panic:

```rust
struct DecrementGuard<A>(EffectTracking<A>);

impl<A> Drop for DecrementGuard<A> {
    fn drop(&mut self) {
        let prev = self.0.counter.fetch_sub(1, Ordering::SeqCst);
        if prev == 1 {
            let _ = self.0.notifier.send(());
        }
    }
}
```

**Usage** in effect execution:
```rust
tokio::spawn(async move {
    let _guard = DecrementGuard(tracking);
    // Effect work here - guard ensures decrement on panic
    if let Some(action) = fut.await {
        store.send(action).await;
    }
    // Guard drops here (or on panic), counter decrements
});
```

### Tracing Integration

All error conditions are logged via `tracing` crate:

```rust
tracing::error!("Effect::Future panicked: {}", e);
tracing::warn!("Effect execution timed out after {:?}", duration);
tracing::trace!("Effect::None (no-op)");
```

**Log Levels**:
- `error!`: Effect panics, lock poisoning, unrecoverable errors
- `warn!`: Timeouts, retryable failures
- `info!`: Normal operational events
- `debug!`: Detailed execution flow
- `trace!`: Very verbose debugging

## Testing Strategy

### Unit Tests

Reducer tests should verify **no panics** for all valid inputs:

```rust
#[test]
fn test_reducer_no_panic() {
    let mut state = State::default();
    let env = TestEnvironment::new();

    // Should not panic for any valid action
    let _ = reducer.reduce(&mut state, Action::Valid, &env);
}
```

### Integration Tests

Effect failure tests should verify **isolation**:

```rust
#[tokio::test]
async fn test_effect_panic_isolated() {
    let store = Store::new(/* ... */);

    // Send action that produces panicking effect
    let handle = store.send(Action::CausesPanic).await;
    handle.wait().await; // Should complete despite panic

    // Store should still work
    let _ = store.send(Action::Normal).await;
    assert!(store.state(|s| s.is_valid()).await);
}
```

### Property Tests

Use `proptest` to verify **no panics for arbitrary inputs**:

```rust
proptest! {
    #[test]
    fn reducer_never_panics(action: Action, state: State) {
        let env = TestEnvironment::new();
        let mut state = state;
        let _ = reducer.reduce(&mut state, action, &env);
        // If we get here, no panic occurred
    }
}
```

## Guidelines for Application Code

### Writing Reducers

**DO**:
- ✅ Return `Effect::None` if no side effects needed
- ✅ Use `Option` and `Result` for fallible operations, handle errors explicitly
- ✅ Validate inputs at the edge (command handlers), not in reducers
- ✅ Write comprehensive unit tests

**DON'T**:
- ❌ Use `.unwrap()`, `.expect()` in reducer code
- ❌ Panic on invalid state (indicates bug in your model)
- ❌ Use `.panic!()` for flow control
- ❌ Do I/O or side effects directly

### Writing Effects

**DO**:
- ✅ Handle errors gracefully (use Result, Option)
- ✅ Return `None` from futures if operation fails
- ✅ Log errors for observability
- ✅ Design for failure (effects may not execute)

**DON'T**:
- ❌ Assume effect will always succeed
- ❌ Panic on expected failures (network errors, etc.)
- ❌ Rely on effect execution order (unless using Sequential)
- ❌ Share mutable state between effects

### Monitoring & Observability

**Production**:
1. Enable `tracing` subscriber with appropriate log level
2. Monitor logs for `ERROR` and `WARN` patterns
3. Set up alerts for lock poison errors (application restart needed)
4. Track effect failure rates via metrics

**Development**:
1. Use `RUST_LOG=debug` or `trace` for detailed execution flow
2. Add custom spans for domain events
3. Use `tracing-test` crate for test log assertions

## Future Enhancements

### Phase 2: Database Errors

When adding persistence, we may add:

```rust
impl Store {
    pub async fn save_snapshot(&self) -> Result<(), StoreError> {
        // Database operations can fail gracefully
    }
}
```

### Phase 3: Saga Compensation

When adding sagas, we may add:

```rust
enum SagaError {
    StepFailed { step: usize, error: String },
    CompensationFailed { step: usize, error: String },
}
```

### Phase 4: Production Hardening

- Circuit breakers for effect execution
- Retry policies for transient failures
- Timeouts for long-running effects
- Backpressure mechanisms

## Summary

| Error Type | Source | Behavior | Recovery |
|------------|--------|----------|----------|
| Reducer Panic | Bug in pure logic | Halt store (poison lock) | Fix bug, restart |
| Effect Panic | Runtime failure | Isolate, log, continue | Automatic |
| Lock Poison | Panic while locked | Halt store | Restart application |

**Key Principle**: Reducers fail fast (bugs), effects fail gracefully (reality).
