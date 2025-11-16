//! Test bincode serialization/deserialization of TicketingEvent

use ticketing::aggregates::InventoryAction;
use ticketing::projections::TicketingEvent;
use ticketing::types::{Capacity, EventId, SeatId};
use chrono::Utc;

#[test]
fn test_bincode_roundtrip() {
    // Create an InventoryAction (same as what the aggregate produces)
    let action = InventoryAction::InventoryInitialized {
        event_id: EventId::new(),
        section: "VIP".to_string(),
        capacity: Capacity(100),
        seats: vec![SeatId::new()],
        initialized_at: Utc::now(),
    };

    // Wrap it in TicketingEvent (same as services.rs line 380)
    let ticketing_event = TicketingEvent::Inventory(action.clone());

    // Serialize with bincode
    let serialized = bincode::serialize(&ticketing_event)
        .expect("Failed to serialize TicketingEvent");

    println!("Serialized {} bytes", serialized.len());

    // Try to deserialize (same as ProjectionManager does)
    let deserialized: TicketingEvent = bincode::deserialize(&serialized)
        .expect("Failed to deserialize TicketingEvent");

    // Verify it worked
    match deserialized {
        TicketingEvent::Inventory(InventoryAction::InventoryInitialized { section, .. }) => {
            assert_eq!(section, "VIP");
            println!("SUCCESS: Bincode roundtrip works!");
        }
        _ => panic!("Wrong event type after deserialization"),
    }
}
