//! End-to-end test for request lifecycle with projection completion tracking.
//!
//! This test demonstrates the complete flow:
//! 1. HTTP handler generates correlation_id
//! 2. Command sent with metadata via send_with_metadata()
//! 3. Events persisted to event store with correlation_id in metadata
//! 4. Events published to event bus with correlation_id
//! 5. Projection consumes events and emits completion event
//! 6. Projection completion tracker notifies waiting handler
//! 7. Subsequent query returns the projected data
//!
//! This enables read-after-write consistency in CQRS systems.

use chrono::Utc;
use composable_rust_core::{
    environment::Clock,
    event::EventMetadata,
    event_bus::EventBus,
    event_store::EventStore,
    projection::Projection,
    stream::StreamId,
};
use tokio_stream::StreamExt;
use composable_rust_postgres::PostgresEventStore;
use composable_rust_projections::{PostgresProjectionCheckpoint, ProjectionStream};
use composable_rust_redpanda::RedpandaEventBus;
use composable_rust_runtime::Store;
use composable_rust_testing::FixedClock;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use ticketing::{
    aggregates::{
        event::{EventEnvironment, EventProjectionQuery, EventReducer},
        event_inventory_saga::{EventInventorySaga, EventInventorySagaAction, EventInventorySagaEnvironment, EventInventorySagaState},
        inventory::{InventoryAction, InventoryProjectionQuery},
        reservation::{ReservationAction, ReservationEnvironment, ReservationProjectionQuery, ReservationReducer},
    },
    projections::{
        query_adapters::PostgresInventoryQuery,
        CorrelationId, PostgresAvailableSeatsProjection, ProjectionCompletionTracker,
        TicketingEvent,
    },
    types::{Capacity, CustomerId, EventDate, EventId, EventState, Money, PricingTier, ReservationId, ReservationState, SeatType, TierType, Venue, VenueSection},
};
use composable_rust_auth::state::UserId;
use tokio::time::timeout;
use uuid::Uuid;

// ============================================================================
// Mock Projection Queries for Testing
// ============================================================================

/// Mock inventory query that returns None (forcing event sourcing fallback)
#[derive(Clone)]
#[allow(dead_code)]
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

/// Mock event query that returns None (forcing event sourcing fallback)
#[derive(Clone)]
struct MockEventQuery;

#[async_trait::async_trait]
impl EventProjectionQuery for MockEventQuery {
    async fn load_event(&self, _event_id: &EventId) -> Result<Option<ticketing::types::Event>, String> {
        Ok(None) // No cached state, use event sourcing
    }

    async fn load_events(&self, _status_filter: Option<ticketing::types::EventStatus>) -> Result<Vec<ticketing::types::Event>, String> {
        Ok(vec![])
    }
}

/// Mock reservation query that returns None (forcing event sourcing fallback)
#[derive(Clone)]
struct MockReservationQuery;

impl ReservationProjectionQuery for MockReservationQuery {
    fn load_reservation(
        &self,
        _reservation_id: &ReservationId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<ticketing::types::Reservation>, String>> + Send + '_>> {
        Box::pin(async move { Ok(None) }) // No cached state, use event sourcing
    }

    fn list_by_customer(
        &self,
        _customer_id: &CustomerId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<ticketing::types::Reservation>, String>> + Send + '_>> {
        Box::pin(async move { Ok(Vec::new()) }) // No cached state, use event sourcing
    }
}

// ============================================================================
// Test Infrastructure
// ============================================================================

/// Test infrastructure setup
struct TestInfrastructure {
    event_store: Arc<PostgresEventStore>,
    event_bus: Arc<RedpandaEventBus>,
    projection_completion_tracker: Arc<ProjectionCompletionTracker>,
    projection_pool: PgPool,
    event_pool: PgPool,
}

impl TestInfrastructure {
    async fn setup() -> Result<Self, Box<dyn std::error::Error>> {
        // Connect to event store database
        let event_pool = PgPool::connect("postgres://postgres:postgres@localhost:5436/ticketing_events")
            .await?;

        // Connect to projection database
        let projection_pool = PgPool::connect("postgres://postgres:postgres@localhost:5433/ticketing_projections")
            .await?;

        // Create event store from pool
        let event_store = Arc::new(PostgresEventStore::from_pool(event_pool.clone()));

        // Create event bus with unique consumer group for this test
        let test_id = Uuid::new_v4();
        let event_bus = Arc::new(
            RedpandaEventBus::builder()
                .brokers("localhost:9092")
                .consumer_group(&format!("test-projection-e2e-{test_id}"))
                .build()?,
        );

        // Create projection completion tracker
        let completion_bus = Arc::clone(&event_bus);
        let projection_completion_tracker =
            Arc::new(ProjectionCompletionTracker::new(completion_bus).await?);

        Ok(Self {
            event_store,
            event_bus,
            projection_completion_tracker,
            projection_pool,
            event_pool,
        })
    }

    async fn cleanup(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Clean up event store
        sqlx::query("TRUNCATE TABLE events CASCADE")
            .execute(&self.event_pool)
            .await?;

        // Clean up projections
        sqlx::query("TRUNCATE TABLE projection_data CASCADE")
            .execute(&self.projection_pool)
            .await?;

        sqlx::query("TRUNCATE TABLE projection_checkpoints CASCADE")
            .execute(&self.projection_pool)
            .await?;

        // Clean up available_seats_projection table
        sqlx::query("TRUNCATE TABLE available_seats_projection CASCADE")
            .execute(&self.projection_pool)
            .await?;

        Ok(())
    }
}

/// Start projection consumer in background
async fn start_projection_consumer(
    infra: &TestInfrastructure,
) -> Result<tokio::task::JoinHandle<()>, Box<dyn std::error::Error>> {
    let projection = PostgresAvailableSeatsProjection::new(Arc::new(infra.projection_pool.clone()));

    let checkpoint = Arc::new(PostgresProjectionCheckpoint::new(infra.projection_pool.clone()));

    // Create dedicated event bus for projection consumption
    let test_id = Uuid::new_v4();
    let projection_event_bus = Arc::new(
        RedpandaEventBus::builder()
            .brokers("localhost:9092")
            .consumer_group(&format!("test-available-seats-{test_id}"))
            .auto_offset_reset("latest")  // Read only new events published during this test
            .build()?,
    );

    let projection_stream = ProjectionStream::new(
        projection_event_bus,
        checkpoint,
        "inventory",
        &format!("test-available-seats-{test_id}"),
        &format!("test-available-seats-{test_id}"),
    )
    .await?;

    let completion_bus = Arc::clone(&infra.event_bus);

    // Spawn projection consumer task
    let handle = tokio::spawn(async move {
        let mut stream = projection_stream;
        let projection_name = projection.name();

        loop {
            match stream.next().await {
                Some(Ok(serialized)) => {
                    // Extract correlation_id from metadata
                    let correlation_id = serialized.metadata.as_ref().and_then(|metadata| {
                        metadata
                            .correlation_id
                            .as_ref()
                            .and_then(|s| Uuid::parse_str(s).ok())
                            .map(CorrelationId::from_uuid)
                    });

                    tracing::debug!(
                        event_type = %serialized.event_type,
                        has_metadata = serialized.metadata.is_some(),
                        correlation_id = ?correlation_id,
                        "Projection consumer received event"
                    );

                    // Deserialize event
                    match bincode::deserialize::<TicketingEvent>(&serialized.data) {
                        Ok(event) => {
                            // Apply to projection
                            if let Err(e) = projection.apply_event(&event).await {
                                tracing::error!(
                                    projection = projection_name,
                                    error = ?e,
                                    "Failed to apply event"
                                );

                                // Publish failure event
                                if let Some(cid) = correlation_id {
                                    let bus_trait: Arc<dyn EventBus> = completion_bus.clone();
                                    publish_projection_failed(
                                        &bus_trait,
                                        projection_name,
                                        &serialized.event_type,
                                        cid,
                                        &e.to_string(),
                                    )
                                    .await;
                                }
                            } else {
                                // Commit checkpoint
                                if let Err(e) = stream.commit().await {
                                    tracing::error!(error = ?e, "Failed to commit checkpoint");
                                } else {
                                    // Only publish completion for domain EVENTS (not commands)
                                    // Events update the projection, commands are ignored
                                    // Events have past-tense names: InventoryInitialized, SeatsReserved, etc.
                                    // Commands have imperative names: ReserveSeats, ConfirmSeats, etc.
                                    let is_domain_event = matches!(
                                        event,
                                        TicketingEvent::Inventory(InventoryAction::InventoryInitialized { .. })
                                            | TicketingEvent::Inventory(InventoryAction::SeatsReserved { .. })
                                            | TicketingEvent::Inventory(InventoryAction::SeatsConfirmed { .. })
                                            | TicketingEvent::Inventory(InventoryAction::SeatsReleased { .. })
                                    );

                                    if is_domain_event {
                                        // Publish completion event
                                        if let Some(cid) = correlation_id {
                                            let bus_trait: Arc<dyn EventBus> = completion_bus.clone();
                                            publish_projection_completed(
                                                &bus_trait,
                                                projection_name,
                                                &serialized.event_type,
                                                cid,
                                            )
                                            .await;
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!(error = ?e, "Failed to deserialize event");
                        }
                    }
                }
                Some(Err(e)) => {
                    tracing::error!(error = ?e, "Error receiving event");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                None => break,
            }
        }
    });

    // Give projection consumer time to subscribe
    tokio::time::sleep(Duration::from_millis(500)).await;

    Ok(handle)
}

/// Start Inventory aggregate consumer in background
///
/// This consumer processes commands from the "inventory" topic and routes them
/// to the Inventory aggregate. This is necessary for the saga workflow to complete.
async fn start_inventory_consumer(
    infra: &TestInfrastructure,
) -> Result<tokio::task::JoinHandle<()>, Box<dyn std::error::Error>> {
    use ticketing::aggregates::inventory::{InventoryReducer, InventoryEnvironment};
    use ticketing::types::InventoryState;

    // Create dedicated event bus for inventory consumption
    let test_id = Uuid::new_v4();
    let inventory_event_bus = Arc::new(
        RedpandaEventBus::builder()
            .brokers("localhost:9092")
            .consumer_group(&format!("test-inventory-consumer-{test_id}"))
            .auto_offset_reset("latest")  // Only process new events published after subscription
            .build()?,
    );

    // Clone infrastructure for use in async task
    let event_store = Arc::clone(&infra.event_store);
    let event_bus = Arc::clone(&infra.event_bus);
    let projection_pool = infra.projection_pool.clone();

    // Subscribe to inventory topic
    let mut stream = inventory_event_bus.subscribe(&["inventory"]).await?;

    tracing::info!("Starting Inventory aggregate consumer");

    // Spawn consumer task
    let handle = tokio::spawn(async move {
        tracing::info!("Inventory consumer loop started");
        loop {
            match stream.next().await {
                Some(Ok(serialized)) => {
                    tracing::debug!("Received event from inventory topic: {}", serialized.event_type);
                    // Deserialize TicketingEvent
                    match TicketingEvent::deserialize(&serialized) {
                        Ok(TicketingEvent::Inventory(action)) => {
                            // Extract event_id from the action to create the right store
                            let event_id = match &action {
                                InventoryAction::InitializeInventory { event_id, .. } => *event_id,
                                InventoryAction::ReserveSeats { event_id, .. } => *event_id,
                                _ => {
                                    tracing::warn!(action = ?action, "Inventory action without event_id");
                                    continue;
                                }
                            };

                            tracing::debug!(
                                action = ?action,
                                event_id = %event_id.as_uuid(),
                                "Processing Inventory command"
                            );

                            // Create Inventory store for this specific event
                            let inventory_stream_id = format!("inventory-{}", event_id.as_uuid());

                            // Create real projection query that reads from database
                            let available_seats = Arc::new(PostgresAvailableSeatsProjection::new(
                                Arc::new(projection_pool.clone())
                            ));
                            let projection_query = Arc::new(PostgresInventoryQuery::new(available_seats));

                            let inventory_env = InventoryEnvironment::new(
                                Arc::new(FixedClock::new(Utc::now())) as Arc<dyn Clock>,
                                Arc::clone(&event_store) as Arc<dyn EventStore>,
                                Arc::clone(&event_bus) as Arc<dyn EventBus>,
                                StreamId::new(&inventory_stream_id),
                                projection_query as Arc<dyn InventoryProjectionQuery>,
                            );

                            let inventory_store = Store::new(
                                InventoryState::new(),
                                InventoryReducer::new(),
                                inventory_env,
                            );

                            // Send command to store with metadata (to propagate correlation_id)
                            if let Err(e) = inventory_store.send_with_metadata(action, serialized.metadata).await {
                                tracing::error!(error = ?e, "Failed to process Inventory command");
                            }
                            // Note: Commits are handled automatically by RedpandaEventBus
                        }
                        Ok(_) => {
                            // Not an Inventory event, skip
                            tracing::debug!("Skipping non-Inventory event");
                        }
                        Err(e) => {
                            tracing::error!(error = ?e, "Failed to deserialize event");
                        }
                    }
                }
                Some(Err(e)) => {
                    tracing::error!(error = ?e, "Error receiving event from inventory topic");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                None => {
                    tracing::info!("Inventory consumer stream ended");
                    break;
                }
            }
        }
    });

    // Give consumer time to subscribe
    tokio::time::sleep(Duration::from_millis(500)).await;

    Ok(handle)
}

async fn publish_projection_completed(
    completion_bus: &Arc<dyn EventBus>,
    projection_name: &str,
    event_type: &str,
    correlation_id: CorrelationId,
) {
    use composable_rust_core::event::SerializedEvent;
    use ticketing::projections::{ProjectionCompleted, ProjectionCompletionEvent};

    let completion = ProjectionCompleted {
        correlation_id,
        projection_name: projection_name.to_string(),
        event_type: event_type.to_string(),
    };

    let event = ProjectionCompletionEvent::Completed(completion);

    if let Ok(data) = bincode::serialize(&event) {
        let mut metadata = EventMetadata::new();
        metadata.correlation_id = Some(correlation_id.to_string());

        let serialized = SerializedEvent::new(
            "ProjectionCompleted".to_string(),
            data,
            Some(metadata),
        );

        let _ = completion_bus
            .publish("projection.completed", &serialized)
            .await;
    }
}

async fn publish_projection_failed(
    completion_bus: &Arc<dyn EventBus>,
    projection_name: &str,
    event_type: &str,
    correlation_id: CorrelationId,
    error: &str,
) {
    use composable_rust_core::event::SerializedEvent;
    use ticketing::projections::{ProjectionCompletionEvent, ProjectionFailed};

    let failure = ProjectionFailed {
        correlation_id,
        projection_name: projection_name.to_string(),
        event_type: event_type.to_string(),
        error: error.to_string(),
    };

    let event = ProjectionCompletionEvent::Failed(failure);

    if let Ok(data) = bincode::serialize(&event) {
        let mut metadata = EventMetadata::new();
        metadata.correlation_id = Some(correlation_id.to_string());

        let serialized = SerializedEvent::new(
            "ProjectionFailed".to_string(),
            data,
            Some(metadata),
        );

        let _ = completion_bus
            .publish("projection.completed", &serialized)
            .await;
    }
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL and Redpanda
async fn test_complete_request_lifecycle_with_projection_completion(
) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init()
        .ok();

    // Setup test infrastructure
    let infra = TestInfrastructure::setup().await?;

    // Clean up from previous runs
    infra.cleanup().await?;

    // ========================================================================
    // Step 1: Create Event + Inventory using EventInventorySaga
    // ========================================================================

    let event_id = EventId::new();

    // Start projection consumer
    let _projection_handle = start_projection_consumer(&infra).await?;

    // Start Inventory aggregate consumer (processes ReserveSeats commands dynamically)
    let _inventory_handle = start_inventory_consumer(&infra).await?;

    // Give consumers time to fully initialize
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Set up EventInventorySaga
    let saga_stream_id = format!("event-inventory-saga-{}", event_id.as_uuid());
    let saga_env = EventInventorySagaEnvironment::new(
        Arc::new(FixedClock::new(Utc::now())) as Arc<dyn Clock>,
        Arc::clone(&infra.event_store) as Arc<dyn EventStore>,
        Arc::clone(&infra.event_bus) as Arc<dyn EventBus>,
        StreamId::new(&saga_stream_id),
    );
    let saga_store = Store::new(
        EventInventorySagaState::new(),
        EventInventorySaga::new(),
        saga_env,
    );

    // Set up Event aggregate (needed for saga)
    let event_stream_id = format!("event-{}", event_id.as_uuid());
    let event_env = EventEnvironment::new(
        Arc::new(FixedClock::new(Utc::now())) as Arc<dyn Clock>,
        Arc::clone(&infra.event_store) as Arc<dyn EventStore>,
        Arc::clone(&infra.event_bus) as Arc<dyn EventBus>,
        StreamId::new(&event_stream_id),
        Arc::new(MockEventQuery) as Arc<dyn EventProjectionQuery>,
    );
    let event_store_agg = Arc::new(Store::new(
        EventState::new(),
        EventReducer::new(),
        event_env,
    ));

    // Spawn Event aggregate consumer (listens to "events" topic)
    // Each consumer MUST have its own EventBus with unique consumer group (Kafka/Redpanda requirement)
    let test_id = Uuid::new_v4();
    let event_consumer_bus = Arc::new(
        RedpandaEventBus::builder()
            .brokers("localhost:9092")
            .consumer_group(&format!("test-event-aggregate-{test_id}"))
            .auto_offset_reset("latest")
            .build()
            .expect("Failed to create event consumer bus"),
    ) as Arc<dyn EventBus>;

    let event_consumer_store = event_store_agg.clone();
    let _event_handle = tokio::spawn(async move {
        let mut stream = event_consumer_bus.subscribe(&["events"]).await.unwrap();
        while let Some(result) = stream.next().await {
            if let Ok(serialized) = result {
                if let Ok(TicketingEvent::Event(action)) =
                    TicketingEvent::deserialize(&serialized)
                {
                    let _ = event_consumer_store.send(action).await;
                }
            }
        }
    });

    // Spawn saga consumers (wires Event/Inventory events → Saga actions)
    // Saga gets its own EventBus with unique consumer group
    let saga_consumer_bus = Arc::new(
        RedpandaEventBus::builder()
            .brokers("localhost:9092")
            .consumer_group(&format!("test-saga-{test_id}"))
            .auto_offset_reset("latest")
            .build()
            .expect("Failed to create saga consumer bus"),
    ) as Arc<dyn EventBus>;

    let saga_consumer_store = Arc::new(saga_store);
    ticketing::aggregates::event_inventory_saga::spawn_event_inventory_saga_consumers(
        saga_consumer_bus,
        saga_consumer_store.clone(),
    );

    // Give consumers time to subscribe
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Create Event + Inventory using saga
    let venue = Venue::new(
        "Test Venue".to_string(),
        Capacity::new(3),
        vec![VenueSection::new(
            "VIP".to_string(),
            Capacity::new(3),
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

    let result = saga_consumer_store
        .send_and_wait_for(
            EventInventorySagaAction::CreateEventWithInventory {
                event_id,
                name: "Test Event".to_string(),
                owner_id: UserId::new(),
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
            Duration::from_secs(10),
        )
        .await?;

    match result {
        EventInventorySagaAction::EventCreationCompleted { .. } => {
            tracing::info!("Event and inventory created successfully via saga");
        }
        EventInventorySagaAction::EventCreationFailed { error, .. } => {
            return Err(format!("Saga failed: {error}").into());
        }
        _ => {
            return Err("Unexpected saga result".into());
        }
    }

    // Wait for events to be fully processed
    tokio::time::sleep(Duration::from_millis(500)).await;

    // ========================================================================
    // Step 2: Create Reservation with correlation_id tracking
    // ========================================================================

    let reservation_id = ReservationId::new();
    let customer_id = CustomerId::new();
    let correlation_id = CorrelationId::new();

    tracing::info!(
        correlation_id = %correlation_id.to_string(),
        reservation_id = %reservation_id.as_uuid(),
        "Starting reservation with correlation tracking"
    );

    // Register interest in projection completion BEFORE sending command
    let completion_receiver = infra
        .projection_completion_tracker
        .register_interest(correlation_id, &["available_seats_projection"]);

    // Create reservation environment
    let reservation_stream_id = format!("reservation-{}", reservation_id.as_uuid());
    let reservation_env = ReservationEnvironment::new(
        Arc::new(FixedClock::new(Utc::now())) as Arc<dyn Clock>,
        Arc::clone(&infra.event_store) as Arc<dyn EventStore>,
        Arc::clone(&infra.event_bus) as Arc<dyn EventBus>,
        StreamId::new(&reservation_stream_id),
        Arc::new(MockReservationQuery),
    );

    let reservation_store = Store::new(
        ReservationState::new(),
        ReservationReducer::new(),
        reservation_env,
    );

    // Prepare metadata with correlation_id
    let metadata = EventMetadata::with_correlation_id(correlation_id.to_string());

    // Send reservation command with metadata
    tracing::info!("Sending InitiateReservation command with metadata");
    let mut handle = reservation_store
        .send_with_metadata(
            ReservationAction::InitiateReservation {
                reservation_id,
                event_id,
                customer_id,
                section: "VIP".to_string(),
                quantity: 2,
                specific_seats: None,
                correlation_id: None, // Will be injected by send_with_metadata
            },
            Some(metadata),
        )
        .await?;

    // Wait for the first 3 effects to complete (AppendEvents + 2x PublishEvent)
    // The 4th effect is a 5-minute delay (reservation expiration timer) which we
    // let run in the background. This ensures the ReserveSeats command reaches
    // Kafka before we proceed to waiting for projection completion.
    tracing::info!("Waiting for first 3 effects to complete (ignoring delay)...");
    handle.wait_until_n_remain(1).await;
    tracing::info!("First 3 effects completed, proceeding to projection tracking");

    // ========================================================================
    // Step 3: Wait for projection completion notification
    // ========================================================================

    tracing::info!("Waiting for projection completion notification...");

    let projection_result = timeout(Duration::from_secs(30), completion_receiver).await??;

    match projection_result {
        ticketing::projections::ProjectionResult::Completed(completed) => {
            tracing::info!(
                projections = ?completed,
                "Projection completed successfully"
            );

            assert_eq!(completed.len(), 1);
            assert_eq!(completed[0].projection_name, "available_seats_projection");
            assert_eq!(completed[0].correlation_id, correlation_id);
        }
        ticketing::projections::ProjectionResult::Failed(failed) => {
            panic!("Projection failed: {:?}", failed);
        }
    }

    // ========================================================================
    // Step 4: Query projection to verify data is available
    // ========================================================================

    tracing::info!("Querying projection for available seats...");

    let availability = PostgresAvailableSeatsProjection::new(Arc::new(infra.projection_pool.clone()))
        .get_availability(&event_id, "VIP")
        .await?;

    tracing::info!(
        availability = ?availability,
        "Query returned availability data"
    );

    // Verify projection data
    let (total, reserved, sold, available) = availability
        .expect("Should have availability data for VIP section");

    assert_eq!(total, 3, "Total capacity should be 3");
    assert_eq!(reserved, 2, "Should have 2 reserved seats");
    assert_eq!(sold, 0, "Should have 0 sold seats");
    assert_eq!(available, 1, "Should have 1 available seat (3 total - 2 reserved)");

    tracing::info!("✅ End-to-end request lifecycle test completed successfully!");

    // Cleanup
    infra.cleanup().await?;

    Ok(())
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL and Redpanda
async fn test_projection_failure_notification() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init()
        .ok();

    let infra = TestInfrastructure::setup().await?;
    infra.cleanup().await?;

    let correlation_id = CorrelationId::new();

    // Register interest before failure occurs
    let completion_receiver = infra
        .projection_completion_tracker
        .register_interest(correlation_id, &["available_seats_projection"]);

    // Simulate projection failure by publishing failure event directly
    let bus_trait: Arc<dyn EventBus> = infra.event_bus.clone();
    publish_projection_failed(
        &bus_trait,
        "available_seats_projection",
        "InventoryAction::SeatsReserved",
        correlation_id,
        "Simulated projection error",
    )
    .await;

    // Wait for failure notification
    let projection_result = timeout(Duration::from_secs(5), completion_receiver).await??;

    match projection_result {
        ticketing::projections::ProjectionResult::Failed(failed) => {
            tracing::info!(failures = ?failed, "Projection failure detected");

            assert_eq!(failed.len(), 1);
            assert_eq!(failed[0].projection_name, "available_seats_projection");
            assert_eq!(failed[0].correlation_id, correlation_id);
            assert!(failed[0].error.contains("Simulated projection error"));
        }
        ticketing::projections::ProjectionResult::Completed(_) => {
            panic!("Expected failure but got completion");
        }
    }

    tracing::info!("✅ Projection failure notification test passed!");

    infra.cleanup().await?;
    Ok(())
}
