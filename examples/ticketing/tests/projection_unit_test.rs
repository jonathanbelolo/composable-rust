//! Unit tests for PostgreSQL-backed projections.
//!
//! These tests directly call `apply_event()` on projections to isolate
//! the projection logic from event bus complexity.
//!
//! # Test Strategy
//!
//! 1. **Unit Tests**: Direct `apply_event()` calls (this file)
//! 2. **Integration Tests**: Full system with event bus (separate file)
//!
//! # Running Tests
//!
//! ```bash
//! # Start projection database
//! docker compose up -d postgres-projections
//!
//! # Run tests
//! PROJECTION_DATABASE_URL="postgresql://postgres:postgres@localhost:5433/ticketing_projections" \
//! cargo test --test projection_unit_test
//! ```

use composable_rust_auth::state::UserId;
use composable_rust_core::projection::Projection;
use sqlx::PgPool;
use std::sync::Arc;
use ticketing::{
    aggregates::{EventAction, InventoryAction, ReservationAction},
    projections::{
        PostgresAvailableSeatsProjection, PostgresCustomerHistoryProjection,
        PostgresEventsProjection, PostgresSalesAnalyticsProjection, TicketingEvent,
    },
    types::{
        Capacity, CustomerId, EventDate, EventId, EventStatus, Money, PricingTier, ReservationId,
        SeatId, TicketId, TierType, Venue, VenueSection,
    },
};

// ============================================================================
// Test Helpers
// ============================================================================

/// Setup test database connection and run migrations.
///
/// # Panics
///
/// Panics if database connection or migrations fail (test setup failure).
async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("PROJECTION_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://postgres:postgres@localhost:5433/ticketing_projections".to_string()
    });

    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to projections database");

    // Run migrations to ensure schema is up to date
    sqlx::migrate!("./migrations_projections")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

/// Create a test event ID and clear any existing data for it.
///
/// Returns (`event_id`, `pool`).
async fn setup_test_event(pool: &PgPool) -> EventId {
    let event_id = EventId::new();

    // Clear any existing test data for this event
    sqlx::query("DELETE FROM available_seats_projection WHERE event_id = $1")
        .bind(event_id.as_uuid())
        .execute(pool)
        .await
        .expect("Failed to clear test data");

    sqlx::query("DELETE FROM events_projection WHERE id = $1")
        .bind(event_id.as_uuid())
        .execute(pool)
        .await
        .expect("Failed to clear events test data");

    event_id
}

/// Clear processed_reservations table for idempotency tests.
async fn clear_processed_reservations(pool: &PgPool) {
    sqlx::query("DELETE FROM processed_reservations")
        .execute(pool)
        .await
        .expect("Failed to clear processed reservations");
}

/// Create a test inventory initialized event.
#[allow(clippy::unwrap_used)] // Test helper
fn create_inventory_initialized(
    event_id: EventId,
    section: &str,
    capacity: u32,
) -> TicketingEvent {
    let seats: Vec<SeatId> = (0..capacity).map(|_| SeatId::new()).collect();

    TicketingEvent::Inventory(InventoryAction::InventoryInitialized {
        event_id,
        section: section.to_string(),
        capacity: Capacity(capacity),
        seats,
        initialized_at: chrono::Utc::now(),
    })
}

/// Create a test seats reserved event.
fn create_seats_reserved(
    event_id: EventId,
    section: &str,
    reservation_id: ReservationId,
    seat_count: usize,
) -> TicketingEvent {
    let seats: Vec<SeatId> = (0..seat_count).map(|_| SeatId::new()).collect();
    let now = chrono::Utc::now();

    TicketingEvent::Inventory(InventoryAction::SeatsReserved {
        event_id,
        section: section.to_string(),
        reservation_id,
        seats,
        expires_at: now + chrono::Duration::minutes(15), // 15 minute expiration
        reserved_at: now,
    })
}

/// Create a test seats confirmed event.
fn create_seats_confirmed(
    event_id: EventId,
    section: &str,
    reservation_id: ReservationId,
    seat_count: usize,
) -> TicketingEvent {
    let seats: Vec<SeatId> = (0..seat_count).map(|_| SeatId::new()).collect();

    TicketingEvent::Inventory(InventoryAction::SeatsConfirmed {
        event_id,
        section: section.to_string(),
        reservation_id,
        customer_id: CustomerId::new(), // Test customer
        seats,
        confirmed_at: chrono::Utc::now(),
    })
}

/// Create a test seats released event.
fn create_seats_released(
    event_id: EventId,
    section: &str,
    reservation_id: ReservationId,
    seat_count: usize,
) -> TicketingEvent {
    let seats: Vec<SeatId> = (0..seat_count).map(|_| SeatId::new()).collect();

    TicketingEvent::Inventory(InventoryAction::SeatsReleased {
        event_id,
        section: section.to_string(),
        reservation_id,
        seats,
        released_at: chrono::Utc::now(),
    })
}

// ============================================================================
// PostgresAvailableSeatsProjection Tests
// ============================================================================

/// Test 1: InventoryInitialized creates availability record.
///
/// # Test Flow
///
/// 1. Create `InventoryInitialized` event (100 seats)
/// 2. Apply event to projection
/// 3. Query database
/// 4. Assert: total=100, reserved=0, sold=0, available=100
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_available_seats_inventory_initialized() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    let projection = PostgresAvailableSeatsProjection::new(Arc::new(pool.clone()));

    // Create and apply InventoryInitialized event
    let event = create_inventory_initialized(event_id, "VIP", 100);
    projection
        .apply_event(&event)
        .await
        .expect("Failed to apply event");

    // Query and verify
    let (total, reserved, sold, available) = projection
        .get_availability(&event_id, "VIP")
        .await
        .expect("Failed to query availability")
        .expect("No availability record found");

    assert_eq!(total, 100, "Expected total_capacity=100");
    assert_eq!(reserved, 0, "Expected reserved=0");
    assert_eq!(sold, 0, "Expected sold=0");
    assert_eq!(available, 100, "Expected available=100");

    println!("✅ InventoryInitialized: Creates availability record correctly");
}

/// Test 2: SeatsReserved updates availability correctly.
///
/// # Test Flow
///
/// 1. Initialize inventory (100 seats)
/// 2. Reserve 10 seats
/// 3. Assert: total=100, reserved=10, sold=0, available=90
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_available_seats_reservation() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    clear_processed_reservations(&pool).await;
    let projection = PostgresAvailableSeatsProjection::new(Arc::new(pool.clone()));

    // Initialize inventory
    let init_event = create_inventory_initialized(event_id, "VIP", 100);
    projection.apply_event(&init_event).await.unwrap();

    // Reserve 10 seats
    let reservation_id = ReservationId::new();
    let reserve_event = create_seats_reserved(event_id, "VIP", reservation_id, 10);
    projection.apply_event(&reserve_event).await.unwrap();

    // Verify
    let (total, reserved, sold, available) = projection
        .get_availability(&event_id, "VIP")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(total, 100);
    assert_eq!(reserved, 10);
    assert_eq!(sold, 0);
    assert_eq!(available, 90);

    println!("✅ SeatsReserved: Updates availability correctly");
}

/// Test 3: SeatsConfirmed moves from reserved to sold.
///
/// # Test Flow
///
/// 1. Initialize inventory (100 seats)
/// 2. Reserve 10 seats
/// 3. Confirm those 10 seats
/// 4. Assert: total=100, reserved=0, sold=10, available=90
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_available_seats_confirmation() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    clear_processed_reservations(&pool).await;
    let projection = PostgresAvailableSeatsProjection::new(Arc::new(pool.clone()));

    // Initialize inventory
    projection
        .apply_event(&create_inventory_initialized(event_id, "VIP", 100))
        .await
        .unwrap();

    // Reserve 10 seats
    let reservation_id = ReservationId::new();
    projection
        .apply_event(&create_seats_reserved(event_id, "VIP", reservation_id, 10))
        .await
        .unwrap();

    // Confirm those seats
    projection
        .apply_event(&create_seats_confirmed(
            event_id,
            "VIP",
            reservation_id,
            10,
        ))
        .await
        .unwrap();

    // Verify: reserved should decrease, sold should increase
    let (total, reserved, sold, available) = projection
        .get_availability(&event_id, "VIP")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(total, 100);
    assert_eq!(reserved, 0, "Reserved should be 0 after confirmation");
    assert_eq!(sold, 10, "Sold should be 10 after confirmation");
    assert_eq!(available, 90, "Available should be 90 (100 - 10 sold)");

    println!("✅ SeatsConfirmed: Moves seats from reserved to sold correctly");
}

/// Test 4: SeatsReleased moves from reserved back to available.
///
/// # Test Flow
///
/// 1. Initialize inventory (100 seats)
/// 2. Reserve 10 seats
/// 3. Release those 10 seats
/// 4. Assert: total=100, reserved=0, sold=0, available=100
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_available_seats_release() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    clear_processed_reservations(&pool).await;
    let projection = PostgresAvailableSeatsProjection::new(Arc::new(pool.clone()));

    // Initialize inventory
    projection
        .apply_event(&create_inventory_initialized(event_id, "VIP", 100))
        .await
        .unwrap();

    // Reserve 10 seats
    let reservation_id = ReservationId::new();
    projection
        .apply_event(&create_seats_reserved(event_id, "VIP", reservation_id, 10))
        .await
        .unwrap();

    // Release those seats
    projection
        .apply_event(&create_seats_released(
            event_id,
            "VIP",
            reservation_id,
            10,
        ))
        .await
        .unwrap();

    // Verify: seats should be back to available
    let (total, reserved, sold, available) = projection
        .get_availability(&event_id, "VIP")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(total, 100);
    assert_eq!(reserved, 0, "Reserved should be 0 after release");
    assert_eq!(sold, 0, "Sold should still be 0");
    assert_eq!(available, 100, "Available should be 100 (all released)");

    println!("✅ SeatsReleased: Returns seats to available correctly");
}

/// Test 5: Multiple sections work independently.
///
/// # Test Flow
///
/// 1. Initialize VIP section (50 seats)
/// 2. Initialize General section (200 seats)
/// 3. Reserve 10 VIP seats
/// 4. Reserve 20 General seats
/// 5. Assert both sections have correct counts
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_available_seats_multiple_sections() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    clear_processed_reservations(&pool).await;
    let projection = PostgresAvailableSeatsProjection::new(Arc::new(pool.clone()));

    // Initialize VIP section
    projection
        .apply_event(&create_inventory_initialized(event_id, "VIP", 50))
        .await
        .unwrap();

    // Initialize General section
    projection
        .apply_event(&create_inventory_initialized(event_id, "General", 200))
        .await
        .unwrap();

    // Reserve 10 VIP seats
    projection
        .apply_event(&create_seats_reserved(
            event_id,
            "VIP",
            ReservationId::new(),
            10,
        ))
        .await
        .unwrap();

    // Reserve 20 General seats
    projection
        .apply_event(&create_seats_reserved(
            event_id,
            "General",
            ReservationId::new(),
            20,
        ))
        .await
        .unwrap();

    // Verify VIP section
    let (total, reserved, _sold, available) = projection
        .get_availability(&event_id, "VIP")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(total, 50);
    assert_eq!(reserved, 10);
    assert_eq!(available, 40);

    // Verify General section
    let (total, reserved, _sold, available) = projection
        .get_availability(&event_id, "General")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(total, 200);
    assert_eq!(reserved, 20);
    assert_eq!(available, 180);

    println!("✅ Multiple sections: Work independently");
}

/// Test 6: Idempotency - duplicate SeatsReserved events are ignored.
///
/// # Test Flow
///
/// 1. Initialize inventory (100 seats)
/// 2. Reserve 10 seats (reservation_id=X)
/// 3. Apply same reserve event again (duplicate)
/// 4. Assert: reserved=10 (not 20), available=90 (not 80)
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_available_seats_idempotency() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    clear_processed_reservations(&pool).await;
    let projection = PostgresAvailableSeatsProjection::new(Arc::new(pool.clone()));

    // Initialize inventory
    projection
        .apply_event(&create_inventory_initialized(event_id, "VIP", 100))
        .await
        .unwrap();

    // Reserve 10 seats
    let reservation_id = ReservationId::new();
    let reserve_event = create_seats_reserved(event_id, "VIP", reservation_id, 10);
    projection.apply_event(&reserve_event).await.unwrap();

    // Apply same event again (idempotency test)
    projection.apply_event(&reserve_event).await.unwrap();

    // Verify: should still only have 10 reserved (not 20)
    let (total, reserved, _sold, available) = projection
        .get_availability(&event_id, "VIP")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(total, 100);
    assert_eq!(reserved, 10, "Idempotency: Reserved should be 10, not 20");
    assert_eq!(available, 90, "Idempotency: Available should be 90, not 80");

    println!("✅ Idempotency: Duplicate SeatsReserved events are ignored correctly");
}

/// Test 7: Complete reservation flow (reserve → confirm).
///
/// # Test Flow
///
/// 1. Initialize inventory (100 seats)
/// 2. Reserve 10 seats
/// 3. Confirm those seats
/// 4. Reserve 5 more seats
/// 5. Assert: reserved=5, sold=10, available=85
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_available_seats_complete_flow() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    clear_processed_reservations(&pool).await;
    let projection = PostgresAvailableSeatsProjection::new(Arc::new(pool.clone()));

    // Initialize inventory
    projection
        .apply_event(&create_inventory_initialized(event_id, "VIP", 100))
        .await
        .unwrap();

    // First reservation: Reserve 10 seats
    let res1 = ReservationId::new();
    projection
        .apply_event(&create_seats_reserved(event_id, "VIP", res1, 10))
        .await
        .unwrap();

    // Confirm first reservation
    projection
        .apply_event(&create_seats_confirmed(event_id, "VIP", res1, 10))
        .await
        .unwrap();

    // Second reservation: Reserve 5 more seats
    let res2 = ReservationId::new();
    projection
        .apply_event(&create_seats_reserved(event_id, "VIP", res2, 5))
        .await
        .unwrap();

    // Verify final state
    let (total, reserved, sold, available) = projection
        .get_availability(&event_id, "VIP")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(total, 100);
    assert_eq!(sold, 10, "Should have 10 sold (first reservation)");
    assert_eq!(reserved, 5, "Should have 5 reserved (second reservation)");
    assert_eq!(available, 85, "Available = 100 - 10 sold - 5 reserved");

    println!("✅ Complete flow: Reserve → Confirm → Reserve works correctly");
}

/// Test 8: Get all sections for an event.
///
/// # Test Flow
///
/// 1. Initialize multiple sections (VIP, General, Balcony)
/// 2. Query all sections
/// 3. Assert correct data for each section
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_available_seats_get_all_sections() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    let projection = PostgresAvailableSeatsProjection::new(Arc::new(pool.clone()));

    // Initialize multiple sections
    projection
        .apply_event(&create_inventory_initialized(event_id, "VIP", 50))
        .await
        .unwrap();
    projection
        .apply_event(&create_inventory_initialized(event_id, "General", 200))
        .await
        .unwrap();
    projection
        .apply_event(&create_inventory_initialized(event_id, "Balcony", 100))
        .await
        .unwrap();

    // Query all sections
    let sections = projection
        .get_all_sections(&event_id)
        .await
        .expect("Failed to query all sections");

    assert_eq!(sections.len(), 3, "Should have 3 sections");

    // Verify sections are present (order may vary)
    let section_names: Vec<String> = sections.iter().map(|s| s.section.clone()).collect();
    assert!(section_names.contains(&"VIP".to_string()));
    assert!(section_names.contains(&"General".to_string()));
    assert!(section_names.contains(&"Balcony".to_string()));

    // Verify total capacities
    for section in &sections {
        match section.section.as_str() {
            "VIP" => assert_eq!(section.total_capacity, 50),
            "General" => assert_eq!(section.total_capacity, 200),
            "Balcony" => assert_eq!(section.total_capacity, 100),
            _ => panic!("Unexpected section: {}", section.section),
        }
    }

    println!("✅ Get all sections: Returns correct data for all sections");
}

/// Test 9: Get total available across all sections.
///
/// # Test Flow
///
/// 1. Initialize VIP (50 seats), General (200 seats)
/// 2. Reserve 10 VIP, 20 General
/// 3. Assert total_available = (50-10) + (200-20) = 220
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_available_seats_get_total_available() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    clear_processed_reservations(&pool).await;
    let projection = PostgresAvailableSeatsProjection::new(Arc::new(pool.clone()));

    // Initialize sections
    projection
        .apply_event(&create_inventory_initialized(event_id, "VIP", 50))
        .await
        .unwrap();
    projection
        .apply_event(&create_inventory_initialized(event_id, "General", 200))
        .await
        .unwrap();

    // Reserve seats
    projection
        .apply_event(&create_seats_reserved(
            event_id,
            "VIP",
            ReservationId::new(),
            10,
        ))
        .await
        .unwrap();
    projection
        .apply_event(&create_seats_reserved(
            event_id,
            "General",
            ReservationId::new(),
            20,
        ))
        .await
        .unwrap();

    // Query total available
    let total_available = projection
        .get_total_available(&event_id)
        .await
        .expect("Failed to query total available");

    // VIP: 50-10=40 available, General: 200-20=180 available, Total: 220
    assert_eq!(
        total_available, 220,
        "Total available should be 220 (40 VIP + 180 General)"
    );

    println!("✅ Get total available: Correctly sums across all sections");
}

// ============================================================================
// PostgresEventsProjection Tests
// ============================================================================

/// Helper to create a test venue.
fn create_test_venue(vip_capacity: u32, general_capacity: u32) -> Venue {
    Venue {
        name: "Test Arena".to_string(),
        capacity: Capacity(vip_capacity + general_capacity),
        sections: vec![
            VenueSection {
                name: "VIP".to_string(),
                capacity: Capacity(vip_capacity),
                seat_type: ticketing::types::SeatType::GeneralAdmission,
            },
            VenueSection {
                name: "General".to_string(),
                capacity: Capacity(general_capacity),
                seat_type: ticketing::types::SeatType::GeneralAdmission,
            },
        ],
    }
}

/// Helper to create test pricing tiers.
fn create_test_pricing_tiers() -> Vec<PricingTier> {
    vec![
        PricingTier {
            tier_type: TierType::Regular,
            section: "VIP".to_string(),
            base_price: Money::from_cents(15000), // $150.00
            available_from: chrono::Utc::now(),
            available_until: None,
        },
        PricingTier {
            tier_type: TierType::Regular,
            section: "General".to_string(),
            base_price: Money::from_cents(5000), // $50.00
            available_from: chrono::Utc::now(),
            available_until: None,
        },
    ]
}

/// Test 10: EventCreated creates event record in projection.
///
/// # Test Flow
///
/// 1. Create `EventCreated` event
/// 2. Apply to projection
/// 3. Query projection
/// 4. Assert event data matches
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_events_projection_event_created() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    let projection = PostgresEventsProjection::new(Arc::new(pool.clone()));

    let venue = create_test_venue(50, 200);
    let pricing_tiers = create_test_pricing_tiers();
    let created_at = chrono::Utc::now();

    // Create EventCreated event
    let event = TicketingEvent::Event(EventAction::EventCreated {
        id: event_id,
        name: "Test Concert".to_string(),
        owner_id: UserId::new(),
        venue: venue.clone(),
        date: EventDate::new(chrono::Utc::now() + chrono::Duration::days(30)),
        pricing_tiers: pricing_tiers.clone(),
        created_at,
    });

    // Apply event
    projection.apply_event(&event).await.unwrap();

    // Query projection
    let stored_event = projection
        .get(event_id.as_uuid())
        .await
        .expect("Failed to query projection")
        .expect("Event not found in projection");

    // Verify
    assert_eq!(stored_event.id, event_id);
    assert_eq!(stored_event.name, "Test Concert");
    assert_eq!(stored_event.status, EventStatus::Draft);

    println!("✅ EventCreated: Creates event record in projection");
}

/// Test 11: EventPublished updates status to Published.
///
/// # Test Flow
///
/// 1. Create event (status=Draft)
/// 2. Publish event
/// 3. Query projection
/// 4. Assert status=Published
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_events_projection_event_published() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    let projection = PostgresEventsProjection::new(Arc::new(pool.clone()));

    // Create event
    let create_event = TicketingEvent::Event(EventAction::EventCreated {
        id: event_id,
        name: "Test Concert".to_string(),
        owner_id: UserId::new(),
        venue: create_test_venue(50, 200),
        date: EventDate::new(chrono::Utc::now() + chrono::Duration::days(30)),
        pricing_tiers: create_test_pricing_tiers(),
        created_at: chrono::Utc::now(),
    });
    projection.apply_event(&create_event).await.unwrap();

    // Publish event
    let publish_event = TicketingEvent::Event(EventAction::EventPublished {
        event_id,
        published_at: chrono::Utc::now(),
    });
    projection.apply_event(&publish_event).await.unwrap();

    // Query and verify
    let stored_event = projection.get(event_id.as_uuid()).await.unwrap().unwrap();
    assert_eq!(stored_event.status, EventStatus::Published);

    println!("✅ EventPublished: Updates status to Published");
}

/// Test 12: SalesOpened updates status to SalesOpen.
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_events_projection_sales_opened() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    let projection = PostgresEventsProjection::new(Arc::new(pool.clone()));

    // Create and publish event
    projection
        .apply_event(&TicketingEvent::Event(EventAction::EventCreated {
            id: event_id,
            name: "Test Concert".to_string(),
            owner_id: UserId::new(),
            venue: create_test_venue(50, 200),
            date: EventDate::new(chrono::Utc::now() + chrono::Duration::days(30)),
            pricing_tiers: create_test_pricing_tiers(),
            created_at: chrono::Utc::now(),
        }))
        .await
        .unwrap();

    // Open sales
    projection
        .apply_event(&TicketingEvent::Event(EventAction::SalesOpened {
            event_id,
            opened_at: chrono::Utc::now(),
        }))
        .await
        .unwrap();

    // Verify
    let stored_event = projection.get(event_id.as_uuid()).await.unwrap().unwrap();
    assert_eq!(stored_event.status, EventStatus::SalesOpen);

    println!("✅ SalesOpened: Updates status to SalesOpen");
}

/// Test 13: SalesClosed updates status to SalesClosed.
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_events_projection_sales_closed() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    let projection = PostgresEventsProjection::new(Arc::new(pool.clone()));

    // Create event
    projection
        .apply_event(&TicketingEvent::Event(EventAction::EventCreated {
            id: event_id,
            name: "Test Concert".to_string(),
            owner_id: UserId::new(),
            venue: create_test_venue(50, 200),
            date: EventDate::new(chrono::Utc::now() + chrono::Duration::days(30)),
            pricing_tiers: create_test_pricing_tiers(),
            created_at: chrono::Utc::now(),
        }))
        .await
        .unwrap();

    // Close sales
    projection
        .apply_event(&TicketingEvent::Event(EventAction::SalesClosed {
            event_id,
            closed_at: chrono::Utc::now(),
        }))
        .await
        .unwrap();

    // Verify
    let stored_event = projection.get(event_id.as_uuid()).await.unwrap().unwrap();
    assert_eq!(stored_event.status, EventStatus::SalesClosed);

    println!("✅ SalesClosed: Updates status to SalesClosed");
}

/// Test 14: EventCancelled updates status to Cancelled.
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_events_projection_event_cancelled() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    let projection = PostgresEventsProjection::new(Arc::new(pool.clone()));

    // Create event
    projection
        .apply_event(&TicketingEvent::Event(EventAction::EventCreated {
            id: event_id,
            name: "Test Concert".to_string(),
            owner_id: UserId::new(),
            venue: create_test_venue(50, 200),
            date: EventDate::new(chrono::Utc::now() + chrono::Duration::days(30)),
            pricing_tiers: create_test_pricing_tiers(),
            created_at: chrono::Utc::now(),
        }))
        .await
        .unwrap();

    // Cancel event
    projection
        .apply_event(&TicketingEvent::Event(EventAction::EventCancelled {
            event_id,
            reason: "Test cancellation".to_string(),
            cancelled_at: chrono::Utc::now(),
        }))
        .await
        .unwrap();

    // Verify
    let stored_event = projection.get(event_id.as_uuid()).await.unwrap().unwrap();
    assert_eq!(stored_event.status, EventStatus::Cancelled);

    println!("✅ EventCancelled: Updates status to Cancelled");
}

/// Test 15: List events with status filter.
#[tokio::test]
#[allow(clippy::unwrap_used)] // Test code
async fn test_events_projection_list_with_filter() {
    let pool = setup_test_db().await;
    let projection = PostgresEventsProjection::new(Arc::new(pool.clone()));

    // Clear all test events
    sqlx::query("DELETE FROM events_projection WHERE data->>'name' LIKE 'Filter Test%'")
        .execute(&pool)
        .await
        .unwrap();

    // Create multiple events with different statuses
    let event1 = EventId::new();
    let event2 = EventId::new();
    let event3 = EventId::new();

    // Event 1: Draft
    projection
        .apply_event(&TicketingEvent::Event(EventAction::EventCreated {
            id: event1,
            name: "Filter Test 1".to_string(),
            owner_id: UserId::new(),
            venue: create_test_venue(50, 200),
            date: EventDate::new(chrono::Utc::now() + chrono::Duration::days(30)),
            pricing_tiers: create_test_pricing_tiers(),
            created_at: chrono::Utc::now(),
        }))
        .await
        .unwrap();

    // Event 2: Published
    projection
        .apply_event(&TicketingEvent::Event(EventAction::EventCreated {
            id: event2,
            name: "Filter Test 2".to_string(),
            owner_id: UserId::new(),
            venue: create_test_venue(50, 200),
            date: EventDate::new(chrono::Utc::now() + chrono::Duration::days(30)),
            pricing_tiers: create_test_pricing_tiers(),
            created_at: chrono::Utc::now(),
        }))
        .await
        .unwrap();
    projection
        .apply_event(&TicketingEvent::Event(EventAction::EventPublished {
            event_id: event2,
            published_at: chrono::Utc::now(),
        }))
        .await
        .unwrap();

    // Event 3: Published
    projection
        .apply_event(&TicketingEvent::Event(EventAction::EventCreated {
            id: event3,
            name: "Filter Test 3".to_string(),
            owner_id: UserId::new(),
            venue: create_test_venue(50, 200),
            date: EventDate::new(chrono::Utc::now() + chrono::Duration::days(30)),
            pricing_tiers: create_test_pricing_tiers(),
            created_at: chrono::Utc::now(),
        }))
        .await
        .unwrap();
    projection
        .apply_event(&TicketingEvent::Event(EventAction::EventPublished {
            event_id: event3,
            published_at: chrono::Utc::now(),
        }))
        .await
        .unwrap();

    // List all events
    let all_events = projection.list(None).await.unwrap();
    assert!(
        all_events.len() >= 3,
        "Should have at least 3 events (may have more from other tests)"
    );

    // List only Published events
    let published_events = projection.list(Some("Published")).await.unwrap();
    let published_count = published_events
        .iter()
        .filter(|e| e.name.starts_with("Filter Test"))
        .count();
    assert_eq!(
        published_count, 2,
        "Should have exactly 2 Published events from this test"
    );

    println!("✅ List events: Correctly filters by status");
}

// ============================================================================
// PostgresSalesAnalyticsProjection Tests
// ============================================================================

/// Helper to create a reservation initiated event.
fn create_reservation_initiated(
    reservation_id: ReservationId,
    event_id: EventId,
    customer_id: CustomerId,
    section: &str,
    quantity: u32,
) -> TicketingEvent {
    TicketingEvent::Reservation(ReservationAction::ReservationInitiated {
        reservation_id,
        event_id,
        customer_id,
        section: section.to_string(),
        quantity,
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(15),
        initiated_at: chrono::Utc::now(),
    })
}

/// Helper to create a seats allocated event.
fn create_seats_allocated(
    reservation_id: ReservationId,
    seat_count: usize,
    total_amount: Money,
) -> TicketingEvent {
    let seats: Vec<SeatId> = (0..seat_count).map(|_| SeatId::new()).collect();
    TicketingEvent::Reservation(ReservationAction::SeatsAllocated {
        reservation_id,
        seats,
        total_amount,
    })
}

/// Helper to create a reservation completed event.
fn create_reservation_completed(
    reservation_id: ReservationId,
    ticket_count: usize,
) -> TicketingEvent {
    let tickets: Vec<TicketId> = (0..ticket_count).map(|_| TicketId::new()).collect();
    TicketingEvent::Reservation(ReservationAction::ReservationCompleted {
        reservation_id,
        tickets_issued: tickets,
        completed_at: chrono::Utc::now(),
    })
}

/// Helper to create a reservation cancelled event.
fn create_reservation_cancelled(reservation_id: ReservationId) -> TicketingEvent {
    TicketingEvent::Reservation(ReservationAction::ReservationCancelled {
        reservation_id,
        reason: "Test cancellation".to_string(),
        cancelled_at: chrono::Utc::now(),
    })
}

/// Test 16: Sales analytics tracks completed reservation.
///
/// # Test Flow
///
/// 1. Initialize projection
/// 2. Create and complete a reservation (2 VIP tickets, $200)
/// 3. Query sales metrics
/// 4. Assert: total_revenue=$200, tickets_sold=2, completed=1
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_sales_analytics_completed_reservation() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    let projection = PostgresSalesAnalyticsProjection::new(Arc::new(pool.clone()));

    let customer_id = CustomerId::new();
    let reservation_id = ReservationId::new();

    // Initiate reservation
    projection
        .apply_event(&create_reservation_initiated(
            reservation_id,
            event_id,
            customer_id,
            "VIP",
            2,
        ))
        .await
        .unwrap();

    // Allocate seats with actual pricing
    projection
        .apply_event(&create_seats_allocated(
            reservation_id,
            2,
            Money::from_cents(20000), // $200
        ))
        .await
        .unwrap();

    // Complete reservation
    projection
        .apply_event(&create_reservation_completed(reservation_id, 2))
        .await
        .unwrap();

    // Verify metrics
    let metrics = projection
        .get_metrics(&event_id)
        .await
        .unwrap()
        .expect("Metrics should exist");

    assert_eq!(
        metrics.total_revenue,
        Money::from_cents(20000),
        "Total revenue should be $200"
    );
    assert_eq!(metrics.tickets_sold, 2, "Should have sold 2 tickets");
    assert_eq!(
        metrics.completed_reservations, 1,
        "Should have 1 completed reservation"
    );
    assert_eq!(
        metrics.average_ticket_price,
        Money::from_cents(10000),
        "Average ticket price should be $100"
    );

    println!("✅ Sales analytics: Tracks completed reservation correctly");
}

/// Test 17: Sales analytics tracks cancelled reservation.
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_sales_analytics_cancelled_reservation() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    let projection = PostgresSalesAnalyticsProjection::new(Arc::new(pool.clone()));

    let reservation_id = ReservationId::new();

    // Initiate reservation
    projection
        .apply_event(&create_reservation_initiated(
            reservation_id,
            event_id,
            CustomerId::new(),
            "General",
            3,
        ))
        .await
        .unwrap();

    // Cancel reservation
    projection
        .apply_event(&create_reservation_cancelled(reservation_id))
        .await
        .unwrap();

    // Verify metrics
    let metrics = projection
        .get_metrics(&event_id)
        .await
        .unwrap()
        .expect("Metrics should exist");

    assert_eq!(
        metrics.total_revenue,
        Money::from_cents(0),
        "Total revenue should be $0"
    );
    assert_eq!(metrics.tickets_sold, 0, "Should have sold 0 tickets");
    assert_eq!(
        metrics.cancelled_reservations, 1,
        "Should have 1 cancelled reservation"
    );

    println!("✅ Sales analytics: Tracks cancelled reservation correctly");
}

/// Test 18: Sales analytics tracks multiple completions.
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_sales_analytics_multiple_completions() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    let projection = PostgresSalesAnalyticsProjection::new(Arc::new(pool.clone()));

    // Complete 3 reservations with different amounts
    for i in 0..3 {
        let reservation_id = ReservationId::new();

        projection
            .apply_event(&create_reservation_initiated(
                reservation_id,
                event_id,
                CustomerId::new(),
                "VIP",
                2,
            ))
            .await
            .unwrap();

        projection
            .apply_event(&create_seats_allocated(
                reservation_id,
                2,
                Money::from_cents((i + 1) * 10000), // $100, $200, $300
            ))
            .await
            .unwrap();

        projection
            .apply_event(&create_reservation_completed(reservation_id, 2))
            .await
            .unwrap();
    }

    // Verify metrics
    let metrics = projection
        .get_metrics(&event_id)
        .await
        .unwrap()
        .expect("Metrics should exist");

    assert_eq!(
        metrics.total_revenue,
        Money::from_cents(60000),
        "Total revenue should be $600"
    );
    assert_eq!(metrics.tickets_sold, 6, "Should have sold 6 tickets");
    assert_eq!(
        metrics.completed_reservations, 3,
        "Should have 3 completed reservations"
    );
    assert_eq!(
        metrics.average_ticket_price,
        Money::from_cents(10000),
        "Average should be $100"
    );

    println!("✅ Sales analytics: Tracks multiple completions correctly");
}

/// Test 19: Sales analytics tracks section metrics.
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_sales_analytics_section_metrics() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    let projection = PostgresSalesAnalyticsProjection::new(Arc::new(pool.clone()));

    // VIP reservation
    let res1 = ReservationId::new();
    projection
        .apply_event(&create_reservation_initiated(
            res1,
            event_id,
            CustomerId::new(),
            "VIP",
            2,
        ))
        .await
        .unwrap();
    projection
        .apply_event(&create_seats_allocated(
            res1,
            2,
            Money::from_cents(20000),
        ))
        .await
        .unwrap();
    projection
        .apply_event(&create_reservation_completed(res1, 2))
        .await
        .unwrap();

    // General reservation
    let res2 = ReservationId::new();
    projection
        .apply_event(&create_reservation_initiated(
            res2,
            event_id,
            CustomerId::new(),
            "General",
            3,
        ))
        .await
        .unwrap();
    projection
        .apply_event(&create_seats_allocated(
            res2,
            3,
            Money::from_cents(15000),
        ))
        .await
        .unwrap();
    projection
        .apply_event(&create_reservation_completed(res2, 3))
        .await
        .unwrap();

    // Query section metrics
    let section_metrics = projection
        .get_section_metrics(&event_id)
        .await
        .unwrap();

    assert_eq!(
        section_metrics.len(),
        2,
        "Should have metrics for 2 sections"
    );

    // Find VIP and General sections
    let vip = section_metrics
        .iter()
        .find(|s| s.section == "VIP")
        .expect("Should have VIP section");
    let general = section_metrics
        .iter()
        .find(|s| s.section == "General")
        .expect("Should have General section");

    assert_eq!(vip.revenue, Money::from_cents(20000));
    assert_eq!(vip.tickets_sold, 2);
    assert_eq!(general.revenue, Money::from_cents(15000));
    assert_eq!(general.tickets_sold, 3);

    println!("✅ Sales analytics: Tracks section metrics correctly");
}

/// Test 20: Sales analytics get most popular section.
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_sales_analytics_most_popular_section() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    let projection = PostgresSalesAnalyticsProjection::new(Arc::new(pool.clone()));

    // VIP: 2 tickets
    let res1 = ReservationId::new();
    projection
        .apply_event(&create_reservation_initiated(
            res1,
            event_id,
            CustomerId::new(),
            "VIP",
            2,
        ))
        .await
        .unwrap();
    projection
        .apply_event(&create_seats_allocated(
            res1,
            2,
            Money::from_cents(20000),
        ))
        .await
        .unwrap();
    projection
        .apply_event(&create_reservation_completed(res1, 2))
        .await
        .unwrap();

    // General: 5 tickets (should be most popular)
    let res2 = ReservationId::new();
    projection
        .apply_event(&create_reservation_initiated(
            res2,
            event_id,
            CustomerId::new(),
            "General",
            5,
        ))
        .await
        .unwrap();
    projection
        .apply_event(&create_seats_allocated(
            res2,
            5,
            Money::from_cents(25000),
        ))
        .await
        .unwrap();
    projection
        .apply_event(&create_reservation_completed(res2, 5))
        .await
        .unwrap();

    // Query most popular
    let (section, tickets) = projection
        .get_most_popular_section(&event_id)
        .await
        .unwrap()
        .expect("Should have a most popular section");

    assert_eq!(section, "General", "General should be most popular");
    assert_eq!(tickets, 5, "Should have 5 tickets sold");

    println!("✅ Sales analytics: Identifies most popular section correctly");
}

// ============================================================================
// PostgresCustomerHistoryProjection Tests
// ============================================================================

/// Test 21: Customer history records purchase.
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_customer_history_records_purchase() {
    let pool = setup_test_db().await;
    let event_id = setup_test_event(&pool).await;
    let projection = PostgresCustomerHistoryProjection::new(Arc::new(pool.clone()));

    let customer_id = CustomerId::new();
    let reservation_id = ReservationId::new();

    // Complete a purchase
    projection
        .apply_event(&create_reservation_initiated(
            reservation_id,
            event_id,
            customer_id,
            "VIP",
            2,
        ))
        .await
        .unwrap();

    projection
        .apply_event(&create_seats_allocated(
            reservation_id,
            2,
            Money::from_cents(20000),
        ))
        .await
        .unwrap();

    projection
        .apply_event(&create_reservation_completed(reservation_id, 2))
        .await
        .unwrap();

    // Verify customer profile
    let profile = projection
        .get_customer_profile(&customer_id)
        .await
        .unwrap()
        .expect("Customer profile should exist");

    assert_eq!(
        profile.total_spent,
        Money::from_cents(20000),
        "Total spent should be $200"
    );
    assert_eq!(profile.total_tickets, 2, "Should have 2 tickets");
    assert_eq!(profile.purchase_count, 1, "Should have 1 purchase");

    // Verify purchase history
    let purchases = projection
        .get_customer_purchases(&customer_id)
        .await
        .unwrap();

    assert_eq!(purchases.len(), 1, "Should have 1 purchase");
    assert_eq!(purchases[0].event_id, event_id);
    assert_eq!(purchases[0].section, "VIP");
    assert_eq!(purchases[0].ticket_count, 2);

    println!("✅ Customer history: Records purchase correctly");
}

/// Test 22: Customer history tracks multiple purchases.
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_customer_history_multiple_purchases() {
    let pool = setup_test_db().await;
    let projection = PostgresCustomerHistoryProjection::new(Arc::new(pool.clone()));

    let customer_id = CustomerId::new();

    // Make 3 purchases
    for i in 0..3 {
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();

        projection
            .apply_event(&create_reservation_initiated(
                reservation_id,
                event_id,
                customer_id,
                "VIP",
                2,
            ))
            .await
            .unwrap();

        projection
            .apply_event(&create_seats_allocated(
                reservation_id,
                2,
                Money::from_cents((i + 1) * 10000),
            ))
            .await
            .unwrap();

        projection
            .apply_event(&create_reservation_completed(reservation_id, 2))
            .await
            .unwrap();
    }

    // Verify customer profile
    let profile = projection
        .get_customer_profile(&customer_id)
        .await
        .unwrap()
        .expect("Customer profile should exist");

    assert_eq!(
        profile.total_spent,
        Money::from_cents(60000),
        "Total spent should be $600 (100+200+300)"
    );
    assert_eq!(profile.total_tickets, 6, "Should have 6 tickets");
    assert_eq!(profile.purchase_count, 3, "Should have 3 purchases");

    // Verify purchase history
    let purchases = projection
        .get_customer_purchases(&customer_id)
        .await
        .unwrap();

    assert_eq!(purchases.len(), 3, "Should have 3 purchases");

    println!("✅ Customer history: Tracks multiple purchases correctly");
}

/// Test 23: Customer history tracks favorite section.
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_customer_history_favorite_section() {
    let pool = setup_test_db().await;
    let projection = PostgresCustomerHistoryProjection::new(Arc::new(pool.clone()));

    let customer_id = CustomerId::new();

    // Purchase VIP twice
    for _ in 0..2 {
        let reservation_id = ReservationId::new();
        projection
            .apply_event(&create_reservation_initiated(
                reservation_id,
                EventId::new(),
                customer_id,
                "VIP",
                1,
            ))
            .await
            .unwrap();
        projection
            .apply_event(&create_seats_allocated(
                reservation_id,
                1,
                Money::from_cents(10000),
            ))
            .await
            .unwrap();
        projection
            .apply_event(&create_reservation_completed(reservation_id, 1))
            .await
            .unwrap();
    }

    // Purchase General once
    let reservation_id = ReservationId::new();
    projection
        .apply_event(&create_reservation_initiated(
            reservation_id,
            EventId::new(),
            customer_id,
            "General",
            1,
        ))
        .await
        .unwrap();
    projection
        .apply_event(&create_seats_allocated(
            reservation_id,
            1,
            Money::from_cents(5000),
        ))
        .await
        .unwrap();
    projection
        .apply_event(&create_reservation_completed(reservation_id, 1))
        .await
        .unwrap();

    // Verify favorite section
    let profile = projection
        .get_customer_profile(&customer_id)
        .await
        .unwrap()
        .expect("Customer profile should exist");

    assert_eq!(
        profile.favorite_section,
        Some("VIP".to_string()),
        "VIP should be favorite section (2 purchases vs 1)"
    );

    println!("✅ Customer history: Tracks favorite section correctly");
}

/// Test 24: Customer history tracks event attendance.
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_customer_history_event_attendance() {
    let pool = setup_test_db().await;
    let projection = PostgresCustomerHistoryProjection::new(Arc::new(pool.clone()));

    let customer_id = CustomerId::new();
    let event1 = EventId::new();
    let event2 = EventId::new();

    // Attend event1
    let res1 = ReservationId::new();
    projection
        .apply_event(&create_reservation_initiated(
            res1,
            event1,
            customer_id,
            "VIP",
            1,
        ))
        .await
        .unwrap();
    projection
        .apply_event(&create_seats_allocated(
            res1,
            1,
            Money::from_cents(10000),
        ))
        .await
        .unwrap();
    projection
        .apply_event(&create_reservation_completed(res1, 1))
        .await
        .unwrap();

    // Check attendance
    assert!(
        projection
            .has_attended_event(&customer_id, &event1)
            .await
            .unwrap(),
        "Customer should have attended event1"
    );
    assert!(
        !projection
            .has_attended_event(&customer_id, &event2)
            .await
            .unwrap(),
        "Customer should NOT have attended event2"
    );

    println!("✅ Customer history: Tracks event attendance correctly");
}

/// Test 25: Customer history does not record cancelled reservations.
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_customer_history_cancelled_not_recorded() {
    let pool = setup_test_db().await;
    let projection = PostgresCustomerHistoryProjection::new(Arc::new(pool.clone()));

    let customer_id = CustomerId::new();
    let reservation_id = ReservationId::new();

    // Initiate reservation
    projection
        .apply_event(&create_reservation_initiated(
            reservation_id,
            EventId::new(),
            customer_id,
            "VIP",
            2,
        ))
        .await
        .unwrap();

    // Cancel reservation
    projection
        .apply_event(&create_reservation_cancelled(reservation_id))
        .await
        .unwrap();

    // Customer should have no profile (no completed purchases)
    let profile = projection
        .get_customer_profile(&customer_id)
        .await
        .unwrap();

    assert!(
        profile.is_none(),
        "Customer should have no profile after cancelled reservation"
    );

    println!("✅ Customer history: Does not record cancelled reservations");
}
