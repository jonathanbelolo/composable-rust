//! Inventory edge case tests.
//!
//! Tests reservation lifecycle, capacity management, and seat assignment edge cases.
//! These tests verify complete flows: reserve → confirm/release and complex scenarios.
//!
//! Run with: `cargo test --test inventory_edge_case_test`

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use ticketing::aggregates::inventory::{InventoryAction, InventoryReducer, InventoryEnvironment};
use ticketing::types::{
    Capacity, CustomerId, EventId, Inventory, InventoryState, ReservationId,
    SeatAssignment, SeatId, SeatStatus,
};
use chrono::{Duration, Utc};
use composable_rust_core::environment::SystemClock;
use composable_rust_core::reducer::Reducer;
use composable_rust_core::stream::StreamId;
use composable_rust_testing::mocks::{InMemoryEventBus, InMemoryEventStore};
use std::sync::Arc;

// Mock projection query for tests
#[derive(Clone)]
struct MockInventoryQuery;

impl ticketing::aggregates::inventory::InventoryProjectionQuery for MockInventoryQuery {
    fn load_inventory(
        &self,
        _event_id: &EventId,
        _section: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<((u32, u32, u32, u32), Vec<SeatAssignment>)>, String>> + Send + '_>> {
        // Return None for tests - state will be built from events
        Box::pin(async move { Ok(None) })
    }
}

fn create_test_env() -> InventoryEnvironment {
    InventoryEnvironment::new(
        Arc::new(SystemClock),
        Arc::new(InMemoryEventStore::new()),
        Arc::new(InMemoryEventBus::new()),
        StreamId::new("inventory-test"),
        Arc::new(MockInventoryQuery),
    )
}

/// Test 1: Complete Reservation → Confirmation Flow
///
/// Verifies the full lifecycle: reserve seats → confirm → seats marked as sold.
#[tokio::test]
async fn test_reservation_to_confirmation_flow() {
    println!("🧪 Test 1: Complete Reservation → Confirmation Flow");

    let event_id = EventId::new();
    let section = "VIP".to_string();
    let reservation_id = ReservationId::new();
    let customer_id = CustomerId::new();

    let reducer = InventoryReducer::new();
    let env = create_test_env();

    // Step 1: Create initial inventory with seat assignments
    let seat_ids: Vec<SeatId> = (0..5).map(|_| SeatId::new()).collect();
    let mut state = InventoryState::new();

    let inventory = Inventory::new(event_id, section.clone(), Capacity::new(5));
    state.inventories.insert((event_id, section.clone()), inventory);

    for seat_id in &seat_ids {
        let assignment = SeatAssignment::new(
            *seat_id,
            event_id,
            section.clone(),
            None,
        );
        state.seat_assignments.insert(*seat_id, assignment);
    }
    state.mark_loaded(event_id, section.clone());

    // Verify initial state
    assert_eq!(state.get_inventory(&event_id, &section).unwrap().available(), 5);
    assert_eq!(state.get_inventory(&event_id, &section).unwrap().reserved, 0);
    assert_eq!(state.get_inventory(&event_id, &section).unwrap().sold, 0);

    // Step 2: Reserve 2 seats
    let _effects = reducer.reduce(
        &mut state,
        InventoryAction::ReserveSeats {
            reservation_id,
            event_id,
            section: section.clone(),
            quantity: 2,
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(15),
        },
        &env,
    );

    // Verify reserved state
    let inventory = state.get_inventory(&event_id, &section).unwrap();
    assert_eq!(inventory.reserved, 2);
    assert_eq!(inventory.sold, 0);
    assert_eq!(inventory.available(), 3); // 5 total - 2 reserved

    // Step 3: Confirm reservation
    let _confirm_effects = reducer.reduce(
        &mut state,
        InventoryAction::ConfirmReservation {
            reservation_id,
            customer_id,
        },
        &env,
    );

    // Verify confirmed state
    let inventory = state.get_inventory(&event_id, &section).unwrap();
    assert_eq!(inventory.reserved, 0); // Moved from reserved to sold
    assert_eq!(inventory.sold, 2);
    assert_eq!(inventory.available(), 3); // 5 total - 2 sold

    // Verify seat statuses
    let sold_seats: Vec<_> = state.seat_assignments.values()
        .filter(|s| s.status == SeatStatus::Sold)
        .collect();
    assert_eq!(sold_seats.len(), 2);
    assert!(sold_seats.iter().all(|s| s.sold_to == Some(customer_id)));

    println!("  ✅ Reservation → Confirmation flow completed successfully");
}

/// Test 2: Complete Reservation → Release Flow
///
/// Verifies: reserve seats → release → seats back to available.
#[tokio::test]
async fn test_reservation_to_release_flow() {
    println!("🧪 Test 2: Complete Reservation → Release Flow");

    let event_id = EventId::new();
    let section = "General".to_string();
    let reservation_id = ReservationId::new();

    let reducer = InventoryReducer::new();
    let env = create_test_env();

    // Create initial inventory
    let seat_ids: Vec<SeatId> = (0..10).map(|_| SeatId::new()).collect();
    let mut state = InventoryState::new();

    let inventory = Inventory::new(event_id, section.clone(), Capacity::new(10));
    state.inventories.insert((event_id, section.clone()), inventory);

    for seat_id in &seat_ids {
        let assignment = SeatAssignment::new(
            *seat_id,
            event_id,
            section.clone(),
            None,
        );
        state.seat_assignments.insert(*seat_id, assignment);
    }
    state.mark_loaded(event_id, section.clone());

    // Step 1: Reserve 3 seats
    let _effects = reducer.reduce(
        &mut state,
        InventoryAction::ReserveSeats {
            reservation_id,
            event_id,
            section: section.clone(),
            quantity: 3,
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(15),
        },
        &env,
    );

    assert_eq!(state.get_inventory(&event_id, &section).unwrap().reserved, 3);
    assert_eq!(state.get_inventory(&event_id, &section).unwrap().available(), 7);

    // Step 2: Release reservation
    let _release_effects = reducer.reduce(
        &mut state,
        InventoryAction::ReleaseReservation { reservation_id },
        &env,
    );

    // Verify seats are back to available
    let inventory = state.get_inventory(&event_id, &section).unwrap();
    assert_eq!(inventory.reserved, 0);
    assert_eq!(inventory.sold, 0);
    assert_eq!(inventory.available(), 10); // All seats available again

    // Verify seat statuses
    let available_seats: Vec<_> = state.seat_assignments.values()
        .filter(|s| s.status == SeatStatus::Available)
        .collect();
    assert_eq!(available_seats.len(), 10);

    println!("  ✅ Reservation → Release flow completed successfully");
}

/// Test 3: Multiple Concurrent Reservations
///
/// Verifies that multiple reservations correctly decrease available capacity.
#[tokio::test]
async fn test_multiple_concurrent_reservations() {
    println!("🧪 Test 3: Multiple Concurrent Reservations");

    let event_id = EventId::new();
    let section = "VIP".to_string();
    let reservation_1 = ReservationId::new();
    let reservation_2 = ReservationId::new();
    let reservation_3 = ReservationId::new();

    let reducer = InventoryReducer::new();
    let env = create_test_env();

    // Create initial inventory with 10 seats
    let seat_ids: Vec<SeatId> = (0..10).map(|_| SeatId::new()).collect();
    let mut state = InventoryState::new();

    let inventory = Inventory::new(event_id, section.clone(), Capacity::new(10));
    state.inventories.insert((event_id, section.clone()), inventory);

    for seat_id in &seat_ids {
        let assignment = SeatAssignment::new(
            *seat_id,
            event_id,
            section.clone(),
            None,
        );
        state.seat_assignments.insert(*seat_id, assignment);
    }
    state.mark_loaded(event_id, section.clone());

    // Reserve 3 seats (reservation 1)
    let _effects1 = reducer.reduce(
        &mut state,
        InventoryAction::ReserveSeats {
            reservation_id: reservation_1,
            event_id,
            section: section.clone(),
            quantity: 3,
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(15),
        },
        &env,
    );

    assert_eq!(state.get_inventory(&event_id, &section).unwrap().reserved, 3);
    assert_eq!(state.get_inventory(&event_id, &section).unwrap().available(), 7);

    // Reserve 4 seats (reservation 2)
    let _effects2 = reducer.reduce(
        &mut state,
        InventoryAction::ReserveSeats {
            reservation_id: reservation_2,
            event_id,
            section: section.clone(),
            quantity: 4,
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(15),
        },
        &env,
    );

    assert_eq!(state.get_inventory(&event_id, &section).unwrap().reserved, 7);
    assert_eq!(state.get_inventory(&event_id, &section).unwrap().available(), 3);

    // Reserve 2 seats (reservation 3)
    let _effects3 = reducer.reduce(
        &mut state,
        InventoryAction::ReserveSeats {
            reservation_id: reservation_3,
            event_id,
            section: section.clone(),
            quantity: 2,
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(15),
        },
        &env,
    );

    // Final state: 9 reserved, 1 available
    let inventory = state.get_inventory(&event_id, &section).unwrap();
    assert_eq!(inventory.reserved, 9);
    assert_eq!(inventory.available(), 1);

    println!("  ✅ Multiple concurrent reservations handled correctly");
}

/// Test 4: Last Seat Scenario
///
/// Verifies that reserving exactly the remaining capacity works.
#[tokio::test]
async fn test_last_seat_reservation() {
    println!("🧪 Test 4: Last Seat Scenario");

    let event_id = EventId::new();
    let section = "VIP".to_string();
    let reservation_id = ReservationId::new();

    let reducer = InventoryReducer::new();
    let env = create_test_env();

    // Create inventory with 1 seat
    let seat_id = SeatId::new();
    let mut state = InventoryState::new();

    let inventory = Inventory::new(event_id, section.clone(), Capacity::new(1));
    state.inventories.insert((event_id, section.clone()), inventory);

    let assignment = SeatAssignment::new(seat_id, event_id, section.clone(), None);
    state.seat_assignments.insert(seat_id, assignment);
    state.mark_loaded(event_id, section.clone());

    // Reserve the last seat
    let _effects = reducer.reduce(
        &mut state,
        InventoryAction::ReserveSeats {
            reservation_id,
            event_id,
            section: section.clone(),
            quantity: 1,
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(15),
        },
        &env,
    );

    // Verify: 1 reserved, 0 available
    let inventory = state.get_inventory(&event_id, &section).unwrap();
    assert_eq!(inventory.reserved, 1);
    assert_eq!(inventory.available(), 0);

    // Verify we can't reserve any more
    let second_reservation = ReservationId::new();
    reducer.reduce(
        &mut state,
        InventoryAction::ReserveSeats {
            reservation_id: second_reservation,
            event_id,
            section: section.clone(),
            quantity: 1,
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(15),
        },
        &env,
    );

    // Should have validation error
    assert!(state.last_error.is_some());
    assert!(state.last_error.as_ref().unwrap().contains("Insufficient inventory"));

    println!("  ✅ Last seat scenario handled correctly");
}

/// Test 5: Sequential Operations (Reserve → Confirm → Reserve More)
///
/// Verifies that after confirming one reservation, we can make another.
#[tokio::test]
async fn test_sequential_reserve_confirm_reserve() {
    println!("🧪 Test 5: Sequential Reserve → Confirm → Reserve More");

    let event_id = EventId::new();
    let section = "General".to_string();
    let reservation_1 = ReservationId::new();
    let reservation_2 = ReservationId::new();
    let customer_1 = CustomerId::new();

    let reducer = InventoryReducer::new();
    let env = create_test_env();

    // Create inventory with 10 seats
    let seat_ids: Vec<SeatId> = (0..10).map(|_| SeatId::new()).collect();
    let mut state = InventoryState::new();

    let inventory = Inventory::new(event_id, section.clone(), Capacity::new(10));
    state.inventories.insert((event_id, section.clone()), inventory);

    for seat_id in &seat_ids {
        let assignment = SeatAssignment::new(
            *seat_id,
            event_id,
            section.clone(),
            None,
        );
        state.seat_assignments.insert(*seat_id, assignment);
    }
    state.mark_loaded(event_id, section.clone());

    // Step 1: Reserve 4 seats
    reducer.reduce(
        &mut state,
        InventoryAction::ReserveSeats {
            reservation_id: reservation_1,
            event_id,
            section: section.clone(),
            quantity: 4,
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(15),
        },
        &env,
    );

    assert_eq!(state.get_inventory(&event_id, &section).unwrap().reserved, 4);
    assert_eq!(state.get_inventory(&event_id, &section).unwrap().available(), 6);

    // Step 2: Confirm first reservation
    reducer.reduce(
        &mut state,
        InventoryAction::ConfirmReservation {
            reservation_id: reservation_1,
            customer_id: customer_1,
        },
        &env,
    );

    assert_eq!(state.get_inventory(&event_id, &section).unwrap().reserved, 0);
    assert_eq!(state.get_inventory(&event_id, &section).unwrap().sold, 4);
    assert_eq!(state.get_inventory(&event_id, &section).unwrap().available(), 6);

    // Step 3: Reserve 5 more seats (second reservation)
    reducer.reduce(
        &mut state,
        InventoryAction::ReserveSeats {
            reservation_id: reservation_2,
            event_id,
            section: section.clone(),
            quantity: 5,
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(15),
        },
        &env,
    );

    // Final state: 4 sold, 5 reserved, 1 available
    let inventory = state.get_inventory(&event_id, &section).unwrap();
    assert_eq!(inventory.sold, 4);
    assert_eq!(inventory.reserved, 5);
    assert_eq!(inventory.available(), 1);

    println!("  ✅ Sequential reserve → confirm → reserve flow works correctly");
}

/// Test 6: Release Returns Seats to Pool
///
/// Verifies that releasing seats makes them available for new reservations.
#[tokio::test]
async fn test_release_returns_seats_to_pool() {
    println!("🧪 Test 6: Release Returns Seats to Pool");

    let event_id = EventId::new();
    let section = "VIP".to_string();
    let reservation_1 = ReservationId::new();
    let reservation_2 = ReservationId::new();

    let reducer = InventoryReducer::new();
    let env = create_test_env();

    // Create inventory with 5 seats
    let seat_ids: Vec<SeatId> = (0..5).map(|_| SeatId::new()).collect();
    let mut state = InventoryState::new();

    let inventory = Inventory::new(event_id, section.clone(), Capacity::new(5));
    state.inventories.insert((event_id, section.clone()), inventory);

    for seat_id in &seat_ids {
        let assignment = SeatAssignment::new(
            *seat_id,
            event_id,
            section.clone(),
            None,
        );
        state.seat_assignments.insert(*seat_id, assignment);
    }
    state.mark_loaded(event_id, section.clone());

    // Step 1: Reserve all 5 seats
    reducer.reduce(
        &mut state,
        InventoryAction::ReserveSeats {
            reservation_id: reservation_1,
            event_id,
            section: section.clone(),
            quantity: 5,
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(15),
        },
        &env,
    );

    assert_eq!(state.get_inventory(&event_id, &section).unwrap().available(), 0);

    // Step 2: Try to reserve (should fail - no capacity)
    reducer.reduce(
        &mut state,
        InventoryAction::ReserveSeats {
            reservation_id: reservation_2,
            event_id,
            section: section.clone(),
            quantity: 1,
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(15),
        },
        &env,
    );

    assert!(state.last_error.is_some());
    assert!(state.last_error.as_ref().unwrap().contains("Insufficient inventory"));

    // Step 3: Release first reservation
    reducer.reduce(
        &mut state,
        InventoryAction::ReleaseReservation { reservation_id: reservation_1 },
        &env,
    );

    assert_eq!(state.get_inventory(&event_id, &section).unwrap().available(), 5);

    // Step 4: Now we CAN reserve (seats are back)
    state.last_error = None; // Clear previous error
    reducer.reduce(
        &mut state,
        InventoryAction::ReserveSeats {
            reservation_id: reservation_2,
            event_id,
            section: section.clone(),
            quantity: 3,
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(15),
        },
        &env,
    );

    // Verify: 3 reserved, 2 available, no error
    let inventory = state.get_inventory(&event_id, &section).unwrap();
    assert_eq!(inventory.reserved, 3);
    assert_eq!(inventory.available(), 2);
    assert!(state.last_error.is_none());

    println!("  ✅ Release correctly returns seats to available pool");
}
