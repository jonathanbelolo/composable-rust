//! [`EventStore`] conformance suite
//!
//! Reusable checks that pin the semantics every [`EventStore`] implementation
//! must share. Two cross-store divergences shipped historically because each
//! store's tests encoded its own behavior (`load(from_version)` inclusivity;
//! conflict classification masked behind aborted transactions) — this suite
//! makes the contract executable.
//!
//! # Usage
//!
//! Run the aggregator against every implementation, with a `prefix` that
//! keeps stream IDs unique per store/run (safe on shared databases):
//!
//! ```rust,ignore
//! use composable_rust_next::conformance;
//!
//! #[tokio::test]
//! async fn my_store_conforms() {
//!     let store = MyEventStore::new(...);
//!     conformance::event_store_conformance(&store, "conformance-mystore").await;
//! }
//! ```
//!
//! Each check is also callable individually for debugging a failure.
//!
//! # Panics
//!
//! Every function panics (via `assert!`/`expect`) on a contract violation —
//! they are test infrastructure, meant to run inside `#[tokio::test]`.

#![allow(clippy::expect_used, clippy::panic)] // Test infrastructure

use crate::{EventMetadata, EventStore, EventStoreError, SerializedEvent, StreamId, Version};

fn event(event_type: &str, payload: &[u8]) -> SerializedEvent {
    SerializedEvent {
        event_type: event_type.to_string(),
        payload: payload.to_vec(),
        metadata: None,
        version: None,
    }
}

fn stream(prefix: &str, name: &str) -> StreamId {
    StreamId::new(format!("{prefix}-{name}"))
}

/// Append then load returns the same events in order, and append returns the
/// final stream version.
///
/// # Panics
///
/// Panics on contract violation.
pub async fn check_append_load_round_trip<S: EventStore>(store: &S, prefix: &str) {
    let id = stream(prefix, "round-trip");

    let final_version = store
        .append(&id, None, vec![event("E1", b"one"), event("E2", b"two")])
        .await
        .expect("append must succeed on a fresh stream");
    assert_eq!(
        final_version,
        Version::new(2),
        "append must return the final stream version"
    );

    let events = store.load(&id, None).await.expect("load must succeed");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "E1");
    assert_eq!(events[0].payload, b"one");
    assert_eq!(events[1].event_type, "E2");
    assert_eq!(events[1].payload, b"two");
}

/// Loaded events carry sequential stamped versions `1..=n`.
///
/// # Panics
///
/// Panics on contract violation.
pub async fn check_version_stamping<S: EventStore>(store: &S, prefix: &str) {
    let id = stream(prefix, "stamping");

    store
        .append(
            &id,
            None,
            vec![event("E1", b"a"), event("E2", b"b"), event("E3", b"c")],
        )
        .await
        .expect("append must succeed");

    let events = store.load(&id, None).await.expect("load must succeed");
    let versions: Vec<u64> = events
        .iter()
        .map(|e| {
            e.version
                .expect("load must stamp a version on every event")
                .as_u64()
        })
        .collect();
    assert_eq!(
        versions,
        vec![1, 2, 3],
        "streams must start at version 1 and be sequential"
    );
}

/// `load(Some(v))` is **inclusive**: returns all events with version `>= v`.
///
/// # Panics
///
/// Panics on contract violation.
pub async fn check_load_from_version_inclusive<S: EventStore>(store: &S, prefix: &str) {
    let id = stream(prefix, "inclusive");

    store
        .append(
            &id,
            None,
            vec![event("E1", b"a"), event("E2", b"b"), event("E3", b"c")],
        )
        .await
        .expect("append must succeed");

    let from_2 = store
        .load(&id, Some(Version::new(2)))
        .await
        .expect("load must succeed");
    assert_eq!(
        from_2.len(),
        2,
        "from_version is INCLUSIVE: v2 and v3 expected"
    );
    assert_eq!(from_2[0].event_type, "E2");
    assert_eq!(from_2[1].event_type, "E3");

    let from_1 = store
        .load(&id, Some(Version::new(1)))
        .await
        .expect("load must succeed");
    assert_eq!(from_1.len(), 3, "from v1 must return the whole stream");
}

/// Loading a stream that was never written returns an empty vec, not an error.
///
/// # Panics
///
/// Panics on contract violation.
pub async fn check_missing_stream_loads_empty<S: EventStore>(store: &S, prefix: &str) {
    let id = stream(prefix, "never-written");
    let events = store
        .load(&id, None)
        .await
        .expect("loading a missing stream must not error");
    assert!(events.is_empty(), "missing stream must load as empty");
}

/// Appending an empty batch is rejected.
///
/// (Only `is_err()` is asserted — implementations currently differ on the
/// error variant.)
///
/// # Panics
///
/// Panics on contract violation.
pub async fn check_empty_append_rejected<S: EventStore>(store: &S, prefix: &str) {
    let id = stream(prefix, "empty-append");
    let result = store.append(&id, None, Vec::new()).await;
    assert!(result.is_err(), "empty appends must be rejected");
}

/// Metadata round-trips: `Some(EventMetadata)` is preserved field-for-field,
/// and `None` stays `None`.
///
/// # Panics
///
/// Panics on contract violation.
pub async fn check_metadata_round_trip<S: EventStore>(store: &S, prefix: &str) {
    let id = stream(prefix, "metadata");

    let metadata = crate::MetadataContext::new()
        .with_correlation_id("conformance-corr")
        .with_causation_id("conformance-cause")
        .with_user_id("conformance-user")
        .to_event_metadata_at(
            chrono::DateTime::from_timestamp_micros(1_700_000_000_000_123)
                .expect("valid timestamp"),
        );

    let mut with_metadata = event("WithMeta", b"m");
    with_metadata.metadata = Some(metadata.clone());
    store
        .append(&id, None, vec![with_metadata, event("NoMeta", b"n")])
        .await
        .expect("append must succeed");

    let events = store.load(&id, None).await.expect("load must succeed");
    let loaded: &EventMetadata = events[0]
        .metadata
        .as_ref()
        .expect("Some metadata must round-trip as Some");
    assert_eq!(loaded.correlation_id, metadata.correlation_id);
    assert_eq!(loaded.causation_id, metadata.causation_id);
    assert_eq!(loaded.user_id, metadata.user_id);
    assert_eq!(
        loaded.timestamp, metadata.timestamp,
        "µs-precision timestamps must round-trip exactly"
    );
    assert!(
        events[1].metadata.is_none(),
        "None metadata must round-trip as None"
    );
}

/// A stale `expected_version` yields `VersionConflict` with a truthful
/// `actual`, and the losing events are not persisted.
///
/// # Panics
///
/// Panics on contract violation.
pub async fn check_stale_expected_version_conflicts<S: EventStore>(store: &S, prefix: &str) {
    let id = stream(prefix, "stale-version");

    store
        .append(&id, None, vec![event("E1", b"a"), event("E2", b"b")])
        .await
        .expect("append must succeed");

    // Expecting version 1 while the stream is at 2.
    let result = store
        .append(&id, Some(Version::new(1)), vec![event("E3", b"c")])
        .await;
    match result {
        Err(EventStoreError::VersionConflict { actual, .. }) => {
            assert_eq!(
                actual,
                Version::new(2),
                "conflict must report the actual current version"
            );
        },
        other => panic!("stale expected_version must yield VersionConflict, got {other:?}"),
    }

    let events = store.load(&id, None).await.expect("load must succeed");
    assert_eq!(events.len(), 2, "the conflicting append must not persist");
}

/// Two concurrent writers with the same `expected_version`: exactly one wins,
/// and the loser's error is a clean `VersionConflict` — never a
/// connection/aborted-transaction error.
///
/// This pins the bug class where an insert-collision handler touches the
/// aborted transaction and masks the conflict as an infrastructure error.
/// (The race takes the pre-check or the insert-collision path
/// nondeterministically; the contract must hold on both.)
///
/// # Panics
///
/// Panics on contract violation.
pub async fn check_concurrent_append_conflicts<S>(store: &S, prefix: &str)
where
    S: EventStore + Clone + Send + Sync + 'static,
{
    let id = stream(prefix, "concurrent");

    store
        .append(&id, None, vec![event("Seed", b"s")])
        .await
        .expect("seed append must succeed");

    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let spawn_writer = |tag: &'static str| {
        let store = store.clone();
        let id = id.clone();
        let barrier = std::sync::Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            store
                .append(&id, Some(Version::new(1)), vec![event(tag, b"w")])
                .await
        })
    };

    let (a, b) = tokio::join!(spawn_writer("WriterA"), spawn_writer("WriterB"));
    let results = [a.expect("writer A panicked"), b.expect("writer B panicked")];

    let winners = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(winners, 1, "exactly one concurrent writer must win");

    let loser = results
        .iter()
        .find_map(|r| r.as_ref().err())
        .expect("one writer must lose");
    assert!(
        matches!(loser, EventStoreError::VersionConflict { .. }),
        "the loser must see a clean VersionConflict (never a \
         connection/aborted-transaction error), got {loser:?}"
    );

    let events = store.load(&id, None).await.expect("load must succeed");
    assert_eq!(events.len(), 2, "seed + exactly one winner");
}

/// Run the full conformance suite against a store.
///
/// `prefix` must be unique per store/run so stream IDs cannot collide on a
/// shared or reused database.
///
/// # Panics
///
/// Panics on the first contract violation.
pub async fn event_store_conformance<S>(store: &S, prefix: &str)
where
    S: EventStore + Clone + Send + Sync + 'static,
{
    check_append_load_round_trip(store, prefix).await;
    check_version_stamping(store, prefix).await;
    check_load_from_version_inclusive(store, prefix).await;
    check_missing_stream_loads_empty(store, prefix).await;
    check_empty_append_rejected(store, prefix).await;
    check_metadata_round_trip(store, prefix).await;
    check_stale_expected_version_conflicts(store, prefix).await;
    check_concurrent_append_conflicts(store, prefix).await;
}

#[cfg(test)]
mod tests {
    use crate::testing::InMemoryEventStore;

    #[tokio::test]
    async fn in_memory_event_store_conforms() {
        let store = InMemoryEventStore::new();
        super::event_store_conformance(&store, "conformance-inmemory").await;
    }
}
