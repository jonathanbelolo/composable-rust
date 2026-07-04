# Durable Sagas

Long-running saga orchestration in `composable-rust-next`: per-call durability,
crash recovery, cooperative cancellation, and operations. This guide covers the
opt-in **durable mode** (`Handler::handle_durable` / `Handler::resume`).

## 1. When to use durable mode

The default saga loop (`Handler::handle`) is **batch-lockstep**: a
`Continue { events, calls }` dispatches every call, waits for the *whole batch*,
and feeds one combined result back. That is right for short orchestrations
(reserve → charge → confirm), and wrong when calls are long, numerous, or
expensive:

| | Batch (`handle`) | Durable (`handle_durable`) |
|---|---|---|
| Durability granularity | the whole batch | **one call** |
| Crash cost | every unpersisted completed call re-runs | only calls literally in flight |
| Pacing | the slowest call in the batch | each completion processed as it lands |
| Concurrency window | fixed waves | saga-controlled sliding window |
| Recovery after restart | none (hand-rolled) | `resume()` + registry sweep |

Rule of thumb: minutes-long calls, fan-outs beyond a handful, or runs that must
survive deploys → durable mode.

## 2. Concepts

**The call journal lives in the saga's own event stream.** Every cycle's append
interleaves domain events with framework marker events:

- `$saga.call_dispatched` — carries the bincode-serialized call (for
  re-dispatch) and the stream ID. The marker's **stream version is the call's
  `CallId`** — deterministic and replayable.
- `$saga.call_completed` — records that a call's result was consumed by a
  feedback cycle *in the same append* as whatever the saga emitted in response.
  Results are never journaled; your domain events record what you chose to keep.

Because completion markers commit atomically with the saga's reaction,
"the completion is durable" and "the saga's response is durable" are the same
write. Recovery computes `outstanding = dispatched \ completed` from a plain
stream load.

**Event types beginning with `$` are reserved.** Domain `event_type_name`s must
never start with `$`; projectors that decode domain events skip framework types
(`is_framework_event_type`).

**Run outcomes** (`DurableOutcome`, non-exhaustive):

- `Completed { version, event_count }` — the saga returned `Done` with nothing
  outstanding (`event_count` counts domain events only).
- `Query(response)` — the initial input was a query (`Respond`).
- `Suspended { outstanding }` — cancelled or drained; the journal lists the
  outstanding calls, `resume` re-dispatches exactly them.
- `NoOutstandingCalls` — returned only by `resume`: nothing to do.

**Cancel ≡ crash ≡ resumable.** Aborting a run, draining it, a watchdog
timeout, a broadcast failure, and a process crash all leave the same thing
behind: a consistent journal whose outstanding entries `resume` re-dispatches.
There is one recovery path, and everything funnels into it.

## 3. Implementing `DurableBusinessLogic`

```rust,ignore
impl BusinessLogic for SpecPipelineSaga {
    type Call = SpecCall;          // must be Serialize + DeserializeOwned
    type CallResult = SpecResult;  // failures are VARIANTS here, not errors
    // ... the usual associated types
}

impl DurableBusinessLogic for SpecPipelineSaga {
    // Persisted in the registry; recovery sweeps filter by it so another
    // saga type's journal is never fed to this decoder. Never change it
    // once instances exist.
    const LOGIC_TAG: &'static str = "spec-pipeline";

    // Feedback must be constructible from the stream ID alone — after a
    // crash there is no prior input to thread a key from. You defined the
    // stream naming in stream_id(), so inverting it is knowledge you have.
    fn completion_input(stream_id: &StreamId, call_id: CallId, result: SpecResult) -> SpecInput {
        let run_id = parse_run_id(stream_id);
        SpecInput::Completion { run_id, call_id, result }
    }
}
```

**The window top-up pattern** — the saga controls its own concurrency; the
framework runs whatever is outstanding:

```rust,ignore
match input {
    SpecInput::Start { .. } => Ok(BusinessResult::Continue {
        events: vec![SpecEvent::PhaseStarted { .. }],
        calls: first_window(WINDOW_SIZE),          // e.g. keep 5 in flight
    }),
    SpecInput::Completion { result, .. } => {
        // one completion = one persisted cycle
        let events = vec![SpecEvent::ChunkDone { .. }];
        let calls = next_call().into_iter().collect(); // top up, or [] to wait
        if all_done() {
            Ok(BusinessResult::Done(final_events))
        } else {
            Ok(BusinessResult::Continue { events, calls })
        }
    }
}
```

**Invariants** (violations are hard errors, raised *before* persisting):

1. Every input of one instance maps to the same `stream_id`.
2. `Done` with calls outstanding → `DoneWithOutstandingCalls`. Consume every
   completion first. Parking at a human-review gate = `Done` with a balanced
   journal; a user command re-enters later.
3. `Continue { calls: [] }` with nothing outstanding → `SagaStuck` (nothing can
   ever wake you).
4. `Respond` outside the initial cycle → `RespondInFeedbackCycle`.
5. Call failures are `CallResult` variants your `process` handles. A
   `process()` **error** on a feedback cycle deliberately leaves the completion
   un-journaled — the call re-runs on every resume, forever, if the error is
   deterministic.

## 4. The executor

Durable mode needs single-call execution; the handler owns the
unordered-completion machinery:

```rust,ignore
impl UnitCallExecutor<SpecCall, SpecResult> for GpuExecutor {
    async fn execute_one(&self, call: SpecCall) -> SpecResult {
        match self.run_inference(call).await {
            Ok(output) => SpecResult::Ok(output),
            Err(e) => SpecResult::Failed { reason: e.to_string() }, // a VARIANT
        }
    }
}

// The Handler struct also wants the batch trait — it's a one-liner:
impl CallExecutor<SpecCall, SpecResult> for GpuExecutor {
    async fn execute(&self, calls: Vec<SpecCall>) -> Vec<SpecResult> {
        futures::future::join_all(calls.into_iter().map(|c| self.execute_one(c))).await
    }
}
```

## 5. Wiring: atomic persistence is required for trustworthy recovery

The registry (`saga_call_journal` table) is a projection of the `$saga.*`
markers. On the **atomic path** the journal rows commit in the same transaction
as the events, so the recovery sweep can trust it. On the non-atomic path a
crash between append and project loses journal rows permanently — see §8.

```rust,ignore
let store = PostgresEventStore::from_pool(pool.clone());
store.run_migrations().await?; // includes saga_call_journal

let journal = SagaJournalProjector::new(pool.clone(), SpecPipelineSaga::LOGIC_TAG);
let atomic = PostgresAtomicPersist::new(
    store.clone(),
    (my_domain_projector, journal), // tuple: both run in the append transaction
);

// environment: atomic_persist() returns Some(&atomic); event_store() returns
// the SAME store the atomic persist wraps.
let handler = Arc::new(HandlerBuilder::new(SpecPipelineSaga)
    .call_executor(gpu_executor)
    .query_fetcher(spec_fetcher)
    .environment(env)
    .max_total_calls(500)                       // lifetime dispatch cap
    .max_call_duration(Duration::from_secs(30 * 60)) // stuck-call watchdog
    .build());
```

## 6. Running

```rust,ignore
let cancel = CancellationToken::new();
match handler.handle_durable(SpecInput::Start { run_id }, cancel.clone()).await? {
    DurableOutcome::Completed { .. } => { /* phase done */ }
    DurableOutcome::Suspended { outstanding } => { /* deploy/cancel; resume later */ }
    other => { /* ... */ }
}
```

- `cancel.cancel()` — **abort**: in-flight calls dropped immediately, run
  suspends. Their journal entries stay outstanding.
- `cancel.drain()` — **graceful**: no new calls start; in-flight ones complete
  and persist their cycles; top-ups requested while draining are journaled but
  not started; then the run suspends. Kinder to expensive calls; waits for the
  slowest in-flight one. `cancel()` upgrades a drain.
- `max_call_duration` — watchdog: if the oldest in-flight call exceeds it, the
  run errors with `CallStuck { call_id, .. }` (crash-equivalent, resumable).
  Alert on it. Note it aborts sibling in-flight work too; that work is re-paid
  on resume.

**Metrics** (all labeled `logic_tag`): `saga.durable.calls_dispatched.total`,
`calls_completed.total`, `calls_resumed.total`, `calls_in_flight` (gauge),
`cycle.duration_seconds` (histogram), `runs.total` (label `outcome` =
`completed|suspended|query|no_outstanding|error`). Tracing spans:
`saga.durable_run` (per run, `mode = start|resume`) and `saga.durable_cycle`
(per completion cycle); version-conflict retries log at `warn`.

## 7. Recovery

**Parked ≠ orphaned.** "Needs recovery" ≡ "has outstanding journal entries" —
nothing else. A saga parked at a review gate returned `Done` with a balanced
journal and never appears in the sweep; `resume` on it is a no-op
(`NoOutstandingCalls`).

Startup (and optionally periodic) sweep:

```rust,ignore
let registry = PostgresSagaRegistry::new(pool.clone());
let report = Arc::clone(&handler)
    .recovery_sweep(&registry, &CancellationToken::new())
    .await?;
tracing::info!(
    resumed = report.resumed.len(),
    skipped = report.no_outstanding.len(),
    failed = report.failed.len(),
    "recovery sweep done"
);
for (stream, error) in &report.failed {
    tracing::error!(%stream, %error, "saga resume failed"); // alert on repeats
}
```

`recovery_sweep` queries by the **type's** `LOGIC_TAG`, so it cannot feed
another saga type's journal to your handler. `resume` is idempotent and safe
against stale registry rows.

**Multi-node:** a concurrent double-resume is *safe* (at-least-once: persists
serialize via optimistic concurrency, duplicate completion markers are
tolerated) but wasteful — each in-flight call may run twice. Skip contended
instances with `SagaSweepLock`:

```rust,ignore
match SagaSweepLock::try_acquire(&pool, &record.stream_id).await? {
    None => continue, // another node owns this instance
    Some(lock) => {
        let outcome = handler.resume(&record.stream_id, token.clone()).await;
        lock.release().await?;
    }
}
```

The lock detaches its connection from the pool before locking, so it **cannot
leak into pooled sessions** — dropping the guard (even on panic) closes the
session and PostgreSQL frees the lock.

**At-least-once:** a call whose completion was never journaled re-executes on
resume. If re-execution isn't naturally idempotent, add payload-hash dedup on
your side of the executor.

## 8. Operational sharp edges

- **Poison loop**: a *deterministic* `process()` error on a feedback cycle
  makes every resume re-pay the call and fail again. Call outcomes belong in
  `CallResult` variants. Alert on repeated `SweepReport::failed` entries for
  one instance.
- **Broadcast is best-effort after persist**: a bus failure ends the run
  crash-equivalently; `resume` recovers the run but does **not** re-broadcast
  already-persisted events.
- **Non-atomic projection loss**: without atomic persist, a crash between
  append and project permanently desyncs the journal table in either
  direction — an instance the sweep can't see, or a zombie row the sweep
  retries forever (harmlessly: `resume` returns `NoOutstandingCalls`). Fix
  direction and symptom with `SagaJournalProjector::rebuild_from_store(prefix)`,
  which replays the `$saga.*` markers through the same idempotent upserts.
- **Shared migrations**: `/migrations` is embedded by both PostgreSQL crates;
  once `saga_call_journal` is applied, binaries compiled before it fail
  `run_migrations()` — rebuild before deploying against a migrated database.
- **Timestamps** are stamped from the injected `Clock` and truncated to
  microseconds, so PostgreSQL `TIMESTAMPTZ` round-trips exactly.

## 9. End to end: the crash-resume drill

The contract in one scenario (mirrored by
`postgres-next/tests/saga_recovery_tests.rs`, section 3):

1. A saga dispatches a window of 3 calls; markers journal atomically with
   `PhaseStarted`.
2. Calls 1 and 2 complete: each drives its own persisted cycle (`ChunkDone` +
   `$saga.call_completed`, one append each). Call 3 is still running.
3. **The process dies.**
4. On restart, the sweep finds the instance (`outstanding = 1`), and `resume`
   re-dispatches **exactly call 3** — the two completed calls are never
   re-paid.
5. Call 3 completes, the saga returns `Done`, the journal balances, the
   registry empties.

Crash cost: the one in-flight call. That is the whole point.
