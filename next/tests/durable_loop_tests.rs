//! Integration tests for the durable saga loop (`Handler::handle_durable`).
//!
//! These pin the WP-3a contract: per-completion durability, saga-driven
//! window top-up, journal exactness, cancellation-as-suspension, the
//! durable-mode error semantics, and internal expected-version threading.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_const_for_fn
)]

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use composable_rust_next::testing::{InMemoryAtomicPersist, InMemoryProjector};
use composable_rust_next::{
    BusinessLogic, BusinessResult, CALL_COMPLETED_EVENT_TYPE, CALL_DISPATCHED_EVENT_TYPE,
    CallCompleted, CallId, CancellationToken, Clock, DurableBusinessLogic, DurableOutcome,
    EventStore, FetchResult, Handler, HandlerBuilder, HandlerError, NoOpQueryFetcher,
    ProjectionQueries, QueryFetcher, SerializedEvent, StreamId, Subject, SubjectId, Version,
    scan_journal,
};
use tokio::sync::oneshot;

// ═══════════════════════════════════════════════════════════════════
// Shared fixtures
// ═══════════════════════════════════════════════════════════════════

mod common;
use common::{
    Gates, WErr, WEv, WIn, WState, WindowSaga, count_type, echo_executor, gated_executor, test_env,
    w_completion_input, w_event_type_name, w_stream_id, wait_for,
};

// ═══════════════════════════════════════════════════════════════════
// Acceptance scenario 2: straggler independence
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn straggler_completions_persist_independently() {
    let env = test_env();
    let store = env.event_store().clone();

    let (tx1, rx1) = oneshot::channel();
    let (tx2, rx2) = oneshot::channel();
    let (tx3, rx3) = oneshot::channel();
    let gates: Gates = Arc::new(Mutex::new(HashMap::from([(1, rx1), (2, rx2), (3, rx3)])));

    let handler = Arc::new(Handler::new(
        WindowSaga::new(3, 3),
        gated_executor(gates),
        NoOpQueryFetcher,
        env,
    ));
    let h = Arc::clone(&handler);
    let run = tokio::spawn(async move {
        h.handle_durable(WIn::Start { id: 7 }, CancellationToken::new())
            .await
    });

    // Fast call 1 completes; its cycle must persist while 2 and 3 are
    // still in flight (no barrier wait on the stragglers).
    tx1.send(101).unwrap();
    wait_for("first completion persisted", || {
        count_type(
            &store.events_for_stream("wsaga-7"),
            CALL_COMPLETED_EVENT_TYPE,
        ) == 1
    })
    .await;
    let events = store.events_for_stream("wsaga-7");
    assert_eq!(
        count_type(&events, "WProgress"),
        1,
        "the fast completion's domain event persisted before the stragglers returned"
    );

    tx2.send(102).unwrap();
    wait_for("second completion persisted", || {
        count_type(
            &store.events_for_stream("wsaga-7"),
            CALL_COMPLETED_EVENT_TYPE,
        ) == 2
    })
    .await;

    tx3.send(103).unwrap();
    let outcome = run.await.unwrap().unwrap();
    assert!(
        matches!(outcome, DurableOutcome::Completed { event_count: 5, .. }),
        "Started + 2×Progress + (Progress, Finished) = 5 domain events, got {outcome:?}"
    );

    let events = store.events_for_stream("wsaga-7");
    assert_eq!(count_type(&events, CALL_DISPATCHED_EVENT_TYPE), 3);
    assert_eq!(count_type(&events, CALL_COMPLETED_EVENT_TYPE), 3);
    let journal = scan_journal(&events).unwrap();
    assert!(journal.outstanding.is_empty());
    assert_eq!(journal.dispatched_count, 3);
}

#[tokio::test]
async fn saga_tops_up_window_on_each_completion() {
    let env = test_env();
    let store = env.event_store().clone();

    let (tx1, rx1) = oneshot::channel();
    let (tx2, rx2) = oneshot::channel();
    let (tx3, rx3) = oneshot::channel();
    let (tx4, rx4) = oneshot::channel();
    let gates: Gates = Arc::new(Mutex::new(HashMap::from([
        (1, rx1),
        (2, rx2),
        (3, rx3),
        (4, rx4),
    ])));
    let executor = gated_executor(gates);
    let executor_handle = executor.clone();

    let handler = Arc::new(Handler::new(
        WindowSaga::new(2, 4),
        executor,
        NoOpQueryFetcher,
        env,
    ));
    let h = Arc::clone(&handler);
    let run = tokio::spawn(async move {
        h.handle_durable(WIn::Start { id: 9 }, CancellationToken::new())
            .await
    });

    wait_for("initial window dispatched", || {
        executor_handle.dispatch_count() == 2
    })
    .await;
    let mut dispatched = executor_handle.dispatched();
    dispatched.sort_unstable();
    assert_eq!(dispatched, vec![1, 2]);

    // Completing call 1 must top the window back up to 2 (dispatch call 3)
    // while call 2 is still in flight — no barrier.
    tx1.send(101).unwrap();
    wait_for("top-up after first completion", || {
        executor_handle.dispatch_count() == 3
    })
    .await;
    assert_eq!(
        count_type(
            &store.events_for_stream("wsaga-9"),
            CALL_COMPLETED_EVENT_TYPE
        ),
        1,
        "call 2 has not completed; the top-up rode on call 1's completion alone"
    );

    tx2.send(102).unwrap();
    wait_for("second top-up", || executor_handle.dispatch_count() == 4).await;
    tx3.send(103).unwrap();
    tx4.send(104).unwrap();

    let outcome = run.await.unwrap().unwrap();
    assert!(matches!(outcome, DurableOutcome::Completed { .. }));
    assert_eq!(executor_handle.dispatch_count(), 4);
}

// ═══════════════════════════════════════════════════════════════════
// Acceptance scenario 4: cancellation = suspension
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cancellation_suspends_with_outstanding_journal() {
    let env = test_env();
    let store = env.event_store().clone();

    let (tx1, rx1) = oneshot::channel();
    let (_tx2, rx2) = oneshot::channel();
    let (_tx3, rx3) = oneshot::channel();
    let (_tx4, rx4) = oneshot::channel();
    let gates: Gates = Arc::new(Mutex::new(HashMap::from([
        (1, rx1),
        (2, rx2),
        (3, rx3),
        (4, rx4),
    ])));
    let executor = gated_executor(gates);
    let executor_handle = executor.clone();

    let cancel = CancellationToken::new();
    let handler = Arc::new(Handler::new(
        WindowSaga::new(2, 4),
        executor,
        NoOpQueryFetcher,
        env,
    ));
    let h = Arc::clone(&handler);
    let token = cancel.clone();
    let run = tokio::spawn(async move { h.handle_durable(WIn::Start { id: 4 }, token).await });

    // One completion lands (and tops up call 3), then we cancel with
    // calls 2 and 3 in flight.
    tx1.send(101).unwrap();
    wait_for("top-up dispatched", || {
        executor_handle.dispatch_count() == 3
    })
    .await;
    cancel.cancel();

    let outcome = run.await.unwrap().unwrap();
    let DurableOutcome::Suspended { mut outstanding } = outcome else {
        panic!("expected Suspended, got {outcome:?}");
    };
    outstanding.sort_unstable();
    assert_eq!(outstanding.len(), 2, "calls 2 and 3 were in flight");

    // The journal agrees exactly with the reported outstanding set.
    let journal = scan_journal(&store.events_for_stream("wsaga-4")).unwrap();
    let journal_outstanding: Vec<CallId> = journal.outstanding.keys().copied().collect();
    assert_eq!(journal_outstanding, outstanding);
    assert_eq!(journal.dispatched_count, 3);

    // No further dispatches after cancellation (call 4 never started).
    assert_eq!(executor_handle.dispatch_count(), 3);
}

#[tokio::test]
async fn pre_cancelled_token_suspends_before_dispatch() {
    let env = test_env();
    let store = env.event_store().clone();
    let executor = echo_executor();
    let executor_handle = executor.clone();

    let cancel = CancellationToken::new();
    cancel.cancel();

    let handler = Handler::new(WindowSaga::new(2, 4), executor, NoOpQueryFetcher, env);
    let outcome = handler
        .handle_durable(WIn::Start { id: 5 }, cancel)
        .await
        .unwrap();

    let DurableOutcome::Suspended { outstanding } = outcome else {
        panic!("expected Suspended, got {outcome:?}");
    };
    assert_eq!(outstanding.len(), 2, "the initial window was journaled");
    assert_eq!(
        executor_handle.dispatch_count(),
        0,
        "no call may start under a pre-cancelled token"
    );
    // The dispatched markers ARE persisted — resume will re-dispatch them.
    let journal = scan_journal(&store.events_for_stream("wsaga-5")).unwrap();
    assert_eq!(journal.outstanding.len(), 2);
}

// ═══════════════════════════════════════════════════════════════════
// Drain-mode cancellation
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn drain_defers_top_ups_and_suspends_when_in_flight_completes() {
    let env = test_env();
    let env_for_resume = env.clone(); // clones share the event store
    let store = env.event_store().clone();

    let (tx1, rx1) = oneshot::channel();
    let (tx2, rx2) = oneshot::channel();
    let (_tx3, rx3) = oneshot::channel();
    let (_tx4, rx4) = oneshot::channel();
    let gates: Gates = Arc::new(Mutex::new(HashMap::from([
        (1, rx1),
        (2, rx2),
        (3, rx3),
        (4, rx4),
    ])));
    let executor = gated_executor(gates);
    let executor_handle = executor.clone();

    let cancel = CancellationToken::new();
    let handler = Arc::new(Handler::new(
        WindowSaga::new(2, 4),
        executor,
        NoOpQueryFetcher,
        env,
    ));
    let h = Arc::clone(&handler);
    let token = cancel.clone();
    let run = tokio::spawn(async move { h.handle_durable(WIn::Start { id: 11 }, token).await });

    wait_for("initial window dispatched", || {
        executor_handle.dispatch_count() == 2
    })
    .await;

    // Drain, then let the two in-flight calls finish.
    cancel.drain();
    tx1.send(101).unwrap();
    tx2.send(102).unwrap();

    let outcome = run.await.unwrap().unwrap();
    let DurableOutcome::Suspended { outstanding } = outcome else {
        panic!("expected Suspended after drain, got {outcome:?}");
    };

    // Both in-flight completions journaled their cycles; the two top-ups
    // (calls 3 and 4) were journaled but never started.
    assert_eq!(
        executor_handle.dispatch_count(),
        2,
        "no call may start after drain"
    );
    let events = store.events_for_stream("wsaga-11");
    assert_eq!(count_type(&events, CALL_COMPLETED_EVENT_TYPE), 2);
    assert_eq!(count_type(&events, CALL_DISPATCHED_EVENT_TYPE), 4);
    let journal = scan_journal(&events).unwrap();
    assert_eq!(journal.outstanding.len(), 2);
    assert_eq!(
        outstanding,
        journal.outstanding.keys().copied().collect::<Vec<_>>(),
        "Suspended must report exactly the journal's outstanding set"
    );

    // Resume runs the deferred calls. (ResumableSaga can decode the same
    // u64 calls; seed its completion state from the journal so it knows
    // 2 of 4 are already done.)
    let completed_state: std::collections::HashSet<CallId> = events
        .iter()
        .filter(|e| e.event_type == CALL_DISPATCHED_EVENT_TYPE)
        .map(|e| CallId::from_version(e.version.unwrap()))
        .filter(|id| !journal.outstanding.contains_key(id))
        .collect();
    let resumer = Handler::new(
        ResumableCountSaga {
            total: 4,
            completed: Arc::new(Mutex::new(completed_state)),
        },
        echo_executor(),
        NoOpQueryFetcher,
        env_for_resume,
    );
    let outcome = resumer
        .resume(&StreamId::new("wsaga-11"), CancellationToken::new())
        .await
        .unwrap();
    assert!(
        matches!(outcome, DurableOutcome::Completed { .. }),
        "resume must run the drained top-ups to completion, got {outcome:?}"
    );
    let journal = scan_journal(&store.events_for_stream("wsaga-11")).unwrap();
    assert!(journal.outstanding.is_empty());
}

#[tokio::test]
async fn pre_drained_token_suspends_before_dispatch() {
    let env = test_env();
    let executor = echo_executor();
    let executor_handle = executor.clone();

    let cancel = CancellationToken::new();
    cancel.drain();

    let handler = Handler::new(WindowSaga::new(2, 4), executor, NoOpQueryFetcher, env);
    let outcome = handler
        .handle_durable(WIn::Start { id: 12 }, cancel)
        .await
        .unwrap();

    let DurableOutcome::Suspended { outstanding } = outcome else {
        panic!("expected Suspended, got {outcome:?}");
    };
    assert_eq!(outstanding.len(), 2, "the initial window was journaled");
    assert_eq!(
        executor_handle.dispatch_count(),
        0,
        "no call may start under a pre-drained token"
    );
}

#[tokio::test]
async fn done_while_draining_completes() {
    let env = test_env();

    let (tx1, rx1) = oneshot::channel();
    let (tx2, rx2) = oneshot::channel();
    let gates: Gates = Arc::new(Mutex::new(HashMap::from([(1, rx1), (2, rx2)])));
    let executor = gated_executor(gates);
    let executor_handle = executor.clone();

    let cancel = CancellationToken::new();
    let handler = Arc::new(Handler::new(
        ResumableCountSaga {
            total: 2,
            completed: Arc::new(Mutex::new(std::collections::HashSet::new())),
        },
        executor,
        NoOpQueryFetcher,
        env,
    ));
    let h = Arc::clone(&handler);
    let token = cancel.clone();
    let run = tokio::spawn(async move { h.handle_durable(WIn::Start { id: 13 }, token).await });

    wait_for("window dispatched", || {
        executor_handle.dispatch_count() == 2
    })
    .await;
    cancel.drain();
    tx1.send(101).unwrap();
    tx2.send(102).unwrap();

    let outcome = run.await.unwrap().unwrap();
    assert!(
        matches!(outcome, DurableOutcome::Completed { .. }),
        "a saga that finishes while draining must complete, got {outcome:?}"
    );
}

#[tokio::test]
async fn drain_then_cancel_aborts_in_flight() {
    let env = test_env();

    let (_tx1, rx1) = oneshot::channel();
    let (_tx2, rx2) = oneshot::channel();
    let gates: Gates = Arc::new(Mutex::new(HashMap::from([(1, rx1), (2, rx2)])));
    let executor = gated_executor(gates);
    let executor_handle = executor.clone();

    let cancel = CancellationToken::new();
    let handler = Arc::new(Handler::new(
        WindowSaga::new(2, 2),
        executor,
        NoOpQueryFetcher,
        env,
    ));
    let h = Arc::clone(&handler);
    let token = cancel.clone();
    let run = tokio::spawn(async move { h.handle_durable(WIn::Start { id: 14 }, token).await });

    wait_for("window dispatched", || {
        executor_handle.dispatch_count() == 2
    })
    .await;
    // Drain first (calls never complete), then escalate to abort.
    cancel.drain();
    cancel.cancel();

    let outcome = run.await.unwrap().unwrap();
    let DurableOutcome::Suspended { outstanding } = outcome else {
        panic!("expected Suspended after abort, got {outcome:?}");
    };
    assert_eq!(
        outstanding.len(),
        2,
        "both in-flight calls stay outstanding"
    );
}

/// Resume-safe count-based saga usable across handler instances (its
/// completion state lives in a shared handle standing in for a projection).
/// Decodes the same `u64` calls the other fixtures journal.
struct ResumableCountSaga {
    total: u64,
    completed: Arc<Mutex<std::collections::HashSet<CallId>>>,
}

impl BusinessLogic for ResumableCountSaga {
    type State = WState;
    type Input = WIn;
    type Event = WEv;
    type Error = WErr;
    type Call = u64;
    type CallResult = u64;
    type Response = ();

    fn stream_id(input: &WIn) -> StreamId {
        w_stream_id("wsaga", input)
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

impl DurableBusinessLogic for ResumableCountSaga {
    const LOGIC_TAG: &'static str = "wsaga";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> WIn {
        w_completion_input("wsaga", stream_id, call_id, result)
    }
}

// ═══════════════════════════════════════════════════════════════════
// Durable-mode contract errors (all before persisting)
// ═══════════════════════════════════════════════════════════════════

struct DoneEarlySaga;

impl BusinessLogic for DoneEarlySaga {
    type State = WState;
    type Input = WIn;
    type Event = WEv;
    type Error = WErr;
    type Call = u64;
    type CallResult = u64;
    type Response = ();

    fn stream_id(input: &WIn) -> StreamId {
        w_stream_id("dearly", input)
    }

    fn process(
        &self,
        input: WIn,
        _clock: &dyn Clock,
    ) -> Result<BusinessResult<WEv, u64, ()>, WErr> {
        match input {
            WIn::Start { id } => Ok(BusinessResult::Continue {
                events: vec![WEv::Started(id)],
                calls: vec![1, 2],
            }),
            // Buggy: declares itself done while call 2 is still in flight.
            WIn::Completion { id, .. } => Ok(BusinessResult::Done(vec![WEv::Finished(id)])),
        }
    }

    fn apply(&self, _state: &mut WState, _event: &WEv) {}

    fn event_type_name(event: &WEv) -> &'static str {
        w_event_type_name(event)
    }
}

impl DurableBusinessLogic for DoneEarlySaga {
    const LOGIC_TAG: &'static str = "dearly";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> WIn {
        w_completion_input("dearly", stream_id, call_id, result)
    }
}

#[tokio::test]
async fn done_with_outstanding_calls_errors_before_persist() {
    let env = test_env();
    let store = env.event_store().clone();

    // Call 2 never completes; call 1 echoes immediately.
    let (_tx2, rx2) = oneshot::channel();
    let gates: Gates = Arc::new(Mutex::new(HashMap::from([(2, rx2)])));

    let handler = Handler::new(DoneEarlySaga, gated_executor(gates), NoOpQueryFetcher, env);
    let err = handler
        .handle_durable(WIn::Start { id: 1 }, CancellationToken::new())
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            HandlerError::DoneWithOutstandingCalls { outstanding: 1 }
        ),
        "got {err:?}"
    );

    // Nothing from the Done cycle persisted: the stream holds only the
    // initial append (Started + 2 dispatched markers).
    let events = store.events_for_stream("dearly-1");
    assert_eq!(events.len(), 3);
    assert_eq!(count_type(&events, CALL_COMPLETED_EVENT_TYPE), 0);
    assert_eq!(count_type(&events, "WFinished"), 0);
}

struct StuckSaga;

impl BusinessLogic for StuckSaga {
    type State = WState;
    type Input = WIn;
    type Event = WEv;
    type Error = WErr;
    type Call = u64;
    type CallResult = u64;
    type Response = ();

    fn stream_id(input: &WIn) -> StreamId {
        w_stream_id("stuck", input)
    }

    fn process(
        &self,
        input: WIn,
        _clock: &dyn Clock,
    ) -> Result<BusinessResult<WEv, u64, ()>, WErr> {
        match input {
            // Buggy: Continue with no calls and nothing outstanding.
            WIn::Start { id } => Ok(BusinessResult::Continue {
                events: vec![WEv::Started(id)],
                calls: Vec::new(),
            }),
            WIn::Completion { .. } => Ok(BusinessResult::done_empty()),
        }
    }

    fn apply(&self, _state: &mut WState, _event: &WEv) {}

    fn event_type_name(event: &WEv) -> &'static str {
        w_event_type_name(event)
    }
}

impl DurableBusinessLogic for StuckSaga {
    const LOGIC_TAG: &'static str = "stuck";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> WIn {
        w_completion_input("stuck", stream_id, call_id, result)
    }
}

#[tokio::test]
async fn continue_with_no_calls_and_none_outstanding_is_stuck() {
    let env = test_env();
    let store = env.event_store().clone();

    let handler = Handler::new(StuckSaga, echo_executor(), NoOpQueryFetcher, env);
    let err = handler
        .handle_durable(WIn::Start { id: 1 }, CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(err, HandlerError::SagaStuck), "got {err:?}");
    assert!(
        store.events_for_stream("stuck-1").is_empty(),
        "nothing may persist from a stuck cycle"
    );
}

struct RespondFeedbackSaga;

impl BusinessLogic for RespondFeedbackSaga {
    type State = WState;
    type Input = WIn;
    type Event = WEv;
    type Error = WErr;
    type Call = u64;
    type CallResult = u64;
    type Response = ();

    fn stream_id(input: &WIn) -> StreamId {
        w_stream_id("respfb", input)
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
            // Buggy: a query response to a call completion.
            WIn::Completion { .. } => Ok(BusinessResult::Respond(())),
        }
    }

    fn apply(&self, _state: &mut WState, _event: &WEv) {}

    fn event_type_name(event: &WEv) -> &'static str {
        w_event_type_name(event)
    }
}

impl DurableBusinessLogic for RespondFeedbackSaga {
    const LOGIC_TAG: &'static str = "respfb";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> WIn {
        w_completion_input("respfb", stream_id, call_id, result)
    }
}

#[tokio::test]
async fn respond_in_feedback_cycle_errors() {
    let env = test_env();
    let store = env.event_store().clone();

    let handler = Handler::new(RespondFeedbackSaga, echo_executor(), NoOpQueryFetcher, env);
    let err = handler
        .handle_durable(WIn::Start { id: 1 }, CancellationToken::new())
        .await
        .unwrap_err();

    assert!(
        matches!(err, HandlerError::RespondInFeedbackCycle),
        "got {err:?}"
    );
    assert_eq!(
        count_type(
            &store.events_for_stream("respfb-1"),
            CALL_COMPLETED_EVENT_TYPE
        ),
        0,
        "the completion could not be journaled"
    );
}

struct EverToppingSaga {
    next_call: Arc<AtomicU64>,
}

impl EverToppingSaga {
    fn new() -> Self {
        Self {
            next_call: Arc::new(AtomicU64::new(2)),
        }
    }
}

impl BusinessLogic for EverToppingSaga {
    type State = WState;
    type Input = WIn;
    type Event = WEv;
    type Error = WErr;
    type Call = u64;
    type CallResult = u64;
    type Response = ();

    fn stream_id(input: &WIn) -> StreamId {
        w_stream_id("evert", input)
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
            // Buggy: dispatches forever, never Done.
            WIn::Completion { .. } => Ok(BusinessResult::Continue {
                events: Vec::new(),
                calls: vec![self.next_call.fetch_add(1, Ordering::SeqCst)],
            }),
        }
    }

    fn apply(&self, _state: &mut WState, _event: &WEv) {}

    fn event_type_name(event: &WEv) -> &'static str {
        w_event_type_name(event)
    }
}

impl DurableBusinessLogic for EverToppingSaga {
    const LOGIC_TAG: &'static str = "evert";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> WIn {
        w_completion_input("evert", stream_id, call_id, result)
    }
}

#[tokio::test]
async fn max_total_calls_guard_stops_runaway_saga() {
    let env = test_env();
    let executor = echo_executor();
    let executor_handle = executor.clone();

    let handler = HandlerBuilder::new(EverToppingSaga::new())
        .call_executor(executor)
        .query_fetcher(NoOpQueryFetcher)
        .environment(env)
        .max_total_calls(3)
        .build();

    let err = handler
        .handle_durable(WIn::Start { id: 1 }, CancellationToken::new())
        .await
        .unwrap_err();

    assert!(
        matches!(err, HandlerError::TotalCallsExceeded { max_total_calls: 3 }),
        "got {err:?}"
    );
    assert!(
        executor_handle.dispatch_count() <= 3,
        "the guard must fire before the fourth dispatch"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Version threading and conflict retry
// ═══════════════════════════════════════════════════════════════════

/// Retry-idempotent single-call saga (its `process` is pure, so a
/// version-conflict retry re-running it is harmless).
struct SingleCallSaga;

impl BusinessLogic for SingleCallSaga {
    type State = WState;
    type Input = WIn;
    type Event = WEv;
    type Error = WErr;
    type Call = u64;
    type CallResult = u64;
    type Response = ();

    fn stream_id(input: &WIn) -> StreamId {
        w_stream_id("single", input)
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
            WIn::Completion { id, result, .. } => Ok(BusinessResult::Done(vec![
                WEv::Progress(result),
                WEv::Finished(id),
            ])),
        }
    }

    fn apply(&self, _state: &mut WState, _event: &WEv) {}

    fn event_type_name(event: &WEv) -> &'static str {
        w_event_type_name(event)
    }
}

impl DurableBusinessLogic for SingleCallSaga {
    const LOGIC_TAG: &'static str = "single";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> WIn {
        w_completion_input("single", stream_id, call_id, result)
    }
}

#[tokio::test]
async fn version_conflict_retry_preserves_completion() {
    let env = test_env();
    let store = env.event_store().clone();

    let (tx1, rx1) = oneshot::channel();
    let gates: Gates = Arc::new(Mutex::new(HashMap::from([(1, rx1)])));
    let executor = gated_executor(gates);
    let executor_handle = executor.clone();

    let handler = Arc::new(Handler::new(
        SingleCallSaga,
        executor,
        NoOpQueryFetcher,
        env,
    ));
    let h = Arc::clone(&handler);
    let run = tokio::spawn(async move {
        h.handle_durable(WIn::Start { id: 3 }, CancellationToken::new())
            .await
    });

    wait_for("call dispatched", || executor_handle.dispatch_count() == 1).await;

    // A concurrent writer appends to the stream while the call is in
    // flight, invalidating the loop's carried expected version.
    store
        .append(
            &StreamId::new("single-3"),
            None,
            vec![SerializedEvent {
                event_type: "Foreign".to_string(),
                payload: Vec::new(),
                metadata: None,
                version: None,
                stream_id: None,
            }],
        )
        .await
        .unwrap();

    tx1.send(9).unwrap();
    let outcome = run.await.unwrap().unwrap();
    assert!(
        matches!(outcome, DurableOutcome::Completed { .. }),
        "the completion cycle must retry past the conflict, got {outcome:?}"
    );

    let events = store.events_for_stream("single-3");
    assert_eq!(
        count_type(&events, CALL_COMPLETED_EVENT_TYPE),
        1,
        "exactly one completion marker despite the retry"
    );
    assert_eq!(count_type(&events, "Foreign"), 1);
    let journal = scan_journal(&events).unwrap();
    assert!(journal.outstanding.is_empty());
}

/// Fetcher whose `expected_version` is frozen: correct only for `Start`,
/// permanently stale for every completion. A loop that trusted it would
/// version-conflict to death; internal version threading must ignore it.
#[derive(Debug, Clone)]
struct FrozenVersionFetcher;

impl<P: ProjectionQueries> QueryFetcher<WIn, P> for FrozenVersionFetcher {
    type Error = Infallible;

    async fn fetch(&self, input: WIn, _projections: &P) -> Result<FetchResult<WIn>, Infallible> {
        let version = match &input {
            WIn::Start { .. } => None,
            WIn::Completion { .. } => Some(Version::new(1)),
        };
        Ok(FetchResult::new(input, version))
    }
}

// ═══════════════════════════════════════════════════════════════════
// Stuck-call watchdog
// ═══════════════════════════════════════════════════════════════════

#[tokio::test(start_paused = true)]
async fn stuck_call_watchdog_fires_and_run_is_resumable() {
    let env = test_env();
    let env_for_resume = env.clone();
    let store = env.event_store().clone();

    // The single call never completes.
    let (_tx1, rx1) = oneshot::channel();
    let gates: Gates = Arc::new(Mutex::new(HashMap::from([(1, rx1)])));

    let handler = HandlerBuilder::new(SingleCallSaga)
        .call_executor(gated_executor(gates))
        .query_fetcher(NoOpQueryFetcher)
        .environment(env)
        .max_call_duration(std::time::Duration::from_secs(5))
        .build();

    // Paused tokio time auto-advances when everything is idle: the gated
    // call parks the loop, the watchdog deadline fires "5 seconds" later.
    let err = handler
        .handle_durable(WIn::Start { id: 21 }, CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, HandlerError::CallStuck { .. }),
        "the watchdog must trip on the hung call, got {err:?}"
    );

    // Crash-equivalent: the call stays outstanding in the journal...
    let journal = scan_journal(&store.events_for_stream("single-21")).unwrap();
    assert_eq!(journal.outstanding.len(), 1);

    // ...and resume with a healthy executor completes the saga.
    let resumer = Handler::new(
        SingleCallSaga,
        echo_executor(),
        NoOpQueryFetcher,
        env_for_resume,
    );
    let outcome = resumer
        .resume(&StreamId::new("single-21"), CancellationToken::new())
        .await
        .unwrap();
    assert!(
        matches!(outcome, DurableOutcome::Completed { .. }),
        "resume must re-dispatch and complete the stuck call, got {outcome:?}"
    );
}

#[tokio::test]
async fn stale_fetcher_version_does_not_livelock() {
    let env = test_env();
    let store = env.event_store().clone();

    let handler = Handler::new(
        WindowSaga::new(1, 2),
        echo_executor(),
        FrozenVersionFetcher,
        env,
    );
    let outcome = handler
        .handle_durable(WIn::Start { id: 6 }, CancellationToken::new())
        .await
        .unwrap();

    assert!(
        matches!(outcome, DurableOutcome::Completed { .. }),
        "feedback cycles must use the carried version, not the stale fetch, got {outcome:?}"
    );
    let journal = scan_journal(&store.events_for_stream("wsaga-6")).unwrap();
    assert!(journal.outstanding.is_empty());
    assert_eq!(journal.dispatched_count, 2);
}

// ═══════════════════════════════════════════════════════════════════
// Journal shape
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn call_ids_are_marker_stream_versions() {
    let env = test_env();
    let store = env.event_store().clone();

    let handler = Handler::new(
        WindowSaga::new(2, 3),
        echo_executor(),
        NoOpQueryFetcher,
        env,
    );
    let outcome = handler
        .handle_durable(WIn::Start { id: 2 }, CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::Completed { .. }));

    let events = store.events_for_stream("wsaga-2");

    let dispatched_versions: Vec<u64> = events
        .iter()
        .filter(|e| e.event_type == CALL_DISPATCHED_EVENT_TYPE)
        .map(|e| e.version.unwrap().as_u64())
        .collect();

    let mut completed_ids: Vec<u64> = events
        .iter()
        .filter(|e| e.event_type == CALL_COMPLETED_EVENT_TYPE)
        .map(|e| {
            let marker: CallCompleted = bincode::deserialize(&e.payload).unwrap();
            assert_eq!(marker.stream_id, "wsaga-2");
            marker.call_id.as_u64()
        })
        .collect();
    completed_ids.sort_unstable();

    let mut expected = dispatched_versions;
    expected.sort_unstable();
    assert_eq!(
        completed_ids, expected,
        "every completion's CallId must be its dispatched marker's stream version"
    );
}

struct WaitingSaga {
    completions: Arc<AtomicU64>,
}

impl WaitingSaga {
    fn new() -> Self {
        Self {
            completions: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl BusinessLogic for WaitingSaga {
    type State = WState;
    type Input = WIn;
    type Event = WEv;
    type Error = WErr;
    type Call = u64;
    type CallResult = u64;
    type Response = ();

    fn stream_id(input: &WIn) -> StreamId {
        w_stream_id("waiting", input)
    }

    fn process(
        &self,
        input: WIn,
        _clock: &dyn Clock,
    ) -> Result<BusinessResult<WEv, u64, ()>, WErr> {
        match input {
            WIn::Start { id } => Ok(BusinessResult::Continue {
                events: vec![WEv::Started(id)],
                calls: vec![1, 2],
            }),
            WIn::Completion { id, .. } => {
                let done = self.completions.fetch_add(1, Ordering::SeqCst) + 1;
                if done == 2 {
                    Ok(BusinessResult::Done(vec![WEv::Finished(id)]))
                } else {
                    // Keep waiting: no events, no new calls.
                    Ok(BusinessResult::Continue {
                        events: Vec::new(),
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

impl DurableBusinessLogic for WaitingSaga {
    const LOGIC_TAG: &'static str = "waiting";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> WIn {
        w_completion_input("waiting", stream_id, call_id, result)
    }
}

#[tokio::test]
async fn marker_only_append_for_waiting_cycle() {
    let env = test_env();
    let store = env.event_store().clone();

    let (tx1, rx1) = oneshot::channel();
    let (tx2, rx2) = oneshot::channel();
    let gates: Gates = Arc::new(Mutex::new(HashMap::from([(1, rx1), (2, rx2)])));

    let handler = Arc::new(Handler::new(
        WaitingSaga::new(),
        gated_executor(gates),
        NoOpQueryFetcher,
        env,
    ));
    let h = Arc::clone(&handler);
    let run = tokio::spawn(async move {
        h.handle_durable(WIn::Start { id: 8 }, CancellationToken::new())
            .await
    });

    wait_for("initial append", || {
        store.events_for_stream("waiting-8").len() == 3
    })
    .await;

    // First completion: the saga emits nothing, but the completion itself
    // must still be durable — a marker-only append.
    tx1.send(101).unwrap();
    wait_for("waiting cycle persisted", || {
        store.events_for_stream("waiting-8").len() == 4
    })
    .await;
    let events = store.events_for_stream("waiting-8");
    assert_eq!(
        events.last().unwrap().event_type,
        CALL_COMPLETED_EVENT_TYPE,
        "the waiting cycle's append contains exactly the completion marker"
    );

    tx2.send(102).unwrap();
    let outcome = run.await.unwrap().unwrap();
    assert!(matches!(outcome, DurableOutcome::Completed { .. }));
}

// ═══════════════════════════════════════════════════════════════════
// Initial-cycle pass-throughs (aggregate-style Done / query Respond)
// ═══════════════════════════════════════════════════════════════════

struct InitialOutcomeSaga;

#[derive(Clone)]
enum IOIn {
    DoneWithEvent { id: u64 },
    DoneEmpty { id: u64 },
    Query { id: u64 },
}

impl BusinessLogic for InitialOutcomeSaga {
    type State = WState;
    type Input = IOIn;
    type Event = WEv;
    type Error = WErr;
    type Call = u64;
    type CallResult = u64;
    type Response = u64;

    fn stream_id(input: &IOIn) -> StreamId {
        let id = match input {
            IOIn::DoneWithEvent { id } | IOIn::DoneEmpty { id } | IOIn::Query { id } => *id,
        };
        StreamId::new(format!("io-{id}"))
    }

    fn process(
        &self,
        input: IOIn,
        _clock: &dyn Clock,
    ) -> Result<BusinessResult<WEv, u64, u64>, WErr> {
        match input {
            IOIn::DoneWithEvent { id } => Ok(BusinessResult::Done(vec![WEv::Started(id)])),
            IOIn::DoneEmpty { .. } => Ok(BusinessResult::done_empty()),
            IOIn::Query { id } => Ok(BusinessResult::Respond(id * 10)),
        }
    }

    fn apply(&self, _state: &mut WState, _event: &WEv) {}

    fn event_type_name(event: &WEv) -> &'static str {
        w_event_type_name(event)
    }
}

impl DurableBusinessLogic for InitialOutcomeSaga {
    const LOGIC_TAG: &'static str = "io";

    fn completion_input(_stream_id: &StreamId, _call_id: CallId, _result: u64) -> IOIn {
        IOIn::DoneEmpty { id: 0 }
    }
}

#[tokio::test]
async fn initial_done_and_respond_pass_through() {
    let env = test_env();
    let store = env.event_store().clone();
    let handler = Handler::new(InitialOutcomeSaga, echo_executor(), NoOpQueryFetcher, env);

    let outcome = handler
        .handle_durable(IOIn::DoneWithEvent { id: 1 }, CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        DurableOutcome::Completed { event_count: 1, .. }
    ));
    assert_eq!(store.events_for_stream("io-1").len(), 1);

    let outcome = handler
        .handle_durable(IOIn::DoneEmpty { id: 2 }, CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        DurableOutcome::Completed { event_count: 0, .. }
    ));
    assert!(store.events_for_stream("io-2").is_empty());

    let outcome = handler
        .handle_durable(IOIn::Query { id: 3 }, CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::Query(30)));
    assert!(store.events_for_stream("io-3").is_empty());
}

// ═══════════════════════════════════════════════════════════════════
// Metadata: origin subject threading
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn origin_subject_id_threaded_across_cycles() {
    let subject = Subject::Authenticated {
        id: SubjectId::new("user-9"),
        attributes: HashMap::new(),
    };
    let env = test_env().with_subject(subject);
    let store = env.event_store().clone();

    let handler = Handler::new(
        WindowSaga::new(1, 2),
        echo_executor(),
        NoOpQueryFetcher,
        env,
    );
    let outcome = handler
        .handle_durable(WIn::Start { id: 1 }, CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::Completed { .. }));

    let events = store.events_for_stream("wsaga-1");
    assert!(events.len() >= 5, "multiple cycles' events expected");
    for event in &events {
        let metadata = event
            .metadata
            .as_ref()
            .expect("every durable event carries metadata");
        assert_eq!(
            metadata.origin_subject_id,
            Some(SubjectId::new("user-9")),
            "origin must thread through every cycle, including markers ({})",
            event.event_type
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
// Atomic persist path
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn atomic_persist_path_journals_markers() {
    let env = test_env();
    let store = env.event_store().clone();
    let atomic_projector = InMemoryProjector::new();
    // The atomic persist MUST share the environment's store (clones share
    // state), or later loads over env.event_store() would see nothing.
    let atomic = InMemoryAtomicPersist::new(store.clone(), atomic_projector.clone());
    let env = env.with_atomic_persist(Arc::new(atomic));
    let env_handle = env.clone();

    let handler = Handler::new(
        WindowSaga::new(1, 1),
        echo_executor(),
        NoOpQueryFetcher,
        env,
    );
    let outcome = handler
        .handle_durable(WIn::Start { id: 1 }, CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(outcome, DurableOutcome::Completed { .. }));

    // Initial append: Started + dispatched marker. Final: Progress,
    // Finished + completed marker. All five went through the atomic path.
    let projected = atomic_projector.projected_events();
    assert_eq!(projected.len(), 5);
    assert_eq!(count_type(&projected, CALL_DISPATCHED_EVENT_TYPE), 1);
    assert_eq!(count_type(&projected, CALL_COMPLETED_EVENT_TYPE), 1);

    // The environment's non-atomic projector was bypassed entirely.
    assert_eq!(env_handle.projector().projection_count(), 0);

    let journal = scan_journal(&store.events_for_stream("wsaga-1")).unwrap();
    assert!(journal.outstanding.is_empty());
}

// ═══════════════════════════════════════════════════════════════════
// process() error on a feedback cycle (documented sharp edge)
// ═══════════════════════════════════════════════════════════════════

struct FailFeedbackSaga;

impl BusinessLogic for FailFeedbackSaga {
    type State = WState;
    type Input = WIn;
    type Event = WEv;
    type Error = WErr;
    type Call = u64;
    type CallResult = u64;
    type Response = ();

    fn stream_id(input: &WIn) -> StreamId {
        w_stream_id("failfb", input)
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
            WIn::Completion { .. } => Err(WErr),
        }
    }

    fn apply(&self, _state: &mut WState, _event: &WEv) {}

    fn event_type_name(event: &WEv) -> &'static str {
        w_event_type_name(event)
    }
}

impl DurableBusinessLogic for FailFeedbackSaga {
    const LOGIC_TAG: &'static str = "failfb";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> WIn {
        w_completion_input("failfb", stream_id, call_id, result)
    }
}

#[tokio::test]
async fn process_error_on_feedback_leaves_call_outstanding() {
    let env = test_env();
    let store = env.event_store().clone();

    let handler = Handler::new(FailFeedbackSaga, echo_executor(), NoOpQueryFetcher, env);
    let err = handler
        .handle_durable(WIn::Start { id: 1 }, CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(err, HandlerError::Business(_)), "got {err:?}");

    // The completion was deliberately NOT journaled: the call stays
    // outstanding and will re-run on resume (at-least-once).
    let events = store.events_for_stream("failfb-1");
    assert_eq!(count_type(&events, CALL_COMPLETED_EVENT_TYPE), 0);
    let journal = scan_journal(&events).unwrap();
    assert_eq!(journal.outstanding.len(), 1);
}
