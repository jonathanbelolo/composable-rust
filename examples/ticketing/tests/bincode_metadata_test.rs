//! Test to reproduce bincode deserialization issue with serde_json::Value metadata.
//!
//! This test demonstrates that SerializedEvent with JSON metadata cannot be
//! deserialized using bincode because serde_json::Value uses deserialize_any internally.

use composable_rust_core::event::SerializedEvent;

#[test]
fn test_bincode_serialization_with_json_metadata() {
    // Create a SerializedEvent with metadata (simulates what happens in production)
    let metadata = serde_json::json!({
        "correlation_id": "550e8400-e29b-41d4-a716-446655440000",
        "user_id": "user123",
        "timestamp": "2025-11-16T09:00:00Z"
    });

    let event = SerializedEvent {
        event_type: "TestEvent".to_string(),
        data: vec![1, 2, 3, 4],
        metadata: Some(metadata),
    };

    // Serialize the ENTIRE SerializedEvent with bincode (this is what Redpanda does)
    let serialized = bincode::serialize(&event)
        .expect("Serialization should succeed");

    // Try to deserialize - THIS WILL FAIL with "deserialize_any not supported"
    let result = bincode::deserialize::<SerializedEvent>(&serialized);

    match result {
        Ok(_) => {
            // If this succeeds, the bug is fixed!
            println!("✅ SerializedEvent with JSON metadata can be serialized/deserialized with bincode");
        }
        Err(e) => {
            // This is the current state - demonstrates the bug
            let error_msg = e.to_string();
            assert!(
                error_msg.contains("deserialize_any"),
                "Expected 'deserialize_any' error, got: {}",
                error_msg
            );
            panic!(
                "❌ BUG REPRODUCED: SerializedEvent with JSON metadata cannot be deserialized with bincode.\n\
                Error: {}\n\
                \n\
                This happens because:\n\
                1. RedpandaEventBus calls bincode::serialize(&SerializedEvent)\n\
                2. SerializedEvent.metadata is Option<serde_json::Value>\n\
                3. serde_json::Value uses deserialize_any internally\n\
                4. bincode 1.x doesn't support deserialize_any\n\
                \n\
                Fix: Change metadata serialization strategy in SerializedEvent or RedpandaEventBus.",
                error_msg
            );
        }
    }
}

#[test]
fn test_bincode_serialization_without_metadata() {
    // SerializedEvent WITHOUT metadata should work fine
    let event = SerializedEvent {
        event_type: "TestEvent".to_string(),
        data: vec![1, 2, 3, 4],
        metadata: None,
    };

    let serialized = bincode::serialize(&event)
        .expect("Serialization should succeed");

    let deserialized: SerializedEvent = bincode::deserialize(&serialized)
        .expect("Deserialization should succeed when metadata is None");

    assert_eq!(event.event_type, deserialized.event_type);
    assert_eq!(event.data, deserialized.data);
    assert!(deserialized.metadata.is_none());
}
