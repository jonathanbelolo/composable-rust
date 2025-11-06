//! Integration tests for `PostgresEventStore` using testcontainers.
//!
//! These tests use a real `PostgreSQL` database to validate all event store operations.
//!
//! # Requirements
//!
//! Docker must be running to execute these tests. The tests will automatically start a
//! `PostgreSQL` 16 container using testcontainers.

#![allow(clippy::expect_used)] // Test code uses expect for clear failure messages

use composable_rust_core::event::SerializedEvent;
use composable_rust_core::event_store::{EventStore, EventStoreError};
use composable_rust_core::stream::{StreamId, Version};
use composable_rust_postgres::PostgresEventStore;
use sqlx::PgPool;
use testcontainers::{GenericImage, ImageExt, runners::AsyncRunner};

/// Helper to start a Postgres container and return a configured event store.
///
/// # Panics
/// Panics if container setup fails (test environment issue).
async fn setup_postgres_event_store() -> PostgresEventStore {
    // Start Postgres container
    let postgres_image = GenericImage::new("postgres", "16")
        .with_exposed_port(5432.into())
        .with_env_var("POSTGRES_USER", "postgres")
        .with_env_var("POSTGRES_PASSWORD", "postgres")
        .with_env_var("POSTGRES_DB", "postgres");

    let container = postgres_image
        .start()
        .await
        .expect("Failed to start postgres container");

    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("Failed to get postgres port");

    // Wait for postgres to be ready
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Connect to database
    let database_url = format!("postgres://postgres:postgres@localhost:{port}/postgres");
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS events (
            stream_id TEXT NOT NULL,
            version BIGINT NOT NULL,
            event_type TEXT NOT NULL,
            event_data BYTEA NOT NULL,
            metadata JSONB,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            PRIMARY KEY (stream_id, version)
        );

        CREATE INDEX IF NOT EXISTS idx_events_created ON events(created_at);
        CREATE INDEX IF NOT EXISTS idx_events_type ON events(event_type);

        CREATE TABLE IF NOT EXISTS snapshots (
            stream_id TEXT PRIMARY KEY,
            version BIGINT NOT NULL,
            state_data BYTEA NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now()
        );
        ",
    )
    .execute(&pool)
    .await
    .expect("Failed to create tables");

    PostgresEventStore::from_pool(pool)
}

/// Helper to create test events.
fn create_test_event(event_type: &str, data: Vec<u8>) -> SerializedEvent {
    SerializedEvent::new(
        event_type.to_string(),
        data,
        Some(serde_json::json!({"test": true})),
    )
}

#[tokio::test]
async fn test_append_and_load_events() {
    let store = setup_postgres_event_store().await;

    // Test data
    let stream_id = StreamId::new("test-stream-1");
    let events = vec![
        create_test_event("Event1", b"data1".to_vec()),
        create_test_event("Event2", b"data2".to_vec()),
    ];

    // Append events
    let version = store
        .append_events(stream_id.clone(), None, events.clone())
        .await
        .expect("Failed to append events");

    assert_eq!(
        version,
        Version::new(1),
        "Should return version 1 for 2 events"
    );

    // Load events
    let loaded = store
        .load_events(stream_id, None)
        .await
        .expect("Failed to load events");

    assert_eq!(loaded.len(), 2, "Should load 2 events");
    assert_eq!(loaded[0].event_type, "Event1");
    assert_eq!(loaded[0].data, b"data1");
    assert_eq!(loaded[1].event_type, "Event2");
    assert_eq!(loaded[1].data, b"data2");
}

#[tokio::test]
async fn test_optimistic_concurrency_check() {
    let store = setup_postgres_event_store().await;
    let stream_id = StreamId::new("test-stream-2");

    // Append first event
    let version1 = store
        .append_events(
            stream_id.clone(),
            None,
            vec![create_test_event("Event1", b"data1".to_vec())],
        )
        .await
        .expect("Failed to append first event");

    assert_eq!(version1, Version::new(0));

    // Try to append with wrong expected version (should fail)
    let result = store
        .append_events(
            stream_id.clone(),
            Some(Version::new(10)), // Wrong expected version
            vec![create_test_event("Event2", b"data2".to_vec())],
        )
        .await;

    assert!(
        matches!(result, Err(EventStoreError::ConcurrencyConflict { .. })),
        "Should fail with concurrency conflict, got: {result:?}"
    );

    // Append with correct expected version (should succeed)
    let version2 = store
        .append_events(
            stream_id.clone(),
            Some(Version::new(0)),
            vec![create_test_event("Event2", b"data2".to_vec())],
        )
        .await
        .expect("Failed to append with correct version");

    assert_eq!(version2, Version::new(1));
}

#[tokio::test]
async fn test_concurrent_appends_race_condition() {
    let store = setup_postgres_event_store().await;
    let stream_id = StreamId::new("concurrent-stream");

    // Simulate concurrent appends (both think they're appending to version 0)
    let store2 = PostgresEventStore::from_pool(store.pool().clone());

    let stream_id1 = stream_id.clone();
    let stream_id2 = stream_id;

    // Spawn concurrent tasks
    let task1 = tokio::spawn(async move {
        store
            .append_events(
                stream_id1,
                Some(Version::new(0)),
                vec![create_test_event("Event1", b"data1".to_vec())],
            )
            .await
    });

    let task2 = tokio::spawn(async move {
        // Small delay to ensure overlap
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        store2
            .append_events(
                stream_id2,
                Some(Version::new(0)),
                vec![create_test_event("Event2", b"data2".to_vec())],
            )
            .await
    });

    let result1 = task1.await.expect("Task 1 panicked");
    let result2 = task2.await.expect("Task 2 panicked");

    // One should succeed, one should fail with concurrency conflict
    let success_count = [result1.is_ok(), result2.is_ok()]
        .iter()
        .filter(|x| **x)
        .count();

    assert_eq!(
        success_count, 1,
        "Exactly one concurrent append should succeed"
    );

    // Check that the one that failed has a concurrency conflict error
    let failure = if result1.is_err() { result1 } else { result2 };

    assert!(
        matches!(failure, Err(EventStoreError::ConcurrencyConflict { .. })),
        "Failed append should be due to concurrency conflict, got: {failure:?}"
    );
}

#[tokio::test]
async fn test_load_events_from_version() {
    let store = setup_postgres_event_store().await;
    let stream_id = StreamId::new("test-stream-3");

    // Append 5 events
    store
        .append_events(
            stream_id.clone(),
            None,
            vec![
                create_test_event("Event1", b"data1".to_vec()),
                create_test_event("Event2", b"data2".to_vec()),
                create_test_event("Event3", b"data3".to_vec()),
                create_test_event("Event4", b"data4".to_vec()),
                create_test_event("Event5", b"data5".to_vec()),
            ],
        )
        .await
        .expect("Failed to append events");

    // Load all events
    let all_events = store
        .load_events(stream_id.clone(), None)
        .await
        .expect("Failed to load all events");

    assert_eq!(all_events.len(), 5);

    // Load events from version 2
    let from_v2 = store
        .load_events(stream_id.clone(), Some(Version::new(2)))
        .await
        .expect("Failed to load events from version 2");

    assert_eq!(from_v2.len(), 3, "Should load events 2, 3, 4");
    assert_eq!(from_v2[0].event_type, "Event3");
    assert_eq!(from_v2[1].event_type, "Event4");
    assert_eq!(from_v2[2].event_type, "Event5");
}

#[tokio::test]
async fn test_save_and_load_snapshot() {
    let store = setup_postgres_event_store().await;
    let stream_id = StreamId::new("test-stream-4");

    // Save snapshot
    let state_data = b"snapshot state data".to_vec();
    store
        .save_snapshot(stream_id.clone(), Version::new(10), state_data.clone())
        .await
        .expect("Failed to save snapshot");

    // Load snapshot
    let loaded = store
        .load_snapshot(stream_id.clone())
        .await
        .expect("Failed to load snapshot");

    assert!(loaded.is_some(), "Snapshot should exist");
    let (version, data) = loaded.expect("Snapshot should be Some");
    assert_eq!(version, Version::new(10));
    assert_eq!(data, state_data);
}

#[tokio::test]
async fn test_snapshot_upsert() {
    let store = setup_postgres_event_store().await;
    let stream_id = StreamId::new("test-stream-5");

    // Save first snapshot
    store
        .save_snapshot(stream_id.clone(), Version::new(5), b"state v5".to_vec())
        .await
        .expect("Failed to save first snapshot");

    // Update snapshot (upsert)
    store
        .save_snapshot(stream_id.clone(), Version::new(10), b"state v10".to_vec())
        .await
        .expect("Failed to update snapshot");

    // Load snapshot - should get the latest
    let loaded = store
        .load_snapshot(stream_id)
        .await
        .expect("Failed to load snapshot")
        .expect("Snapshot should exist");

    assert_eq!(loaded.0, Version::new(10));
    assert_eq!(loaded.1, b"state v10");
}

#[tokio::test]
async fn test_load_snapshot_not_found() {
    let store = setup_postgres_event_store().await;
    let stream_id = StreamId::new("nonexistent-stream");

    // Try to load non-existent snapshot
    let loaded = store
        .load_snapshot(stream_id)
        .await
        .expect("Should not error on missing snapshot");

    assert!(loaded.is_none(), "Should return None for missing snapshot");
}

#[tokio::test]
async fn test_empty_event_list_error() {
    let store = setup_postgres_event_store().await;
    let stream_id = StreamId::new("test-stream-6");

    // Try to append empty event list
    let result = store.append_events(stream_id, None, vec![]).await;

    assert!(
        matches!(result, Err(EventStoreError::DatabaseError(_))),
        "Should fail with database error for empty events"
    );
}

#[tokio::test]
async fn test_multiple_streams_isolation() {
    let store = setup_postgres_event_store().await;

    let stream1 = StreamId::new("stream-1");
    let stream2 = StreamId::new("stream-2");

    // Append to stream 1
    store
        .append_events(
            stream1.clone(),
            None,
            vec![create_test_event("Event1", b"data1".to_vec())],
        )
        .await
        .expect("Failed to append to stream 1");

    // Append to stream 2
    store
        .append_events(
            stream2.clone(),
            None,
            vec![create_test_event("Event2", b"data2".to_vec())],
        )
        .await
        .expect("Failed to append to stream 2");

    // Load from stream 1
    let events1 = store
        .load_events(stream1, None)
        .await
        .expect("Failed to load stream 1");

    // Load from stream 2
    let events2 = store
        .load_events(stream2, None)
        .await
        .expect("Failed to load stream 2");

    assert_eq!(events1.len(), 1);
    assert_eq!(events2.len(), 1);
    assert_eq!(events1[0].event_type, "Event1");
    assert_eq!(events2[0].event_type, "Event2");
}
