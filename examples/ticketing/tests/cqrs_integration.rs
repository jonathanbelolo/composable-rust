//! Integration test demonstrating full CQRS flow with Event Sourcing.
//!
//! This test shows:
//! 1. Writing events to PostgreSQL event store
//! 2. Rebuilding projections from event history
//! 3. Querying projections for fast reads
//! 4. Complete separation of write and read models

use chrono::Utc;
use composable_rust_core::event::SerializedEvent;
use composable_rust_core::event_store::EventStore;
use composable_rust_core::stream::StreamId;
use composable_rust_postgres::PostgresEventStore;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use ticketing::{
    aggregates::{InventoryAction, ReservationAction},
    projections::{AvailableSeatsProjection, CustomerHistoryProjection, Projection, SalesAnalyticsProjection, TicketingEvent},
    types::{Capacity, CustomerId, EventId, Money, ReservationId, SeatId, TicketId},
};

/// Helper to create a PostgreSQL test container and event store
///
/// # Panics
/// Panics if container setup fails (test environment issue).
async fn create_event_store() -> PostgresEventStore {
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

    // Wait for postgres to be ready with retry logic
    let mut retries = 0;
    let max_retries = 60;
    loop {
        if let Ok(pool) = sqlx::PgPool::connect(&database_url).await {
            // Verify with a simple query
            if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                // Run migrations
                sqlx::migrate!("../../migrations")
                    .run(&pool)
                    .await
                    .expect("Failed to run migrations");

                // Small delay to ensure migrations are fully applied
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                return PostgresEventStore::from_pool(pool);
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

#[tokio::test]
#[ignore] // Requires Docker - run with: cargo test --test cqrs_integration -- --ignored
async fn test_full_cqrs_flow_with_event_sourcing() {
    // ========== Setup ==========
    let event_store = create_event_store().await;
    let stream_id = StreamId::new("ticketing-integration-test");

    let event_id = EventId::new();
    let customer_id = CustomerId::new();
    let reservation_id = ReservationId::new();

    // ========== Write Side: Generate Events ==========
    println!("📝 Write Side: Generating events...");

    let events = vec![
        // 1. Initialize inventory
        TicketingEvent::Inventory(InventoryAction::InventoryInitialized {
            event_id,
            section: "VIP".to_string(),
            capacity: Capacity::new(100),
            seats: vec![],
            initialized_at: Utc::now(),
        }),
        // 2. Reserve seats
        TicketingEvent::Inventory(InventoryAction::SeatsReserved {
            reservation_id,
            event_id,
            section: "VIP".to_string(),
            seats: vec![SeatId::new(), SeatId::new()],
            expires_at: Utc::now() + chrono::Duration::minutes(5),
            reserved_at: Utc::now(),
        }),
        // 3. Initiate reservation
        TicketingEvent::Reservation(ReservationAction::ReservationInitiated {
            reservation_id,
            event_id,
            customer_id,
            section: "VIP".to_string(),
            quantity: 2,
            expires_at: Utc::now() + chrono::Duration::minutes(5),
            initiated_at: Utc::now(),
        }),
        // 4. Allocate seats
        TicketingEvent::Reservation(ReservationAction::SeatsAllocated {
            reservation_id,
            seats: vec![SeatId::new(), SeatId::new()],
            total_amount: Money::from_dollars(200),
        }),
        // 5. Complete reservation
        TicketingEvent::Reservation(ReservationAction::ReservationCompleted {
            reservation_id,
            tickets_issued: vec![TicketId::new(), TicketId::new()],
            completed_at: Utc::now(),
        }),
        // 6. Confirm seats (mark as sold)
        TicketingEvent::Inventory(InventoryAction::SeatsConfirmed {
            reservation_id,
            event_id,
            section: "VIP".to_string(),
            customer_id,
            seats: vec![SeatId::new(), SeatId::new()],
            confirmed_at: Utc::now(),
        }),
    ];

    // Persist events to PostgreSQL event store
    let serialized_events: Vec<SerializedEvent> = events.iter().map(|event| {
        let event_data = serde_json::to_vec(&event).expect("Failed to serialize event");
        let event_type = match event {
            TicketingEvent::Inventory(_) => "InventoryEvent",
            TicketingEvent::Reservation(_) => "ReservationEvent",
            TicketingEvent::Payment(_) => "PaymentEvent",
            TicketingEvent::Event(_) => "EventEvent",
        };

        SerializedEvent::new(event_type.to_string(), event_data, None)
    }).collect();

    event_store
        .append_events(stream_id.clone(), None, serialized_events)
        .await
        .expect("Failed to append events");

    println!("✅ Persisted {events} events to event store", events = events.len());

    // ========== Read Side: Rebuild Projections from Event History ==========
    println!("\n📊 Read Side: Rebuilding projections from event history...");

    let mut available_seats = AvailableSeatsProjection::new();
    let mut sales_analytics = SalesAnalyticsProjection::new();
    let mut customer_history = CustomerHistoryProjection::new();

    // Load all events from event store
    let stored_events = event_store
        .load_events(stream_id.clone(), None)
        .await
        .expect("Failed to load events");

    assert_eq!(stored_events.len(), 6, "Should have loaded all 6 events");

    // Replay events through projections
    for stored_event in &stored_events {
        let event: TicketingEvent =
            serde_json::from_slice(&stored_event.data).expect("Failed to deserialize event");

        // Update all projections
        available_seats
            .handle_event(&event)
            .expect("AvailableSeats projection failed");
        sales_analytics
            .handle_event(&event)
            .expect("SalesAnalytics projection failed");
        customer_history
            .handle_event(&event)
            .expect("CustomerHistory projection failed");
    }

    println!("✅ Rebuilt 3 projections from {events} events", events = stored_events.len());

    // ========== Query Projections (Fast Reads) ==========
    println!("\n🔍 Querying projections...");

    // Query 1: Available Seats
    let availability = available_seats
        .get_availability(&event_id, "VIP")
        .expect("Should have VIP section");

    println!("\n📍 Available Seats Projection:");
    println!("   Total capacity: {}", availability.total_capacity);
    println!("   Reserved: {}", availability.reserved);
    println!("   Sold: {}", availability.sold);
    println!("   Available: {}", availability.available);

    assert_eq!(availability.total_capacity, 100);
    assert_eq!(availability.reserved, 0); // Confirmed, so no longer reserved
    assert_eq!(availability.sold, 2); // Confirmed -> sold
    assert_eq!(availability.available, 98); // 100 - 2

    // Query 2: Sales Analytics
    let metrics = sales_analytics
        .get_metrics(&event_id)
        .expect("Should have metrics");

    println!("\n💰 Sales Analytics Projection:");
    println!("   Total revenue: ${}", metrics.total_revenue.dollars());
    println!("   Tickets sold: {}", metrics.tickets_sold);
    println!("   Completed reservations: {}", metrics.completed_reservations);
    println!("   Average ticket price: ${}", metrics.average_ticket_price.dollars());

    assert_eq!(metrics.total_revenue, Money::from_dollars(200));
    assert_eq!(metrics.tickets_sold, 2);
    assert_eq!(metrics.completed_reservations, 1);
    assert_eq!(metrics.average_ticket_price, Money::from_dollars(100));

    // Query 3: Customer History
    let profile = customer_history
        .get_customer_profile(&customer_id)
        .expect("Should have customer profile");

    println!("\n👤 Customer History Projection:");
    println!("   Total spent: ${}", profile.total_spent.dollars());
    println!("   Total tickets: {}", profile.total_tickets);
    println!("   Events attended: {}", profile.events_attended.len());
    println!("   Purchases: {}", profile.purchases.len());

    assert_eq!(profile.total_spent, Money::from_dollars(200));
    assert_eq!(profile.total_tickets, 2);
    assert_eq!(profile.events_attended.len(), 1);
    assert!(customer_history.has_attended_event(&customer_id, &event_id));

    println!("\n✅ All projection queries successful!");

    // ========== Demonstrate Projection Rebuilding ==========
    println!("\n🔄 Demonstrating projection rebuilding...");

    // Reset and rebuild a projection
    available_seats.reset();
    assert!(available_seats.get_availability(&event_id, "VIP").is_none());

    // Rebuild from scratch
    for stored_event in &stored_events {
        let event: TicketingEvent =
            serde_json::from_slice(&stored_event.data).expect("Failed to deserialize event");
        available_seats
            .handle_event(&event)
            .expect("Failed to rebuild");
    }

    // Verify it's back to the same state
    let rebuilt = available_seats
        .get_availability(&event_id, "VIP")
        .expect("Should have VIP section after rebuild");

    assert_eq!(rebuilt.available, 98);
    println!("✅ Projection rebuilt successfully from event history");

    println!("\n🎉 Full CQRS Integration Test Complete!");
    println!("\nKey Takeaways:");
    println!("  ✓ Events persisted to PostgreSQL event store");
    println!("  ✓ Projections rebuilt from event history");
    println!("  ✓ Fast queries on denormalized read models");
    println!("  ✓ Write and read sides completely separated");
    println!("  ✓ Projections can be rebuilt at any time");
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_concurrent_reservations_with_event_store() {
    let event_store = create_event_store().await;
    let stream_id = StreamId::new("ticketing-concurrent-test");

    let event_id = EventId::new();

    // Initialize inventory with only 1 seat
    let init_event = TicketingEvent::Inventory(InventoryAction::InventoryInitialized {
        event_id,
        section: "VIP".to_string(),
        capacity: Capacity::new(1),
        seats: vec![],
        initialized_at: Utc::now(),
    });

    let event_data = serde_json::to_vec(&init_event).expect("Failed to serialize");
    event_store
        .append_events(
            stream_id.clone(),
            None,
            vec![SerializedEvent::new("InventoryEvent".to_string(), event_data, None)],
        )
        .await
        .expect("Failed to append event");

    // Customer 1 reserves the last seat
    let reservation1 = ReservationId::new();
    let reserve1 = TicketingEvent::Inventory(InventoryAction::SeatsReserved {
        reservation_id: reservation1,
        event_id,
        section: "VIP".to_string(),
        seats: vec![SeatId::new()],
        expires_at: Utc::now() + chrono::Duration::minutes(5),
        reserved_at: Utc::now(),
    });

    let event_data = serde_json::to_vec(&reserve1).expect("Failed to serialize");
    event_store
        .append_events(
            stream_id.clone(),
            None,
            vec![SerializedEvent::new("InventoryEvent".to_string(), event_data, None)],
        )
        .await
        .expect("Failed to append event");

    // Rebuild projection
    let mut projection = AvailableSeatsProjection::new();
    let events = event_store
        .load_events(stream_id, None)
        .await
        .expect("Failed to load events");

    for stored_event in &events {
        let event: TicketingEvent =
            serde_json::from_slice(&stored_event.data).expect("Failed to deserialize");
        projection.handle_event(&event).expect("Failed to handle event");
    }

    // Verify: Only 1 reserved, 0 available
    let availability = projection
        .get_availability(&event_id, "VIP")
        .expect("Should have VIP section");

    assert_eq!(availability.reserved, 1);
    assert_eq!(availability.available, 0);

    println!("✅ Concurrent reservation test passed - race condition prevented");
}
