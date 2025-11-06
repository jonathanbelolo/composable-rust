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
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;

/// Run database migrations using sqlx migrate
async fn run_migrations(pool: &sqlx::PgPool) {
    sqlx::migrate!("../migrations")
        .run(pool)
        .await
        .expect("Failed to run migrations");

    // Small delay to ensure migrations are fully applied
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}

/// Helper to start a Postgres container and return a configured event store.
///
/// Returns both the container (to keep it alive) and the event store.
///
/// # Panics
/// Panics if container setup fails (test environment issue).
async fn setup_postgres_event_store() -> (ContainerAsync<Postgres>, PostgresEventStore) {
    // Start Postgres container using the official module
    let container = Postgres::default()
        .start()
        .await
        .expect("Failed to start postgres container");

    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("Failed to get postgres port");

    // Use the connection string from the module
    let database_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    // Container started successfully

    // Wait for postgres to be ready with retry logic
    let mut retries = 0;
    let max_retries = 60;
    loop {
        if let Ok(pool) = sqlx::PgPool::connect(&database_url).await {
            // Verify with a simple query
            if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                // Run migrations
                run_migrations(&pool).await;

                // Return both container (to keep it alive) and event store
                return (container, PostgresEventStore::from_pool(pool));
            }
        }

        assert!(
            retries < max_retries,
            "Failed to connect after {max_retries} retries"
        );
        retries += 1;
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
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
    let (_container, store) = setup_postgres_event_store().await;

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
        Version::new(2),
        "Should return version 2 (last event) when appending 2 events starting from 0"
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
    let (_container, store) = setup_postgres_event_store().await;
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

    assert_eq!(
        version1,
        Version::new(1),
        "First event should be at version 1"
    );

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
    // After appending first event at version 1, expected version is 1
    let version2 = store
        .append_events(
            stream_id.clone(),
            Some(Version::new(1)),
            vec![create_test_event("Event2", b"data2".to_vec())],
        )
        .await
        .expect("Failed to append with correct version");

    assert_eq!(
        version2,
        Version::new(2),
        "Second event should be at version 2"
    );
}

#[tokio::test]
async fn test_concurrent_appends_race_condition() {
    let (_container, store) = setup_postgres_event_store().await;
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
    let (_container, store) = setup_postgres_event_store().await;
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

    assert_eq!(
        from_v2.len(),
        4,
        "Should load events at versions 2, 3, 4, 5"
    );
    assert_eq!(from_v2[0].event_type, "Event2");
    assert_eq!(from_v2[1].event_type, "Event3");
    assert_eq!(from_v2[2].event_type, "Event4");
    assert_eq!(from_v2[3].event_type, "Event5");
}

#[tokio::test]
async fn test_save_and_load_snapshot() {
    let (_container, store) = setup_postgres_event_store().await;
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
    let (_container, store) = setup_postgres_event_store().await;
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
    let (_container, store) = setup_postgres_event_store().await;
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
    let (_container, store) = setup_postgres_event_store().await;
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
    let (_container, store) = setup_postgres_event_store().await;

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

// ========== append_batch Tests ==========

#[tokio::test]
async fn test_append_batch_success_multiple_streams() {
    use composable_rust_core::event_store::BatchAppend;

    let (_container, store) = setup_postgres_event_store().await;

    let stream1 = StreamId::new("batch-stream-1");
    let stream2 = StreamId::new("batch-stream-2");
    let stream3 = StreamId::new("batch-stream-3");

    // Create batch with 3 streams
    let batch = vec![
        BatchAppend::new(
            stream1.clone(),
            Some(Version::new(0)),
            vec![
                create_test_event("Stream1Event1", b"data1".to_vec()),
                create_test_event("Stream1Event2", b"data2".to_vec()),
            ],
        ),
        BatchAppend::new(
            stream2.clone(),
            Some(Version::new(0)),
            vec![create_test_event("Stream2Event1", b"data3".to_vec())],
        ),
        BatchAppend::new(
            stream3.clone(),
            Some(Version::new(0)),
            vec![
                create_test_event("Stream3Event1", b"data4".to_vec()),
                create_test_event("Stream3Event2", b"data5".to_vec()),
                create_test_event("Stream3Event3", b"data6".to_vec()),
            ],
        ),
    ];

    // Execute batch
    let results = store
        .append_batch(batch)
        .await
        .expect("Batch should not fail at transaction level");

    // Verify all succeeded
    assert_eq!(results.len(), 3);
    assert!(results[0].is_ok(), "Stream 1 should succeed");
    assert!(results[1].is_ok(), "Stream 2 should succeed");
    assert!(results[2].is_ok(), "Stream 3 should succeed");

    // Test assertions - safe to use expect after verifying is_ok()
    #[allow(clippy::expect_used)]
    {
        assert_eq!(
            results[0].as_ref().expect("Stream 1 already verified as Ok"),
            &Version::new(2),
            "Stream 1 should be at version 2"
        );
        assert_eq!(
            results[1].as_ref().expect("Stream 2 already verified as Ok"),
            &Version::new(1),
            "Stream 2 should be at version 1"
        );
        assert_eq!(
            results[2].as_ref().expect("Stream 3 already verified as Ok"),
            &Version::new(3),
            "Stream 3 should be at version 3"
        );
    }

    // Verify events were persisted correctly
    let stream1_events = store
        .load_events(stream1, None)
        .await
        .expect("Should load stream 1");
    assert_eq!(stream1_events.len(), 2);

    let stream2_events = store
        .load_events(stream2, None)
        .await
        .expect("Should load stream 2");
    assert_eq!(stream2_events.len(), 1);

    let stream3_events = store
        .load_events(stream3, None)
        .await
        .expect("Should load stream 3");
    assert_eq!(stream3_events.len(), 3);
}

#[tokio::test]
async fn test_append_batch_partial_failure_concurrency_conflict() {
    use composable_rust_core::event_store::BatchAppend;

    let (_container, store) = setup_postgres_event_store().await;

    let stream1 = StreamId::new("batch-conflict-1");
    let stream2 = StreamId::new("batch-conflict-2");

    // Pre-populate stream1 with an event (so it's at version 1)
    store
        .append_events(
            stream1.clone(),
            Some(Version::new(0)),
            vec![create_test_event("PreExisting", b"pre".to_vec())],
        )
        .await
        .expect("Pre-populate should succeed");

    // Create batch where stream1 expects wrong version
    let batch = vec![
        BatchAppend::new(
            stream1.clone(),
            Some(Version::new(0)), // WRONG! Should be 1
            vec![create_test_event("ShouldFail", b"fail".to_vec())],
        ),
        BatchAppend::new(
            stream2.clone(),
            Some(Version::new(0)), // Correct for new stream
            vec![create_test_event("ShouldSucceed", b"success".to_vec())],
        ),
    ];

    // Execute batch
    let results = store
        .append_batch(batch)
        .await
        .expect("Batch should not fail at transaction level");

    // Verify results
    assert_eq!(results.len(), 2);
    assert!(
        results[0].is_err(),
        "Stream 1 should fail with concurrency conflict"
    );
    assert!(
        matches!(
            results[0],
            Err(EventStoreError::ConcurrencyConflict { .. })
        ),
        "Should be concurrency conflict"
    );
    assert!(results[1].is_ok(), "Stream 2 should succeed");

    // Verify stream1 was NOT modified
    let stream1_events = store
        .load_events(stream1, None)
        .await
        .expect("Should load stream 1");
    assert_eq!(
        stream1_events.len(),
        1,
        "Stream 1 should still have only 1 event"
    );
    assert_eq!(stream1_events[0].event_type, "PreExisting");

    // Verify stream2 WAS modified
    let stream2_events = store
        .load_events(stream2, None)
        .await
        .expect("Should load stream 2");
    assert_eq!(stream2_events.len(), 1);
    assert_eq!(stream2_events[0].event_type, "ShouldSucceed");
}

#[tokio::test]
async fn test_append_batch_empty_events_validation() {
    use composable_rust_core::event_store::BatchAppend;

    let (_container, store) = setup_postgres_event_store().await;

    let stream1 = StreamId::new("batch-empty-1");
    let stream2 = StreamId::new("batch-empty-2");

    // Batch with one empty events list and one valid
    let batch = vec![
        BatchAppend::new(stream1.clone(), Some(Version::new(0)), vec![]), // Empty!
        BatchAppend::new(
            stream2.clone(),
            Some(Version::new(0)),
            vec![create_test_event("Valid", b"data".to_vec())],
        ),
    ];

    let results = store
        .append_batch(batch)
        .await
        .expect("Batch should not fail at transaction level");

    assert_eq!(results.len(), 2);
    assert!(results[0].is_err(), "Empty events should fail");
    assert!(
        matches!(results[0], Err(EventStoreError::DatabaseError(_))),
        "Should be database error"
    );
    assert!(results[1].is_ok(), "Valid operation should succeed");

    // Verify stream2 was created
    let stream2_events = store
        .load_events(stream2, None)
        .await
        .expect("Should load stream 2");
    assert_eq!(stream2_events.len(), 1);
}

#[tokio::test]
async fn test_append_batch_empty_batch() {
    let (_container, store) = setup_postgres_event_store().await;

    // Empty batch
    let results = store
        .append_batch(vec![])
        .await
        .expect("Empty batch should succeed");

    assert_eq!(results.len(), 0, "Empty batch should return empty results");
}

#[tokio::test]
async fn test_append_batch_atomicity_all_or_nothing_at_stream_level() {
    use composable_rust_core::event_store::BatchAppend;

    let (_container, store) = setup_postgres_event_store().await;

    let stream1 = StreamId::new("batch-atomic-1");
    let stream2 = StreamId::new("batch-atomic-2");

    // Create batch where second operation will fail
    let batch = vec![
        BatchAppend::new(
            stream1.clone(),
            Some(Version::new(0)),
            vec![create_test_event("Event1", b"data1".to_vec())],
        ),
        BatchAppend::new(
            stream2.clone(),
            Some(Version::new(5)), // Wrong expected version for new stream
            vec![create_test_event("Event2", b"data2".to_vec())],
        ),
    ];

    let results = store
        .append_batch(batch)
        .await
        .expect("Transaction should commit despite per-operation failures");

    // Verify: first operation succeeds, second fails
    assert!(results[0].is_ok(), "First operation should succeed");
    assert!(results[1].is_err(), "Second operation should fail");

    // CRITICAL: Verify first stream WAS created (not rolled back)
    // This is the current behavior: per-operation results, not all-or-nothing
    let stream1_events = store
        .load_events(stream1, None)
        .await
        .expect("Should load stream 1");
    assert_eq!(
        stream1_events.len(),
        1,
        "First stream should have been created despite second operation failing"
    );

    // Verify second stream was NOT created
    let stream2_events = store
        .load_events(stream2, None)
        .await
        .expect("Should load stream 2");
    assert_eq!(stream2_events.len(), 0, "Second stream should be empty");
}

#[tokio::test]
async fn test_append_batch_performance_vs_sequential() {
    use composable_rust_core::event_store::BatchAppend;
    use std::time::Instant;

    let (_container, store) = setup_postgres_event_store().await;

    // Prepare 10 streams with 5 events each
    let num_streams = 10;
    let events_per_stream = 5;

    let mut batch_operations = Vec::new();
    let mut individual_operations = Vec::new();

    for i in 0..num_streams {
        let stream_id = StreamId::new(format!("perf-stream-{i}"));
        let events: Vec<_> = (0..events_per_stream)
            .map(|j| create_test_event(&format!("Event{j}"), vec![i as u8, j as u8]))
            .collect();

        batch_operations.push(BatchAppend::new(
            stream_id.clone(),
            Some(Version::new(0)),
            events.clone(),
        ));
        individual_operations.push((stream_id, events));
    }

    // Measure batch append
    let batch_start = Instant::now();
    let batch_results = store
        .append_batch(batch_operations)
        .await
        .expect("Batch should succeed");
    let batch_duration = batch_start.elapsed();

    assert_eq!(batch_results.len(), num_streams);
    assert!(batch_results.iter().all(|r| r.is_ok()));

    println!("Batch append ({} streams): {:?}", num_streams, batch_duration);

    // Measure sequential appends (different stream names)
    let sequential_start = Instant::now();
    for (i, (_stream_id, events)) in individual_operations.into_iter().enumerate() {
        let seq_stream = StreamId::new(format!("seq-stream-{i}"));
        store
            .append_events(seq_stream, Some(Version::new(0)), events)
            .await
            .expect("Sequential append should succeed");
    }
    let sequential_duration = sequential_start.elapsed();

    println!(
        "Sequential append ({} streams): {:?}",
        num_streams, sequential_duration
    );

    // Batch should be faster (even with current implementation)
    // Note: With multi-row INSERT, this should be significantly faster
    println!(
        "Speedup: {:.2}x",
        sequential_duration.as_secs_f64() / batch_duration.as_secs_f64()
    );
}
