//! Event-Inventory Saga
//!
//! **Responsibility**: Coordinate event creation with automatic inventory initialization.
//!
//! **Why a saga?**: Creating an event REQUIRES inventory. Without inventory, customers
//! can't make reservations. This is a multi-aggregate transaction that must be coordinated.
//!
//! **Pattern**: This is a **parent saga** that receives commands and orchestrates child
//! aggregates (Event and Inventory). Each saga instance handles ONE event creation workflow.
//!
//! **Flow**:
//! 1. API → Saga: `CreateEventWithInventory` command
//! 2. Saga → Event aggregate: Publish `CreateEvent` command to "events" topic
//! 3. Event aggregate → EventBus: Publishes `EventCreated` event
//! 4. Saga receives `EventCreated` notification (via EventBus subscription)
//! 5. Saga → Inventory aggregate: Publish `InitializeInventory` for each venue section
//! 6. Inventory aggregate → EventBus: Publishes `InventoryInitialized` for each section
//! 7. Saga receives notifications and tracks completion
//! 8. When all sections done: Saga emits `EventCreationCompleted`
//!
//! **Instance Model**: One saga instance per event creation request. The saga persists
//! its state via events in its own event stream and completes when all steps are done.

use crate::aggregates::{
    EventAction, EventEnvironment, EventReducer, InventoryAction, InventoryEnvironment,
    InventoryReducer,
};
use crate::projections::TicketingEvent;
use crate::types::{
    Capacity, EventDate, EventId, EventState, InventoryState, PricingTier, ResponseChannel, Venue,
};
use chrono::{DateTime, Utc};
use composable_rust_auth::state::UserId;
use composable_rust_core::{
    append_events, effect::Effect, environment::Clock,
    event_store::EventStore, reducer::Reducer, smallvec,
    stream::{StreamId, Version},
    SmallVec,
};
use composable_rust_runtime::Store;
use composable_rust_macros::Action;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

// ============================================================================
// State
// ============================================================================

/// Saga state for a SINGLE event creation workflow
///
/// Each saga instance handles ONE event creation. When the workflow completes,
/// the saga is done. State is persisted via events in the saga's event stream.
#[derive(Clone, Debug)]
pub struct EventInventorySagaState {
    /// The event being created (None if not yet initiated)
    pub event_id: Option<EventId>,
    /// Sections that still need inventory initialized
    pub pending_sections: HashSet<String>,
    /// Section capacities from the venue (section_name -> capacity)
    pub section_capacities: std::collections::HashMap<String, Capacity>,
    /// Whether the Event aggregate has created the event
    pub event_created: bool,
    /// Whether all inventory has been initialized
    pub inventory_complete: bool,
    /// Whether the entire saga completed
    pub completed: bool,
    /// Last error
    pub last_error: Option<String>,
    /// Stream version
    pub version: Version,
}

impl EventInventorySagaState {
    /// Creates a new empty state
    #[must_use]
    pub fn new() -> Self {
        Self {
            event_id: None,
            pending_sections: HashSet::new(),
            section_capacities: std::collections::HashMap::new(),
            event_created: false,
            inventory_complete: false,
            completed: false,
            last_error: None,
            version: Version::new(0),
        }
    }

    /// Check if saga is complete
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.completed
    }

    /// Check if event creation is in progress
    #[must_use]
    pub const fn is_in_progress(&self) -> bool {
        self.event_id.is_some() && !self.completed
    }
}

impl Default for EventInventorySagaState {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Actions (Commands + Events)
// ============================================================================

/// Actions for Event-Inventory Saga
///
/// This saga coordinates Event creation with Inventory initialization.
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum EventInventorySagaAction {
    // ========== COMMANDS (from API) ==========
    /// Create event with automatic inventory initialization
    #[command]
    CreateEventWithInventory {
        /// Event ID (provided by caller for idempotency)
        event_id: EventId,
        /// Event name
        name: String,
        /// Event owner
        owner_id: UserId,
        /// Venue (contains sections that need inventory)
        venue: Venue,
        /// Event date
        date: EventDate,
        /// Pricing tiers
        pricing_tiers: Vec<PricingTier>,
    },

    // ========== EVENTS (saga lifecycle) ==========
    /// Saga initiated - event creation started
    #[event]
    EventCreationInitiated {
        /// Event ID
        event_id: EventId,
        /// Event name
        name: String,
        /// Number of sections that need inventory
        section_count: u32,
        /// Section capacities from the venue (section_name -> capacity)
        section_capacities: std::collections::HashMap<String, Capacity>,
        /// When initiated
        initiated_at: DateTime<Utc>,
    },

    /// Event aggregate created the event successfully
    #[event]
    EventCreated {
        /// Event ID
        event_id: EventId,
        /// Sections that need inventory initialization
        sections: Vec<String>,
        /// When event was created
        created_at: DateTime<Utc>,
    },

    /// Inventory initialized for a section
    #[event]
    SectionInventoryInitialized {
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
        /// When initialized
        initialized_at: DateTime<Utc>,
    },

    /// All inventory initialized - saga complete
    #[event]
    EventCreationCompleted {
        /// Event ID
        event_id: EventId,
        /// Total sections initialized
        sections_initialized: u32,
        /// When completed
        completed_at: DateTime<Utc>,
    },

    /// Event creation failed
    #[event]
    EventCreationFailed {
        /// Event ID
        event_id: EventId,
        /// Error message
        error: String,
        /// When failed
        failed_at: DateTime<Utc>,
    },

    /// Inventory initialization failed for a section
    #[event]
    InventoryInitializationFailed {
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
        /// Error message
        error: String,
        /// When failed
        failed_at: DateTime<Utc>,
    },

    /// Validation failed
    #[event]
    ValidationFailed {
        /// Error message
        error: String,
    },

    /// Stream version updated
    #[event]
    VersionUpdated {
        /// New version
        version: Version,
    },
}

// ============================================================================
// Environment
// ============================================================================

/// Environment for Event-Inventory Saga
///
/// Uses **direct orchestration** pattern: saga creates child aggregate stores
/// and sends commands directly, replacing event bus choreography.
#[derive(Clone)]
pub struct EventInventorySagaEnvironment {
    /// Clock for timestamps
    pub clock: Arc<dyn Clock>,
    /// Event store for saga persistence
    pub event_store: Arc<dyn EventStore>,
    /// Stream ID for this saga instance
    pub stream_id: StreamId,

    /// Factory function to create Event aggregate stores
    pub create_event_store: Arc<
        dyn Fn(EventId) -> Store<EventState, EventAction, EventEnvironment, EventReducer>
            + Send
            + Sync,
    >,

    /// Factory function to create Inventory aggregate stores
    pub create_inventory_store: Arc<
        dyn Fn(EventId) -> Store<InventoryState, InventoryAction, InventoryEnvironment, InventoryReducer>
            + Send
            + Sync,
    >,
}

impl EventInventorySagaEnvironment {
    /// Creates a new environment with factory functions for child aggregate stores
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        event_store: Arc<dyn EventStore>,
        stream_id: StreamId,
        create_event_store: Arc<
            dyn Fn(EventId) -> Store<EventState, EventAction, EventEnvironment, EventReducer>
                + Send
                + Sync,
        >,
        create_inventory_store: Arc<
            dyn Fn(EventId) -> Store<InventoryState, InventoryAction, InventoryEnvironment, InventoryReducer>
                + Send
                + Sync,
        >,
    ) -> Self {
        Self {
            clock,
            event_store,
            stream_id,
            create_event_store,
            create_inventory_store,
        }
    }
}

// ============================================================================
// Reducer
// ============================================================================

/// Saga coordinator for event creation with inventory
///
/// This is a **parent saga** that orchestrates the Event and Inventory aggregates.
/// Each saga instance handles ONE event creation workflow.
#[derive(Clone, Debug)]
pub struct EventInventorySaga;

impl EventInventorySaga {
    /// Creates a new saga
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Creates effects for persisting saga events
    ///
    /// Note: With direct orchestration, we only persist to event store.
    /// No event bus publishing needed since projections rebuild from event store.
    fn create_effects(
        event: EventInventorySagaAction,
        expected_version: Version,
        env: &EventInventorySagaEnvironment,
    ) -> SmallVec<[Effect<EventInventorySagaAction>; 4]> {
        let ticketing_event = TicketingEvent::EventInventorySaga(event);
        let Ok(serialized) = ticketing_event.serialize() else {
            return SmallVec::new();
        };

        smallvec![
            append_events! {
                store: env.event_store,
                stream: env.stream_id.as_str(),
                expected_version: Some(expected_version),
                events: vec![serialized.clone()],
                on_success: |version| Some(EventInventorySagaAction::VersionUpdated { version }),
                on_error: |error| Some(EventInventorySagaAction::ValidationFailed {
                    error: error.to_string()
                })
            }
        ]
    }

    /// Validates create event command
    fn validate_create_event(
        state: &EventInventorySagaState,
        name: &str,
        venue: &Venue,
    ) -> Result<(), String> {
        // Check if saga already started
        if state.event_id.is_some() {
            return Err("Saga already initiated".to_string());
        }

        // Basic validation
        if name.is_empty() {
            return Err("Event name cannot be empty".to_string());
        }

        if venue.sections.is_empty() {
            return Err("Venue must have at least one section".to_string());
        }

        // Validate venue sections
        for section in &venue.sections {
            if section.capacity.value() == 0 {
                return Err(format!(
                    "Section '{}' has zero capacity",
                    section.name
                ));
            }
        }

        Ok(())
    }

    /// Applies an event to state
    fn apply_event(state: &mut EventInventorySagaState, action: &EventInventorySagaAction) {
        match action {
            EventInventorySagaAction::EventCreationInitiated {
                event_id,
                section_capacities,
                ..
            } => {
                state.event_id = Some(*event_id);
                state.section_capacities = section_capacities.clone();
                state.last_error = None;
            }

            EventInventorySagaAction::EventCreated {
                event_id,
                sections,
                ..
            } => {
                state.event_id = Some(*event_id);
                state.event_created = true;
                state.pending_sections = sections.iter().cloned().collect();
                state.last_error = None;
            }

            EventInventorySagaAction::SectionInventoryInitialized { section, .. } => {
                state.pending_sections.remove(section);

                // Check if all done
                if state.pending_sections.is_empty() {
                    state.inventory_complete = true;
                }
                state.last_error = None;
            }

            EventInventorySagaAction::EventCreationCompleted { .. } => {
                state.completed = true;
                state.last_error = None;
            }

            EventInventorySagaAction::EventCreationFailed { error, .. }
            | EventInventorySagaAction::InventoryInitializationFailed { error, .. }
            | EventInventorySagaAction::ValidationFailed { error } => {
                state.last_error = Some(error.clone());
            }

            EventInventorySagaAction::VersionUpdated { version } => {
                state.version = *version;
            }

            // Commands don't modify state
            EventInventorySagaAction::CreateEventWithInventory { .. } => {}
        }
    }
}

impl Default for EventInventorySaga {
    fn default() -> Self {
        Self::new()
    }
}

impl Reducer for EventInventorySaga {
    type State = EventInventorySagaState;
    type Action = EventInventorySagaAction;
    type Environment = EventInventorySagaEnvironment;

    #[allow(clippy::too_many_lines)]
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ========== Step 1: Initiate Event Creation ==========
            EventInventorySagaAction::CreateEventWithInventory {
                event_id,
                name,
                owner_id,
                venue,
                date,
                pricing_tiers,
            } => {
                tracing::info!(
                    event_id = %event_id.as_uuid(),
                    name = %name,
                    sections = venue.sections.len(),
                    "Saga: CreateEventWithInventory command received"
                );

                // Validate
                if let Err(error) = Self::validate_create_event(state, &name, &venue) {
                    Self::apply_event(state, &EventInventorySagaAction::ValidationFailed { error });
                    return SmallVec::new();
                }

                // Create saga initiation event
                #[allow(clippy::cast_possible_truncation)]
                let section_capacities: std::collections::HashMap<String, Capacity> = venue
                    .sections
                    .iter()
                    .map(|section| (section.name.clone(), section.capacity))
                    .collect();

                let initiated = EventInventorySagaAction::EventCreationInitiated {
                    event_id,
                    name: name.clone(),
                    section_count: venue.sections.len() as u32,
                    section_capacities,
                    initiated_at: env.clock.now(),
                };
                let expected_version = state.version;
                Self::apply_event(state, &initiated);

                // Persist saga event
                let mut effects = Self::create_effects(initiated, expected_version, env);

                // Direct orchestration: Create Event + Inventory stores and send commands
                let create_event_store = env.create_event_store.clone();
                let create_inventory_store = env.create_inventory_store.clone();
                let venue_clone = venue.clone();
                let now = env.clock.now();

                effects.push(Effect::Future(Box::pin(async move {
                    tracing::debug!(
                        event_id = %event_id.as_uuid(),
                        "Saga: Orchestrating Event creation"
                    );

                    // Step 1: Create Event
                    let event_store = create_event_store(event_id);

                    let create_event_action = EventAction::CreateEvent {
                        id: event_id,
                        name,
                        owner_id,
                        venue: venue_clone.clone(),
                        date,
                        pricing_tiers,
                        respond_to: ResponseChannel::none(),
                    };

                    // Send CreateEvent command and wait for completion
                    if let Err(e) = event_store.send(create_event_action).await {
                        tracing::error!(
                            event_id = %event_id.as_uuid(),
                            error = %e,
                            "Saga: Event creation failed"
                        );
                        return Some(EventInventorySagaAction::EventCreationFailed {
                            event_id,
                            error: e.to_string(),
                            failed_at: now,
                        });
                    }

                    tracing::info!(
                        event_id = %event_id.as_uuid(),
                        "Saga: Event created successfully"
                    );

                    // Step 2: Initialize Inventory for each section
                    for section in &venue_clone.sections {
                        tracing::debug!(
                            event_id = %event_id.as_uuid(),
                            section = %section.name,
                            "Saga: Initializing inventory"
                        );

                        let inventory_store = create_inventory_store(event_id);

                        let init_action = InventoryAction::InitializeInventory {
                            event_id,
                            section: section.name.clone(),
                            capacity: section.capacity,
                            seat_numbers: None,
                            respond_to: ResponseChannel::none(),
                        };

                        if let Err(e) = inventory_store.send(init_action).await {
                            tracing::error!(
                                event_id = %event_id.as_uuid(),
                                section = %section.name,
                                error = %e,
                                "Saga: Inventory initialization failed"
                            );
                            return Some(EventInventorySagaAction::InventoryInitializationFailed {
                                event_id,
                                section: section.name.clone(),
                                error: e.to_string(),
                                failed_at: now,
                            });
                        }
                    }

                    // Saga completed successfully
                    tracing::info!(
                        event_id = %event_id.as_uuid(),
                        sections = venue_clone.sections.len(),
                        "Saga: Event creation with inventory completed successfully"
                    );

                    #[allow(clippy::cast_possible_truncation)]
                    Some(EventInventorySagaAction::EventCreationCompleted {
                        event_id,
                        sections_initialized: venue_clone.sections.len() as u32,
                        completed_at: now,
                    })
                })));

                effects
            }

            // ========== OBSOLETE: EventCreated (replaced by direct orchestration) ==========
            EventInventorySagaAction::EventCreated { .. } => {
                // This event is obsolete with direct orchestration.
                // The CreateEventWithInventory handler orchestrates everything directly.
                SmallVec::new()
            }

            // ========== OBSOLETE: SectionInventoryInitialized (replaced by direct orchestration) ==========
            EventInventorySagaAction::SectionInventoryInitialized { .. } => {
                // This event is obsolete with direct orchestration.
                // The CreateEventWithInventory handler orchestrates everything directly.
                SmallVec::new()
            }

            // ========== Other events ==========
            event => {
                Self::apply_event(state, &event);
                SmallVec::new()
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::aggregates::inventory::InventoryProjectionQuery;
    use crate::types::{Money, SeatAssignment, SeatType, VenueSection};
    use chrono::Duration;
    use composable_rust_core::environment::SystemClock;
    use composable_rust_testing::{
        assertions,
        mocks::InMemoryEventStore,
        ReducerTest,
    };

    // Mock projection queries for tests
    #[derive(Clone)]
    struct MockEventQuery;

    #[async_trait::async_trait]
    impl crate::aggregates::event::EventProjectionQuery for MockEventQuery {
        async fn load_event(&self, _event_id: &EventId) -> Result<Option<crate::types::Event>, String> {
            Ok(None) // No cached state, use event sourcing
        }

        async fn load_events(&self, _status_filter: Option<crate::types::EventStatus>) -> Result<Vec<crate::types::Event>, String> {
            Ok(vec![])
        }
    }

    #[derive(Clone)]
    struct MockInventoryQuery;

    impl InventoryProjectionQuery for MockInventoryQuery {
        fn load_inventory(
            &self,
            _event_id: &EventId,
            _section: &str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<((u32, u32, u32, u32), Vec<SeatAssignment>)>, String>> + Send + '_>> {
            Box::pin(async move { Ok(None) }) // No cached state, use event sourcing
        }

        fn get_all_sections(
            &self,
            _event_id: &EventId,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<crate::aggregates::inventory::SectionAvailabilityData>, String>> + Send + '_>> {
            Box::pin(async move { Ok(vec![]) })
        }

        fn get_section_availability(
            &self,
            _event_id: &EventId,
            _section: &str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<crate::aggregates::inventory::SectionAvailabilityData>, String>> + Send + '_>> {
            Box::pin(async move { Ok(None) })
        }

        fn get_total_available(
            &self,
            _event_id: &EventId,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u32, String>> + Send + '_>> {
            Box::pin(async move { Ok(0) })
        }
    }

    fn create_test_env() -> EventInventorySagaEnvironment {
        // Placeholder factory functions for testing
        let create_event_store = Arc::new(|_event_id: EventId| {
            let event_env = EventEnvironment::new(
                Arc::new(SystemClock),
                Arc::new(InMemoryEventStore::new()),
                StreamId::new("test-event"),
                Arc::new(MockEventQuery),
                crate::types::GlobalActionChannels {
                    event_actions: tokio::sync::broadcast::channel(1000).0,
                    inventory_actions: tokio::sync::broadcast::channel(1000).0,
                    reservation_actions: tokio::sync::broadcast::channel(1000).0,
                    payment_actions: tokio::sync::broadcast::channel(1000).0,
                },
            );
            Store::new(EventState::new(), EventReducer::new(), event_env)
        });

        let create_inventory_store = Arc::new(|_event_id: EventId| {
            let inventory_env = InventoryEnvironment::new(
                Arc::new(SystemClock),
                Arc::new(InMemoryEventStore::new()),
                StreamId::new("test-inventory"),
                Arc::new(MockInventoryQuery),
                Arc::new(MockEventQuery),
                crate::types::GlobalActionChannels {
                    event_actions: tokio::sync::broadcast::channel(1000).0,
                    inventory_actions: tokio::sync::broadcast::channel(1000).0,
                    reservation_actions: tokio::sync::broadcast::channel(1000).0,
                    payment_actions: tokio::sync::broadcast::channel(1000).0,
                },
            );
            Store::new(InventoryState::new(), InventoryReducer::new(), inventory_env)
        });

        EventInventorySagaEnvironment::new(
            Arc::new(SystemClock),
            Arc::new(InMemoryEventStore::new()),
            StreamId::new("test-saga"),
            create_event_store,
            create_inventory_store,
        )
    }

    fn create_test_venue() -> Venue {
        Venue::new(
            "Test Venue".to_string(),
            Capacity::new(200),
            vec![
                VenueSection::new(
                    "VIP".to_string(),
                    Capacity::new(50),
                    SeatType::GeneralAdmission,
                ),
                VenueSection::new(
                    "General".to_string(),
                    Capacity::new(150),
                    SeatType::GeneralAdmission,
                ),
            ],
        )
    }

    fn create_test_pricing_tiers() -> Vec<PricingTier> {
        vec![PricingTier::new(
            crate::types::TierType::Regular,
            "General".to_string(),
            Money::from_dollars(50),
            Utc::now(),
            None,
        )]
    }

    #[test]
    fn test_create_event_with_inventory_initiates_saga() {
        let event_id = EventId::new();
        let venue = create_test_venue();

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state(EventInventorySagaState::new())
            .when_action(EventInventorySagaAction::CreateEventWithInventory {
                event_id,
                name: "Test Event".to_string(),
                owner_id: UserId::new(),
                venue,
                date: crate::types::EventDate::new(Utc::now() + Duration::days(30)),
                pricing_tiers: create_test_pricing_tiers(),
            })
            .then_state(move |state| {
                assert_eq!(state.event_id, Some(event_id));
                assert!(!state.event_created);
                assert!(!state.completed);
                assert!(state.pending_sections.is_empty()); // Not yet populated
            })
            .then_effects(|effects| {
                // Should return:
                // - 1 effect for EventCreationInitiated (AppendEvents only, no PublishEvent with direct orchestration)
                // - 1 effect for orchestrating Event + Inventory creation (Effect::Future)
                assert_eq!(effects.len(), 2);
            })
            .run();
    }

    // NOTE: The following tests were removed as they test obsolete choreography-based flows.
    // With direct orchestration, EventCreated and SectionInventoryInitialized events are
    // no longer emitted. The CreateEventWithInventory handler orchestrates everything directly.
    //
    // Removed tests:
    // - test_event_created_triggers_inventory_initialization
    // - test_section_initialized_removes_from_pending
    // - test_last_section_initialized_completes_saga

    #[test]
    fn test_validation_fails_for_empty_name() {
        let event_id = EventId::new();

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state(EventInventorySagaState::new())
            .when_action(EventInventorySagaAction::CreateEventWithInventory {
                event_id,
                name: String::new(), // Empty name
                owner_id: UserId::new(),
                venue: create_test_venue(),
                date: crate::types::EventDate::new(Utc::now() + Duration::days(30)),
                pricing_tiers: create_test_pricing_tiers(),
            })
            .then_state(|state| {
                assert!(state.event_id.is_none());
                assert!(state.last_error.is_some());
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("cannot be empty"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_validation_fails_for_no_sections() {
        let event_id = EventId::new();
        let mut venue = create_test_venue();
        venue.sections.clear(); // No sections

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state(EventInventorySagaState::new())
            .when_action(EventInventorySagaAction::CreateEventWithInventory {
                event_id,
                name: "Test Event".to_string(),
                owner_id: UserId::new(),
                venue,
                date: crate::types::EventDate::new(Utc::now() + Duration::days(30)),
                pricing_tiers: create_test_pricing_tiers(),
            })
            .then_state(|state| {
                assert!(state.last_error.is_some());
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("at least one section"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_saga_already_initiated_fails() {
        let event_id = EventId::new();

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = EventInventorySagaState::new();
                state.event_id = Some(event_id); // Already initiated
                state
            })
            .when_action(EventInventorySagaAction::CreateEventWithInventory {
                event_id,
                name: "Test Event".to_string(),
                owner_id: UserId::new(),
                venue: create_test_venue(),
                date: crate::types::EventDate::new(Utc::now() + Duration::days(30)),
                pricing_tiers: create_test_pricing_tiers(),
            })
            .then_state(|state| {
                assert!(state.last_error.is_some());
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("already initiated"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }
}