//! Durable incremental call execution for long-running sagas
//!
//! This module provides the opt-in **durable mode** for sagas whose calls are
//! long (minutes), numerous (hundreds per run), or expensive to re-execute.
//! It replaces the batch call loop's barrier semantics with **per-call
//! durability**:
//!
//! - Each call's completion is delivered to the saga as its own feedback →
//!   `process()` → persist cycle. Events emitted in that cycle are persisted
//!   before further completions are consumed, so a crash costs at most the
//!   calls that were literally in flight.
//! - The handler journals framework marker events (`$saga.call_dispatched` /
//!   `$saga.call_completed`) **in the saga's own event stream**, in the same
//!   append as the cycle's domain events. Recovery computes
//!   `dispatched \ completed` from the stream alone.
//! - The saga tops up its own concurrency window: on each completion it may
//!   return `Continue { events, calls: new_calls }` to keep a constant number
//!   of calls in flight, or `calls: []` to keep waiting.
//!
//! # Reserved event-type namespace
//!
//! Event types beginning with `$` are reserved for the framework. Domain
//! event type names (from
//! [`BusinessLogic::event_type_name`](crate::BusinessLogic::event_type_name))
//! must never start with `$`.
//!
//! Marker events flow through the normal persist → project → broadcast path,
//! so projections and bus subscribers observe saga progress as ordinary
//! events. Projectors that decode domain events must skip framework types
//! (check [`is_framework_event_type`]).
//!
//! # Invariants durable sagas must uphold
//!
//! - **Stream-ID stability**: every input of one saga instance — the
//!   initial command and every [`completion_input`](DurableBusinessLogic::completion_input)
//!   — must map to the same [`BusinessLogic::stream_id`](crate::BusinessLogic::stream_id).
//!   The loop derives the stream once, at entry.
//! - **Fetchers preserve input fields**: on a version-conflict retry the
//!   prepared input is re-fetched; a `QueryFetcher` that drops or rewrites
//!   input fields while populating `fetched` would corrupt the retried
//!   cycle (this matches the batch path's existing contract).
//! - **Call failures are `CallResult` variants**, never `process()` errors:
//!   a `process()` error on a feedback cycle deliberately leaves the
//!   completion un-journaled, so the call re-runs on every resume.
//! - **Broadcast is best-effort after persist** (as in batch mode): a
//!   broadcast failure kills the run crash-equivalently; the journal stays
//!   consistent and `resume` recovers, but does **not** re-broadcast
//!   already-persisted events.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tracing::Instrument;

use crate::{
    BusinessLogic, BusinessResult, CallExecutor, CancellationToken, EventMetadata, EventStore,
    EventStoreError, FetchResult, Handler, HandlerEnvironment, HandlerError, InvocationContext,
    QueryFetcher, SagaRegistry, SerializationError, SerializedEvent, StreamId, Subject, SubjectId,
    UnitCallExecutor, Version,
};

/// Event type of the framework marker persisted when a saga call is
/// dispatched. The marker's **stream version is the call's [`CallId`]**.
pub const CALL_DISPATCHED_EVENT_TYPE: &str = "$saga.call_dispatched";

/// Event type of the framework marker persisted when a saga call's result
/// has been consumed by a feedback cycle.
pub const CALL_COMPLETED_EVENT_TYPE: &str = "$saga.call_completed";

/// Whether an event type belongs to the reserved framework namespace (`$`).
///
/// Domain projections and state rebuilding skip these events.
#[must_use]
pub fn is_framework_event_type(event_type: &str) -> bool {
    event_type.starts_with('$')
}

/// Identifier of one dispatched saga call.
///
/// A `CallId` is the stream version of the call's `$saga.call_dispatched`
/// marker event — deterministic, unique per stream, and recoverable from a
/// plain stream load (the store stamps versions on loaded events).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct CallId(u64);

impl CallId {
    /// Create a call ID from a raw value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Create a call ID from the dispatched marker's stream version.
    #[must_use]
    pub const fn from_version(version: Version) -> Self {
        Self(version.as_u64())
    }

    /// The raw value (the dispatched marker's stream version).
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for CallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "call@v{}", self.0)
    }
}

/// Payload of a `$saga.call_dispatched` marker event.
///
/// Carries the bincode-serialized call so recovery can re-dispatch it
/// without re-running any business logic. The marker deliberately does
/// **not** contain its own [`CallId`]: the ID is the marker's stream
/// version, which is unknowable before the append and self-describing
/// after a load (the store stamps versions).
///
/// `stream_id` is embedded because projectors and event-bus subscribers
/// receive [`SerializedEvent`]s without stream context — the payload is the
/// only place they can learn which saga instance the marker belongs to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallDispatched {
    /// The saga instance's stream ID.
    pub stream_id: String,

    /// Bincode-serialized call payload, for re-dispatch on resume.
    pub call: Vec<u8>,
}

/// Payload of a `$saga.call_completed` marker event.
///
/// Marks that the call's result was consumed by a feedback cycle whose
/// events persisted in the same append as this marker. The result itself is
/// **not** journaled — the saga's own domain events record whatever it chose
/// to keep.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallCompleted {
    /// The saga instance's stream ID.
    pub stream_id: String,

    /// The completed call (the stream version of its dispatched marker).
    pub call_id: CallId,
}

/// Business logic that can run in durable mode.
///
/// Extends [`BusinessLogic`] with what per-call durability needs:
///
/// - **Serializable calls** (`Call: Serialize + DeserializeOwned`): the
///   handler journals each dispatched call's payload in a
///   `$saga.call_dispatched` marker so recovery can re-dispatch it without
///   re-running business logic.
/// - **A stable logic tag** ([`LOGIC_TAG`](Self::LOGIC_TAG)): recovery
///   sweeps filter the saga registry by this tag so a stream journaled by
///   one saga type is never fed to another type's handler (whose `Call`
///   deserialization would read garbage).
/// - **Crash-safe feedback construction**
///   ([`completion_input`](Self::completion_input)): after a restart there
///   is no prior input to thread a correlation key from, so the feedback
///   input for a completion must be constructible from the stream ID alone.
///
/// # Durable feedback is uniform
///
/// Durable mode **never** calls
/// [`feedback_input`](BusinessLogic::feedback_input) /
/// [`feedback_input_from`](BusinessLogic::feedback_input_from). Every
/// completion — in a live run and after a resume alike — is delivered
/// through [`completion_input`](Self::completion_input), so in-run and
/// post-resume behavior are identical by construction.
pub trait DurableBusinessLogic: BusinessLogic
where
    Self::Call: Serialize + DeserializeOwned,
{
    /// Stable identifier for this saga type.
    ///
    /// Persisted in the saga registry (keyed per journal row) and used by
    /// recovery sweeps to select only streams this logic can decode. Like
    /// event type names, it must never change once instances exist in
    /// production.
    const LOGIC_TAG: &'static str;

    /// Build the feedback input for one completed call.
    ///
    /// Called once per completion with the saga's stream ID, the completed
    /// call's ID, and its result. The saga's stream naming is defined by
    /// [`stream_id`](BusinessLogic::stream_id), so extracting the typed
    /// correlation key (e.g. a saga UUID) from `stream_id` is application
    /// knowledge the implementation already has.
    ///
    /// The returned input flows through the normal
    /// `QueryFetcher` fetch, which supplies the saga's current state from
    /// its projection — exactly as for any other input.
    fn completion_input(
        stream_id: &StreamId,
        call_id: CallId,
        result: Self::CallResult,
    ) -> Self::Input;
}

/// The call journal folded out of a saga stream.
///
/// Produced by [`scan_journal`]; consumed by recovery to re-dispatch exactly
/// the calls whose completions were never persisted.
#[derive(Debug, Default)]
pub struct JournalState {
    /// Dispatched-but-uncompleted calls: `CallId` → serialized call payload.
    pub outstanding: BTreeMap<CallId, Vec<u8>>,

    /// Total number of `$saga.call_dispatched` markers seen (completed ones
    /// included). Seeds the `max_total_calls` guard on resume so the cap
    /// means "calls this instance has ever dispatched", stable across
    /// crash/resume cycles.
    pub dispatched_count: u64,
}

/// Fold a loaded stream into its call-journal state.
///
/// Scans for the framework marker events and computes
/// `dispatched \ completed` with set semantics:
///
/// - Duplicate completions (possible under at-least-once double-resume
///   races) are silently tolerated.
/// - A completion for a **never-dispatched** call ID is logged as a warning
///   and ignored — that distinguishes benign duplicates from journal
///   corruption.
/// - Non-marker events are skipped.
///
/// # Errors
///
/// Returns [`SerializationError::Decode`] if a marker payload is
/// undecodable or a dispatched marker lacks a stamped stream version. These
/// are framework-authored bytes: tolerating them would silently drop an
/// outstanding call and violate journal exactness.
pub fn scan_journal(events: &[SerializedEvent]) -> Result<JournalState, SerializationError> {
    let mut outstanding: BTreeMap<CallId, Vec<u8>> = BTreeMap::new();
    let mut completed: HashSet<CallId> = HashSet::new();
    let mut dispatched_count: u64 = 0;

    for event in events {
        match event.event_type.as_str() {
            CALL_DISPATCHED_EVENT_TYPE => {
                let version = event.version.ok_or_else(|| {
                    SerializationError::Decode(format!(
                        "{CALL_DISPATCHED_EVENT_TYPE}: marker missing stream version"
                    ))
                })?;
                let marker: CallDispatched = bincode::deserialize(&event.payload).map_err(|e| {
                    SerializationError::Decode(format!(
                        "{CALL_DISPATCHED_EVENT_TYPE} at {version}: {e}"
                    ))
                })?;
                let call_id = CallId::from_version(version);
                dispatched_count += 1;
                if !completed.contains(&call_id) {
                    outstanding.insert(call_id, marker.call);
                }
            },
            CALL_COMPLETED_EVENT_TYPE => {
                let marker: CallCompleted = bincode::deserialize(&event.payload).map_err(|e| {
                    SerializationError::Decode(format!(
                        "{CALL_COMPLETED_EVENT_TYPE} at {:?}: {e}",
                        event.version
                    ))
                })?;
                let was_outstanding = outstanding.remove(&marker.call_id).is_some();
                if !was_outstanding && !completed.contains(&marker.call_id) {
                    tracing::warn!(
                        call_id = %marker.call_id,
                        stream_id = %marker.stream_id,
                        "completion marker for a call that was never dispatched; ignoring"
                    );
                }
                completed.insert(marker.call_id);
            },
            _ => {},
        }
    }

    Ok(JournalState {
        outstanding,
        dispatched_count,
    })
}

/// Outcome of a durable saga run.
///
/// Returned by `Handler::handle_durable` (and later `Handler::resume`).
/// Deliberately distinct from [`HandleResult`](crate::HandleResult): durable
/// runs have outcomes the batch path cannot produce.
#[derive(Debug, Clone)]
pub enum DurableOutcome<R = ()> {
    /// The saga returned `Done` with zero outstanding calls.
    Completed {
        /// The stream version after the final persist.
        ///
        /// Parity quirk inherited from the batch path: an initial `Done` with
        /// no events under a version-less fetcher reports `v0` even if the
        /// stream is non-empty (nothing was persisted, so no true version was
        /// observed).
        version: Version,
        /// Total **domain** events persisted across the whole run (journal
        /// markers excluded).
        event_count: usize,
    },

    /// The initial input was a query (`Respond`); nothing was persisted.
    Query(R),

    /// The run was cancelled.
    ///
    /// In-flight calls were aborted (their futures dropped) and no further
    /// calls were dispatched. The journal still lists `outstanding` as
    /// dispatched-but-uncompleted, so a later `resume` re-dispatches exactly
    /// them: cancel ≡ crash ≡ resumable.
    Suspended {
        /// The calls that were outstanding at suspension.
        outstanding: Vec<CallId>,
    },

    /// Returned only by `resume`: the stream has no outstanding calls.
    ///
    /// The instance is parked at a gate, already completed, or the stream
    /// doesn't exist — deliberately indistinguishable here, because none of
    /// them are the recovery driver's business to act on.
    NoOutstandingCalls,
}

/// Mutable state threaded through one durable run.
struct RunState {
    /// Dispatched-but-uncompleted call IDs (mirrors the journal).
    outstanding: BTreeSet<CallId>,
    /// Dispatch instants of the calls whose futures are actually in flight
    /// (drain-deferred calls are absent). Drives the stuck-call watchdog.
    in_flight_since: BTreeMap<CallId, tokio::time::Instant>,
    /// Calls dispatched over the instance's lifetime (journal-seeded on
    /// resume); checked against `max_total_calls`.
    dispatched_total: u64,
    /// Domain events persisted this run (markers excluded).
    domain_events_total: usize,
    /// The expected version for the next persist.
    ///
    /// `None` until the first persist (the initial cycle trusts the
    /// fetcher's version); afterwards always the previous persist's returned
    /// final version. Feedback cycles must NOT trust the fetcher here:
    /// marker events advance the stream on every append, and a projection
    /// that derives its version from recognized domain events would go
    /// permanently stale — fetch supplies state, this field supplies the
    /// version.
    carried_version: Option<Version>,
    /// The initiating subject's ID, stamped as `origin_subject_id` on every
    /// cycle's events and markers.
    origin: Option<SubjectId>,
}

/// RAII guard for the `saga.durable.calls_in_flight` gauge.
///
/// Increments on dispatch, decrements on completion — and `Drop` decrements
/// whatever remains, so the gauge stays correct on **every** exit path:
/// completion, suspension, error, and the caller dropping the run future
/// mid-await (which no explicit code path could catch).
struct InFlightGauge {
    logic_tag: &'static str,
    count: u64,
}

impl InFlightGauge {
    const fn new(logic_tag: &'static str) -> Self {
        Self {
            logic_tag,
            count: 0,
        }
    }

    fn increment(&mut self) {
        self.count += 1;
        metrics::gauge!("saga.durable.calls_in_flight", "logic_tag" => self.logic_tag)
            .increment(1.0);
    }

    fn decrement(&mut self) {
        if self.count > 0 {
            self.count -= 1;
            metrics::gauge!("saga.durable.calls_in_flight", "logic_tag" => self.logic_tag)
                .decrement(1.0);
        }
    }
}

impl Drop for InFlightGauge {
    fn drop(&mut self) {
        if self.count > 0 {
            #[allow(clippy::cast_precision_loss)] // In-flight counts are tiny
            metrics::gauge!("saga.durable.calls_in_flight", "logic_tag" => self.logic_tag)
                .decrement(self.count as f64);
        }
    }
}

/// What one durable cycle decided.
enum CycleOutcome<C, R> {
    /// `Respond` on the initial cycle.
    Query(R),
    /// `Done` persisted (or the no-op initial `Done([])`).
    Completed { version: Version },
    /// `Continue` persisted; these calls (with their journal-assigned IDs)
    /// must now be dispatched.
    Dispatched { calls: Vec<(CallId, C)> },
}

/// Wrap a single call execution so its completion carries the [`CallId`].
///
/// Every dispatch site funnels through this one function so all in-flight
/// futures share a single opaque type (one return-position `impl Future` =
/// one type), which is what lets them live in one `FuturesUnordered`.
async fn dispatch_one<C, R, E>(executor: &E, call_id: CallId, call: C) -> (CallId, R)
where
    E: UnitCallExecutor<C, R>,
    C: Send,
    R: Send,
{
    (call_id, executor.execute_one(call).await)
}

impl<T, E, QF, Env> Handler<T, E, QF, Env>
where
    T: DurableBusinessLogic,
    T::Input: Clone,
    T::Call: Serialize + DeserializeOwned,
    E: CallExecutor<T::Call, T::CallResult> + UnitCallExecutor<T::Call, T::CallResult>,
    QF: QueryFetcher<T::Input, Env::Projections>,
    Env: HandlerEnvironment,
{
    /// Record the `saga.durable.runs.total` outcome counter.
    fn record_run_outcome(result: &Result<DurableOutcome<T::Response>, HandlerError<T::Error>>) {
        let outcome = match result {
            Ok(DurableOutcome::Completed { .. }) => "completed",
            Ok(DurableOutcome::Query(_)) => "query",
            Ok(DurableOutcome::Suspended { .. }) => "suspended",
            Ok(DurableOutcome::NoOutstandingCalls) => "no_outstanding",
            Err(_) => "error",
        };
        metrics::counter!(
            "saga.durable.runs.total",
            "logic_tag" => T::LOGIC_TAG,
            "outcome" => outcome
        )
        .increment(1);
    }

    /// Handle input in durable mode: per-completion feedback cycles with a
    /// crash-safe call journal.
    ///
    /// Differences from [`handle`](Handler::handle):
    ///
    /// - Calls execute **concurrently**; each completion is delivered to the
    ///   saga as its own `process()` → persist cycle (durability granularity
    ///   = one call). The saga tops up its window by returning new calls on
    ///   each completion.
    /// - Every cycle's append includes framework journal markers
    ///   (`$saga.call_dispatched` / `$saga.call_completed`) so recovery can
    ///   compute exactly which calls never completed.
    /// - The runaway guard is [`max_total_calls`](crate::HandlerBuilder::max_total_calls)
    ///   (lifetime dispatched calls), not `max_saga_iterations`.
    /// - Each cycle gets a fresh `max_retries` version-conflict budget (the
    ///   batch path shares one budget across the whole run).
    /// - Cancellation via [`CancellationToken::cancel`] (abort) suspends the
    ///   run: in-flight calls are dropped and stay outstanding in the
    ///   journal, so a later `resume` re-dispatches exactly them.
    ///   [`CancellationToken::drain`] is the graceful variant: no new calls
    ///   start, in-flight ones complete and journal their cycles, then the
    ///   run suspends (top-ups requested while draining are journaled but
    ///   not started — resume runs them). Both are observed between
    ///   completion cycles (latency = the in-progress cycle).
    ///
    /// # Errors
    ///
    /// Everything [`handle`](Handler::handle) returns, plus the durable-mode
    /// contract errors — all raised **before** persisting anything:
    ///
    /// - [`HandlerError::DoneWithOutstandingCalls`]: `Done` while calls are
    ///   outstanding.
    /// - [`HandlerError::SagaStuck`]: `Continue` with no calls and none
    ///   outstanding (nothing can ever wake the saga).
    /// - [`HandlerError::RespondInFeedbackCycle`]: `Respond` outside the
    ///   initial cycle.
    /// - [`HandlerError::TotalCallsExceeded`]: the lifetime dispatch cap.
    ///
    /// A `process()` error on a feedback cycle deliberately leaves that
    /// completion un-journaled (at-least-once): the call re-runs on resume.
    /// Call failures must therefore be `CallResult` variants the saga
    /// handles, not `process()` errors.
    pub async fn handle_durable(
        &self,
        input: T::Input,
        cancel: CancellationToken,
    ) -> Result<DurableOutcome<T::Response>, HandlerError<T::Error>> {
        let stream_id = T::stream_id(&input);
        let span = tracing::info_span!(
            "saga.durable_run",
            stream_id = %stream_id,
            logic_tag = T::LOGIC_TAG,
            mode = "start",
        );
        let result = self
            .handle_durable_inner(stream_id, input, cancel)
            .instrument(span)
            .await;
        Self::record_run_outcome(&result);
        result
    }

    async fn handle_durable_inner(
        &self,
        stream_id: StreamId,
        input: T::Input,
        cancel: CancellationToken,
    ) -> Result<DurableOutcome<T::Response>, HandlerError<T::Error>> {
        let origin = self
            .env
            .current_subject()
            .unwrap_or(Subject::System)
            .id()
            .cloned();

        let mut run = RunState {
            outstanding: BTreeSet::new(),
            in_flight_since: BTreeMap::new(),
            dispatched_total: 0,
            domain_events_total: 0,
            carried_version: None,
            origin,
        };

        let initial_calls = match self
            .durable_cycle(&stream_id, input, None, &mut run)
            .await?
        {
            CycleOutcome::Query(response) => return Ok(DurableOutcome::Query(response)),
            CycleOutcome::Completed { version } => {
                return Ok(DurableOutcome::Completed {
                    version,
                    event_count: run.domain_events_total,
                });
            },
            CycleOutcome::Dispatched { calls } => calls,
        };

        self.run_completion_loop(&stream_id, initial_calls, run, &cancel)
            .await
    }

    /// Resume a durable saga instance from its journal after a crash,
    /// cancellation, or process restart.
    ///
    /// Loads the stream, computes the outstanding calls
    /// (`dispatched \ completed` from the `$saga.*` markers), re-dispatches
    /// **exactly** those calls, and re-enters the normal completion loop. No
    /// state is rebuilt and no initial `process()` runs — the first feedback
    /// cycle's `QueryFetcher` fetch supplies the saga's state from its
    /// projection, exactly as in a live run, and feedback inputs come from
    /// [`completion_input`](DurableBusinessLogic::completion_input).
    ///
    /// # Parked ≠ orphaned
    ///
    /// A stream with **zero** outstanding calls returns
    /// [`DurableOutcome::NoOutstandingCalls`] immediately — nothing is
    /// processed, appended, or executed. A saga parked at a human-review
    /// gate (it returned `Done` and awaits a user command), an already
    /// completed saga, and a nonexistent stream are deliberately
    /// indistinguishable here: none of them are a recovery driver's business.
    /// This makes `resume` idempotent and safe to call from stale sweep
    /// lists.
    ///
    /// # At-least-once
    ///
    /// A call whose completion was never journaled re-executes. Two drivers
    /// resuming the same stream concurrently both re-dispatch the
    /// outstanding set (persists serialize via optimistic concurrency;
    /// duplicate completion markers are tolerated by set semantics) — safe
    /// but wasteful; use an advisory lock to avoid it (below).
    ///
    /// If `resume` for one instance **repeatedly** fails with a business
    /// error, the saga's `process` is returning `Err` for a call outcome —
    /// that must be a `CallResult` variant instead (see
    /// [`handle_durable`](Handler::handle_durable)). Alert on repeated
    /// resume failures rather than retrying forever.
    ///
    /// # Recovery driver (startup sweep)
    ///
    /// The framework deliberately ships no daemon; a sweep is ~15 lines.
    /// Enumerate instances with outstanding calls from a `SagaRegistry`
    /// implementation, filtered by YOUR logic's tag — never feed another
    /// saga type's streams to this handler:
    ///
    /// ```rust,ignore
    /// let registry = PostgresSagaRegistry::new(pool.clone());
    /// for rec in registry
    ///     .instances_with_outstanding_calls(MySagaLogic::LOGIC_TAG)
    ///     .await?
    /// {
    ///     let handler = Arc::clone(&handler);
    ///     tokio::spawn(async move {
    ///         match handler.resume(&rec.stream_id, CancellationToken::new()).await {
    ///             Ok(DurableOutcome::NoOutstandingCalls) => {} // parked or done: leave it alone
    ///             Ok(outcome) => tracing::info!(?outcome, stream = %rec.stream_id, "saga resumed"),
    ///             Err(e) => tracing::error!(%e, stream = %rec.stream_id, "resume failed"),
    ///         }
    ///     });
    /// }
    /// ```
    ///
    /// Re-run the sweep on an interval if you like — `resume` is idempotent,
    /// and an instance a live loop is already driving will conflict-and-lose
    /// harmlessly.
    ///
    /// **Multi-node deployments**: skip instances another node owns with a
    /// Postgres advisory lock — but advisory locks are **session-scoped**
    /// and sqlx pools reuse sessions. Take the lock on a connection held for
    /// the entire resume and release it deliberately: either a dedicated
    /// non-pooled `PgConnection::connect` (dropping it closes the session
    /// and releases the lock), or a held `PoolConnection` with an explicit
    /// `SELECT pg_advisory_unlock(...)` before returning it. Never run
    /// `pg_try_advisory_lock` bare on the pool — the lock leaks into the
    /// pooled session and silently blocks every future sweep of that stream.
    ///
    /// # Errors
    ///
    /// - [`HandlerError::Load`]: the stream could not be read.
    /// - [`HandlerError::Serialization`]: a journal marker or call payload
    ///   failed to decode. These are framework-authored bytes — most likely
    ///   the wrong logic type's handler was pointed at this stream (check
    ///   the `LOGIC_TAG` filter).
    /// - Everything [`handle_durable`](Handler::handle_durable) can return
    ///   once the loop is running.
    pub async fn resume(
        &self,
        stream_id: &StreamId,
        cancel: CancellationToken,
    ) -> Result<DurableOutcome<T::Response>, HandlerError<T::Error>> {
        let span = tracing::info_span!(
            "saga.durable_run",
            stream_id = %stream_id,
            logic_tag = T::LOGIC_TAG,
            mode = "resume",
        );
        let result = self.resume_inner(stream_id, cancel).instrument(span).await;
        Self::record_run_outcome(&result);
        result
    }

    async fn resume_inner(
        &self,
        stream_id: &StreamId,
        cancel: CancellationToken,
    ) -> Result<DurableOutcome<T::Response>, HandlerError<T::Error>> {
        let events = self
            .env
            .event_store()
            .load(stream_id, None)
            .await
            .map_err(HandlerError::Load)?;

        let journal = scan_journal(&events).map_err(HandlerError::Serialization)?;

        if journal.outstanding.is_empty() {
            return Ok(DurableOutcome::NoOutstandingCalls);
        }

        let mut initial_calls = Vec::with_capacity(journal.outstanding.len());
        for (call_id, payload) in journal.outstanding {
            let call: T::Call = bincode::deserialize(&payload).map_err(|e| {
                HandlerError::Serialization(SerializationError::Decode(format!(
                    "{CALL_DISPATCHED_EVENT_TYPE} {call_id}: call payload: {e}"
                )))
            })?;
            initial_calls.push((call_id, call));
        }

        metrics::counter!("saga.durable.calls_resumed.total", "logic_tag" => T::LOGIC_TAG)
            .increment(initial_calls.len() as u64);

        let run = RunState {
            outstanding: BTreeSet::new(),
            in_flight_since: BTreeMap::new(),
            // Lifetime guard continuity: count every call this instance has
            // ever dispatched (re-dispatching outstanding ones is not new).
            dispatched_total: journal.dispatched_count,
            domain_events_total: 0,
            // The stream's true version; feedback cycles must not trust a
            // possibly-marker-stale fetcher (see RunState::carried_version).
            carried_version: events.last().and_then(|e| e.version),
            // Recover the run initiator from the stream: durable mode stamps
            // origin_subject_id on every event from the initial cycle, so
            // the first stamped origin IS the initiating subject. Post-
            // resume events stay attributed to them, while author_id
            // reflects whoever runs the resume (typically System).
            origin: events.iter().find_map(|e| {
                e.metadata
                    .as_ref()
                    .and_then(|m| m.origin_subject_id.clone())
            }),
        };

        self.run_completion_loop(stream_id, initial_calls, run, &cancel)
            .await
    }

    /// Drive the unordered-completion loop until the saga finishes, errs, or
    /// is cancelled. `initial_calls` are dispatched first (their journal
    /// markers are already persisted).
    ///
    /// Drain semantics need no dedicated select arm — the token is polled at
    /// three decision points: loop entry (covers pre-drained tokens), every
    /// top-up (journal the calls, don't start them), and the
    /// in-flight-empty arm (suspend instead of erroring).
    async fn run_completion_loop(
        &self,
        stream_id: &StreamId,
        initial_calls: Vec<(CallId, T::Call)>,
        mut run: RunState,
        cancel: &CancellationToken,
    ) -> Result<DurableOutcome<T::Response>, HandlerError<T::Error>> {
        for (id, _) in &initial_calls {
            run.outstanding.insert(*id);
        }

        // A pre-cancelled OR pre-drained token suspends before any call
        // starts (abort implies draining); the dispatched markers are
        // journaled, so resume re-dispatches them.
        if cancel.is_draining() {
            return Ok(DurableOutcome::Suspended {
                outstanding: run.outstanding.iter().copied().collect(),
            });
        }

        let mut in_flight_gauge = InFlightGauge::new(T::LOGIC_TAG);
        let mut in_flight = FuturesUnordered::new();
        for (id, call) in initial_calls {
            run.in_flight_since.insert(id, tokio::time::Instant::now());
            in_flight_gauge.increment();
            in_flight.push(dispatch_one(&self.call_executor, id, call));
        }

        loop {
            // Stuck-call watchdog: fires when the OLDEST in-flight call
            // exceeds max_call_duration. Recomputed per iteration (arms are
            // rebuilt anyway); None disables the arm.
            let stuck_watch = self.max_call_duration.and_then(|max| {
                run.in_flight_since
                    .iter()
                    .min_by_key(|(_, since)| **since)
                    .map(|(id, since)| (*id, *since + max, max))
            });
            let watchdog_deadline =
                stuck_watch.map_or_else(tokio::time::Instant::now, |(_, deadline, _)| deadline);

            tokio::select! {
                biased;

                () = cancel.cancelled() => {
                    // ABORT: dropping `in_flight` aborts the calls; their
                    // journal entries stay outstanding → resumable.
                    return Ok(DurableOutcome::Suspended {
                        outstanding: run.outstanding.iter().copied().collect(),
                    });
                },

                next = in_flight.next() => {
                    // Bound and matched here, NOT in the arm pattern: a
                    // refutable arm pattern would silently disable the branch
                    // on None and park on cancelled() forever.
                    let Some((call_id, result)) = next else {
                        if cancel.is_draining() {
                            // DRAIN complete: everything in flight finished
                            // and journaled; top-ups deferred to resume.
                            return Ok(DurableOutcome::Suspended {
                                outstanding: run.outstanding.iter().copied().collect(),
                            });
                        }
                        // Defensive: the loop invariant (every cycle either
                        // finishes the run or keeps calls in flight) makes
                        // this unreachable outside drain.
                        return Err(HandlerError::SagaStuck);
                    };
                    // Remove BEFORE the cycle so the Done-check counts only
                    // *other* calls; this completion's marker persists in the
                    // same append as the saga's reaction to it.
                    run.outstanding.remove(&call_id);
                    run.in_flight_since.remove(&call_id);
                    in_flight_gauge.decrement();
                    metrics::counter!(
                        "saga.durable.calls_completed.total",
                        "logic_tag" => T::LOGIC_TAG
                    )
                    .increment(1);
                    let input = T::completion_input(stream_id, call_id, result);
                    match self
                        .durable_cycle(stream_id, input, Some(call_id), &mut run)
                        .await?
                    {
                        CycleOutcome::Completed { version } => {
                            return Ok(DurableOutcome::Completed {
                                version,
                                event_count: run.domain_events_total,
                            });
                        },
                        CycleOutcome::Dispatched { calls } => {
                            let draining = cancel.is_draining();
                            for (id, call) in calls {
                                run.outstanding.insert(id);
                                // DRAIN: the top-up's markers persisted in
                                // this cycle's append (journal exact), but
                                // the call is not started — resume runs it.
                                if !draining {
                                    run.in_flight_since
                                        .insert(id, tokio::time::Instant::now());
                                    in_flight_gauge.increment();
                                    in_flight.push(
                                        dispatch_one(&self.call_executor, id, call),
                                    );
                                }
                            }
                        },
                        // durable_cycle rejects Respond on feedback cycles.
                        CycleOutcome::Query(_) => {
                            return Err(HandlerError::RespondInFeedbackCycle);
                        },
                    }
                },

                // Ordered after completions (biased): a completion that is
                // ready always beats the watchdog.
                () = tokio::time::sleep_until(watchdog_deadline), if stuck_watch.is_some() => {
                    if let Some((call_id, _, max_call_duration)) = stuck_watch {
                        // Crash-equivalent: nothing persisted, all in-flight
                        // calls stay outstanding, resume re-dispatches them.
                        return Err(HandlerError::CallStuck {
                            call_id,
                            max_call_duration,
                        });
                    }
                },
            }
        }
    }

    /// One durable cycle: fetch → process → persist (events + journal
    /// markers, one append) → broadcast.
    ///
    /// `completed` is the call whose result produced this cycle's input
    /// (`None` for the initial cycle); its `$saga.call_completed` marker is
    /// persisted atomically with whatever the saga emitted in response.
    ///
    /// Version-conflict retries are per-cycle (fresh `attempts` budget) and
    /// re-fetch for fresh state while `completed` stays in hand.
    async fn durable_cycle(
        &self,
        stream_id: &StreamId,
        input: T::Input,
        completed: Option<CallId>,
        run: &mut RunState,
    ) -> Result<CycleOutcome<T::Call, T::Response>, HandlerError<T::Error>> {
        let span = tracing::debug_span!("saga.durable_cycle", call_id = ?completed);
        let started = std::time::Instant::now();
        let result = self
            .durable_cycle_inner(stream_id, input, completed, run)
            .instrument(span)
            .await;
        metrics::histogram!("saga.durable.cycle.duration_seconds", "logic_tag" => T::LOGIC_TAG)
            .record(started.elapsed().as_secs_f64());
        result
    }

    #[allow(clippy::too_many_lines)] // One cycle's full fetch→process→persist state machine
    async fn durable_cycle_inner(
        &self,
        stream_id: &StreamId,
        input: T::Input,
        completed: Option<CallId>,
        run: &mut RunState,
    ) -> Result<CycleOutcome<T::Call, T::Response>, HandlerError<T::Error>> {
        let mut current_input = input;
        let mut attempts: u32 = 0;

        loop {
            let subject = self.env.current_subject().unwrap_or(Subject::System);
            let metadata_context = self.env.metadata();
            let ctx = InvocationContext {
                clock: self.env.clock(),
                subject: &subject,
                correlation_id: metadata_context.correlation_id.as_deref(),
                causation_id: metadata_context.causation_id.as_deref(),
                origin_subject_id: run.origin.as_ref(),
            };

            let FetchResult {
                input: prepared_input,
                expected_version: fetched_version,
            } = self
                .query_fetcher
                .fetch_with_context(current_input.clone(), self.env.projections(), &ctx)
                .await
                .map_err(|e| HandlerError::QueryFetch(e.to_string()))?;

            // The initial cycle trusts the fetcher; every later cycle uses
            // the version returned by the previous persist (see RunState).
            let expected_version = run.carried_version.or(fetched_version);

            let result = self
                .business
                .process_with_context(prepared_input.clone(), &ctx)
                .map_err(HandlerError::Business)?;

            match result {
                BusinessResult::Respond(data) => {
                    if completed.is_some() || !run.outstanding.is_empty() {
                        return Err(HandlerError::RespondInFeedbackCycle);
                    }
                    return Ok(CycleOutcome::Query(data));
                },

                BusinessResult::Done(events) => {
                    if !run.outstanding.is_empty() {
                        return Err(HandlerError::DoneWithOutstandingCalls {
                            outstanding: run.outstanding.len(),
                        });
                    }

                    if events.is_empty() && completed.is_none() {
                        // Aggregate-style no-op on the initial cycle: batch
                        // parity, nothing persisted.
                        return Ok(CycleOutcome::Completed {
                            version: expected_version.unwrap_or_else(Version::initial),
                        });
                    }

                    let metadata = Self::stamped_metadata(metadata_context, &ctx);
                    let mut serialized = Self::serialize_events_with(&events, &metadata)?;
                    Self::reject_reserved_type_names(&serialized)?;
                    if let Some(call_id) = completed {
                        serialized.push(Self::completed_marker(stream_id, call_id, &metadata)?);
                    }

                    match self
                        .persist_and_project(stream_id, serialized, expected_version)
                        .await
                    {
                        Ok((final_version, versioned)) => {
                            self.broadcast(&versioned).await?;
                            run.domain_events_total += events.len();
                            run.carried_version = Some(final_version);
                            return Ok(CycleOutcome::Completed {
                                version: final_version,
                            });
                        },
                        Err(HandlerError::Persist(EventStoreError::VersionConflict {
                            actual,
                            ..
                        })) if attempts < self.max_retries => {
                            attempts += 1;
                            tracing::warn!(
                                attempt = attempts,
                                actual_version = actual.as_u64(),
                                "version conflict; retrying durable cycle"
                            );
                            run.carried_version = Some(actual);
                            current_input = prepared_input;
                        },
                        Err(e) => return Err(e),
                    }
                },

                BusinessResult::Continue { events, calls } => {
                    if calls.is_empty() && run.outstanding.is_empty() {
                        return Err(HandlerError::SagaStuck);
                    }

                    let new_dispatched_total = run.dispatched_total + calls.len() as u64;
                    if new_dispatched_total > u64::from(self.max_total_calls) {
                        return Err(HandlerError::TotalCallsExceeded {
                            max_total_calls: self.max_total_calls,
                        });
                    }

                    let metadata = Self::stamped_metadata(metadata_context, &ctx);
                    let mut serialized = Self::serialize_events_with(&events, &metadata)?;
                    Self::reject_reserved_type_names(&serialized)?;
                    if let Some(call_id) = completed {
                        serialized.push(Self::completed_marker(stream_id, call_id, &metadata)?);
                    }
                    for call in &calls {
                        serialized.push(Self::dispatched_marker(stream_id, call, &metadata)?);
                    }

                    match self
                        .persist_and_project(stream_id, serialized, expected_version)
                        .await
                    {
                        Ok((final_version, versioned)) => {
                            self.broadcast(&versioned).await?;
                            run.domain_events_total += events.len();
                            run.dispatched_total = new_dispatched_total;
                            run.carried_version = Some(final_version);
                            metrics::counter!(
                                "saga.durable.calls_dispatched.total",
                                "logic_tag" => T::LOGIC_TAG
                            )
                            .increment(calls.len() as u64);

                            // The dispatched markers are the LAST `calls.len()`
                            // events of the append, so their (version-derived)
                            // IDs count back from the final version.
                            let base = final_version.as_u64() - calls.len() as u64 + 1;
                            let identified = calls
                                .into_iter()
                                .enumerate()
                                .map(|(i, call)| (CallId::new(base + i as u64), call))
                                .collect();
                            return Ok(CycleOutcome::Dispatched { calls: identified });
                        },
                        Err(HandlerError::Persist(EventStoreError::VersionConflict {
                            actual,
                            ..
                        })) if attempts < self.max_retries => {
                            attempts += 1;
                            tracing::warn!(
                                attempt = attempts,
                                actual_version = actual.as_u64(),
                                "version conflict; retrying durable cycle"
                            );
                            run.carried_version = Some(actual);
                            current_input = prepared_input;
                        },
                        Err(e) => return Err(e),
                    }
                },
            }
        }
    }

    /// Reject domain events whose type names invade the reserved `$`
    /// namespace. Enforced only on the durable path so the batch path stays
    /// byte-for-byte unchanged.
    fn reject_reserved_type_names(
        events: &[SerializedEvent],
    ) -> Result<(), HandlerError<T::Error>> {
        for event in events {
            if is_framework_event_type(&event.event_type) {
                return Err(HandlerError::Serialization(SerializationError::Encode(
                    format!(
                        "domain event type '{}' uses the reserved '$' framework prefix",
                        event.event_type
                    ),
                )));
            }
        }
        Ok(())
    }

    /// Build a `$saga.call_completed` marker event.
    fn completed_marker(
        stream_id: &StreamId,
        call_id: CallId,
        metadata: &EventMetadata,
    ) -> Result<SerializedEvent, HandlerError<T::Error>> {
        let marker = CallCompleted {
            stream_id: stream_id.as_str().to_string(),
            call_id,
        };
        let payload = bincode::serialize(&marker).map_err(|e| {
            HandlerError::Serialization(SerializationError::Encode(format!(
                "{CALL_COMPLETED_EVENT_TYPE}: {e}"
            )))
        })?;
        Ok(SerializedEvent {
            event_type: CALL_COMPLETED_EVENT_TYPE.to_string(),
            payload,
            metadata: Some(metadata.clone()),
            version: None,
        })
    }

    /// Build a `$saga.call_dispatched` marker event carrying the serialized
    /// call for re-dispatch on resume.
    fn dispatched_marker(
        stream_id: &StreamId,
        call: &T::Call,
        metadata: &EventMetadata,
    ) -> Result<SerializedEvent, HandlerError<T::Error>> {
        let call_bytes = bincode::serialize(call).map_err(|e| {
            HandlerError::Serialization(SerializationError::Encode(format!(
                "{CALL_DISPATCHED_EVENT_TYPE}: call payload: {e}"
            )))
        })?;
        let marker = CallDispatched {
            stream_id: stream_id.as_str().to_string(),
            call: call_bytes,
        };
        let payload = bincode::serialize(&marker).map_err(|e| {
            HandlerError::Serialization(SerializationError::Encode(format!(
                "{CALL_DISPATCHED_EVENT_TYPE}: {e}"
            )))
        })?;
        Ok(SerializedEvent {
            event_type: CALL_DISPATCHED_EVENT_TYPE.to_string(),
            payload,
            metadata: Some(metadata.clone()),
            version: None,
        })
    }
}

/// Result of a [`recovery_sweep`](Handler::recovery_sweep).
#[derive(Debug, Default)]
pub struct SweepReport {
    /// Instances that were driven: resumed to completion or re-suspended
    /// (cancelled/drained mid-sweep).
    pub resumed: Vec<StreamId>,

    /// Instances with nothing outstanding — parked at a gate, already
    /// finished, or stale registry rows. Left untouched, as they must be.
    pub no_outstanding: Vec<StreamId>,

    /// Instances whose resume failed, with the rendered error. Repeated
    /// failures for the same instance deserve an alert (see
    /// [`resume`](Handler::resume) on the process-error sharp edge).
    pub failed: Vec<(StreamId, String)>,
}

impl<T, E, QF, Env> Handler<T, E, QF, Env>
where
    T: DurableBusinessLogic,
    T::Input: Clone,
    T::Call: Serialize + DeserializeOwned,
    E: CallExecutor<T::Call, T::CallResult> + UnitCallExecutor<T::Call, T::CallResult> + 'static,
    QF: QueryFetcher<T::Input, Env::Projections> + 'static,
    Env: HandlerEnvironment + 'static,
{
    /// Resume every instance of **this saga type** that has outstanding
    /// calls: the packaged recovery sweep.
    ///
    /// Queries the registry with [`LOGIC_TAG`](DurableBusinessLogic::LOGIC_TAG)
    /// — the tag comes from the type, so a sweep can never feed another saga
    /// type's journal to this handler — then spawns one `resume` task per
    /// instance (callers hold the handler in an `Arc`; runs on the ambient
    /// tokio runtime) and reports the outcomes.
    ///
    /// Run it at startup and optionally on an interval: `resume` is
    /// idempotent, instances with nothing outstanding land in
    /// [`no_outstanding`](SweepReport::no_outstanding) untouched, and an
    /// instance a live loop is already driving conflicts-and-loses
    /// harmlessly. Multi-node deployments can skip contended instances with
    /// an advisory lock (see `SagaSweepLock` in the `PostgreSQL` crate).
    ///
    /// # Errors
    ///
    /// Returns the registry's error if the sweep query fails. Per-instance
    /// resume failures do NOT fail the sweep — they land in
    /// [`SweepReport::failed`].
    pub async fn recovery_sweep<R: SagaRegistry>(
        self: Arc<Self>,
        registry: &R,
        cancel: &CancellationToken,
    ) -> Result<SweepReport, R::Error> {
        let records = registry
            .instances_with_outstanding_calls(T::LOGIC_TAG)
            .await?;

        let mut tasks = Vec::with_capacity(records.len());
        for record in records {
            let handler = Arc::clone(&self);
            let token = cancel.clone();
            let stream_id = record.stream_id.clone();
            tasks.push((
                record.stream_id,
                tokio::spawn(async move { handler.resume(&stream_id, token).await }),
            ));
        }

        let mut report = SweepReport::default();
        for (stream_id, task) in tasks {
            match task.await {
                Ok(Ok(DurableOutcome::NoOutstandingCalls)) => {
                    report.no_outstanding.push(stream_id);
                },
                Ok(Ok(_)) => report.resumed.push(stream_id),
                Ok(Err(e)) => report.failed.push((stream_id, e.to_string())),
                Err(join_error) => report
                    .failed
                    .push((stream_id, format!("resume task panicked: {join_error}"))),
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatched_event(stream_id: &str, version: u64, call: &[u8]) -> SerializedEvent {
        let marker = CallDispatched {
            stream_id: stream_id.to_string(),
            call: call.to_vec(),
        };
        SerializedEvent {
            event_type: CALL_DISPATCHED_EVENT_TYPE.to_string(),
            #[allow(clippy::unwrap_used)]
            payload: bincode::serialize(&marker).unwrap(),
            metadata: None,
            version: Some(Version::new(version)),
        }
    }

    fn completed_event(stream_id: &str, version: u64, call_id: u64) -> SerializedEvent {
        let marker = CallCompleted {
            stream_id: stream_id.to_string(),
            call_id: CallId::new(call_id),
        };
        SerializedEvent {
            event_type: CALL_COMPLETED_EVENT_TYPE.to_string(),
            #[allow(clippy::unwrap_used)]
            payload: bincode::serialize(&marker).unwrap(),
            metadata: None,
            version: Some(Version::new(version)),
        }
    }

    fn domain_event(version: u64) -> SerializedEvent {
        SerializedEvent {
            event_type: "SomethingHappened".to_string(),
            payload: vec![1, 2, 3],
            metadata: None,
            version: Some(Version::new(version)),
        }
    }

    #[test]
    fn scan_computes_dispatched_minus_completed() {
        let events = vec![
            domain_event(1),
            dispatched_event("saga-1", 2, b"call-a"),
            dispatched_event("saga-1", 3, b"call-b"),
            completed_event("saga-1", 4, 2),
            dispatched_event("saga-1", 5, b"call-c"),
        ];

        #[allow(clippy::unwrap_used)]
        let journal = scan_journal(&events).unwrap();

        assert_eq!(journal.dispatched_count, 3);
        assert_eq!(journal.outstanding.len(), 2);
        assert_eq!(
            journal.outstanding.get(&CallId::new(3)),
            Some(&b"call-b".to_vec())
        );
        assert_eq!(
            journal.outstanding.get(&CallId::new(5)),
            Some(&b"call-c".to_vec())
        );
    }

    #[test]
    fn scan_of_balanced_journal_has_no_outstanding() {
        let events = vec![
            dispatched_event("saga-1", 1, b"call-a"),
            completed_event("saga-1", 2, 1),
        ];

        #[allow(clippy::unwrap_used)]
        let journal = scan_journal(&events).unwrap();

        assert!(journal.outstanding.is_empty());
        assert_eq!(journal.dispatched_count, 1);
    }

    #[test]
    fn scan_journal_tolerates_duplicate_completions() {
        // A double-resume race can persist the same completion twice.
        let events = vec![
            dispatched_event("saga-1", 1, b"call-a"),
            completed_event("saga-1", 2, 1),
            completed_event("saga-1", 3, 1),
        ];

        #[allow(clippy::unwrap_used)]
        let journal = scan_journal(&events).unwrap();

        assert!(journal.outstanding.is_empty());
        assert_eq!(journal.dispatched_count, 1);
    }

    #[test]
    fn scan_ignores_domain_events_entirely() {
        let events = vec![domain_event(1), domain_event(2)];

        #[allow(clippy::unwrap_used)]
        let journal = scan_journal(&events).unwrap();

        assert!(journal.outstanding.is_empty());
        assert_eq!(journal.dispatched_count, 0);
    }

    #[test]
    fn scan_errors_on_dispatched_marker_without_version() {
        let mut event = dispatched_event("saga-1", 1, b"call-a");
        event.version = None;

        let result = scan_journal(&[event]);
        assert!(matches!(result, Err(SerializationError::Decode(_))));
    }

    #[test]
    fn scan_errors_on_undecodable_marker_payload() {
        let event = SerializedEvent {
            event_type: CALL_DISPATCHED_EVENT_TYPE.to_string(),
            payload: vec![0xFF; 3],
            metadata: None,
            version: Some(Version::new(1)),
        };

        let result = scan_journal(&[event]);
        assert!(matches!(result, Err(SerializationError::Decode(_))));
    }

    #[test]
    fn call_id_display_and_conversions() {
        let id = CallId::from_version(Version::new(42));
        assert_eq!(id.as_u64(), 42);
        assert_eq!(format!("{id}"), "call@v42");
        assert_eq!(CallId::new(42), id);
    }

    #[test]
    fn in_flight_gauge_counts_and_saturates() {
        // No metrics recorder installed: the macros are no-ops, so this
        // pins the guard's internal accounting (what Drop will flush).
        let mut gauge = InFlightGauge::new("test");
        gauge.increment();
        gauge.increment();
        assert_eq!(gauge.count, 2);
        gauge.decrement();
        assert_eq!(gauge.count, 1);
        gauge.decrement();
        gauge.decrement(); // saturates at zero, no underflow
        assert_eq!(gauge.count, 0);
        drop(gauge); // Drop with zero remainder: no-op, no panic
    }

    #[test]
    fn in_flight_gauge_drop_flushes_remainder() {
        let mut gauge = InFlightGauge::new("test");
        gauge.increment();
        gauge.increment();
        // Dropping with count > 0 must not panic (it decrements the
        // remainder on the real recorder in production).
        drop(gauge);
    }

    #[test]
    fn is_framework_event_type_matches_reserved_prefix() {
        assert!(is_framework_event_type(CALL_DISPATCHED_EVENT_TYPE));
        assert!(is_framework_event_type(CALL_COMPLETED_EVENT_TYPE));
        assert!(is_framework_event_type("$anything"));
        assert!(!is_framework_event_type("OrderPlaced"));
    }

    // ── rebuild_state_from_serialized skips framework markers ──

    use crate::{BusinessResult, Clock};
    use std::convert::Infallible;

    #[derive(Clone, Serialize, Deserialize)]
    enum MiniEv {
        Bumped,
    }

    #[derive(Default)]
    struct MiniState {
        bumps: u32,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("mini error")]
    struct MiniErr;

    struct MiniLogic;

    impl BusinessLogic for MiniLogic {
        type State = MiniState;
        type Input = ();
        type Event = MiniEv;
        type Error = MiniErr;
        type Call = Infallible;
        type CallResult = Infallible;
        type Response = ();

        fn stream_id(_input: &()) -> StreamId {
            StreamId::new("mini")
        }

        fn process(
            &self,
            _input: (),
            _clock: &dyn Clock,
        ) -> Result<BusinessResult<MiniEv, Infallible, ()>, MiniErr> {
            Ok(BusinessResult::done_empty())
        }

        fn apply(&self, state: &mut MiniState, _event: &MiniEv) {
            state.bumps += 1;
        }

        fn event_type_name(_event: &MiniEv) -> &'static str {
            "Bumped"
        }
    }

    #[test]
    fn rebuild_state_skips_framework_markers() {
        let bump = |version: u64| SerializedEvent {
            event_type: "Bumped".to_string(),
            #[allow(clippy::unwrap_used)]
            payload: bincode::serialize(&MiniEv::Bumped).unwrap(),
            metadata: None,
            version: Some(Version::new(version)),
        };

        // Domain events interleaved with journal markers, as a durable saga
        // stream really looks.
        let events = vec![
            bump(1),
            dispatched_event("mini", 2, b"call"),
            completed_event("mini", 3, 2),
            bump(4),
        ];

        #[allow(clippy::unwrap_used)]
        let state = MiniLogic.rebuild_state_from_serialized(&events).unwrap();
        assert_eq!(state.bumps, 2, "markers must not reach apply()");
    }
}
