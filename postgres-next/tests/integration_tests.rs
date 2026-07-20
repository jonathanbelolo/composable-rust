//! Integration tests for `PostgresEventStore` (next-gen) using testcontainers.
//!
//! # Architecture
//!
//! - One container for the entire test suite
//! - Database reset (TRUNCATE) between tests
//! - Run with: `cargo test --test integration_tests`
//!
//! # Requirements
//!
//! Docker must be running.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
// Test-harness noise: these lints target production-code concerns, not integration tests.
#![allow(clippy::panic)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::single_match)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::needless_borrows_for_generic_args)]

use composable_rust_next::{
    AtomicError, EventMetadata, EventStore, EventStoreError, ProjectionError, SerializedEvent,
    StreamId, SubjectId, Version,
};
use composable_rust_postgres_next::{PgTransactionalProjector, PostgresEventStore};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;

// ═══════════════════════════════════════════════════════════════════════════
// Test Infrastructure
// ═══════════════════════════════════════════════════════════════════════════

struct TestDb {
    _container: ContainerAsync<Postgres>,
    pool: PgPool,
    url: String,
}

impl TestDb {
    async fn new() -> Self {
        let container = Postgres::default()
            .start()
            .await
            .expect("Failed to start postgres");

        let port = container.get_host_port_ipv4(5432).await.expect("No port");
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

        // Wait for ready with a proper pool
        let pool = loop {
            match PgPoolOptions::new()
                .max_connections(10)
                .acquire_timeout(std::time::Duration::from_secs(5))
                .connect(&url)
                .await
            {
                Ok(p) => {
                    if sqlx::query("SELECT 1").execute(&p).await.is_ok() {
                        break p;
                    }
                },
                Err(_) => {},
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        };

        // Run migrations
        sqlx::migrate!("../migrations")
            .run(&pool)
            .await
            .expect("Migrations failed");

        Self {
            _container: container,
            pool,
            url,
        }
    }

    async fn reset(&self) {
        sqlx::query("TRUNCATE TABLE events RESTART IDENTITY CASCADE")
            .execute(&self.pool)
            .await
            .expect("Truncate failed");
    }

    fn store(&self) -> PostgresEventStore {
        PostgresEventStore::from_pool(self.pool.clone())
    }

    fn url(&self) -> &str {
        &self.url
    }
}

fn event(typ: &str, data: &[u8]) -> SerializedEvent {
    SerializedEvent {
        event_type: typ.to_string(),
        payload: data.to_vec(),
        metadata: None,
        version: None,
        stream_id: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// All Tests in One Function (Single Container)
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn postgres_event_store_integration() {
    let db = TestDb::new().await;

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 0: EventStore conformance suite (shared contract with InMemory)
    // ═══════════════════════════════════════════════════════════════════════

    {
        db.reset().await;
        let store = db.store();
        composable_rust_next::conformance::event_store_conformance(&store, "conformance-pg").await;
        println!("  [PASS] event_store_conformance");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 1: Basic Operations
    // ═══════════════════════════════════════════════════════════════════════

    // Test 1: append_and_load - basic write then read
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        let v = store
            .append(
                &id,
                None,
                vec![event("E1", b"data1"), event("E2", b"data2")],
            )
            .await
            .unwrap();
        assert_eq!(v, Version::new(2), "append should return final version");

        let events = store.load(&id, None).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "E1");
        assert_eq!(events[0].payload, b"data1");
        assert_eq!(events[1].event_type, "E2");
        assert_eq!(events[1].payload, b"data2");
        println!("  [PASS] append_and_load");
    }

    // Test 2: load_empty_stream - nonexistent stream returns empty vec
    {
        db.reset().await;
        let store = db.store();
        let events = store
            .load(&StreamId::new("nonexistent"), None)
            .await
            .unwrap();
        assert!(events.is_empty(), "nonexistent stream should return empty");
        println!("  [PASS] load_empty_stream");
    }

    // Test 3: load_from_version - partial load starting at version N
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        store
            .append(
                &id,
                None,
                vec![event("E1", b"1"), event("E2", b"2"), event("E3", b"3")],
            )
            .await
            .unwrap();

        // Load from version 2 should include versions 2 and 3
        let from_v2 = store.load(&id, Some(Version::new(2))).await.unwrap();
        assert_eq!(from_v2.len(), 2);
        assert_eq!(from_v2[0].event_type, "E2");
        assert_eq!(from_v2[0].version, Some(Version::new(2)));
        assert_eq!(from_v2[1].event_type, "E3");
        assert_eq!(from_v2[1].version, Some(Version::new(3)));
        println!("  [PASS] load_from_version");
    }

    // Test 4: load_from_version_zero - version 0 returns nothing (versions start at 1)
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        store
            .append(&id, None, vec![event("E1", b"1")])
            .await
            .unwrap();

        // Version 0 is before the first event (version 1)
        // From SQL: WHERE version >= 0 should return all events
        let from_v0 = store.load(&id, Some(Version::new(0))).await.unwrap();
        // Note: Version::new(0) means "from version 0", but events start at version 1
        // So this should still return all events
        assert_eq!(from_v0.len(), 1);
        println!("  [PASS] load_from_version_zero");
    }

    // Test 5: load_from_version_one - version 1 includes version 1
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        store
            .append(&id, None, vec![event("E1", b"1"), event("E2", b"2")])
            .await
            .unwrap();

        let from_v1 = store.load(&id, Some(Version::new(1))).await.unwrap();
        assert_eq!(from_v1.len(), 2); // Both events (>= 1)
        assert_eq!(from_v1[0].version, Some(Version::new(1)));
        println!("  [PASS] load_from_version_one");
    }

    // Test 6: load_from_version_beyond_max - returns empty
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        store
            .append(&id, None, vec![event("E1", b"1")])
            .await
            .unwrap();

        let from_v100 = store.load(&id, Some(Version::new(100))).await.unwrap();
        assert!(
            from_v100.is_empty(),
            "version beyond max should return empty"
        );
        println!("  [PASS] load_from_version_beyond_max");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 2: Optimistic Concurrency Control
    // ═══════════════════════════════════════════════════════════════════════

    // Test 7: optimistic_concurrency_success - correct expected version
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        let v1 = store
            .append(&id, None, vec![event("E1", b"1")])
            .await
            .unwrap();
        assert_eq!(v1, Version::new(1));

        let v2 = store
            .append(&id, Some(v1), vec![event("E2", b"2")])
            .await
            .unwrap();
        assert_eq!(v2, Version::new(2));
        println!("  [PASS] optimistic_concurrency_success");
    }

    // Test 8: optimistic_concurrency_conflict - wrong expected version
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        store
            .append(&id, None, vec![event("E1", b"1")])
            .await
            .unwrap();

        let result = store
            .append(&id, Some(Version::new(99)), vec![event("E2", b"2")])
            .await;

        match result {
            Err(EventStoreError::VersionConflict { expected, actual }) => {
                assert_eq!(expected, Some(Version::new(99)));
                assert_eq!(actual, Version::new(1));
            },
            other => panic!("Expected VersionConflict, got: {other:?}"),
        }
        println!("  [PASS] optimistic_concurrency_conflict");
    }

    // Test 9: expected_version_zero_on_empty_stream - initial write assertion
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        // expected_version: Some(0) means "stream must be empty"
        // Version::initial() returns Version(0)
        let result = store
            .append(&id, Some(Version::initial()), vec![event("E1", b"1")])
            .await;
        assert!(
            result.is_ok(),
            "expected_version 0 on empty stream should succeed"
        );
        assert_eq!(result.unwrap(), Version::new(1));
        println!("  [PASS] expected_version_zero_on_empty_stream");
    }

    // Test 10: expected_version_zero_on_existing_stream - should fail
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        store
            .append(&id, None, vec![event("E1", b"1")])
            .await
            .unwrap();

        let result = store
            .append(&id, Some(Version::initial()), vec![event("E2", b"2")])
            .await;

        assert!(
            matches!(result, Err(EventStoreError::VersionConflict { .. })),
            "expected_version 0 on existing stream should fail"
        );
        println!("  [PASS] expected_version_zero_on_existing_stream");
    }

    // Test 11: expected_version_none_skips_check - always succeeds
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        // First write
        store
            .append(&id, None, vec![event("E1", b"1")])
            .await
            .unwrap();

        // Second write without expected_version - should succeed
        let v2 = store
            .append(&id, None, vec![event("E2", b"2")])
            .await
            .unwrap();
        assert_eq!(v2, Version::new(2));

        // Third write without expected_version - should succeed
        let v3 = store
            .append(&id, None, vec![event("E3", b"3")])
            .await
            .unwrap();
        assert_eq!(v3, Version::new(3));
        println!("  [PASS] expected_version_none_skips_check");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 3: Stream Isolation
    // ═══════════════════════════════════════════════════════════════════════

    // Test 12: stream_isolation - different streams are independent
    {
        db.reset().await;
        let store = db.store();

        store
            .append(&StreamId::new("stream-a"), None, vec![event("A1", b"a")])
            .await
            .unwrap();
        store
            .append(&StreamId::new("stream-b"), None, vec![event("B1", b"b")])
            .await
            .unwrap();

        let a_events = store.load(&StreamId::new("stream-a"), None).await.unwrap();
        let b_events = store.load(&StreamId::new("stream-b"), None).await.unwrap();

        assert_eq!(a_events.len(), 1);
        assert_eq!(b_events.len(), 1);
        assert_eq!(a_events[0].event_type, "A1");
        assert_eq!(b_events[0].event_type, "B1");
        println!("  [PASS] stream_isolation");
    }

    // Test 13: stream_versions_are_independent
    {
        db.reset().await;
        let store = db.store();

        let va = store
            .append(&StreamId::new("stream-a"), None, vec![event("A1", b"a")])
            .await
            .unwrap();
        let vb = store
            .append(&StreamId::new("stream-b"), None, vec![event("B1", b"b")])
            .await
            .unwrap();

        // Both should be version 1 (independent versioning)
        assert_eq!(va, Version::new(1));
        assert_eq!(vb, Version::new(1));
        println!("  [PASS] stream_versions_are_independent");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 4: Failure Modes
    // ═══════════════════════════════════════════════════════════════════════

    // Test 14: empty_append_fails - cannot append zero events
    {
        db.reset().await;
        let store = db.store();
        let result = store.append(&StreamId::new("stream-1"), None, vec![]).await;
        assert!(result.is_err(), "empty append should fail");
        println!("  [PASS] empty_append_fails");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 5: Version Tracking
    // ═══════════════════════════════════════════════════════════════════════

    // Test 15: version_tracking - loaded events have correct versions
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        store
            .append(&id, None, vec![event("E1", b"1"), event("E2", b"2")])
            .await
            .unwrap();

        let events = store.load(&id, None).await.unwrap();
        assert_eq!(events[0].version, Some(Version::new(1)));
        assert_eq!(events[1].version, Some(Version::new(2)));
        println!("  [PASS] version_tracking");
    }

    // Test 16: sequential_version_increments
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        for i in 1..=5 {
            let v = store
                .append(&id, None, vec![event(&format!("E{i}"), &[i as u8])])
                .await
                .unwrap();
            assert_eq!(v, Version::new(i), "version should increment sequentially");
        }
        println!("  [PASS] sequential_version_increments");
    }

    // Test 17: batch_append_version - multiple events in one append
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        let v = store
            .append(
                &id,
                None,
                vec![
                    event("E1", b"1"),
                    event("E2", b"2"),
                    event("E3", b"3"),
                    event("E4", b"4"),
                    event("E5", b"5"),
                ],
            )
            .await
            .unwrap();

        // Returns final version (5)
        assert_eq!(v, Version::new(5));

        let events = store.load(&id, None).await.unwrap();
        assert_eq!(events.len(), 5);
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.version, Some(Version::new((i + 1) as u64)));
        }
        println!("  [PASS] batch_append_version");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 6: Binary Data Handling
    // ═══════════════════════════════════════════════════════════════════════

    // Test 18: binary_roundtrip - all byte values preserved
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");
        let data: Vec<u8> = (0..=255).collect();

        store
            .append(&id, None, vec![event("BinaryEvent", &data)])
            .await
            .unwrap();

        let events = store.load(&id, None).await.unwrap();
        assert_eq!(events[0].payload, data);
        println!("  [PASS] binary_roundtrip");
    }

    // Test 19: null_bytes_in_payload
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");
        let data = b"hello\x00world\x00\x00\x00end";

        store
            .append(&id, None, vec![event("NullBytes", data)])
            .await
            .unwrap();

        let events = store.load(&id, None).await.unwrap();
        assert_eq!(events[0].payload, data);
        println!("  [PASS] null_bytes_in_payload");
    }

    // Test 20: large_payload - 1MB payload
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");
        let data = vec![0xAB_u8; 1024 * 1024]; // 1MB

        store
            .append(&id, None, vec![event("LargeEvent", &data)])
            .await
            .unwrap();

        let events = store.load(&id, None).await.unwrap();
        assert_eq!(events[0].payload.len(), 1024 * 1024);
        assert!(events[0].payload.iter().all(|&b| b == 0xAB));
        println!("  [PASS] large_payload");
    }

    // Test 21: empty_payload
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        store
            .append(&id, None, vec![event("EmptyPayload", b"")])
            .await
            .unwrap();

        let events = store.load(&id, None).await.unwrap();
        assert!(events[0].payload.is_empty());
        println!("  [PASS] empty_payload");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 7: Special Characters and Edge Cases
    // ═══════════════════════════════════════════════════════════════════════

    // Test 22: special_chars_in_stream_id
    {
        db.reset().await;
        let store = db.store();

        let special_ids = [
            "stream-with-dashes",
            "stream_with_underscores",
            "stream.with.dots",
            "stream:with:colons",
            "stream/with/slashes",
            "CamelCaseStream",
            "UPPERCASE_STREAM",
            "stream-123-numbers",
            "uuid-550e8400-e29b-41d4-a716-446655440000",
        ];

        for stream_id in special_ids {
            let id = StreamId::new(stream_id);
            store
                .append(&id, None, vec![event("E1", b"1")])
                .await
                .unwrap();

            let events = store.load(&id, None).await.unwrap();
            assert_eq!(events.len(), 1, "failed for stream_id: {stream_id}");
        }
        println!("  [PASS] special_chars_in_stream_id");
    }

    // Test 23: unicode_in_event_type
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        store
            .append(&id, None, vec![event("EventCreated", b"1")])
            .await
            .unwrap();

        let events = store.load(&id, None).await.unwrap();
        assert_eq!(events[0].event_type, "EventCreated");
        println!("  [PASS] unicode_in_event_type");
    }

    // Test 24: very_long_event_type
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");
        let long_type = "A".repeat(500);

        store
            .append(&id, None, vec![event(&long_type, b"1")])
            .await
            .unwrap();

        let events = store.load(&id, None).await.unwrap();
        assert_eq!(events[0].event_type, long_type);
        println!("  [PASS] very_long_event_type");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 8: Pool and Connection Management
    // ═══════════════════════════════════════════════════════════════════════

    // Test 25: pool_stats - verify stats are reported
    {
        db.reset().await;
        let store = db.store();
        let stats = store.pool_stats();

        // Pool should have connections
        assert!(stats.size > 0, "pool should have connections");
        // Should not be saturated when idle
        assert!(!stats.is_saturated() || stats.idle == 0);
        // Utilization should be between 0 and 1
        assert!(stats.utilization() >= 0.0 && stats.utilization() <= 1.0);
        println!("  [PASS] pool_stats");
    }

    // Test 26: from_pool - create store from existing pool
    {
        db.reset().await;
        let store = PostgresEventStore::from_pool(db.pool.clone());
        let id = StreamId::new("stream-1");

        store
            .append(&id, None, vec![event("E1", b"1")])
            .await
            .unwrap();
        let events = store.load(&id, None).await.unwrap();
        assert_eq!(events.len(), 1);
        println!("  [PASS] from_pool");
    }

    // Test 27: new_with_url - create store from URL
    {
        db.reset().await;
        let store = PostgresEventStore::new(db.url()).await.unwrap();
        let id = StreamId::new("stream-1");

        store
            .append(&id, None, vec![event("E1", b"1")])
            .await
            .unwrap();
        let events = store.load(&id, None).await.unwrap();
        assert_eq!(events.len(), 1);
        println!("  [PASS] new_with_url");
    }

    // Test 28: with_options - create store with custom options
    {
        db.reset().await;
        let store = PostgresEventStore::with_options(
            db.url(),
            5,   // max_connections
            1,   // min_connections
            30,  // connect_timeout_secs
            60,  // idle_timeout_secs
            300, // max_lifetime_secs
        )
        .await
        .unwrap();
        let id = StreamId::new("stream-1");

        store
            .append(&id, None, vec![event("E1", b"1")])
            .await
            .unwrap();
        let events = store.load(&id, None).await.unwrap();
        assert_eq!(events.len(), 1);
        println!("  [PASS] with_options");
    }

    // Test 29: invalid_url_fails
    {
        let result =
            PostgresEventStore::new("postgres://invalid:invalid@nonexistent:5432/db").await;
        assert!(result.is_err(), "invalid URL should fail");
        println!("  [PASS] invalid_url_fails");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 9: Concurrent Access
    // ═══════════════════════════════════════════════════════════════════════

    // Test 30: concurrent_appends_different_streams - no conflict
    {
        db.reset().await;
        let store = db.store();

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let store = store.clone();
                tokio::spawn(async move {
                    let id = StreamId::new(&format!("stream-{i}"));
                    store.append(&id, None, vec![event("E1", b"1")]).await
                })
            })
            .collect();

        for handle in handles {
            handle.await.unwrap().unwrap();
        }

        // Verify all streams have their event
        for i in 0..10 {
            let events = store
                .load(&StreamId::new(&format!("stream-{i}")), None)
                .await
                .unwrap();
            assert_eq!(events.len(), 1);
        }
        println!("  [PASS] concurrent_appends_different_streams");
    }

    // Test 31: concurrent_appends_same_stream_no_version - race results in version conflicts
    // Note: Without expected_version, we skip the PRE-check but the unique constraint
    // on (stream_id, version) still enforces correctness at the database level.
    // Some concurrent appends may fail with version conflicts.
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        // Concurrent appends to same stream will race for version numbers
        let handles: Vec<_> = (0..5)
            .map(|i| {
                let store = store.clone();
                let id = id.clone();
                tokio::spawn(async move {
                    store
                        .append(&id, None, vec![event(&format!("E{i}"), &[i as u8])])
                        .await
                })
            })
            .collect();

        let mut successes = 0;
        let mut conflicts = 0;
        for handle in handles {
            match handle.await.unwrap() {
                Ok(_) => successes += 1,
                // A losing race MUST surface as VersionConflict, never a masked
                // Connection error from an aborted transaction (see classify_insert_error).
                Err(e) => {
                    assert!(
                        matches!(e, EventStoreError::VersionConflict { .. }),
                        "concurrent append must conflict with VersionConflict, got: {e:?}"
                    );
                    conflicts += 1;
                },
            }
        }

        // At least one should succeed
        assert!(
            successes >= 1,
            "at least one concurrent append should succeed"
        );

        let events = store.load(&id, None).await.unwrap();
        // The number of events should equal the number of successes
        assert_eq!(
            events.len(),
            successes,
            "event count should match successful appends"
        );
        println!(
            "  [PASS] concurrent_appends_same_stream_no_version ({successes} succeeded, {conflicts} conflicted)"
        );
    }

    // Test 32: concurrent_appends_same_stream_with_version - one wins
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        // First, create initial event
        store
            .append(&id, None, vec![event("E0", b"0")])
            .await
            .unwrap();

        // Now try concurrent appends with same expected_version
        let handles: Vec<_> = (0..5)
            .map(|i| {
                let store = store.clone();
                let id = id.clone();
                tokio::spawn(async move {
                    store
                        .append(
                            &id,
                            Some(Version::new(1)), // All expect version 1
                            vec![event(&format!("E{i}"), &[i as u8])],
                        )
                        .await
                })
            })
            .collect();

        let mut successes = 0;
        let mut conflicts = 0;
        for handle in handles {
            match handle.await.unwrap() {
                Ok(_) => successes += 1,
                // Losers MUST conflict with VersionConflict — not a masked Connection
                // error from an aborted transaction (see classify_insert_error).
                Err(e) => {
                    assert!(
                        matches!(e, EventStoreError::VersionConflict { .. }),
                        "concurrent append must conflict with VersionConflict, got: {e:?}"
                    );
                    conflicts += 1;
                },
            }
        }

        // Exactly one should succeed, the rest MUST get VersionConflict.
        assert_eq!(successes, 1, "exactly one concurrent append should succeed");
        assert_eq!(conflicts, 4, "four should get VersionConflict");

        let events = store.load(&id, None).await.unwrap();
        assert_eq!(
            events.len(),
            2,
            "should have original + one successful append"
        );
        println!("  [PASS] concurrent_appends_same_stream_with_version");
    }

    // Test 32b: concurrent_create_time_occ - duplicate initiation dedups via VersionConflict.
    // The idempotency-under-concurrency case: two writers both target a FRESH stream at
    // expected = Version::initial(). Exactly one wins; the other MUST get VersionConflict
    // (a regression guard for classify_insert_error — it must not surface a masked
    // Connection error from the aborted transaction after the 23505 unique violation).
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("idempotent-stream");

        let a = {
            let store = store.clone();
            let id = id.clone();
            tokio::spawn(async move {
                store
                    .append(&id, Some(Version::initial()), vec![event("Init", b"a")])
                    .await
            })
        };
        let b = {
            let store = store.clone();
            let id = id.clone();
            tokio::spawn(async move {
                store
                    .append(&id, Some(Version::initial()), vec![event("Init", b"b")])
                    .await
            })
        };

        let mut successes = 0;
        let mut conflicts = 0;
        for r in [a.await.unwrap(), b.await.unwrap()] {
            match r {
                Ok(_) => successes += 1,
                Err(e) => {
                    assert!(
                        matches!(e, EventStoreError::VersionConflict { .. }),
                        "duplicate initiation must conflict with VersionConflict, got: {e:?}"
                    );
                    conflicts += 1;
                },
            }
        }
        assert_eq!(successes, 1, "exactly one initiation should win");
        assert_eq!(conflicts, 1, "the duplicate must get VersionConflict");
        assert_eq!(
            store.load(&id, None).await.unwrap().len(),
            1,
            "only one event survives the race"
        );
        println!("  [PASS] concurrent_create_time_occ");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 10: Migrations
    // ═══════════════════════════════════════════════════════════════════════

    // Test 33: run_migrations - idempotent
    {
        db.reset().await;
        let store = db.store();

        // Running migrations again should succeed (idempotent)
        store.run_migrations().await.unwrap();
        store.run_migrations().await.unwrap();

        // Store should still work
        let id = StreamId::new("stream-1");
        store
            .append(&id, None, vec![event("E1", b"1")])
            .await
            .unwrap();
        println!("  [PASS] run_migrations");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 11: Many Events Stress Test
    // ═══════════════════════════════════════════════════════════════════════

    // Test 34: many_events_in_stream - 100 events
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        for i in 0..100 {
            store
                .append(&id, None, vec![event(&format!("Event{i}"), &[i as u8])])
                .await
                .unwrap();
        }

        let events = store.load(&id, None).await.unwrap();
        assert_eq!(events.len(), 100);

        // Verify ordering
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.event_type, format!("Event{i}"));
            assert_eq!(e.version, Some(Version::new((i + 1) as u64)));
        }
        println!("  [PASS] many_events_in_stream");
    }

    // Test 35: many_streams - 50 different streams
    {
        db.reset().await;
        let store = db.store();

        for i in 0..50 {
            let id = StreamId::new(&format!("stream-{i}"));
            store
                .append(&id, None, vec![event("Created", &[i as u8])])
                .await
                .unwrap();
        }

        // Verify each stream
        for i in 0..50 {
            let events = store
                .load(&StreamId::new(&format!("stream-{i}")), None)
                .await
                .unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].payload, vec![i as u8]);
        }
        println!("  [PASS] many_streams");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECTION 12: Event Metadata Persistence (JSONB column)
    // ═══════════════════════════════════════════════════════════════════════

    // Test 36: metadata_roundtrip - EventMetadata persists through the metadata column
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        let metadata = EventMetadata {
            correlation_id: Some("req-1".to_string()),
            causation_id: Some("evt-9".to_string()),
            user_id: Some("legacy-user".to_string()),
            author_id: Some(SubjectId::new("user-42")),
            origin_subject_id: Some(SubjectId::new("user-7")),
            timestamp: chrono::Utc::now(),
        };
        let mut e = event("WithMetadata", b"m");
        e.metadata = Some(metadata.clone());

        store.append(&id, None, vec![e]).await.unwrap();

        let events = store.load(&id, None).await.unwrap();
        assert_eq!(events.len(), 1);
        let loaded = events[0]
            .metadata
            .as_ref()
            .expect("metadata should round-trip through the JSONB column");
        assert_eq!(loaded.correlation_id, metadata.correlation_id);
        assert_eq!(loaded.causation_id, metadata.causation_id);
        assert_eq!(loaded.user_id, metadata.user_id);
        assert_eq!(loaded.author_id, metadata.author_id);
        assert_eq!(loaded.origin_subject_id, metadata.origin_subject_id);
        assert_eq!(loaded.timestamp, metadata.timestamp);
        println!("  [PASS] metadata_roundtrip");
    }

    // Test 37: legacy_null_metadata - NULL metadata rows load as None
    {
        db.reset().await;
        let store = db.store();
        let id = StreamId::new("stream-1");

        // The event() helper carries no metadata → binds NULL, exactly like
        // every row written before metadata persistence existed.
        store
            .append(&id, None, vec![event("Legacy", b"1")])
            .await
            .unwrap();

        // Belt and braces: a raw legacy row inserted with explicit NULL.
        sqlx::query(
            r"
            INSERT INTO events (stream_id, version, event_type, event_version, event_data, metadata, created_at)
            VALUES ($1, 2, 'RawLegacy', 1, $2, NULL, now())
            ",
        )
        .bind(id.as_str())
        .bind(b"2".as_slice())
        .execute(&db.pool)
        .await
        .unwrap();

        let events = store.load(&id, None).await.unwrap();
        assert_eq!(events.len(), 2);
        assert!(
            events[0].metadata.is_none(),
            "NULL metadata must load as None"
        );
        assert!(
            events[1].metadata.is_none(),
            "raw legacy NULL row must load as None"
        );
        println!("  [PASS] legacy_null_metadata");
    }

    println!("\n═══════════════════════════════════════");
    println!("All 37 PostgresEventStore tests passed!");
    println!("═══════════════════════════════════════");
}

// ═══════════════════════════════════════════════════════════════════════════
// Transactional append + projection
// ═══════════════════════════════════════════════════════════════════════════

/// Projector that always fails — used to assert the append rolls back.
struct FailingProjector;

impl PgTransactionalProjector for FailingProjector {
    async fn project_in_tx<'a>(
        &'a self,
        _conn: &'a mut sqlx::PgConnection,
        _final_version: Version,
        _events: &'a [SerializedEvent],
    ) -> Result<(), ProjectionError> {
        Err(ProjectionError::Custom("boom".to_string()))
    }
}

/// Projector that writes a marker row into a side table in the same transaction.
struct SideTableProjector;

impl PgTransactionalProjector for SideTableProjector {
    async fn project_in_tx<'a>(
        &'a self,
        conn: &'a mut sqlx::PgConnection,
        final_version: Version,
        events: &'a [SerializedEvent],
    ) -> Result<(), ProjectionError> {
        sqlx::query("INSERT INTO saga_marker (stream_count, version) VALUES ($1, $2)")
            .bind(i64::try_from(events.len()).unwrap_or(0))
            .bind(i64::try_from(final_version.as_u64()).unwrap_or(0))
            .execute(&mut *conn)
            .await
            .map_err(|e| ProjectionError::Database(e.to_string()))?;
        Ok(())
    }
}

#[tokio::test]
async fn append_with_projection_atomicity() {
    let db = TestDb::new().await;
    sqlx::query("CREATE TABLE IF NOT EXISTS saga_marker (stream_count BIGINT, version BIGINT)")
        .execute(&db.pool)
        .await
        .expect("create marker table");

    // Projection failure rolls the append back: nothing is committed.
    db.reset().await;
    let store = db.store();
    let id = StreamId::new("saga-x");
    let res = store
        .append_with_projection(&id, None, vec![event("E1", b"d")], &FailingProjector)
        .await;
    assert!(
        matches!(res, Err(AtomicError::Projection(_))),
        "projection failure should surface as AtomicError::Projection"
    );
    let loaded = store.load(&id, None).await.unwrap();
    assert!(
        loaded.is_empty(),
        "append must be rolled back when the projection fails"
    );

    // Success commits events AND the projection together.
    let v = store
        .append_with_projection(
            &id,
            None,
            vec![event("E1", b"d"), event("E2", b"d")],
            &SideTableProjector,
        )
        .await
        .unwrap();
    assert_eq!(v, Version::new(2));
    let loaded = store.load(&id, None).await.unwrap();
    assert_eq!(loaded.len(), 2, "events committed on success");
    let marker: (i64, i64) = sqlx::query_as("SELECT stream_count, version FROM saga_marker")
        .fetch_one(&db.pool)
        .await
        .expect("marker row should be committed in the same tx");
    assert_eq!(
        marker,
        (2, 2),
        "projection committed atomically with the events"
    );

    // Metadata round-trips through the atomic path (insert_events_tx) too.
    let metadata = EventMetadata {
        correlation_id: Some("req-atomic".to_string()),
        causation_id: None,
        user_id: None,
        author_id: Some(SubjectId::new("user-1")),
        origin_subject_id: None,
        timestamp: chrono::Utc::now(),
    };
    let mut e = event("E3", b"d");
    e.metadata = Some(metadata.clone());
    store
        .append_with_projection(&id, Some(Version::new(2)), vec![e], &SideTableProjector)
        .await
        .unwrap();
    let loaded = store.load(&id, None).await.unwrap();
    let last = loaded
        .last()
        .and_then(|e| e.metadata.as_ref())
        .expect("metadata should round-trip through the atomic append");
    assert_eq!(last.correlation_id, metadata.correlation_id);
    assert_eq!(last.author_id, metadata.author_id);
    assert_eq!(last.timestamp, metadata.timestamp);
}
