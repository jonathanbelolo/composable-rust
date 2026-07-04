//! Shared fixtures for the durable-saga test suites.

#![allow(
    dead_code, // each test binary uses a different subset
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_const_for_fn
)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use composable_rust_next::testing::{RecordingUnitExecutor, TestEnvironment};
use composable_rust_next::{
    BusinessLogic, BusinessResult, CallId, Clock, DurableBusinessLogic, FixedClock,
    SerializedEvent, StreamId,
};
use tokio::sync::oneshot;

#[derive(Clone)]
#[allow(dead_code)] // `call_id` is carried for realism; some sagas key off `result`
pub enum WIn {
    Start {
        id: u64,
    },
    Completion {
        id: u64,
        call_id: CallId,
        result: u64,
    },
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum WEv {
    Started(u64),
    Progress(u64),
    Finished(u64),
}

#[derive(Debug, thiserror::Error)]
#[error("saga error")]
pub struct WErr;

#[derive(Default)]
pub struct WState;

pub fn w_event_type_name(event: &WEv) -> &'static str {
    match event {
        WEv::Started(_) => "WStarted",
        WEv::Progress(_) => "WProgress",
        WEv::Finished(_) => "WFinished",
    }
}

pub fn w_stream_id(prefix: &str, input: &WIn) -> StreamId {
    let id = match input {
        WIn::Start { id } | WIn::Completion { id, .. } => *id,
    };
    StreamId::new(format!("{prefix}-{id}"))
}

pub fn w_completion_input(prefix: &str, stream_id: &StreamId, call_id: CallId, result: u64) -> WIn {
    let id = stream_id
        .as_str()
        .strip_prefix(prefix)
        .and_then(|s| s.strip_prefix('-'))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    WIn::Completion {
        id,
        call_id,
        result,
    }
}

/// Saga that keeps `window` calls in flight until `total` calls completed.
///
/// Internal atomics stand in for projection state (tests run under
/// `NoOpQueryFetcher`); no test using this saga triggers version-conflict
/// retries, which would re-run `process` and double-count. NOT resume-safe
/// (counters die with the instance) — recovery tests use resume-safe sagas.
pub struct WindowSaga {
    pub window: u64,
    pub total: u64,
    pub next_call: Arc<AtomicU64>,
    pub completions: Arc<AtomicU64>,
}

impl WindowSaga {
    pub fn new(window: u64, total: u64) -> Self {
        Self {
            window,
            total,
            next_call: Arc::new(AtomicU64::new(1)),
            completions: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl BusinessLogic for WindowSaga {
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
            WIn::Start { id } => {
                let width = self.window.min(self.total);
                let calls: Vec<u64> = (1..=width).collect();
                self.next_call.store(width + 1, Ordering::SeqCst);
                Ok(BusinessResult::Continue {
                    events: vec![WEv::Started(id)],
                    calls,
                })
            },
            WIn::Completion { id, result, .. } => {
                let done = self.completions.fetch_add(1, Ordering::SeqCst) + 1;
                if done == self.total {
                    return Ok(BusinessResult::Done(vec![
                        WEv::Progress(result),
                        WEv::Finished(id),
                    ]));
                }
                let next = self.next_call.load(Ordering::SeqCst);
                let calls = if next <= self.total {
                    self.next_call.store(next + 1, Ordering::SeqCst);
                    vec![next]
                } else {
                    Vec::new()
                };
                Ok(BusinessResult::Continue {
                    events: vec![WEv::Progress(result)],
                    calls,
                })
            },
        }
    }

    fn apply(&self, _state: &mut WState, _event: &WEv) {}

    fn event_type_name(event: &WEv) -> &'static str {
        w_event_type_name(event)
    }
}

impl DurableBusinessLogic for WindowSaga {
    const LOGIC_TAG: &'static str = "wsaga";

    fn completion_input(stream_id: &StreamId, call_id: CallId, result: u64) -> WIn {
        w_completion_input("wsaga", stream_id, call_id, result)
    }
}

pub type Gates = Arc<Mutex<HashMap<u64, oneshot::Receiver<u64>>>>;

/// Executor whose listed calls complete only when their oneshot gate fires;
/// unlisted calls echo immediately.
pub fn gated_executor(gates: Gates) -> RecordingUnitExecutor<u64, u64> {
    RecordingUnitExecutor::new(move |call: u64| {
        let gate = gates.lock().unwrap().remove(&call);
        async move {
            match gate {
                Some(rx) => rx.await.unwrap_or(call),
                None => call,
            }
        }
    })
}

pub fn echo_executor() -> RecordingUnitExecutor<u64, u64> {
    RecordingUnitExecutor::new(|call: u64| async move { call })
}

pub fn test_env() -> TestEnvironment<FixedClock> {
    TestEnvironment::new(FixedClock::new(Utc::now()))
}

pub fn count_type(events: &[SerializedEvent], event_type: &str) -> usize {
    events.iter().filter(|e| e.event_type == event_type).count()
}

pub async fn wait_for(what: &str, cond: impl Fn() -> bool) {
    let deadline = tokio::time::timeout(Duration::from_secs(5), async {
        while !cond() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(deadline.is_ok(), "timeout waiting for: {what}");
}
