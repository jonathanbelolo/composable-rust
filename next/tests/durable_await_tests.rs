//! Integration tests for the first-class saga `Await` primitive: a durable
//! saga parks on a correlation (`BusinessResult::Await`), and is later woken via
//! `Handler::resume_awaiting`.
//!
//! These pin: the park→wake→resume happy path (in-memory, atomic-persist), the
//! two fail-loud guards (`AwaitRequiresAtomicPersist` / `AwaitRequiresDurable`),
//! and the `scan_awaiting` structural dedup of duplicate / wrong-correlation
//! wakes (hostile-review SB-1 / MA-2 / MI-3).

#![allow(
    dead_code, // Completion's call_id/result exist for the completion_input signature
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_const_for_fn
)]

mod common;
use common::echo_executor;

use std::sync::Arc;

use chrono::Utc;
use composable_rust_next::testing::{InMemoryAtomicPersist, InMemoryEventStore, InMemoryProjector, TestEnvironment};
use composable_rust_next::{
    BusinessLogic, BusinessResult, CallId, CancellationToken, Clock, Correlation,
    DurableBusinessLogic, DurableOutcome, FixedClock, Handler, HandlerError, NoOpQueryFetcher,
    StreamId, scan_awaiting,
};

// ── A minimal durable saga that parks on Start and finishes on Wake ──────────

#[derive(Clone)]
enum AwaitIn {
    Start { id: u64 },
    Wake { id: u64 },
    Completion { id: u64, call_id: CallId, result: u64 },
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum AwaitEv {
    Started(u64),
    Reserved(u64),
    Finished(u64),
}

#[derive(Debug, thiserror::Error)]
#[error("await saga error")]
struct AwaitErr;

#[derive(Default)]
struct AwaitState;

/// On `Start` the saga parks on `(order_id, 42)`. On `Wake` it finishes —
/// unless `wake_dispatches`, in which case it dispatches a call first (so the
/// wake→completion-loop path is exercised).
struct AwaitSaga {
    wake_dispatches: bool,
}

impl BusinessLogic for AwaitSaga {
    type State = AwaitState;
    type Input = AwaitIn;
    type Event = AwaitEv;
    type Error = AwaitErr;
    type Call = u64;
    type CallResult = u64;
    type Response = ();

    fn stream_id(input: &AwaitIn) -> StreamId {
        let id = match input {
            AwaitIn::Start { id } | AwaitIn::Wake { id } | AwaitIn::Completion { id, .. } => *id,
        };
        StreamId::new(format!("await-saga-{id}"))
    }

    fn process(
        &self,
        input: AwaitIn,
        _clock: &dyn Clock,
    ) -> Result<BusinessResult<AwaitEv, u64, ()>, AwaitErr> {
        match input {
            AwaitIn::Start { id } => Ok(BusinessResult::Await {
                events: vec![AwaitEv::Started(id)],
                resume_on: Correlation::new("order_id", "42"),
                timeout: None,
            }),
            AwaitIn::Wake { id } => {
                if self.wake_dispatches {
                    Ok(BusinessResult::Continue {
                        events: vec![AwaitEv::Reserved(id)],
                        calls: vec![id],
                    })
                } else {
                    Ok(BusinessResult::Done(vec![AwaitEv::Finished(id)]))
                }
            },
            AwaitIn::Completion { id, .. } => Ok(BusinessResult::Done(vec![AwaitEv::Finished(id)])),
        }
    }

    fn apply(&self, _state: &mut AwaitState, _event: &AwaitEv) {}

    fn event_type_name(event: &AwaitEv) -> &'static str {
        match event {
            AwaitEv::Started(_) => "AwaitStarted",
            AwaitEv::Reserved(_) => "AwaitReserved",
            AwaitEv::Finished(_) => "AwaitFinished",
        }
    }
}

impl DurableBusinessLogic for AwaitSaga {
    const LOGIC_TAG: &'static str = "await-saga";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> AwaitIn {
        let id = stream_id
            .as_str()
            .strip_prefix("await-saga-")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        AwaitIn::Completion { id, call_id, result }
    }
}

/// A `TestEnvironment` whose atomic-persist path writes to the SAME shared
/// in-memory store `env.event_store()` reads from — required so
/// `resume_awaiting` (which loads via `env.event_store()`) observes the
/// atomically-appended `$saga.awaiting` marker.
fn await_env() -> (TestEnvironment<FixedClock>, InMemoryEventStore) {
    let env0 = TestEnvironment::new(FixedClock::new(Utc::now()));
    let store = env0.event_store().clone(); // InMemoryEventStore is share-on-clone
    let ap = InMemoryAtomicPersist::new(store.clone(), InMemoryProjector::new());
    let env = env0.with_atomic_persist(Arc::new(ap));
    (env, store)
}

fn corr() -> Correlation {
    Correlation::new("order_id", "42")
}

#[tokio::test]
async fn park_then_wake_completes() {
    let (env, store) = await_env();
    let handler = Handler::new(
        AwaitSaga { wake_dispatches: false },
        echo_executor(),
        NoOpQueryFetcher,
        env,
    );
    let sid = StreamId::new("await-saga-1");

    // Park: Start → Await.
    let outcome = handler
        .handle_durable(AwaitIn::Start { id: 1 }, CancellationToken::new())
        .await
        .unwrap();
    assert!(
        matches!(outcome, DurableOutcome::Awaiting { .. }),
        "Start should park, got {outcome:?}"
    );
    assert_eq!(
        scan_awaiting(&store.events_for_stream("await-saga-1")).unwrap(),
        Some(corr()),
        "the $saga.awaiting marker registers the live park"
    );

    // Wake: resume_awaiting → Done.
    let outcome = handler
        .resume_awaiting(&sid, corr(), AwaitIn::Wake { id: 1 }, CancellationToken::new())
        .await
        .unwrap();
    assert!(
        matches!(outcome, DurableOutcome::Completed { .. }),
        "the wake should complete the saga, got {outcome:?}"
    );
    assert_eq!(
        scan_awaiting(&store.events_for_stream("await-saga-1")).unwrap(),
        None,
        "the wake's awaiting_cleared marker deregistered the park"
    );
}

#[tokio::test]
async fn await_without_atomic_persist_is_rejected() {
    // Default TestEnvironment has no atomic_persist.
    let env = TestEnvironment::new(FixedClock::new(Utc::now()));
    let handler = Handler::new(
        AwaitSaga { wake_dispatches: false },
        echo_executor(),
        NoOpQueryFetcher,
        env,
    );
    let result = handler
        .handle_durable(AwaitIn::Start { id: 1 }, CancellationToken::new())
        .await;
    assert!(
        matches!(result, Err(HandlerError::AwaitRequiresAtomicPersist)),
        "Await without atomic_persist must fail loud (else silent lost saga), got {result:?}"
    );
}

#[tokio::test]
async fn await_on_the_batch_path_is_rejected() {
    let env = TestEnvironment::new(FixedClock::new(Utc::now()));
    let handler = Handler::new(
        AwaitSaga { wake_dispatches: false },
        echo_executor(),
        NoOpQueryFetcher,
        env,
    );
    let result = handler.handle(AwaitIn::Start { id: 1 }).await;
    assert!(
        matches!(result, Err(HandlerError::AwaitRequiresDurable)),
        "Await on the non-durable batch path must be rejected, got {result:?}"
    );
}

#[tokio::test]
async fn duplicate_wake_noops() {
    let (env, _store) = await_env();
    let handler = Handler::new(
        AwaitSaga { wake_dispatches: false },
        echo_executor(),
        NoOpQueryFetcher,
        env,
    );
    let sid = StreamId::new("await-saga-1");
    handler
        .handle_durable(AwaitIn::Start { id: 1 }, CancellationToken::new())
        .await
        .unwrap();
    // First wake completes the saga (clears the park).
    handler
        .resume_awaiting(&sid, corr(), AwaitIn::Wake { id: 1 }, CancellationToken::new())
        .await
        .unwrap();
    // A duplicate delivery finds no live park → structural no-op.
    let dup = handler
        .resume_awaiting(&sid, corr(), AwaitIn::Wake { id: 1 }, CancellationToken::new())
        .await
        .unwrap();
    assert!(
        matches!(dup, DurableOutcome::NoOutstandingCalls),
        "a duplicate wake must no-op (scan_awaiting dedup), got {dup:?}"
    );
}

#[tokio::test]
async fn wrong_correlation_wake_leaves_the_park_intact() {
    let (env, store) = await_env();
    let handler = Handler::new(
        AwaitSaga { wake_dispatches: false },
        echo_executor(),
        NoOpQueryFetcher,
        env,
    );
    let sid = StreamId::new("await-saga-1");
    handler
        .handle_durable(AwaitIn::Start { id: 1 }, CancellationToken::new())
        .await
        .unwrap();
    // Wake with the WRONG correlation → no-op; the real park stays registered.
    let out = handler
        .resume_awaiting(
            &sid,
            Correlation::new("order_id", "99"),
            AwaitIn::Wake { id: 1 },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(
        matches!(&out, DurableOutcome::Awaiting { correlation } if *correlation == corr()),
        "a wrong-correlation wake must no-op and report the live park, got {out:?}"
    );
    assert_eq!(
        scan_awaiting(&store.events_for_stream("await-saga-1")).unwrap(),
        Some(corr()),
        "the real park must not be cleared by a wrong-correlation wake"
    );
}

#[tokio::test]
async fn wake_that_dispatches_calls_is_driven_to_completion() {
    let (env, store) = await_env();
    let handler = Handler::new(
        AwaitSaga { wake_dispatches: true },
        echo_executor(),
        NoOpQueryFetcher,
        env,
    );
    let sid = StreamId::new("await-saga-1");
    handler
        .handle_durable(AwaitIn::Start { id: 1 }, CancellationToken::new())
        .await
        .unwrap();
    // The wake dispatches a call; the normal completion loop drives it to Done.
    let out = handler
        .resume_awaiting(&sid, corr(), AwaitIn::Wake { id: 1 }, CancellationToken::new())
        .await
        .unwrap();
    assert!(
        matches!(out, DurableOutcome::Completed { .. }),
        "a wake that dispatches calls should drive to completion, got {out:?}"
    );
    assert_eq!(
        scan_awaiting(&store.events_for_stream("await-saga-1")).unwrap(),
        None,
        "the wake cleared the park even though it also dispatched a call"
    );
}
