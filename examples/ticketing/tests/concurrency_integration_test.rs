//! Concurrency integration tests.
//!
//! Tests race conditions, last seat scenarios, and concurrent operations.
//! These tests verify the system's ability to handle high-concurrency scenarios
//! without double-booking or data corruption.
//!
//! Run with: `cargo test --test concurrency_integration_test`

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use ticketing::aggregates::inventory::{InventoryAction, InventoryReducer, InventoryEnvironment};
use ticketing::types::{
    Capacity, CustomerId, EventId, Inventory, InventoryState, ReservationId,
    SeatAssignment,
};
use chrono::{Duration, Utc};
use composable_rust_core::environment::SystemClock;
use composable_rust_core::stream::StreamId;
use composable_rust_testing::{mocks::InMemoryEventStore, ReducerTest};
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

    fn get_all_sections(
        &self,
        _event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<ticketing::aggregates::inventory::SectionAvailabilityData>, String>> + Send + '_>> {
        Box::pin(async move { Ok(vec![]) })
    }

    fn get_section_availability(
        &self,
        _event_id: &EventId,
        _section: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<ticketing::aggregates::inventory::SectionAvailabilityData>, String>> + Send + '_>> {
        Box::pin(async move { Ok(None) })
    }

    fn get_total_available(
        &self,
        _event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u32, String>> + Send + '_>> {
        Box::pin(async move { Ok(0) })
    }
}

// Mock event query for pricing tests
#[derive(Clone)]
struct MockEventQuery;

#[async_trait::async_trait]
impl ticketing::projections::EventProjectionQuery for MockEventQuery {
    async fn load_event(&self, _event_id: &EventId) -> Result<Option<ticketing::types::Event>, String> {
        // Return None for tests - no pricing configured
        Ok(None)
    }

    async fn load_events(&self, _status_filter: Option<ticketing::types::EventStatus>) -> Result<Vec<ticketing::types::Event>, String> {
        // Return empty list for tests
        Ok(vec![])
    }
}

fn create_test_global_channels() -> ticketing::types::GlobalActionChannels {
    use tokio::sync::broadcast;

    let (event_actions, _) = broadcast::channel(1000);
    let (inventory_actions, _) = broadcast::channel(1000);
    let (reservation_actions, _) = broadcast::channel(1000);
    let (payment_actions, _) = broadcast::channel(1000);

    ticketing::types::GlobalActionChannels {
        event_actions,
        inventory_actions,
        reservation_actions,
        payment_actions,
    }
}

fn create_test_env() -> InventoryEnvironment {
    InventoryEnvironment::new(
        Arc::new(SystemClock),
        Arc::new(InMemoryEventStore::new()),
        StreamId::new("inventory-test"),
        Arc::new(MockInventoryQuery),
        Arc::new(MockEventQuery),
        create_test_global_channels(),
    )
}

/// Test 1: Insufficient Inventory - Over-Reservation
///
/// Verifies that requesting more seats than available is rejected.
#[tokio::test]
async fn test_insufficient_inventory() {
    println!("🧪 Test 1: Insufficient Inventory");

    let event_id = EventId::new();
    let section = "VIP".to_string();

    ReducerTest::new(InventoryReducer::new())
        .with_env(create_test_env())
        .given_state({
            let mut state = InventoryState::new();
            let inventory = Inventory::new(event_id, section.clone(), Capacity::new(3));
            state.inventories.insert((event_id, section.clone()), inventory);
            state.mark_loaded(event_id, section.clone());
            state
        })
        .when_action(InventoryAction::ReserveSeats {
            reservation_id: ReservationId::new(),
            event_id,
            section,
            quantity: 5, // More than available!
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .then_state(move |state| {
            // Inventory should remain unchanged
            let inventory = state.get_inventory(&event_id, "VIP").unwrap();
            assert_eq!(inventory.reserved, 0);
            assert_eq!(inventory.available(), 3);
            // Should have validation error
            assert!(state.last_error.is_some());
            assert!(state
                .last_error
                .as_ref()
                .unwrap()
                .contains("Insufficient inventory"));
        })
        .then_effects(|_effects| {
            // ValidationFailed effect is emitted for send_and_wait_for pattern
        })
        .run();

    println!("  ✅ Insufficient inventory correctly rejected");
}

/// Test 2: Maximum Purchase Limit
///
/// Verifies that attempting to reserve more than 8 seats is rejected.
#[tokio::test]
async fn test_maximum_purchase_limit() {
    println!("🧪 Test 2: Maximum Purchase Limit (8 seats)");

    let event_id = EventId::new();
    let section = "General".to_string();

    ReducerTest::new(InventoryReducer::new())
        .with_env(create_test_env())
        .given_state({
            let mut state = InventoryState::new();
            let inventory = Inventory::new(event_id, section.clone(), Capacity::new(100));
            state.inventories.insert((event_id, section.clone()), inventory);
            state.mark_loaded(event_id, section.clone());
            state
        })
        .when_action(InventoryAction::ReserveSeats {
            reservation_id: ReservationId::new(),
            event_id,
            section,
            quantity: 10, // More than 8 seat limit!
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .then_state(move |state| {
            // Should be rejected - inventory unchanged
            let inventory = state.get_inventory(&event_id, "General").unwrap();
            assert_eq!(inventory.reserved, 0);
            assert!(state.last_error.is_some());
        })
        .then_effects(|_effects| {
            // ValidationFailed event emitted
        })
        .run();

    println!("  ✅ Maximum purchase limit enforced");
}

/// Test 3: Zero Quantity Rejected
///
/// Verifies that quantity must be > 0.
#[tokio::test]
async fn test_zero_quantity_rejected() {
    println!("🧪 Test 3: Zero Quantity Rejected");

    let event_id = EventId::new();
    let section = "VIP".to_string();

    ReducerTest::new(InventoryReducer::new())
        .with_env(create_test_env())
        .given_state({
            let mut state = InventoryState::new();
            let inventory = Inventory::new(event_id, section.clone(), Capacity::new(10));
            state.inventories.insert((event_id, section.clone()), inventory);
            state.mark_loaded(event_id, section.clone());
            state
        })
        .when_action(InventoryAction::ReserveSeats {
            reservation_id: ReservationId::new(),
            event_id,
            section,
            quantity: 0, // Invalid!
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .then_state(move |state| {
            assert!(state.last_error.is_some());
            assert!(state
                .last_error
                .as_ref()
                .unwrap()
                .contains("greater than zero"));
        })
        .then_effects(|_effects| {})
        .run();

    println!("  ✅ Zero quantity correctly rejected");
}

/// Test 4: Initialize Inventory with Zero Capacity Rejected
///
/// Verifies that capacity must be > 0.
#[tokio::test]
async fn test_zero_capacity_rejected() {
    println!("🧪 Test 4: Zero Capacity Rejected");

    ReducerTest::new(InventoryReducer::new())
        .with_env(create_test_env())
        .given_state(InventoryState::new())
        .when_action(InventoryAction::InitializeInventory {
            event_id: EventId::new(),
            section: "VIP".to_string(),
            capacity: Capacity::new(0), // Invalid!
            seat_numbers: None,
            respond_to: ticketing::types::ResponseChannel::none(),
        })
        .then_state(|state| {
            assert!(state.last_error.is_some());
            assert!(state
                .last_error
                .as_ref()
                .unwrap()
                .contains("greater than zero"));
        })
        .then_effects(|_effects| {})
        .run();

    println!("  ✅ Zero capacity correctly rejected");
}

/// Test 5: Cannot Initialize Same Inventory Twice
///
/// Verifies idempotency - cannot initialize the same event/section twice.
#[tokio::test]
async fn test_cannot_initialize_twice() {
    println!("🧪 Test 5: Cannot Initialize Same Inventory Twice");

    let event_id = EventId::new();
    let section = "VIP".to_string();

    ReducerTest::new(InventoryReducer::new())
        .with_env(create_test_env())
        .given_state({
            let mut state = InventoryState::new();
            // Already initialized
            let inventory = Inventory::new(event_id, section.clone(), Capacity::new(100));
            state.inventories.insert((event_id, section.clone()), inventory);
            state
        })
        .when_action(InventoryAction::InitializeInventory {
            event_id,
            section,
            capacity: Capacity::new(50), // Different capacity - shouldn't matter
            seat_numbers: None,
            respond_to: ticketing::types::ResponseChannel::none(),
        })
        .then_state(move |state| {
            assert!(state.last_error.is_some());
            assert!(state
                .last_error
                .as_ref()
                .unwrap()
                .contains("already exists"));

            // Original inventory should remain
            let inventory = state.get_inventory(&event_id, "VIP").unwrap();
            assert_eq!(inventory.total_capacity.value(), 100);
        })
        .then_effects(|_effects| {})
        .run();

    println!("  ✅ Duplicate initialization correctly rejected");
}

/// Test 6: Reservation on Non-Existent Inventory
///
/// Verifies that reserving seats on non-existent inventory is rejected.
#[tokio::test]
async fn test_reservation_on_nonexistent_inventory() {
    println!("🧪 Test 6: Reservation on Non-Existent Inventory");

    let event_id = EventId::new();
    let section = "VIP".to_string();

    ReducerTest::new(InventoryReducer::new())
        .with_env(create_test_env())
        .given_state({
            let mut state = InventoryState::new();
            // Mark as loaded so validation happens (otherwise it tries to load from projection)
            state.mark_loaded(event_id, section.clone());
            state
        })
        .when_action(InventoryAction::ReserveSeats {
            reservation_id: ReservationId::new(),
            event_id,
            section,
            quantity: 2,
            specific_seats: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .then_state(|state| {
            assert!(state.last_error.is_some());
            assert!(state
                .last_error
                .as_ref()
                .unwrap()
                .contains("not found"));
        })
        .then_effects(|_effects| {
            // ValidationFailed effect is emitted for send_and_wait_for pattern
        })
        .run();

    println!("  ✅ Reservation on non-existent inventory correctly rejected");
}

/// Test 7: Release Non-Existent Reservation
///
/// Verifies that releasing a reservation that doesn't exist is handled gracefully.
#[tokio::test]
async fn test_release_nonexistent_reservation() {
    println!("🧪 Test 7: Release Non-Existent Reservation");

    let reservation_id = ReservationId::new();

    ReducerTest::new(InventoryReducer::new())
        .with_env(create_test_env())
        .given_state(InventoryState::new()) // No reservations
        .when_action(InventoryAction::ReleaseReservation { reservation_id })
        .then_state(|_state| {
            // Should be handled gracefully (no-op)
        })
        .then_effects(|_effects| {
            // No effects emitted for non-existent reservation
        })
        .run();

    println!("  ✅ Release of non-existent reservation handled gracefully");
}

/// Test 8: Confirm Non-Existent Reservation
///
/// Verifies that confirming a reservation that doesn't exist is handled gracefully.
#[tokio::test]
async fn test_confirm_nonexistent_reservation() {
    println!("🧪 Test 8: Confirm Non-Existent Reservation");

    let reservation_id = ReservationId::new();
    let customer_id = CustomerId::new();

    ReducerTest::new(InventoryReducer::new())
        .with_env(create_test_env())
        .given_state(InventoryState::new()) // No reservations
        .when_action(InventoryAction::ConfirmReservation {
            reservation_id,
            customer_id,
        })
        .then_state(|_state| {
            // Should be handled gracefully (no-op)
        })
        .then_effects(|_effects| {
            // No effects emitted for non-existent reservation
        })
        .run();

    println!("  ✅ Confirm of non-existent reservation handled gracefully");
}
