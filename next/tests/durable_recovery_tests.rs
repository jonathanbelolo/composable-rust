//! Recovery tests for `Handler::resume` (WP-3b).
//!
//! These pin the consumer's acceptance scenarios 1 (crash-resume at call
//! granularity) and 3 (parked-at-gate safety), plus journal-corruption
//! handling, guard seeding across restarts, and double-resume
//! at-least-once safety.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_const_for_fn
)]

mod common;
use common::{
    Gates, WErr, WEv, WIn, WState, count_type, echo_executor, gated_executor, test_env,
    w_completion_input, w_event_type_name, w_stream_id, wait_for,
};

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use composable_rust_next::{
    BusinessLogic, BusinessResult, CALL_COMPLETED_EVENT_TYPE, CALL_DISPATCHED_EVENT_TYPE,
    CallDispatched, CallId, CancellationToken, Clock, DurableBusinessLogic, DurableOutcome,
    EventStore, Handler, HandlerBuilder, HandlerError, NoOpQueryFetcher, SerializedEvent, StreamId,
    Subject, SubjectId, scan_journal,
};
use tokio::sync::oneshot;

// ═══════════════════════════════════════════════════════════════════
// Resume-safe saga: completion state lives in a shared handle that
// stands in for the saga's projection (it survives the "crash" because
// the test hands the same handle to the second saga instance).
// ═══════════════════════════════════════════════════════════════════

struct ResumableSaga {
    total: u64,
    completed: Arc<Mutex<HashSet<CallId>>>,
}

impl ResumableSaga {
    fn new(total: u64) -> (Self, Arc<Mutex<HashSet<CallId>>>) {
        let completed = Arc::new(Mutex::new(HashSet::new()));
        (
            Self {
                total,
                completed: Arc::clone(&completed),
            },
            completed,
        )
    }

    fn with_state(total: u64, completed: Arc<Mutex<HashSet<CallId>>>) -> Self {
        Self { total, completed }
    }
}

impl BusinessLogic for ResumableSaga {
    type State = WState;
    type Input = WIn;
    type Event = WEv;
    type Error = WErr;
    type Call = u64;
    type CallResult = u64;
    type Response = ();

    fn stream_id(input: &WIn) -> StreamId {
        w_stream_id("rsaga", input)
    }

    fn process(
        &self,
        input: WIn,
        _clock: &dyn Clock,
    ) -> Result<BusinessResult<WEv, u64, ()>, WErr> {
        match input {
            WIn::Start { id } => Ok(BusinessResult::Continue {
                events: vec![WEv::Started(id)],
                calls: (1..=self.total).collect(),
            }),
            WIn::Completion {
                id,
                call_id,
                result,
            } => {
                let mut done = self.completed.lock().unwrap();
                done.insert(call_id);
                if done.len() as u64 == self.total {
                    Ok(BusinessResult::Done(vec![WEv::Finished(id)]))
                } else {
                    Ok(BusinessResult::Continue {
                        events: vec![WEv::Progress(result)],
                        calls: Vec::new(),
                    })
                }
            },
        }
    }

    fn apply(&self, _state: &mut WState, _event: &WEv) {}

    fn event_type_name(event: &WEv) -> &'static str {
        w_event_type_name(event)
    }
}

impl DurableBusinessLogic for ResumableSaga {
    const LOGIC_TAG: &'static str = "rsaga";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> WIn {
        w_completion_input("rsaga", stream_id, call_id, result)
    }
}

// ═══════════════════════════════════════════════════════════════════
// Acceptance scenario 1: crash-resume at call granularity
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn crash_resume_redispatches_exactly_the_incomplete_call() {
    let env = test_env();
    let store = env.event_store().clone();

    // Window of 3: calls 1 and 2 complete on cue, call 3 hangs forever.
    let (tx1, rx1) = oneshot::channel();
    let (tx2, rx2) = oneshot::channel();
    let (_tx3, rx3) = oneshot::channel();
    let gates: Gates = Arc::new(Mutex::new(HashMap::from([(1, rx1), (2, rx2), (3, rx3)])));
    let first_executor = gated_executor(gates);
    let first_executor_handle = first_executor.clone();

    let (saga, completed_state) = ResumableSaga::new(3);
    let handler = Arc::new(Handler::new(
        saga,
        first_executor,
        NoOpQueryFetcher,
        env.clone(),
    ));
    let h = Arc::clone(&handler);
    let run = tokio::spawn(async move {
        h.handle_durable(WIn::Start { id: 1 }, CancellationToken::new())
            .await
    });

    // Two completions persist, one call stays in flight...
    tx1.send(101).unwrap();
    tx2.send(102).unwrap();
    wait_for("two completions persisted", || {
        count_type(
            &store.events_for_stream("rsaga-1"),
            CALL_COMPLETED_EVENT_TYPE,
        ) == 2
    })
    .await;
    assert_eq!(first_executor_handle.dispatch_count(), 3);

    // ...and the process "crashes".
    run.abort();
    assert!(run.await.unwrap_err().is_cancelled());

    // A fresh handler over the same store (same "database"), with a fresh
    // executor whose calls all complete immediately.
    let second_executor = echo_executor();
    let second_executor_handle = second_executor.clone();
    let handler = Handler::new(
        ResumableSaga::with_state(3, completed_state),
        second_executor,
        NoOpQueryFetcher,
        env,
    );

    let outcome = handler
        .resume(&StreamId::new("rsaga-1"), CancellationToken::new())
        .await
        .unwrap();
    assert!(
        matches!(outcome, DurableOutcome::Completed { .. }),
        "resume must drive the saga to completion, got {outcome:?}"
    );

    // Exactly the one incomplete call was re-dispatched — the two completed
    // calls were never re-paid.
    assert_eq!(
        second_executor_handle.dispatched(),
        vec![3],
        "resume must re-dispatch exactly the incomplete call"
    );

    let events = store.events_for_stream("rsaga-1");
    assert_eq!(count_type(&events, CALL_DISPATCHED_EVENT_TYPE), 3);
    assert_eq!(count_type(&events, CALL_COMPLETED_EVENT_TYPE), 3);
    assert_eq!(count_type(&events, "WFinished"), 1);
    let journal = scan_journal(&events).unwrap();
    assert!(journal.outstanding.is_empty());
}

// ═══════════════════════════════════════════════════════════════════
// Acceptance scenario 3: parked-at-gate safety
// ═══════════════════════════════════════════════════════════════════

/// Saga that parks (returns `Done`) after its single call completes; a new
/// user command re-enters it for another round.
struct GateSaga;

impl BusinessLogic for GateSaga {
    type State = WState;
    type Input = WIn;
    type Event = WEv;
    type Error = WErr;
    type Call = u64;
    type CallResult = u64;
    type Response = ();

    fn stream_id(input: &WIn) -> StreamId {
        w_stream_id("gate", input)
    }

    fn process(
        &self,
        input: WIn,
        _clock: &dyn Clock,
    ) -> Result<BusinessResult<WEv, u64, ()>, WErr> {
        match input {
            WIn::Start { id } => Ok(BusinessResult::Continue {
                events: vec![WEv::Started(id)],
                calls: vec![1],
            }),
            // Phase complete → park at the review gate.
            WIn::Completion { id, .. } => Ok(BusinessResult::Done(vec![WEv::Finished(id)])),
        }
    }

    fn apply(&self, _state: &mut WState, _event: &WEv) {}

    fn event_type_name(event: &WEv) -> &'static str {
        w_event_type_name(event)
    }
}

impl DurableBusinessLogic for GateSaga {
    const LOGIC_TAG: &'static str = "gate";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> WIn {
        w_completion_input("gate", stream_id, call_id, result)
    }
}

#[tokio::test]
async fn parked_saga_is_left_alone_by_resume_and_reenters_normally() {
    let env = test_env();
    let store = env.event_store().clone();

    // Phase 1 runs to the gate.
    let handler = Handler::new(GateSaga, echo_executor(), NoOpQueryFetcher, env.clone());
    let outcome = handler
        .handle_durable(WIn::Start { id: 2 }, CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::Completed { .. }));
    let parked_event_count = store.events_for_stream("gate-2").len();

    // A recovery sweep hits the parked instance: it must not be driven.
    let sweep_executor = echo_executor();
    let sweep_executor_handle = sweep_executor.clone();
    let sweep_handler = Handler::new(GateSaga, sweep_executor, NoOpQueryFetcher, env.clone());
    let outcome = sweep_handler
        .resume(&StreamId::new("gate-2"), CancellationToken::new())
        .await
        .unwrap();
    assert!(
        matches!(outcome, DurableOutcome::NoOutstandingCalls),
        "a parked instance has zero outstanding calls, got {outcome:?}"
    );
    assert_eq!(
        sweep_executor_handle.dispatch_count(),
        0,
        "resume must not execute anything on a parked instance"
    );
    assert_eq!(
        store.events_for_stream("gate-2").len(),
        parked_event_count,
        "resume must not persist anything on a parked instance"
    );

    // A user command re-enters the parked saga normally (next phase).
    let outcome = handler
        .handle_durable(WIn::Start { id: 2 }, CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::Completed { .. }));
    assert!(store.events_for_stream("gate-2").len() > parked_event_count);
}

#[tokio::test]
async fn resume_recovers_origin_subject_attribution() {
    // Base env cloned BEFORE the subject fixture: same shared store, no
    // subject — the resuming process runs as System.
    let base = test_env();
    let resume_env = base.clone();
    let store = base.event_store().clone();
    let start_env = base.with_subject(Subject::Authenticated {
        id: SubjectId::new("user-9"),
        attributes: HashMap::new(),
    });

    // user-9 starts the run; a pre-cancelled token journals the dispatch
    // and suspends without executing anything.
    let (saga, completed_state) = ResumableSaga::new(1);
    let starter = Handler::new(saga, echo_executor(), NoOpQueryFetcher, start_env);
    let pre_cancelled = CancellationToken::new();
    pre_cancelled.cancel();
    let outcome = starter
        .handle_durable(WIn::Start { id: 6 }, pre_cancelled)
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::Suspended { .. }));
    let events_before_resume = store.events_for_stream("rsaga-6").len();

    // Resume as System: post-resume events must stay attributed to the
    // initiating user (origin recovered from the stream) while author_id
    // reflects the resuming subject (None for System).
    let resumer = Handler::new(
        ResumableSaga::with_state(1, completed_state),
        echo_executor(),
        NoOpQueryFetcher,
        resume_env,
    );
    let outcome = resumer
        .resume(&StreamId::new("rsaga-6"), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::Completed { .. }));

    let events = store.events_for_stream("rsaga-6");
    let post_resume = &events[events_before_resume..];
    assert!(!post_resume.is_empty(), "resume must have appended events");
    for event in post_resume {
        let metadata = event.metadata.as_ref().expect("metadata stamped");
        assert_eq!(
            metadata.origin_subject_id,
            Some(SubjectId::new("user-9")),
            "post-resume events must keep the initiator's origin ({})",
            event.event_type
        );
        assert_eq!(
            metadata.author_id, None,
            "System-authored: author_id reflects the resuming subject"
        );
    }
}

#[tokio::test]
async fn resume_recovers_the_latest_origin_across_phases() {
    // Multi-phase workflow: phase 1 initiated by user-9, parked at a gate;
    // phase 2 re-entered by user-13, crashed; resumed by System. Post-resume
    // events must be attributed to user-13 (the CURRENT run's initiator),
    // not user-9 — resume takes the LAST stamped origin, not the first.
    let base = test_env();
    let resume_env = base.clone();
    let store = base.event_store().clone();
    let env_user9 = base.clone().with_subject(Subject::Authenticated {
        id: SubjectId::new("user-9"),
        attributes: HashMap::new(),
    });
    let env_user13 = base.with_subject(Subject::Authenticated {
        id: SubjectId::new("user-13"),
        attributes: HashMap::new(),
    });

    // Phase 1 (user-9): runs to the gate, journal balanced.
    let phase1 = Handler::new(GateSaga, echo_executor(), NoOpQueryFetcher, env_user9);
    let outcome = phase1
        .handle_durable(WIn::Start { id: 77 }, CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::Completed { .. }));

    // Phase 2 (user-13): journaled dispatch, then "crash" (pre-cancelled).
    let phase2 = Handler::new(GateSaga, echo_executor(), NoOpQueryFetcher, env_user13);
    let pre_cancelled = CancellationToken::new();
    pre_cancelled.cancel();
    let outcome = phase2
        .handle_durable(WIn::Start { id: 77 }, pre_cancelled)
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::Suspended { .. }));
    let events_before_resume = store.events_for_stream("gate-77").len();

    // Resume as System.
    let resumer = Handler::new(GateSaga, echo_executor(), NoOpQueryFetcher, resume_env);
    let outcome = resumer
        .resume(&StreamId::new("gate-77"), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::Completed { .. }));

    let events = store.events_for_stream("gate-77");
    let post_resume = &events[events_before_resume..];
    assert!(!post_resume.is_empty());
    for event in post_resume {
        let metadata = event.metadata.as_ref().expect("metadata stamped");
        assert_eq!(
            metadata.origin_subject_id,
            Some(SubjectId::new("user-13")),
            "post-resume events belong to phase 2's initiator, not phase 1's ({})",
            event.event_type
        );
    }
}

#[tokio::test]
async fn resume_of_nonexistent_stream_is_a_no_op() {
    let env = test_env();
    let executor = echo_executor();
    let executor_handle = executor.clone();
    let handler = Handler::new(GateSaga, executor, NoOpQueryFetcher, env);

    let outcome = handler
        .resume(&StreamId::new("gate-404"), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::NoOutstandingCalls));
    assert_eq!(executor_handle.dispatch_count(), 0);
}

// ═══════════════════════════════════════════════════════════════════
// Journal corruption
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn resume_errors_on_corrupted_marker_payload() {
    let env = test_env();
    let store = env.event_store().clone();

    // A dispatched marker whose payload is garbage: framework-authored
    // bytes must decode — tolerating this would silently drop a call.
    store
        .append(
            &StreamId::new("rsaga-99"),
            None,
            vec![SerializedEvent {
                event_type: CALL_DISPATCHED_EVENT_TYPE.to_string(),
                payload: vec![0xFF, 0xFE, 0xFD],
                metadata: None,
                version: None,
                stream_id: None,
            }],
        )
        .await
        .unwrap();

    let (saga, _) = ResumableSaga::new(1);
    let handler = Handler::new(saga, echo_executor(), NoOpQueryFetcher, env);
    let err = handler
        .resume(&StreamId::new("rsaga-99"), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(err, HandlerError::Serialization(_)), "got {err:?}");
}

// ═══════════════════════════════════════════════════════════════════
// Guard seeding across restarts
// ═══════════════════════════════════════════════════════════════════

/// Saga that answers every completion with one more call, forever.
struct TopUpForeverSaga;

impl BusinessLogic for TopUpForeverSaga {
    type State = WState;
    type Input = WIn;
    type Event = WEv;
    type Error = WErr;
    type Call = u64;
    type CallResult = u64;
    type Response = ();

    fn stream_id(input: &WIn) -> StreamId {
        w_stream_id("topup", input)
    }

    fn process(
        &self,
        input: WIn,
        _clock: &dyn Clock,
    ) -> Result<BusinessResult<WEv, u64, ()>, WErr> {
        match input {
            WIn::Start { id } => Ok(BusinessResult::Continue {
                events: vec![WEv::Started(id)],
                calls: vec![1],
            }),
            WIn::Completion { result, .. } => Ok(BusinessResult::Continue {
                events: Vec::new(),
                calls: vec![result + 1],
            }),
        }
    }

    fn apply(&self, _state: &mut WState, _event: &WEv) {}

    fn event_type_name(event: &WEv) -> &'static str {
        w_event_type_name(event)
    }
}

impl DurableBusinessLogic for TopUpForeverSaga {
    const LOGIC_TAG: &'static str = "topup";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> WIn {
        w_completion_input("topup", stream_id, call_id, result)
    }
}

/// Hand-craft a journal with `dispatched` dispatched markers of which the
/// first `completed` are completed. Payloads are real bincode `u64` calls.
async fn seed_journal(
    store: &composable_rust_next::testing::InMemoryEventStore,
    stream: &str,
    dispatched: u64,
    completed: u64,
) {
    let mut events = Vec::new();
    for call in 1..=dispatched {
        let marker = CallDispatched {
            stream_id: stream.to_string(),
            call: bincode::serialize(&call).unwrap(),
        };
        events.push(SerializedEvent {
            event_type: CALL_DISPATCHED_EVENT_TYPE.to_string(),
            payload: bincode::serialize(&marker).unwrap(),
            metadata: None,
            version: None,
            stream_id: None,
        });
    }
    store
        .append(&StreamId::new(stream), None, events)
        .await
        .unwrap();

    // The dispatched markers landed at versions 1..=dispatched.
    let mut completions = Vec::new();
    for call_version in 1..=completed {
        let marker = composable_rust_next::CallCompleted {
            stream_id: stream.to_string(),
            call_id: CallId::new(call_version),
        };
        completions.push(SerializedEvent {
            event_type: CALL_COMPLETED_EVENT_TYPE.to_string(),
            payload: bincode::serialize(&marker).unwrap(),
            metadata: None,
            version: None,
            stream_id: None,
        });
    }
    if !completions.is_empty() {
        store
            .append(&StreamId::new(stream), None, completions)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn resume_seeds_max_total_calls_guard_from_journal_history() {
    let env = test_env();
    let store = env.event_store().clone();

    // History: 3 calls ever dispatched, 2 completed, 1 outstanding.
    seed_journal(&store, "topup-1", 3, 2).await;

    // Cap of 3: the instance has already spent its lifetime budget, so the
    // saga's first post-resume top-up must trip the guard. If seeding were
    // broken (counter reset to 0), the top-up would pass.
    let handler = HandlerBuilder::new(TopUpForeverSaga)
        .call_executor(echo_executor())
        .query_fetcher(NoOpQueryFetcher)
        .environment(env)
        .max_total_calls(3)
        .build();

    let err = handler
        .resume(&StreamId::new("topup-1"), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, HandlerError::TotalCallsExceeded { max_total_calls: 3 }),
        "got {err:?}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Double resume: at-least-once, journal stays exact
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn concurrent_double_resume_is_safe() {
    let env = test_env();
    let store = env.event_store().clone();

    // Journal a run with its single call outstanding, without executing
    // anything: a pre-cancelled token suspends after persisting dispatch.
    let (saga, completed_state) = ResumableSaga::new(1);
    let setup = Handler::new(saga, echo_executor(), NoOpQueryFetcher, env.clone());
    let pre_cancelled = CancellationToken::new();
    pre_cancelled.cancel();
    let outcome = setup
        .handle_durable(WIn::Start { id: 5 }, pre_cancelled)
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::Suspended { .. }));

    // Two drivers race to resume the same instance.
    let handler_a = Handler::new(
        ResumableSaga::with_state(1, Arc::clone(&completed_state)),
        echo_executor(),
        NoOpQueryFetcher,
        env.clone(),
    );
    let handler_b = Handler::new(
        ResumableSaga::with_state(1, completed_state),
        echo_executor(),
        NoOpQueryFetcher,
        env,
    );

    let stream = StreamId::new("rsaga-5");
    let (a, b) = tokio::join!(
        handler_a.resume(&stream, CancellationToken::new()),
        handler_b.resume(&stream, CancellationToken::new()),
    );

    // Both drivers finish without error (at-least-once: the call ran twice,
    // the persists serialized via optimistic concurrency).
    assert!(a.is_ok(), "driver A failed: {a:?}");
    assert!(b.is_ok(), "driver B failed: {b:?}");

    // The journal stays exact: duplicate completion markers are tolerated
    // by set semantics and nothing is left outstanding.
    let journal = scan_journal(&store.events_for_stream("rsaga-5")).unwrap();
    assert!(journal.outstanding.is_empty());
    assert_eq!(journal.dispatched_count, 1);
}

// ═══════════════════════════════════════════════════════════════════
// Recovery sweep
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn recovery_sweep_buckets_instances_correctly() {
    let env = test_env();
    let store = env.event_store().clone();

    // Instance 30: suspended with one outstanding call (needs recovery).
    let (saga, _) = ResumableSaga::new(1);
    let starter = Handler::new(saga, echo_executor(), NoOpQueryFetcher, env.clone());
    let pre_cancelled = CancellationToken::new();
    pre_cancelled.cancel();
    let outcome = starter
        .handle_durable(WIn::Start { id: 30 }, pre_cancelled)
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::Suspended { .. }));

    // Instance 31: ran to completion (balanced journal, stale registry row).
    let (saga, _) = ResumableSaga::new(1);
    let finisher = Handler::new(saga, echo_executor(), NoOpQueryFetcher, env.clone());
    let outcome = finisher
        .handle_durable(WIn::Start { id: 31 }, CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::Completed { .. }));

    // The registry lists both, plus a row for a stream that never existed.
    let record = |name: &str| composable_rust_next::SagaInstanceRecord {
        stream_id: StreamId::new(name),
        logic_tag: "rsaga".to_string(),
        outstanding_calls: 1,
        oldest_dispatched_at: chrono::Utc::now(),
    };
    let registry = StaticRegistry {
        records: vec![record("rsaga-30"), record("rsaga-31"), record("rsaga-32")],
    };

    let sweeper = Arc::new(Handler::new(
        ResumableSaga::with_state(1, Arc::new(Mutex::new(HashSet::new()))),
        echo_executor(),
        NoOpQueryFetcher,
        env,
    ));
    let report = sweeper
        .recovery_sweep(&registry, &CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(
        report.resumed,
        vec![StreamId::new("rsaga-30")],
        "only the suspended instance is driven"
    );
    assert_eq!(
        report.no_outstanding,
        vec![StreamId::new("rsaga-31"), StreamId::new("rsaga-32")],
        "parked/finished and stale rows are left alone"
    );
    assert!(report.failed.is_empty(), "failed: {:?}", report.failed);

    // The driven instance actually completed.
    let journal = scan_journal(&store.events_for_stream("rsaga-30")).unwrap();
    assert!(journal.outstanding.is_empty());
}

// ═══════════════════════════════════════════════════════════════════
// Registry trait shape (compile-level pin + record semantics)
// ═══════════════════════════════════════════════════════════════════

struct StaticRegistry {
    records: Vec<composable_rust_next::SagaInstanceRecord>,
}

impl composable_rust_next::SagaRegistry for StaticRegistry {
    type Error = std::convert::Infallible;

    async fn instances_with_outstanding_calls(
        &self,
        logic_tag: &str,
    ) -> Result<Vec<composable_rust_next::SagaInstanceRecord>, Self::Error> {
        Ok(self
            .records
            .iter()
            .filter(|r| r.logic_tag == logic_tag)
            .cloned()
            .collect())
    }
}

#[tokio::test]
async fn registry_filters_by_logic_tag() {
    use composable_rust_next::{SagaInstanceRecord, SagaRegistry};

    let registry = StaticRegistry {
        records: vec![
            SagaInstanceRecord {
                stream_id: StreamId::new("rsaga-1"),
                logic_tag: "rsaga".to_string(),
                outstanding_calls: 1,
                oldest_dispatched_at: chrono::Utc::now(),
            },
            SagaInstanceRecord {
                stream_id: StreamId::new("other-1"),
                logic_tag: "other".to_string(),
                outstanding_calls: 2,
                oldest_dispatched_at: chrono::Utc::now(),
            },
        ],
    };

    let mine = registry
        .instances_with_outstanding_calls("rsaga")
        .await
        .unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].stream_id, StreamId::new("rsaga-1"));
}
