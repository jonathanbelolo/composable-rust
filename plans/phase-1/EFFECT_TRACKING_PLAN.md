# Effect Tracking & EffectHandle Implementation Plan

**Date**: 2025-11-05
**Status**: Final Design
**Goal**: Add deterministic effect completion tracking for both testing and production

---

## Problem Statement

Phase 1 implementation has three critical issues (from PHASE1_REVIEW.md):

1. **Test timing flakiness** - Tests use arbitrary `tokio::time::sleep()` durations
2. **Effect execution inconsistency** - Some effects block, others don't
3. **No completion mechanism** - Can't know when effects are "done"

### Current Test Pattern (Flaky)

```rust
#[tokio::test]
async fn test_effect_delay() {
    let store = Store::new(reducer, env, state);

    store.send(TestAction::ProduceDelayedAction).await;

    // ❌ Hope 50ms is enough... what if CI is slow?
    tokio::time::sleep(Duration::from_millis(50)).await;

    let value = store.state(|s| s.value).await;
    assert_eq!(value, 1);
}
```

### What We Need

- **Tests**: Deterministic way to know when effects complete (no arbitrary sleeps)
- **Production**: Wait for specific operations (graceful shutdown, HTTP handlers)
- **Flexibility**: Sometimes wait for direct effects only, sometimes entire cascade

---

## Design Evolution & Decisions

### ❌ Rejected: Global Counter Approach

**Initial idea**: Add `active_effects: Arc<AtomicUsize>` to Store, track all in-flight effects.

**Why rejected**: Race conditions and semantic confusion.

```rust
// Task A
store.send(Action::StartBatch).await;   // 10 effects spawn
store.wait_until_idle().await;          // Waiting for counter to hit 0...

// Task B (concurrently)
store.send(Action::NewRequest).await;   // Counter goes back up!

// Task A keeps waiting... might never finish!
```

**Problems**:
1. Conflates all work together - can't distinguish "my effects" from "other effects"
2. Production use cases break (shutdown waits for new requests)
3. Tests need to ensure no concurrent sends (fragile)
4. Not composable - can't wait for multiple specific operations

**Key insight**: This is like trying to track all JavaScript promises globally instead of individually.

---

### ✅ Chosen: Per-Action Effect Handles

**Design**: Each `send()` returns a handle to track THAT action's effects, just like JavaScript async/await.

```rust
let handle = store.send(action).await;  // Returns handle
handle.wait().await;                     // Wait for THIS action's effects
```

**Benefits**:
1. ✅ No race conditions - each handle tracks independent work
2. ✅ Production-friendly - wait for specific operations
3. ✅ Composable - `join_all(handles)` for multiple operations
4. ✅ Natural Rust async - matches JavaScript promise model
5. ✅ Fire-and-forget still works - just drop the handle

---

### ✅ Chosen: Two Tracking Modes

**Question**: If an effect produces `Action::Next`, does that join the same handle?

**Answer**: User chooses via tracking mode.

#### Mode 1: Direct (Default)

Wait for direct effects only. If those effects produce new actions, those are separate.

```rust
let handle = store.send(Action::Start).await;
// Reducer produces Effect::Future
// Effect spawns and is tracked

handle.wait().await;
// ✅ Completes when Effect::Future finishes
// If Effect produced Action::Next, that's independent
```

**Use case**: Step-by-step test assertions, most production scenarios.

#### Mode 2: Cascading (Opt-in)

Wait for entire cascade - effects that produce actions that produce more effects.

```rust
let handle = store.send_cascading(Action::StartWorkflow).await;
// Reducer produces Effect::Future (tracked)
// Effect produces Action::Middle (also tracked!)
// Reducer produces another Effect (also tracked!)
// Effect produces Action::End (also tracked!)

handle.wait().await;
// ✅ Completes when entire tree finishes
```

**Use case**: End-to-end workflow tests, batch processing completion.

**Why both?**:
- Tests often want step-by-step assertions (direct mode)
- Some workflows need full completion guarantee (cascading mode)
- Production: graceful shutdown (cascading), HTTP handlers (usually direct)

---

### ✅ Chosen: TestStore with Effect Queue

**Initial plan**: Skip TestStore, use Store directly in tests.

**Why reconsidered**: Need to observe and control the feedback loop!

**The problem**: With auto-feedback, you can't:
- Assert on intermediate actions produced by effects
- Inspect state between cascading actions
- Control when feedback happens
- Debug multi-step workflows

**The solution**: TestStore queues actions instead of auto-feeding them back.

```rust
// Production Store: Auto-feedback
let _handle = store.send(Action::Start).await;
// Effect produces Action::Middle, automatically feeds back
// Can't observe or control this!

// TestStore: Manual stepping
let store = TestStore::new(/*...*/);
store.send(Action::Start).await;
store.receive(Action::Middle).await?;  // Assert what effect produced
// Now Action::Middle is processed, and we can assert state
```

**Inspired by Swift TCA**: TestStore uses the same pattern as Swift's Composable Architecture `TestStore.receive()`.

**Key features**:
- Effects produce actions into a queue (don't auto-feed back)
- Test code explicitly receives and asserts on each action
- Type system encodes ordering semantics (`Vec` = ordered, `HashSet` = unordered)
- Forces step-by-step workflow verification

---

### ✅ Chosen: Configurable Timeouts (No Defaults)

**Question**: Should `wait()` have a default timeout (e.g., 5 seconds)?

**Problem**: Arbitrary defaults break real use cases:
- LLM API calls: 30-60 seconds
- Batch processing: Minutes
- Fast unit tests: Sub-second

**Solution**: User chooses appropriate timeout per use case.

```rust
// Fast test
handle.wait_with_timeout(Duration::from_secs(1)).await?;

// LLM test
handle.wait_with_timeout(Duration::from_secs(60)).await?;

// Production graceful shutdown
handle.wait_with_timeout(Duration::from_secs(30)).await?;
```

---

## Architecture

### TrackingMode Enum

```rust
pub enum TrackingMode {
    /// Wait for direct effects only (default)
    Direct,

    /// Wait for entire cascade (recursive)
    Cascading {
        children: Arc<Mutex<Vec<EffectHandle>>>,
    },

    // Future possibilities:
    // Depth(usize),  // Wait for N levels
    // UntilAction(fn(&A) -> bool),  // Wait until specific action
}
```

**Design note**: Single enum allows future modes without API changes.

---

### EffectTracking (Internal Type)

Internal context for tracking effect execution:

```rust
// Internal to runtime crate, not public API
struct EffectTracking<A> {
    mode: TrackingMode,
    counter: Arc<AtomicUsize>,
    notifier: watch::Sender<()>,
    feedback_dest: FeedbackDestination<A>,
}

enum FeedbackDestination<A> {
    /// Auto-feedback: send actions back to store
    Auto(Weak<Store<S, A, E, R>>),

    /// Queued: push actions to queue (TestStore mode)
    Queued(Arc<Mutex<VecDeque<A>>>),
}

impl<A> EffectTracking<A> {
    fn increment(&self) {
        self.counter.fetch_add(1, Ordering::SeqCst);
    }

    fn decrement(&self) {
        if self.counter.fetch_sub(1, Ordering::SeqCst) == 1 {
            // Last effect completed, signal
            let _ = self.notifier.send(());
        }
    }

    fn clone_for_spawn(&self) -> Self {
        Self {
            mode: self.mode.clone(),
            counter: self.counter.clone(),
            notifier: self.notifier.clone(),
            feedback_dest: self.feedback_dest.clone(),
        }
    }
}

/// RAII guard to ensure decrement on panic
struct DecrementGuard<A>(EffectTracking<A>);

impl<A> Drop for DecrementGuard<A> {
    fn drop(&mut self) {
        self.0.decrement();
    }
}
```

**Key Points**:
- `EffectTracking` passes through effect execution, not stored in Store
- `FeedbackDestination` enables both production (Auto) and test (Queued) modes
- `DecrementGuard` ensures counter consistency even if effects panic
- Store has **zero** production overhead (no queue field)

---

### EffectHandle Type

```rust
/// Handle to track completion of effects from a single `send()` call
///
/// Returned by [`Store::send`] and [`Store::send_cascading`]. Allows waiting
/// for effect completion in both tests and production code.
///
/// # Modes
///
/// - **Direct**: Tracks only direct effects (default via `send()`)
/// - **Cascading**: Tracks full effect tree including actions produced by effects
///
/// # Examples
///
/// ```rust
/// // Direct mode - wait for immediate effects only
/// let handle = store.send(Action::Increment).await;
/// handle.wait_with_timeout(Duration::from_secs(5)).await?;
///
/// // Cascading mode - wait for entire workflow
/// let handle = store.send_cascading(Action::StartWorkflow).await;
/// handle.wait_with_timeout(Duration::from_secs(30)).await?;
///
/// // Fire-and-forget - just drop the handle
/// let _handle = store.send(Action::LogMetric).await;
/// ```
#[derive(Clone)]  // Allow multiple waiters
pub struct EffectHandle {
    mode: TrackingMode,
    effects: Arc<AtomicUsize>,
    completion: watch::Receiver<()>,
}

impl EffectHandle {
    /// Create handle that's already complete (no effects)
    ///
    /// Used internally when chaining receives or for testing.
    pub fn completed() -> Self {
        let (tx, rx) = watch::channel(());
        let _ = tx.send(());  // Signal immediately

        Self {
            mode: TrackingMode::Direct,
            effects: Arc::new(AtomicUsize::new(0)),
            completion: rx,
        }
    }

    /// Wait for effects to complete with timeout
    ///
    /// Behavior depends on tracking mode:
    /// - **Direct**: Returns when direct effects finish
    /// - **Cascading**: Returns when entire effect tree finishes (recursive)
    ///
    /// # Errors
    ///
    /// Returns [`EffectTimeoutError`] if timeout expires before completion.
    ///
    /// # Example
    ///
    /// ```rust
    /// let handle = store.send(Action::Process).await;
    /// handle.wait_with_timeout(Duration::from_secs(10)).await?;
    /// ```
    pub async fn wait_with_timeout(
        mut self,
        timeout: Duration,
    ) -> Result<(), EffectTimeoutError> {
        let start = std::time::Instant::now();

        // Wait for direct effects
        let remaining = timeout.saturating_sub(start.elapsed());
        tokio::time::timeout(remaining, self.completion.changed())
            .await
            .map_err(|_| EffectTimeoutError {
                active_count: self.effects.load(Ordering::SeqCst),
                elapsed: start.elapsed(),
            })?;

        // If cascading, drain children until none remain (handles grandchildren)
        if let TrackingMode::Cascading { children } = self.mode {
            loop {
                // Take current batch of children
                let handles = {
                    let mut guard = children.lock().unwrap();
                    if guard.is_empty() {
                        break;  // All done!
                    }
                    guard.drain(..).collect::<Vec<_>>()
                };

                // Wait for this batch
                for handle in handles {
                    let remaining = timeout.saturating_sub(start.elapsed());
                    handle.wait_with_timeout(remaining).await?;
                }

                // Loop - more children might have been added while waiting
            }
        }

        Ok(())
    }

    /// Check if effects are complete (non-blocking)
    ///
    /// # Example
    ///
    /// ```rust
    /// let handle = store.send(Action::Process).await;
    /// if handle.is_complete() {
    ///     println!("Already done!");
    /// } else {
    ///     handle.wait_with_timeout(Duration::from_secs(5)).await?;
    /// }
    /// ```
    pub fn is_complete(&self) -> bool {
        self.effects.load(Ordering::SeqCst) == 0
    }
}

/// Error when effects don't complete within timeout
#[derive(Debug)]
pub struct EffectTimeoutError {
    pub active_count: usize,
    pub elapsed: Duration,
}

impl std::fmt::Display for EffectTimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Effects did not complete after {:?} ({} still active)",
            self.elapsed, self.active_count
        )
    }
}

impl std::error::Error for EffectTimeoutError {}
```

---

### Store API Changes

```rust
// Store structure - NO queue field
pub struct Store<S, A, E, R> {
    state: Arc<RwLock<S>>,
    reducer: Arc<R>,
    environment: Arc<E>,
    // Note: No effect_queue field! Zero production overhead.
}

impl Store {
    /// Send action, return handle to direct effects
    ///
    /// Returns immediately after spawning effects. Use the returned handle
    /// to wait for effect completion if needed.
    ///
    /// # Fire-and-Forget Semantics
    ///
    /// The handle can be dropped to ignore completion (fire-and-forget):
    ///
    /// ```rust
    /// let _handle = store.send(Action::LogMetric).await;
    /// // Effect runs in background, we don't wait
    /// ```
    ///
    /// # Direct vs Cascading
    ///
    /// This method tracks **direct effects only**. If effects produce new actions,
    /// those are not included in this handle. Use [`send_cascading`] to track
    /// the entire effect tree.
    ///
    /// # Example
    ///
    /// ```rust
    /// // Wait for specific action's effects
    /// let handle = store.send(Action::CreateOrder).await;
    /// handle.wait_with_timeout(Duration::from_secs(10)).await?;
    ///
    /// // Now safe to query order state
    /// let order_id = store.state(|s| s.last_order_id).await;
    /// ```
    pub async fn send(&self, action: A) -> EffectHandle {
        self.send_internal(
            action,
            TrackingMode::Direct,
            FeedbackDestination::Auto(Arc::downgrade(&Arc::new(self.clone()))),
        ).await
    }

    /// Send action, track entire cascade
    ///
    /// Like [`send`], but the returned handle tracks the full effect tree:
    /// if effects produce actions that produce more effects, all are tracked.
    ///
    /// # Use Cases
    ///
    /// - Waiting for multi-step workflows to complete
    /// - Graceful shutdown (wait for all cascading work)
    /// - Batch processing completion
    /// - End-to-end test assertions
    ///
    /// # Example
    ///
    /// ```rust
    /// // Wait for entire workflow
    /// let handle = store.send_cascading(Action::StartWorkflow).await;
    /// handle.wait_with_timeout(Duration::from_secs(60)).await?;
    ///
    /// // All cascading effects complete
    /// assert_eq!(store.state(|s| s.status).await, Status::Done);
    /// ```
    pub async fn send_cascading(&self, action: A) -> EffectHandle {
        self.send_internal(
            action,
            TrackingMode::Cascading {
                children: Arc::new(Mutex::new(Vec::new())),
            },
            FeedbackDestination::Auto(Arc::downgrade(&Arc::new(self.clone()))),
        ).await
    }

    /// Internal: used by TestStore to pass queue
    pub(crate) async fn send_with_queue(
        &self,
        action: A,
        queue: Arc<Mutex<VecDeque<A>>>,
    ) -> EffectHandle {
        self.send_internal(
            action,
            TrackingMode::Direct,
            FeedbackDestination::Queued(queue),
        ).await
    }

    /// Internal: used by TestStore for cascading + queue
    pub(crate) async fn send_cascading_with_queue(
        &self,
        action: A,
        queue: Arc<Mutex<VecDeque<A>>>,
    ) -> EffectHandle {
        self.send_internal(
            action,
            TrackingMode::Cascading {
                children: Arc::new(Mutex::new(Vec::new())),
            },
            FeedbackDestination::Queued(queue),
        ).await
    }

    /// Core send implementation
    async fn send_internal(
        &self,
        action: A,
        mode: TrackingMode,
        feedback_dest: FeedbackDestination<A>,
    ) -> EffectHandle {
        // Create tracking context
        let (notifier, completion) = watch::channel(());
        let tracking = EffectTracking {
            mode: mode.clone(),
            counter: Arc::new(AtomicUsize::new(0)),
            notifier,
            feedback_dest,
        };

        // Run reducer
        let effects = {
            let mut state = self.state.write().await;
            self.reducer.reduce(&mut state, action, &self.environment)
        };

        // Execute effects with tracking
        for effect in effects {
            self.execute_effect_internal(effect, tracking.clone_for_spawn());
        }

        // If no effects, signal immediately
        if tracking.counter.load(Ordering::SeqCst) == 0 {
            let _ = tracking.notifier.send(());
        }

        // Return handle
        EffectHandle {
            mode,
            effects: tracking.counter,
            completion,
        }
    }
}
```

---

### Effect Execution with Tracking

Each effect execution increments counter on spawn, decrements on completion (with panic safety):

```rust
fn execute_effect_internal(&self, effect: Effect<A>, tracking: EffectTracking<A>) {
    match effect {
        Effect::None => {
            // No spawn needed
        }

        Effect::Future(fut) => {
            tracking.increment();

            let store = self.clone();
            let tracking_clone = tracking.clone_for_spawn();

            tokio::spawn(async move {
                // Guard ensures decrement even on panic
                let _guard = DecrementGuard(tracking_clone.clone());

                if let Some(action) = fut.await {
                    Self::send_action_to_destination(
                        action,
                        &tracking_clone.feedback_dest,
                        &tracking_clone.mode,
                        &store,
                    ).await;
                }
            });
        }

        Effect::Delay { duration, action } => {
            tracking.increment();

            let store = self.clone();
            let tracking_clone = tracking.clone_for_spawn();

            tokio::spawn(async move {
                let _guard = DecrementGuard(tracking_clone.clone());

                tokio::time::sleep(duration).await;

                if let Some(action) = action {
                    Self::send_action_to_destination(
                        action,
                        &tracking_clone.feedback_dest,
                        &tracking_clone.mode,
                        &store,
                    ).await;
                }
            });
        }

        Effect::Parallel(effects) => {
            // Each sub-effect handles its own tracking
            for effect in effects {
                self.execute_effect_internal(effect, tracking.clone_for_spawn());
            }
        }

        Effect::Sequential(effects) => {
            tracking.increment();

            let store = self.clone();
            let tracking_clone = tracking.clone_for_spawn();

            tokio::spawn(async move {
                let _guard = DecrementGuard(tracking_clone.clone());

                // Execute each effect and wait for completion
                for effect in effects {
                    // Create sub-tracking for this effect
                    let (sub_tx, mut sub_rx) = watch::channel(());
                    let sub_tracking = EffectTracking {
                        mode: TrackingMode::Direct,
                        counter: Arc::new(AtomicUsize::new(0)),
                        notifier: sub_tx,
                        feedback_dest: tracking_clone.feedback_dest.clone(),
                    };

                    // Execute effect
                    store.execute_effect_internal(effect, sub_tracking.clone_for_spawn());

                    // Wait for it to complete before continuing
                    if sub_tracking.counter.load(Ordering::SeqCst) > 0 {
                        let _ = sub_rx.changed().await;
                    }
                }
            });
        }
    }
}

/// Helper: send action to feedback destination
async fn send_action_to_destination(
    action: A,
    feedback_dest: &FeedbackDestination<A>,
    mode: &TrackingMode,
    store: &Store<S, A, E, R>,
) {
    match feedback_dest {
        FeedbackDestination::Auto(_weak_store) => {
            // Auto-feedback to store
            match mode {
                TrackingMode::Direct => {
                    let _handle = store.send(action).await;
                }
                TrackingMode::Cascading { children } => {
                    let child = store.send_cascading(action).await;
                    children.lock().unwrap().push(child);
                }
            }
        }
        FeedbackDestination::Queued(queue) => {
            // Push to queue (TestStore mode)
            queue.lock().unwrap().push_back(action);
        }
    }
}
```

**Key Points**:
- **DecrementGuard**: RAII ensures counter decrements even on panic
- **Sequential**: Creates sub-tracking for each effect, waits before continuing
- **Feedback dispatch**: Centralized helper handles Auto vs Queued
- **Panic safety**: All effect types use guard pattern

---

## TestStore: Effect Queue & Observation

TestStore wraps Store and intercepts the feedback loop, allowing step-by-step assertions.

### Architecture

```rust
use std::collections::{HashSet, VecDeque};

/// Test-specific store with effect observation and control
///
/// Unlike production [`Store`], TestStore captures actions produced by effects
/// in a queue instead of immediately feeding them back. This enables:
///
/// - Asserting on intermediate actions
/// - Inspecting state between cascading actions
/// - Controlling when feedback happens
/// - Debugging multi-step workflows
///
/// Inspired by Swift's Composable Architecture TestStore.
pub struct TestStore<S, A, E, R> {
    store: Store<S, A, E, R>,
    effect_queue: Arc<Mutex<VecDeque<A>>>,
}

impl<S, A, E, R> TestStore<S, A, E, R>
where
    R: Reducer<State = S, Action = A, Environment = E> + Send + Sync + 'static,
    A: Send + 'static,
    S: Send + Sync + 'static,
    E: Send + Sync + Clone + 'static,
{
    /// Create test store
    pub fn new(reducer: R, environment: E, initial_state: S) -> Self {
        Self {
            store: Store::new_queued(reducer, environment, initial_state),
            effect_queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Send action (effects queue instead of auto-feedback)
    pub async fn send(&self, action: A) -> EffectHandle {
        self.store.send_queued(action, self.effect_queue.clone()).await
    }

    /// Send with cascading tracking
    pub async fn send_cascading(&self, action: A) -> EffectHandle {
        self.store.send_cascading_queued(action, self.effect_queue.clone()).await
    }

    /// Receive expected action(s) from queue (ordered)
    ///
    /// Type-based semantics:
    /// - `A` (single): Must match front of queue
    /// - `Vec<A>`: Must match front in order
    ///
    /// After matching, actions are removed from queue and fed back to store.
    ///
    /// # Example
    ///
    /// ```rust
    /// // Single action
    /// store.receive(Action::Loaded).await?;
    ///
    /// // Multiple ordered
    /// store.receive(vec![Action::Step1, Action::Step2]).await?;
    /// ```
    pub async fn receive<E>(&self, expected: E) -> Result<EffectHandle, TestStoreError>
    where
        E: ExpectedActions<A>,
    {
        let mut queue = self.effect_queue.lock().unwrap();
        let actions = expected.match_and_consume(&mut queue)?;
        drop(queue);

        // Feed actions back to store, return last handle
        let mut last_handle = EffectHandle::completed();
        for action in actions {
            last_handle = self.store.send_queued(action, self.effect_queue.clone()).await;
        }

        Ok(last_handle)
    }

    /// Receive expected action after handle completes (ordered)
    ///
    /// Waits for handle to complete, then receives from queue.
    /// More ergonomic than separate `handle.wait()` + `receive()`.
    ///
    /// # Example
    ///
    /// ```rust
    /// let h = store.send(Action::Start).await;
    /// tokio::time::advance(Duration::from_millis(100)).await;
    ///
    /// // Wait and receive in one call
    /// store.receive_after(Action::Middle, h).await?;
    /// ```
    pub async fn receive_after<E>(
        &self,
        expected: E,
        handle: EffectHandle,
    ) -> Result<EffectHandle, TestStoreError>
    where
        E: ExpectedActions<A>,
    {
        // Wait for handle first (30s default timeout)
        handle.wait_with_timeout(Duration::from_secs(30)).await
            .map_err(TestStoreError::Timeout)?;

        // Then receive from queue
        self.receive(expected).await
    }

    /// Receive actions in any order (parallel effects)
    ///
    /// Searches queue for matching actions regardless of position.
    /// Allows duplicate actions (unlike HashSet which would deduplicate).
    ///
    /// # Example
    ///
    /// ```rust
    /// // Parallel effects - don't care which completes first
    /// store.receive_unordered(vec![
    ///     Action::FetchA,
    ///     Action::FetchB,
    /// ]).await?;
    ///
    /// // Duplicates work correctly
    /// store.receive_unordered(vec![
    ///     Action::Loaded(data1),
    ///     Action::Loaded(data2),  // Same action type, different data
    /// ]).await?;
    /// ```
    pub async fn receive_unordered(&self, expected: Vec<A>) -> Result<EffectHandle, TestStoreError>
    where
        A: PartialEq + Debug,
    {
        let mut queue = self.effect_queue.lock().unwrap();

        // Find and remove each expected action (any position)
        let mut found = Vec::new();
        for expected_action in expected {
            let pos = queue
                .iter()
                .position(|a| a == &expected_action)
                .ok_or_else(|| TestStoreError::ActionNotFound(
                    format!("{:?}", expected_action)
                ))?;

            found.push(queue.remove(pos).unwrap());
        }

        drop(queue);

        // Feed actions back to store, return last handle
        let mut last_handle = EffectHandle::completed();
        for action in found {
            last_handle = self.store.send_with_queue(action, self.effect_queue.clone()).await;
        }

        Ok(last_handle)
    }

    /// Receive actions after handle completes (unordered)
    pub async fn receive_unordered_after(
        &self,
        expected: Vec<A>,
        handle: EffectHandle,
    ) -> Result<EffectHandle, TestStoreError>
    where
        A: PartialEq + Debug,
    {
        handle.wait_with_timeout(Duration::from_secs(30)).await
            .map_err(TestStoreError::Timeout)?;

        self.receive_unordered(expected).await
    }

    /// Access state
    pub async fn state<T>(&self, f: impl FnOnce(&S) -> T) -> T {
        self.store.state(f).await
    }

    /// Peek at next queued action without consuming
    pub fn peek_next(&self) -> Option<A>
    where
        A: Clone,
    {
        self.effect_queue.lock().unwrap().front().cloned()
    }

    /// Number of pending actions in queue
    pub fn pending_count(&self) -> usize {
        self.effect_queue.lock().unwrap().len()
    }

    /// Assert queue is empty (call at test end)
    ///
    /// # Panics
    ///
    /// Panics if there are unprocessed actions. This catches bugs where
    /// effects produce unexpected actions.
    ///
    /// # Example
    ///
    /// ```rust
    /// #[tokio::test]
    /// async fn test() {
    ///     let store = TestStore::new(/*...*/);
    ///     // ... test code ...
    ///     store.assert_no_pending_actions();
    /// }
    /// ```
    #[allow(clippy::missing_panics_doc)]
    pub fn assert_no_pending_actions(&self) {
        let queue = self.effect_queue.lock().unwrap();
        if !queue.is_empty() {
            panic!(
                "Test ended with {} unprocessed actions: {:?}",
                queue.len(),
                queue
            );
        }
    }

    /// Clear queue without processing (opt-out of assertions)
    pub fn skip_pending_actions(&self) {
        self.effect_queue.lock().unwrap().clear();
    }
}

/// Drop implementation: panic if unprocessed actions remain
impl<S, A, E, R> Drop for TestStore<S, A, E, R>
where
    A: Debug,
{
    fn drop(&mut self) {
        // Don't panic during unwinding (double panic is bad)
        if std::thread::panicking() {
            return;
        }

        let queue = self.effect_queue.lock().unwrap();
        if !queue.is_empty() {
            // Format error message
            let mut msg = format!(
                "\n⚠️  TestStore dropped with {} unprocessed actions:\n",
                queue.len()
            );
            for (i, action) in queue.iter().enumerate() {
                msg.push_str(&format!("  [{}] {:?}\n", i + 1, action));
            }
            msg.push_str("\nDid you forget to call assert_no_pending_actions()?\n");

            // Panic to fail test
            panic!("{}", msg);
        }
    }
}
```

**Key Point**: Drop impl ensures tests fail if actions aren't processed. Strict enforcement catches bugs early.

---

### ExpectedActions Trait

Type-based dispatch for ordered matching:

```rust
/// Trait for matching expected actions against queue
///
/// Implemented for:
/// - `A`: Single action (must match front of queue)
/// - `Vec<A>`: Multiple actions in order (must match front of queue)
///
/// Note: For unordered matching, use `receive_unordered()` directly.
pub trait ExpectedActions<A> {
    /// Match and consume actions from queue
    ///
    /// # Errors
    ///
    /// Returns error if queue doesn't match expectations.
    fn match_and_consume(
        &self,
        queue: &mut VecDeque<A>,
    ) -> Result<Vec<A>, TestStoreError>;
}

/// Single action: Must match front of queue
impl<A: PartialEq + Debug + Clone> ExpectedActions<A> for A {
    fn match_and_consume(
        &self,
        queue: &mut VecDeque<A>,
    ) -> Result<Vec<A>, TestStoreError> {
        let action = queue.pop_front()
            .ok_or(TestStoreError::NoActionProduced)?;

        if &action != self {
            return Err(TestStoreError::UnexpectedAction {
                expected: format!("{:?}", self),
                actual: format!("{:?}", action),
            });
        }

        Ok(vec![action])
    }
}

/// Vec: Multiple actions in order (front of queue)
impl<A: PartialEq + Debug + Clone> ExpectedActions<A> for Vec<A> {
    fn match_and_consume(
        &self,
        queue: &mut VecDeque<A>,
    ) -> Result<Vec<A>, TestStoreError> {
        if queue.len() < self.len() {
            return Err(TestStoreError::NotEnoughActions {
                expected: self.len(),
                actual: queue.len(),
            });
        }

        // Check front matches expected order
        for (i, expected_action) in self.iter().enumerate() {
            if &queue[i] != expected_action {
                return Err(TestStoreError::OrderMismatch {
                    position: i,
                    expected: format!("{:?}", expected_action),
                    actual: format!("{:?}", &queue[i]),
                });
            }
        }

        // Consume
        let actions: Vec<_> = (0..self.len())
            .map(|_| queue.pop_front().unwrap())
            .collect();

        Ok(actions)
    }
}
```

### TestStoreError

```rust
/// Errors from TestStore receive operations
#[derive(Debug)]
pub enum TestStoreError {
    /// Effect completed but produced no action
    NoActionProduced,

    /// Effect produced unexpected action
    UnexpectedAction {
        expected: String,
        actual: String,
    },

    /// Queue has fewer actions than expected
    NotEnoughActions {
        expected: usize,
        actual: usize,
    },

    /// Action order doesn't match (Vec semantics)
    OrderMismatch {
        position: usize,
        expected: String,
        actual: String,
    },

    /// Expected action not found in queue (HashSet semantics)
    ActionNotFound(String),

    /// Effect timeout
    Timeout(#[from] EffectTimeoutError),
}

impl std::fmt::Display for TestStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoActionProduced => write!(f, "Effect produced no action"),
            Self::UnexpectedAction { expected, actual } => {
                write!(f, "Expected action {}, got {}", expected, actual)
            }
            Self::NotEnoughActions { expected, actual } => {
                write!(f, "Expected {} actions, queue has {}", expected, actual)
            }
            Self::OrderMismatch { position, expected, actual } => {
                write!(f, "Action at position {} doesn't match: expected {}, got {}",
                       position, expected, actual)
            }
            Self::ActionNotFound(action) => {
                write!(f, "Action not found in queue: {}", action)
            }
            Self::Timeout(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for TestStoreError {}
```

---

## Test Patterns

### Pattern 1: Step-by-Step Assertions with TestStore (Recommended)

Use **TestStore** with `receive_after()` to assert at each step:

```rust
#[tokio::test(start_paused = true)]
async fn test_multi_step_workflow() {
    let store = TestStore::new(reducer, env, State::default());

    // Step 1: Send initial action
    let h1 = store.send(Action::Start).await;

    // Assert state after Start processed (before effects run)
    assert_eq!(store.state(|s| s.step).await, Step::Started);

    // Step 2: Effect produced Action::Middle, wait and assert on it
    tokio::time::advance(Duration::from_millis(100)).await;
    let h2 = store.receive_after(Action::Middle, h1).await.unwrap();

    // Assert state after Middle processed
    assert_eq!(store.state(|s| s.step).await, Step::MiddleComplete);

    // Step 3: Effect produced Action::End
    tokio::time::advance(Duration::from_millis(100)).await;
    store.receive_after(Action::End, h2).await.unwrap();

    // Assert final state
    assert_eq!(store.state(|s| s.step).await, Step::Complete);

    // Verify no unexpected actions (Drop impl will also check)
    store.assert_no_pending_actions();
}
```

**Benefits**:
- ✅ Explicitly assert on each action produced by effects
- ✅ Inspect state between cascading actions
- ✅ Clear which step failed
- ✅ `receive_after()` waits on handle automatically (cleaner than manual wait)
- ✅ Drop impl catches forgotten assertions

### Pattern 2: Ordered Multiple Actions (Vec)

Use **Vec** when effects produce multiple actions in a specific order:

```rust
#[tokio::test(start_paused = true)]
async fn test_sequential_workflow() {
    let store = TestStore::new(reducer, env, State::default());

    // Trigger workflow
    let h = store.send(Action::StartWorkflow).await;

    tokio::time::advance(Duration::from_secs(1)).await;

    // Wait and expect specific ordered sequence
    store.receive_after(vec![
        Action::Step1Complete,
        Action::Step2Complete,
        Action::Step3Complete,
        Action::WorkflowComplete,
    ], h).await.unwrap();

    // Assert final state
    assert_eq!(store.state(|s| s.status).await, Status::Done);
    store.assert_no_pending_actions();
}
```

**Use when**:
- Effects produce known sequence of actions
- Order matters (sequential effects)
- Want to assert entire chain at once

### Pattern 3: Unordered Multiple Actions (receive_unordered)

Use **receive_unordered_after()** when effects run in parallel (order doesn't matter):

```rust
#[tokio::test(start_paused = true)]
async fn test_parallel_fetch() {
    let store = TestStore::new(reducer, env, State::default());

    // Trigger parallel fetches
    let h1 = store.send(Action::FetchAll).await;

    tokio::time::advance(Duration::from_secs(1)).await;

    // Don't care which completes first - use receive_unordered_after
    let h2 = store.receive_unordered_after(vec![
        Action::UsersLoaded(users),
        Action::PostsLoaded(posts),
    ], h1).await.unwrap();

    // After both complete, merge happens
    tokio::time::advance(Duration::from_millis(100)).await;
    store.receive_after(Action::MergeComplete, h2).await.unwrap();

    assert_eq!(store.state(|s| s.users).await, users);
    assert_eq!(store.state(|s| s.posts).await, posts);
    store.assert_no_pending_actions();
}
```

**Use when**:
- Effects run in parallel
- Order is non-deterministic
- Only care that all actions occur
- Duplicates possible (Vec allows duplicates, unlike HashSet)

### Pattern 4: Production Store (When TestStore Not Needed)

Some tests don't need effect observation - use Store directly:

```rust
#[tokio::test]
async fn test_simple_state_change() {
    let store = Store::new(reducer, env, State::default());

    // No effects produced, just state changes
    let handle = store.send(Action::Increment).await;
    handle.wait_with_timeout(Duration::from_secs(1)).await.unwrap();

    assert_eq!(store.state(|s| s.count).await, 1);
}
```

**Use when**:
- Reducer produces no effects
- Don't care about effect-produced actions
- Simple integration tests

---

## Production Use Cases

### Graceful Shutdown

```rust
async fn shutdown_handler(store: Store<MyState, MyAction, MyEnv, MyReducer>) {
    info!("Shutdown signal received");

    // Send shutdown action
    let handle = store.send_cascading(Action::Shutdown).await;

    // Wait for all cascading effects (cleanup, save state, etc.)
    match handle.wait_with_timeout(Duration::from_secs(30)).await {
        Ok(()) => info!("Graceful shutdown complete"),
        Err(e) => warn!("Shutdown timeout: {}", e),
    }
}
```

### HTTP Handler

```rust
async fn create_order_handler(
    store: Store<OrderState, OrderAction, OrderEnv, OrderReducer>,
    order: Order,
) -> Result<Response> {
    // Send order creation action
    let handle = store.send(OrderAction::Create(order)).await;

    // Wait for direct effects (DB save, validation)
    handle.wait_with_timeout(Duration::from_secs(10)).await?;

    // Query resulting state
    let order_id = store.state(|s| s.last_order_id).await;

    Ok(Response::Created(order_id))
}
```

### Health Check

```rust
async fn health_check(
    store: Store<AppState, AppAction, AppEnv, AppReducer>
) -> HealthStatus {
    let handle = store.send(AppAction::HealthCheck).await;

    match handle.wait_with_timeout(Duration::from_secs(1)).await {
        Ok(()) => HealthStatus::Healthy,
        Err(_) => HealthStatus::Degraded,
    }
}
```

### Fire-and-Forget (Metrics, Logs)

```rust
async fn log_metric(store: Store<...>, metric: Metric) {
    // Just drop the handle - we don't care when it completes
    let _handle = store.send(Action::LogMetric(metric)).await;
}
```

---

## Implementation Phases

### Phase A: Core Infrastructure (90-120 min)

**Goal**: Add EffectHandle, EffectTracking, TestStore with all support types

**Files**: `runtime/src/lib.rs`, `core/src/lib.rs`, `testing/src/lib.rs`

**Tasks**:
1. Define `TrackingMode` enum in core
2. Define `EffectTracking<A>`, `FeedbackDestination<A>`, `DecrementGuard<A>` in runtime (internal)
3. Define `EffectHandle` struct in runtime (public, Clone)
4. Define `EffectTimeoutError` in runtime
5. Add `tokio::sync::watch` dependency (for completion notification)
6. Implement `EffectHandle::completed()`, `wait_with_timeout()`, `is_complete()`
7. Change `Store::send()` signature to return `EffectHandle`
8. Add `Store::send_cascading()`
9. Add internal `Store::send_internal()` method
10. Add internal `Store::send_with_queue()`, `send_cascading_with_queue()` methods
11. Update `execute_effect_internal()` with panic guards, Sequential algorithm, feedback dispatch
12. Define `TestStore` struct in testing crate
13. Define `ExpectedActions` trait with impls for `A`, `Vec<A>`
14. Define `TestStoreError` enum
15. Implement `TestStore::send()`, `receive()`, `receive_after()`, `receive_unordered()`, `receive_unordered_after()`, `state()`, `assert_no_pending_actions()`
16. Implement `Drop for TestStore` (panic on unprocessed actions)
17. Write tests for TestStore itself (8-10 test cases)

**Success**: Code compiles, TestStore tests pass, existing Store tests fail (need migration).

---

### Phase B: Test Migration (90-120 min)

**Goal**: Update all tests to use EffectHandle or TestStore with receive_after()

**Files**:
- `runtime/src/lib.rs` (integration tests)
- `examples/counter/tests/integration_test.rs`

**Tasks**:
1. Update `test_store_creation` - no changes needed (no effects)
2. Update `test_send_action` - add `handle.wait_with_timeout()`
3. Update `test_effect_future` - use TestStore, add `start_paused`, use `receive_after()`
4. Update `test_effect_delay` - use TestStore, add `start_paused`, use `receive_after()`
5. Update `test_effect_parallel` - use TestStore, test `receive_unordered_after()` with Vec
6. Update `test_effect_sequential` - use TestStore, test ordered `receive_after()` with Vec
7. Update `test_state_access` - add `handle.wait_with_timeout()`
8. Update `test_concurrent_increments` - add handles (Store is fine, no effect observation needed)
9. Update `test_store_clone` - add handle
10. Update counter integration tests - mix of Store (simple) and TestStore (when observing effects)
11. Debug any async timing issues that emerge
12. Verify all tests pass with `start_paused`

**Pattern for simple tests (Store)**:
```rust
// After - Store with handle
#[tokio::test]
async fn test() {
    let store = Store::new(reducer, env, state);
    let handle = store.send(action).await;
    handle.wait_with_timeout(Duration::from_secs(1)).await.unwrap();
    assert_eq!(...);
}
```

**Pattern for effect observation (TestStore)**:
```rust
// After - TestStore with receive_after
#[tokio::test(start_paused = true)]
async fn test() {
    let store = TestStore::new(reducer, env, state);
    let h = store.send(action).await;
    tokio::time::advance(Duration::from_millis(100)).await;
    store.receive_after(Action::Expected, h).await.unwrap();
    assert_eq!(...);
    store.assert_no_pending_actions();
}
```

**Success**: All tests pass, no arbitrary sleeps, effect-producing actions are observed.

---

### Phase C: Cleanup & Documentation (30-45 min)

**Goal**: Remove `Effect::None` pattern, update all documentation

**Files**: Multiple

**Tasks**:
1. Replace `vec![Effect::None]` with `vec![]` in:
   - `runtime/src/lib.rs` (tests)
   - `examples/counter/src/lib.rs` (reducer)
   - `examples/counter/tests/integration_test.rs`
2. Update `Store::send()` documentation (fire-and-forget, handle usage)
3. Update `Store::send_cascading()` documentation
4. Update `EffectHandle` documentation (Clone, completed(), cascading loop)
5. Update `EffectTracking` internal comments
6. Document panic handling (DecrementGuard)
7. Document TestStore Drop behavior
8. Add examples to docs (graceful shutdown, HTTP handlers, test patterns)
9. Update CLAUDE.md (new effect patterns, TestStore usage)
10. Update README.md (if needed)
11. Run `./scripts/check.sh` to verify all quality checks pass

**Success**: No `vec![Effect::None]` in codebase, comprehensive docs, all checks pass.

---

## Effect::None Cleanup

Replace all instances:

```rust
// Before
match action {
    Action::Simple => {
        state.field = value;
    }
}
vec![Effect::None]

// After
match action {
    Action::Simple => {
        state.field = value;
    }
}
vec![]  // or Vec::new()
```

**Files to check**:
```bash
rg "Effect::None" --type rust
```

---

## Success Criteria

After implementation:

1. ✅ `Store::send()` returns `EffectHandle`
2. ✅ `Store::send_cascading()` available for cascade tracking
3. ✅ `TestStore` implemented with effect queue, `receive()`, and `receive_unordered()`
4. ✅ `ExpectedActions` trait with type-based semantics (A, Vec)
5. ✅ All tests pass without arbitrary `tokio::time::sleep()`
6. ✅ Tests use `#[tokio::test(start_paused = true)]` for virtual time
7. ✅ Effect execution is uniform (all effects tracked per-handle)
8. ✅ No `vec![Effect::None]` in codebase
9. ✅ Test suite runs in < 100ms (no real sleeps)
10. ✅ Documentation explains handle usage, modes, and TestStore patterns
11. ✅ Production use cases documented (shutdown, HTTP handlers)

---

## Estimated Time

- **Phase A** (Core Infrastructure + TestStore): 60 minutes
- **Phase B** (Test Migration): 75 minutes
- **Phase C** (Cleanup + Documentation): 30 minutes

**Total**: ~2 hours 45 minutes

---

## Risk Assessment

### Low Risk
- Adding EffectHandle type (new code)
- TestStore implementation (new code, isolated)
- Documentation updates
- Effect::None cleanup

### Medium Risk
- Changing `send()` signature (breaks existing code, but easy to fix)
- Effect tracking logic (must test thoroughly)
- Cascading mode implementation (recursive tracking is complex)
- TestStore queued mode (Store needs to support both auto-feedback and queued)
- ExpectedActions trait (trait bounds must work for all types)

### Mitigation
- Implement incrementally, run tests after each phase
- Test cascading mode thoroughly with multi-level workflows
- Test all ExpectedActions impls (single, Vec, HashSet) separately
- Ensure Store queued mode doesn't affect production Store behavior
- Add debug logging to track handle lifecycle during development

---

## Open Questions

### 1. Should we add a convenience method?

```rust
impl EffectHandle {
    /// Wait with default timeout (30 seconds)
    pub async fn wait(self) -> Result<(), EffectTimeoutError> {
        self.wait_with_timeout(Duration::from_secs(30)).await
    }
}
```

**Recommendation**: Yes - 30s is reasonable for most tests, but always allow override.

---

### 2. Should EffectHandle be Clone?

Currently not cloneable. Should multiple callers be able to wait on same handle?

```rust
let handle = store.send(action).await;
let h1 = handle.clone();
let h2 = handle.clone();

tokio::join!(h1.wait(), h2.wait());  // Both wait for same effects
```

**Recommendation**: Not needed for Phase 1, add if use case emerges.

---

### 3. Should we add handle.await_action()?

Wait until a specific action is produced:

```rust
let handle = store.send_cascading(Action::Start).await;
handle.await_action(|a| matches!(a, Action::Complete)).await;
```

**Recommendation**: Defer to Phase 2 - adds complexity, unclear benefit.

---

### 4. Effect::None - Keep or Remove?

**Option A**: Keep in enum, discourage use (return empty vec)
**Option B**: Remove entirely (breaking change)

**Recommendation**: Keep for Phase 1 (minimal disruption), revisit in Phase 2.

---

## Related Documents

- `PHASE1_REVIEW.md` - Issues this plan addresses
- `specs/architecture.md` - Effect system design
- `plans/implementation-roadmap.md` - Overall project phases

---

## Design Rationale Summary

### Why EffectHandle instead of global counter?
- **Isolation**: Each operation tracked independently (no race conditions)
- **Composability**: Can wait for multiple specific operations
- **Production-friendly**: Meaningful for graceful shutdown, HTTP handlers
- **Rust-like**: Matches async/await patterns from other languages

### Why two tracking modes?
- **Tests**: Often want step-by-step assertions (direct mode)
- **Workflows**: Some need full completion guarantee (cascading mode)
- **Flexibility**: User chooses appropriate level per use case

### Why TestStore with effect queue?
- **Observability**: Need to assert on intermediate actions produced by effects
- **Control**: Pause feedback loop to inspect state between cascading actions
- **Debuggability**: Know exactly which effect produced which action
- **TCA-inspired**: Proven pattern from Swift's Composable Architecture
- **Clear semantics**: `receive()` = ordered, `receive_unordered()` = unordered (explicit method names)
- **Duplicate support**: Vec-based approach handles duplicate actions correctly

### Why configurable timeouts?
- **Use-case dependent**: LLM calls (60s) vs unit tests (1s) vs batch jobs (minutes)
- **No magic numbers**: Arbitrary defaults break real use cases
- **Explicit**: Forces user to think about appropriate timeout

---

## Next Steps

1. ✅ Review and approve this plan
2. Implement Phase A (core infrastructure)
3. Run tests to verify signature changes
4. Implement Phase B (test migration)
5. Verify all tests pass
6. Implement Phase C (cleanup)
7. Final test suite run
8. Update PHASE1_REVIEW.md status
9. Commit Phase 1
