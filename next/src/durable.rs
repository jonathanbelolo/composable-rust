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

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{BusinessLogic, SerializationError, SerializedEvent, StreamId, Version};

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
