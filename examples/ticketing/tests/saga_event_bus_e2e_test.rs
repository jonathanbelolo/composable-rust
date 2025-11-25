//! End-to-End Saga Event Bus Integration Tests
//!
//! These tests verify that the complete saga choreography works correctly
//! with real event bus routing between aggregates.
//!
//! Unlike `saga_integration_test.rs` which uses `ReducerTest` (unit-level testing),
//! these tests verify the actual runtime behavior with event bus subscriptions,
//! simulating the production `main.rs` setup.
//!
//! Test Coverage:
//! - EventInventorySaga creates events with automatic inventory initialization
//! - Event bus subscriptions for child aggregates
//! - Saga command publishing from parent to children
//! - Complete happy path: Reservation → Inventory → Payment → Completion
//! - Compensation flow: Payment failure → seat release
//! - Timeout handling: Expired reservations release seats

#![allow(clippy::expect_used, clippy::unwrap_used)] // Test code can use unwrap/expect

use composable_rust_core::{
    environment::{Clock, SystemClock},
    event_store::EventStore,
    stream::StreamId,
};
use composable_rust_runtime::Store;
use composable_rust_testing::mocks::InMemoryEventStore;
use std::sync::Arc;
use std::time::Duration;
use ticketing::{
    aggregates::{
        event::{EventEnvironment, EventReducer},
        event_inventory_saga::{EventInventorySaga, EventInventorySagaAction, EventInventorySagaEnvironment, EventInventorySagaState},
        inventory::{InventoryEnvironment, InventoryReducer, InventoryProjectionQuery},
        payment::{PaymentEnvironment, PaymentReducer, PaymentProjectionQuery},
        reservation::{ReservationAction, ReservationEnvironment, ReservationReducer, ReservationProjectionQuery},
        InventoryAction, PaymentAction,
    },
    types::{
        Capacity, CustomerId, EventDate, EventId, EventState, InventoryState, Money, Payment, PaymentId, PaymentState,
        PricingTier, Reservation, ReservationId, ReservationState, ReservationStatus, TierType, Venue, VenueSection, SeatType,
    },
};
use composable_rust_auth::state::UserId;
use chrono::Utc;

// ============================================================================
// Test Helpers
// ============================================================================

/// Helper to create test global actions channels
fn create_test_global_actions() -> ticketing::types::GlobalActionChannels {
    use tokio::sync::broadcast;
    let (event_tx, _) = broadcast::channel(100);
    let (inventory_tx, _) = broadcast::channel(100);
    let (reservation_tx, _) = broadcast::channel(100);
    let (payment_tx, _) = broadcast::channel(100);
    ticketing::types::GlobalActionChannels {
        event_actions: event_tx,
        inventory_actions: inventory_tx,
        reservation_actions: reservation_tx,
        payment_actions: payment_tx,
    }
}

// ============================================================================
// Mock Projection Queries for Testing
// ============================================================================

/// Mock event query that returns None (forcing event sourcing fallback)
#[derive(Clone)]
struct MockEventQuery;

#[async_trait::async_trait]
impl ticketing::aggregates::event::EventProjectionQuery for MockEventQuery {
    async fn load_event(&self, _event_id: &EventId) -> Result<Option<ticketing::types::Event>, String> {
        Ok(None) // No cached state, use event sourcing
    }

    async fn load_events(&self, _status_filter: Option<ticketing::types::EventStatus>) -> Result<Vec<ticketing::types::Event>, String> {
        Ok(Vec::new())
    }
}

/// Mock inventory query that returns None (forcing event sourcing fallback)
#[derive(Clone)]
struct MockInventoryQuery;

impl InventoryProjectionQuery for MockInventoryQuery {
    fn load_inventory(
        &self,
        _event_id: &EventId,
        _section: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<((u32, u32, u32, u32), Vec<ticketing::SeatAssignment>)>, String>> + Send + '_>> {
        Box::pin(async move { Ok(None) }) // No cached state, use event sourcing
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

/// Mock payment query that returns None (forcing event sourcing fallback)
#[derive(Clone)]
struct MockPaymentQuery;

#[async_trait::async_trait]
impl PaymentProjectionQuery for MockPaymentQuery {
    async fn load_payment(
        &self,
        _payment_id: &PaymentId,
    ) -> Result<Option<Payment>, String> {
        Ok(None) // No cached state, use event sourcing
    }

    async fn load_customer_payments(&self, _customer_id: &CustomerId, _limit: usize, _offset: usize) -> Result<Vec<Payment>, String> {
        Ok(Vec::new())
    }
}

/// Mock reservation query that returns None (forcing event sourcing fallback)
#[derive(Clone)]
struct MockReservationQuery;

impl ReservationProjectionQuery for MockReservationQuery {
    fn load_reservation(
        &self,
        _reservation_id: &ReservationId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Reservation>, String>> + Send + '_>> {
        Box::pin(async move { Ok(None) }) // No cached state, use event sourcing
    }

    fn list_by_customer(
        &self,
        _customer_id: &CustomerId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Reservation>, String>> + Send + '_>> {
        Box::pin(async move { Ok(Vec::new()) }) // No cached state, use event sourcing
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Creates Event + Inventory using EventInventorySaga
///
/// This helper replaces manual inventory initialization with the production
/// saga pattern. It sets up Event aggregate, EventInventorySaga, and all
/// necessary event bus subscriptions to create an event with initialized inventory.
///
/// **Requires**: Inventory aggregate must already be set up with consumer subscribed to "inventory" topic.
///
/// Returns the event_id once saga completes successfully.
async fn create_event_with_inventory(
    event_store: Arc<dyn EventStore>,
    clock: Arc<dyn Clock>,
    event_name: String,
    venue: Venue,
    pricing_tiers: Vec<PricingTier>,
    _inventory_store: Arc<Store<InventoryState, InventoryAction, InventoryEnvironment, InventoryReducer>>,
    global_channels: ticketing::types::GlobalActionChannels,
) -> Result<EventId, Box<dyn std::error::Error>> {
    // Initialize tracing for debugging
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();

    let event_id = EventId::new();
    let owner_id = UserId::new();

    // Set up Event aggregate
    let event_env = EventEnvironment::new(
        clock.clone(),
        event_store.clone(),
        StreamId::new(&format!("event-{}", event_id.as_uuid())),
        Arc::new(MockEventQuery),
        global_channels.clone(),
    );
    let _event_store_agg = Arc::new(Store::new(
        EventState::new(),
        EventReducer::new(),
        event_env,
    ));

    // Set up EventInventorySaga with factory functions
    let event_store_for_factory = event_store.clone();
    let clock_for_factory = clock.clone();
    let global_channels_for_event_factory = global_channels.clone();
    let create_event_store = Arc::new(move |event_id: EventId| {
        let event_env = EventEnvironment::new(
            clock_for_factory.clone(),
            event_store_for_factory.clone(),
            StreamId::new(&format!("event-{}", event_id.as_uuid())),
            Arc::new(MockEventQuery),
            global_channels_for_event_factory.clone(),
        );
        Store::new(EventState::new(), EventReducer::new(), event_env)
    });

    let inventory_store_for_factory = event_store.clone();
    let inventory_clock_for_factory = clock.clone();
    let global_channels_for_inventory_factory = global_channels.clone();
    let create_inventory_store = Arc::new(move |event_id: EventId| {
        let inventory_env = InventoryEnvironment::new(
            inventory_clock_for_factory.clone(),
            inventory_store_for_factory.clone(),
            StreamId::new(&format!("inventory-{}-section", event_id.as_uuid())),
            Arc::new(MockInventoryQuery),
            Arc::new(MockEventQuery),
            global_channels_for_inventory_factory.clone(),
        );
        Store::new(InventoryState::new(), InventoryReducer::new(), inventory_env)
    });

    let saga_env = EventInventorySagaEnvironment::new(
        clock.clone(),
        event_store.clone(),
        StreamId::new(&format!("event-inventory-saga-{}", event_id.as_uuid())),
        create_event_store,
        create_inventory_store,
    );
    let saga_store = Arc::new(Store::new(
        EventInventorySagaState::new(),
        EventInventorySaga::new(),
        saga_env,
    ));

    // Direct orchestration - consumers are now spawned via spawn_channel_consumers()

    // NOTE: Saga consumer spawning removed in Phase 5 (orchestration pattern)
    // TODO: Phase 10 - Update this test for orchestration or remove if no longer relevant
    // ticketing::aggregates::event_inventory_saga::spawn_event_inventory_saga_consumers(
    //     event_bus.clone(),
    //     saga_store.clone(),
    // );

    // Give consumers time to subscribe
    // NOTE: No longer needed with orchestration pattern
    // tokio::time::sleep(Duration::from_millis(200)).await;

    // Send CreateEventWithInventory and wait for completion
    let result = saga_store
        .send_and_wait_for(
            EventInventorySagaAction::CreateEventWithInventory {
                event_id,
                name: event_name,
                owner_id,
                venue,
                date: EventDate::new(Utc::now() + chrono::Duration::days(30)),
                pricing_tiers,
            },
            |action| {
                matches!(
                    action,
                    EventInventorySagaAction::EventCreationCompleted { .. }
                        | EventInventorySagaAction::EventCreationFailed { .. }
                )
            },
            Duration::from_secs(5),
        )
        .await?;

    match result {
        EventInventorySagaAction::EventCreationCompleted { event_id, .. } => Ok(event_id),
        EventInventorySagaAction::EventCreationFailed { error, .. } => {
            Err(format!("Event creation failed: {error}").into())
        }
        _ => Err("Unexpected saga result".into()),
    }
}

/// Spawn background consumers for child aggregates (mimics main.rs setup).
///
/// This is the key piece that enables saga choreography:
/// - Inventory aggregate subscribes to "inventory" topic
/// - Payment aggregate subscribes to "payment" topic
/// - Parent Reservation aggregate publishes commands to these topics
/// Spawn background consumers for global action channels (pure direct orchestration).
///
/// Clean architecture - all inter-aggregate communication via typed channels:
/// - Reservation sends commands → inventory_actions and payment_actions
/// - Inventory/Payment send responses → reservation_actions
/// - Each aggregate subscribes to its own channel
fn spawn_channel_consumers(
    global_channels: ticketing::types::GlobalActionChannels,
    inventory: Arc<Store<InventoryState, InventoryAction, InventoryEnvironment, InventoryReducer>>,
    payment: Arc<Store<PaymentState, PaymentAction, PaymentEnvironment, PaymentReducer>>,
    reservation: Arc<Store<ReservationState, ReservationAction, ReservationEnvironment, ReservationReducer>>,
) {
    // Inventory subscribes to inventory_actions (receives commands from Reservation)
    let mut inventory_rx = global_channels.inventory_actions.subscribe();
    tokio::spawn(async move {
        while let Ok(action) = inventory_rx.recv().await {
            let _ = inventory.send(action).await;
        }
    });

    // Payment subscribes to payment_actions (receives commands from Reservation)
    let mut payment_rx = global_channels.payment_actions.subscribe();
    tokio::spawn(async move {
        while let Ok(action) = payment_rx.recv().await {
            let _ = payment.send(action).await;
        }
    });

    // Reservation subscribes to reservation_actions (receives responses from Inventory/Payment)
    let mut reservation_rx = global_channels.reservation_actions.subscribe();
    tokio::spawn(async move {
        while let Ok(action) = reservation_rx.recv().await {
            let _ = reservation.send(action).await;
        }
    });
}

#[tokio::test]
#[ignore = "Requires actual projection infrastructure - MockInventoryQuery returns None"]
async fn test_e2e_saga_happy_path_with_event_bus() {
    // Setup: Create infrastructure (event store + event bus)
    let event_store = Arc::new(InMemoryEventStore::new());
    let clock = Arc::new(SystemClock) as Arc<dyn Clock>;

    // Create test data
    let customer_id = CustomerId::new();
    let reservation_id = ReservationId::new();
    // ✨ NEW: Create shared global action channels (for direct orchestration)
    let global_channels = create_test_global_actions();

    // Initialize inventory aggregate
    let inventory_env = InventoryEnvironment::new(
        clock.clone(),
        event_store.clone(),
        StreamId::new("inventory"),
        Arc::new(MockInventoryQuery),
        Arc::new(MockEventQuery),
        global_channels.clone(),
    );
    let inventory = Arc::new(Store::new(
        InventoryState::new(),
        InventoryReducer::new(),
        inventory_env,
    ));

    // Initialize payment aggregate
    let payment_env = PaymentEnvironment::new(
        clock.clone(),
        event_store.clone(),
        StreamId::new("payment"),
        Arc::new(MockPaymentQuery),
        global_channels.clone(),
    );
    let payment = Arc::new(Store::new(
        PaymentState::new(),
        PaymentReducer::new(),
        payment_env,
    ));

    // Initialize reservation aggregate (saga coordinator)
    let reservation_env = ReservationEnvironment::new(
        clock.clone(),
        event_store.clone(),
        StreamId::new("reservation"),
        Arc::new(MockReservationQuery),
        global_channels.clone(),
    );
    let reservation = Arc::new(Store::new(
        ReservationState::new(),
        ReservationReducer::new(),
        reservation_env,
    ));

    // Step 1: Create Event + Inventory using EventInventorySaga
    let venue = Venue::new(
        "Test Venue".to_string(),
        Capacity::new(100),
        vec![VenueSection::new(
            "General".to_string(),
            Capacity::new(100),
            SeatType::GeneralAdmission,
        )],
    );
    let pricing_tiers = vec![PricingTier::new(
        TierType::Regular,
        "General".to_string(),
        Money::from_dollars(50),
        Utc::now(),
        None,
    )];

    let event_id = create_event_with_inventory(
        event_store.clone(),
        clock.clone(),
        "Test Event".to_string(),
        venue,
        pricing_tiers,
        inventory.clone(),
        global_channels.clone(),
    )
    .await
    .expect("Failed to create event with inventory");

    // ✨ Clean: Spawn channel consumers for pure direct orchestration
    spawn_channel_consumers(
        global_channels.clone(),
        inventory.clone(),
        payment.clone(),
        reservation.clone(),
    );

    // Step 2: Initiate reservation and wait for completion
    let result = reservation
        .send_and_wait_for(
            ReservationAction::InitiateReservation {
                reservation_id,
                event_id,
                customer_id,
                section: "General".to_string(),
                quantity: 2,
                specific_seats: None,
                correlation_id: None,
                respond_to: ticketing::types::ResponseChannel::none(),
            },
            |action| {
                matches!(action, ReservationAction::ReservationCompleted { .. })
            },
            Duration::from_secs(5),
        )
        .await
        .expect("Failed to complete reservation");

    // Verify: Saga should have completed successfully
    assert!(
        matches!(result, ReservationAction::ReservationCompleted { .. }),
        "Expected ReservationCompleted, got: {:?}",
        result
    );

    // Verify final state
    let (status, seat_count) = reservation.state(|state| {
        let res = state.get(&reservation_id).unwrap();
        (res.status.clone(), res.seats.len())
    }).await;
    assert_eq!(
        status,
        ReservationStatus::Completed,
        "Reservation should be completed after successful saga"
    );
    assert_eq!(seat_count, 2, "Should have 2 seats allocated");

    println!("✅ E2E Happy Path Test Passed!");
}

#[tokio::test]
#[ignore = "Requires actual projection infrastructure - MockInventoryQuery returns None"]
async fn test_e2e_saga_compensation_flow() {
    // Setup: Create infrastructure
    let event_store = Arc::new(InMemoryEventStore::new());
    let clock = Arc::new(SystemClock) as Arc<dyn Clock>;

    // Create test data
    let customer_id = CustomerId::new();
    let reservation_id = ReservationId::new();
    // ✨ NEW: Create shared global action channels (for direct orchestration)
    let global_channels = create_test_global_actions();

    // Initialize aggregates
    let inventory_env = InventoryEnvironment::new(
        clock.clone(),
        event_store.clone(),
        StreamId::new("inventory"),
        Arc::new(MockInventoryQuery),
        Arc::new(MockEventQuery),
        global_channels.clone(),
    );
    let inventory = Arc::new(Store::new(
        InventoryState::new(),
        InventoryReducer::new(),
        inventory_env,
    ));

    let payment_env = PaymentEnvironment::new(
        clock.clone(),
        event_store.clone(),
        StreamId::new("payment"),
        Arc::new(MockPaymentQuery),
        global_channels.clone(),
    );
    let _payment = Arc::new(Store::new(
        PaymentState::new(),
        PaymentReducer::new(),
        payment_env,
    ));

    let reservation_env = ReservationEnvironment::new(
        clock.clone(),
        event_store.clone(),
        StreamId::new("reservation"),
        Arc::new(MockReservationQuery),
        global_channels.clone(),
    );
    let reservation = Arc::new(Store::new(
        ReservationState::new(),
        ReservationReducer::new(),
        reservation_env,
    ));

    // Create Event + Inventory using EventInventorySaga
    let venue = Venue::new(
        "VIP Venue".to_string(),
        Capacity::new(50),
        vec![VenueSection::new(
            "VIP".to_string(),
            Capacity::new(50),
            SeatType::GeneralAdmission,
        )],
    );
    let pricing_tiers = vec![PricingTier::new(
        TierType::Regular,
        "VIP".to_string(),
        Money::from_dollars(100),
        Utc::now(),
        None,
    )];

    let event_id = create_event_with_inventory(
        event_store.clone(),
        clock.clone(),
        "VIP Event".to_string(),
        venue,
        pricing_tiers,
        inventory.clone(),
        global_channels.clone(),
    )
    .await
    .expect("Failed to create event with inventory");

    // ✨ Clean: Spawn channel consumers WITHOUT payment consumer
    // (we want to manually control payment failure for this test)
    let mut inventory_rx = global_channels.inventory_actions.subscribe();
    let inv = inventory.clone();
    tokio::spawn(async move {
        while let Ok(action) = inventory_rx.recv().await {
            let _ = inv.send(action).await;
        }
    });

    let mut reservation_rx = global_channels.reservation_actions.subscribe();
    let res = reservation.clone();
    tokio::spawn(async move {
        while let Ok(action) = reservation_rx.recv().await {
            let _ = res.send(action).await;
        }
    });

    // Initiate reservation (this will trigger inventory allocation automatically)
    reservation
        .send(ReservationAction::InitiateReservation {
            reservation_id,
            event_id,
            customer_id,
            section: "VIP".to_string(),
            quantity: 2,
            specific_seats: None,
            correlation_id: None,
            respond_to: ticketing::types::ResponseChannel::none(),
        })
        .await
        .expect("Failed to initiate reservation");

    // Wait for orchestration: InitiateReservation → Inventory allocates → SeatsAllocated response
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Verify PaymentPending (payment command was sent but we're not processing it)
    let status = reservation.state(|state| {
        state.get(&reservation_id).unwrap().status.clone()
    }).await;
    assert_eq!(status, ReservationStatus::PaymentPending);

    // ⚠️ Manually inject payment failure to trigger compensation
    // (in a real scenario, Payment aggregate would send this)
    reservation
        .send(ReservationAction::PaymentFailed {
            reservation_id,
            payment_id: PaymentId::new(),
            reason: "Insufficient funds".to_string(),
        })
        .await
        .expect("Failed to process payment failure");

    // Wait for compensation to complete
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Verify: Reservation should be compensated
    let status = reservation.state(|state| {
        state.get(&reservation_id).unwrap().status.clone()
    }).await;
    assert_eq!(
        status,
        ReservationStatus::Compensated,
        "Reservation should be compensated after payment failure"
    );

    println!("✅ E2E Compensation Flow Test Passed!");
}

#[tokio::test]
#[ignore = "Requires actual projection infrastructure - MockInventoryQuery returns None"]
async fn test_e2e_manual_cancellation() {
    // Setup: Create infrastructure
    let event_store = Arc::new(InMemoryEventStore::new());
    let clock = Arc::new(SystemClock) as Arc<dyn Clock>;

    // Create test data
    let customer_id = CustomerId::new();
    let reservation_id = ReservationId::new();
    // ✨ NEW: Create shared global action channels (for direct orchestration)
    let global_channels = create_test_global_actions();

    // Initialize aggregates
    let inventory_env = InventoryEnvironment::new(
        clock.clone(),
        event_store.clone(),
        StreamId::new("inventory"),
        Arc::new(MockInventoryQuery),
        Arc::new(MockEventQuery),
        global_channels.clone(),
    );
    let inventory = Arc::new(Store::new(
        InventoryState::new(),
        InventoryReducer::new(),
        inventory_env,
    ));

    let payment_env = PaymentEnvironment::new(
        clock.clone(),
        event_store.clone(),
        StreamId::new("payment"),
        Arc::new(MockPaymentQuery),
        global_channels.clone(),
    );
    let _payment = Arc::new(Store::new(
        PaymentState::new(),
        PaymentReducer::new(),
        payment_env,
    ));

    let reservation_env = ReservationEnvironment::new(
        clock.clone(),
        event_store.clone(),
        StreamId::new("reservation"),
        Arc::new(MockReservationQuery),
        global_channels.clone(),
    );
    let reservation = Arc::new(Store::new(
        ReservationState::new(),
        ReservationReducer::new(),
        reservation_env,
    ));

    // Create Event + Inventory using EventInventorySaga
    let venue = Venue::new(
        "General Venue".to_string(),
        Capacity::new(100),
        vec![VenueSection::new(
            "General".to_string(),
            Capacity::new(100),
            SeatType::GeneralAdmission,
        )],
    );
    let pricing_tiers = vec![PricingTier::new(
        TierType::Regular,
        "General".to_string(),
        Money::from_dollars(50),
        Utc::now(),
        None,
    )];

    let event_id = create_event_with_inventory(
        event_store.clone(),
        clock.clone(),
        "General Event".to_string(),
        venue,
        pricing_tiers,
        inventory.clone(),
        global_channels.clone(),
    )
    .await
    .expect("Failed to create event with inventory");

    // ✨ Clean: Spawn channel consumers WITHOUT payment consumer
    // (we want to test cancellation before payment completes)
    let mut inventory_rx = global_channels.inventory_actions.subscribe();
    let inv = inventory.clone();
    tokio::spawn(async move {
        while let Ok(action) = inventory_rx.recv().await {
            let _ = inv.send(action).await;
        }
    });

    let mut reservation_rx = global_channels.reservation_actions.subscribe();
    let res = reservation.clone();
    tokio::spawn(async move {
        while let Ok(action) = reservation_rx.recv().await {
            let _ = res.send(action).await;
        }
    });

    // Initiate reservation (this will trigger inventory allocation automatically)
    reservation
        .send(ReservationAction::InitiateReservation {
            reservation_id,
            event_id,
            customer_id,
            section: "General".to_string(),
            quantity: 1,
            specific_seats: None,
            correlation_id: None,
            respond_to: ticketing::types::ResponseChannel::none(),
        })
        .await
        .expect("Failed to initiate reservation");

    // Wait for orchestration: InitiateReservation → Inventory allocates → SeatsAllocated response
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Verify PaymentPending (payment command was sent but we're not processing it)
    let status = reservation.state(|state| {
        state.get(&reservation_id).unwrap().status.clone()
    }).await;
    assert_eq!(status, ReservationStatus::PaymentPending);

    // Customer manually cancels
    reservation
        .send(ReservationAction::CancelReservation { reservation_id })
        .await
        .expect("Failed to cancel reservation");

    // Wait for cancellation to complete
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Verify: Reservation should be cancelled
    let status = reservation.state(|state| {
        state.get(&reservation_id).unwrap().status.clone()
    }).await;
    assert_eq!(
        status,
        ReservationStatus::Cancelled,
        "Reservation should be cancelled"
    );

    println!("✅ E2E Manual Cancellation Test Passed!");
}
