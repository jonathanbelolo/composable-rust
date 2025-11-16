//! Test bincode serialization of SerializedEvent with metadata

use composable_rust_core::event::SerializedEvent;
use ticketing::aggregates::InventoryAction;
use ticketing::projections::TicketingEvent;
use ticketing::types::{Capacity, EventId, SeatId};
use chrono::Utc;

#[test]
fn test_serialized_event_with_metadata_bincode() {
    // Create a TicketingEvent
    let action = InventoryAction::InventoryInitialized {
        event_id: EventId::new(),
        section: "VIP".to_string(),
        capacity: Capacity(100),
        seats: vec![SeatId::new()],
        initialized_at: Utc::now(),
    };
    let ticketing_event = TicketingEvent::Inventory(action);

    // Serialize the event data with bincode
    let event_data = bincode::serialize(&ticketing_event)
        .expect("Failed to serialize event data");

    // Create SerializedEvent with metadata (like services.rs does)
    let correlation_id = uuid::Uuid::new_v4();
    let metadata = Some(serde_json::json!({
        "correlation_id": correlation_id.to_string()
    }));

    let serialized_event = SerializedEvent::new(
        "InventoryInitialized".to_string(),
        event_data,
        metadata,
    );

    println!("Created SerializedEvent with metadata");

    // Try to serialize the ENTIRE SerializedEvent with bincode (like RedpandaEventBus does)
    let result = bincode::serialize(&serialized_event);

    match result {
        Ok(bytes) => {
            println!("SUCCESS: Serialized {} bytes", bytes.len());

            // Try to deserialize
            let deserialized: Result<SerializedEvent, _> = bincode::deserialize(&bytes);
            match deserialized {
                Ok(event) => {
                    println!("SUCCESS: Deserialized SerializedEvent");
                    assert_eq!(event.event_type, "InventoryInitialized");
                }
                Err(e) => {
                    panic!("FAILED to deserialize: {}", e);
                }
            }
        }
        Err(e) => {
            panic!("FAILED to serialize SerializedEvent: {}", e);
        }
    }
}
