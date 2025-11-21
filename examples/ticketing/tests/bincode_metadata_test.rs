//! Test to verify bincode serialization works with EventMetadata.
//!
//! This test validates that SerializedEvent with EventMetadata can be
//! serialized/deserialized using bincode (fixing the previous serde_json::Value issue).

use composable_rust_core::event::{EventMetadata, SerializedEvent};

#[test]
fn test_bincode_serialization_with_metadata() {
    // Create a SerializedEvent with metadata (simulates what happens in production)
    let metadata = EventMetadata {
        correlation_id: Some("550e8400-e29b-41d4-a716-446655440000".to_string()),
        user_id: Some("user123".to_string()),
        timestamp: Some("2025-11-16T09:00:00Z".to_string()),
        causation_id: None,
    };

    let event = SerializedEvent {
        event_type: "TestEvent".to_string(),
        event_version: 1,
        data: vec![1, 2, 3, 4],
        metadata: Some(metadata.clone()),
    };

    // Serialize the ENTIRE SerializedEvent with bincode (this is what Redpanda does)
    let serialized = bincode::serialize(&event)
        .expect("Serialization should succeed");

    // Deserialize - should now work with EventMetadata!
    let deserialized: SerializedEvent = bincode::deserialize(&serialized)
        .expect("✅ Deserialization should succeed with EventMetadata");

    // Verify the deserialized event matches
    assert_eq!(event.event_type, deserialized.event_type);
    assert_eq!(event.data, deserialized.data);
    assert_eq!(event.metadata, deserialized.metadata);

    // Verify metadata fields
    let deserialized_metadata = deserialized.metadata.expect("Metadata should be present");
    assert_eq!(deserialized_metadata.correlation_id, Some("550e8400-e29b-41d4-a716-446655440000".to_string()));
    assert_eq!(deserialized_metadata.user_id, Some("user123".to_string()));
    assert_eq!(deserialized_metadata.timestamp, Some("2025-11-16T09:00:00Z".to_string()));
    assert_eq!(deserialized_metadata.causation_id, None)
}

#[test]
fn test_bincode_serialization_without_metadata() {
    // SerializedEvent WITHOUT metadata should work fine
    let event = SerializedEvent {
        event_type: "TestEvent".to_string(),
        event_version: 1,
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
