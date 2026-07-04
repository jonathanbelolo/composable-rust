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

use std::collections::{BTreeMap, BTreeSet, HashSet};

use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    BusinessLogic, BusinessResult, CallExecutor, CancellationToken, EventMetadata, EventStoreError,
    FetchResult, Handler, HandlerEnvironment, HandlerError, InvocationContext, QueryFetcher,
    SerializationError, SerializedEvent, StreamId, Subject, SubjectId, UnitCallExecutor, Version,
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
    /// - Cancellation via `cancel` suspends the run: in-flight calls are
    ///   aborted and stay outstanding in the journal, so a later `resume`
    ///   re-dispatches exactly them. Cancellation is observed between
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
        let origin = self
            .env
            .current_subject()
            .unwrap_or(Subject::System)
            .id()
            .cloned();

        let mut run = RunState {
            outstanding: BTreeSet::new(),
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

    /// Drive the unordered-completion loop until the saga finishes, errs, or
    /// is cancelled. `initial_calls` are dispatched first (their journal
    /// markers are already persisted).
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

        // A pre-cancelled token suspends before any call starts; the
        // dispatched markers are journaled, so resume re-dispatches them.
        if cancel.is_cancelled() {
            return Ok(DurableOutcome::Suspended {
                outstanding: run.outstanding.iter().copied().collect(),
            });
        }

        let mut in_flight = FuturesUnordered::new();
        for (id, call) in initial_calls {
            in_flight.push(dispatch_one(&self.call_executor, id, call));
        }

        loop {
            tokio::select! {
                biased;

                () = cancel.cancelled() => {
                    // Dropping `in_flight` aborts the calls; their journal
                    // entries stay outstanding → resumable.
                    return Ok(DurableOutcome::Suspended {
                        outstanding: run.outstanding.iter().copied().collect(),
                    });
                },

                next = in_flight.next() => {
                    // Bound and matched here, NOT in the arm pattern: a
                    // refutable arm pattern would silently disable the branch
                    // on None and park on cancelled() forever.
                    let Some((call_id, result)) = next else {
                        // Defensive: the loop invariant (every cycle either
                        // finishes the run or keeps calls in flight) makes
                        // this unreachable.
                        return Err(HandlerError::SagaStuck);
                    };
                    // Remove BEFORE the cycle so the Done-check counts only
                    // *other* calls; this completion's marker persists in the
                    // same append as the saga's reaction to it.
                    run.outstanding.remove(&call_id);
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
                            for (id, call) in calls {
                                run.outstanding.insert(id);
                                in_flight.push(dispatch_one(&self.call_executor, id, call));
                            }
                        },
                        // durable_cycle rejects Respond on feedback cycles.
                        CycleOutcome::Query(_) => {
                            return Err(HandlerError::RespondInFeedbackCycle);
                        },
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
    #[allow(clippy::too_many_lines)] // One cycle's full fetch→process→persist state machine
    async fn durable_cycle(
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
