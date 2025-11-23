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

use crate::aggregates::{EventAction, InventoryAction};
use crate::projections::TicketingEvent;
use crate::types::{Capacity, EventDate, EventId, PricingTier, Venue};
use chrono::{DateTime, Utc};
use composable_rust_auth::state::UserId;
use composable_rust_core::{
    append_events, effect::Effect, environment::Clock, event_bus::EventBus,
    event_store::EventStore, publish_event, reducer::Reducer, smallvec,
    stream::{StreamId, Version},
    SmallVec,
};
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
/// Contains only side-effect dependencies. No child stores (saga publishes
/// commands to EventBus topics, not direct store calls).
#[derive(Clone)]
pub struct EventInventorySagaEnvironment {
    /// Clock for timestamps
    pub clock: Arc<dyn Clock>,
    /// Event store for saga persistence
    pub event_store: Arc<dyn EventStore>,
    /// Event bus for publishing to child aggregates
    pub event_bus: Arc<dyn EventBus>,
    /// Stream ID for this saga instance
    pub stream_id: StreamId,
}

impl EventInventorySagaEnvironment {
    /// Creates a new environment
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        event_store: Arc<dyn EventStore>,
        event_bus: Arc<dyn EventBus>,
        stream_id: StreamId,
    ) -> Self {
        Self {
            clock,
            event_store,
            event_bus,
            stream_id,
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

    /// Creates effects for persisting and publishing a saga event
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
            },
            publish_event! {
                bus: env.event_bus,
                topic: "event-inventory-saga",
                event: serialized,
                on_success: || None,
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

                // Step 1: Publish CreateEvent command to Event aggregate
                let create_event_cmd = EventAction::CreateEvent {
                    id: event_id,
                    name,
                    owner_id,
                    venue: venue.clone(),
                    date,
                    pricing_tiers,
                };

                let ticketing_event = TicketingEvent::Event(create_event_cmd);
                match ticketing_event.serialize() {
                    Ok(serialized) => {
                        tracing::debug!(
                            event_id = %event_id.as_uuid(),
                            "Publishing CreateEvent command to events topic"
                        );

                        // Clone timestamp for closure
                        let now = env.clock.now();

                        effects.push(publish_event! {
                            bus: env.event_bus,
                            topic: "events",
                            event: serialized,
                            on_success: || None,
                            on_error: |error| Some(EventInventorySagaAction::EventCreationFailed {
                                event_id,
                                error: error.to_string(),
                                failed_at: now,
                            })
                        });
                    }
                    Err(e) => {
                        let failed = EventInventorySagaAction::EventCreationFailed {
                            event_id,
                            error: format!("Failed to serialize CreateEvent: {e}"),
                            failed_at: env.clock.now(),
                        };
                        let expected_version_2 = state.version;
                        Self::apply_event(state, &failed);

                        let mut effects = Self::create_effects(failed.clone(), expected_version_2, env);

                        // Emit as observable action for send_and_wait_for
                        let failed_clone = failed;
                        effects.push(Effect::Future(Box::pin(async move {
                            Some(failed_clone)
                        })));

                        return effects;
                    }
                }

                // NOTE: We wait for EventCreated notification from Event aggregate
                // (handled via EventBus subscription in bootstrap)

                effects
            }

            // ========== Step 2: Event Created - Initialize Inventory ==========
            EventInventorySagaAction::EventCreated {
                event_id,
                sections,
                created_at,
            } => {
                tracing::info!(
                    event_id = %event_id.as_uuid(),
                    sections = sections.len(),
                    "Saga: EventCreated received - initializing inventory for all sections"
                );

                // Clone sections before moving action
                let sections_clone = sections.clone();

                let expected_version = state.version;
                let action_for_apply = EventInventorySagaAction::EventCreated {
                    event_id,
                    sections,
                    created_at,
                };
                Self::apply_event(state, &action_for_apply);

                let mut effects = Self::create_effects(action_for_apply, expected_version, env);

                // For each section, publish InitializeInventory command
                // We need to extract capacity from sections - but sections only have names here
                // The venue info was in the original command, but we need it here
                //
                // SOLUTION: The EventCreated event should include venue sections with capacity
                // For now, we'll work with what we have (section names)
                // In production, EventCreated would include full section details

                for section_name in &sections_clone {
                    // Get capacity from saga state (populated during EventCreationInitiated)
                    let capacity = state
                        .section_capacities
                        .get(section_name)
                        .copied()
                        .unwrap_or_else(|| {
                            tracing::error!(
                                section = %section_name,
                                "Section capacity not found in saga state - using default"
                            );
                            Capacity::new(100) // Fallback
                        });

                    let init_inventory = InventoryAction::InitializeInventory {
                        event_id,
                        section: section_name.clone(),
                        capacity,
                        seat_numbers: None,
                    };

                    let ticketing_event = TicketingEvent::Inventory(init_inventory);
                    match ticketing_event.serialize() {
                        Ok(serialized) => {
                            tracing::debug!(
                                event_id = %event_id.as_uuid(),
                                section = %section_name,
                                "Publishing InitializeInventory to inventory topic"
                            );

                            // Clone values for closure - need Arc for Fn closure
                            let section_for_error = Arc::new(section_name.clone());
                            let now = env.clock.now();

                            effects.push(publish_event! {
                                bus: env.event_bus,
                                topic: "inventory",
                                event: serialized,
                                on_success: || None,
                                on_error: |error| {
                                    let section_clone = (*section_for_error).clone();
                                    Some(
                                        EventInventorySagaAction::InventoryInitializationFailed {
                                            event_id,
                                            section: section_clone,
                                            error: error.to_string(),
                                            failed_at: now,
                                        }
                                    )
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!(
                                event_id = %event_id.as_uuid(),
                                section = %section_name,
                                error = %e,
                                "Failed to serialize InitializeInventory"
                            );
                        }
                    }
                }

                effects
            }

            // ========== Step 3: Section Inventory Initialized ==========
            EventInventorySagaAction::SectionInventoryInitialized {
                event_id,
                section,
                initialized_at,
            } => {
                tracing::debug!(
                    event_id = %event_id.as_uuid(),
                    section = %section,
                    "Saga: Section inventory initialized"
                );

                let expected_version = state.version;
                let action_for_apply = EventInventorySagaAction::SectionInventoryInitialized {
                    event_id,
                    section,
                    initialized_at,
                };
                Self::apply_event(state, &action_for_apply);

                let mut effects = Self::create_effects(action_for_apply, expected_version, env);

                // Check if all sections are complete
                if state.pending_sections.is_empty() && state.inventory_complete {
                    // Count how many sections we initialized
                    // Since we don't track the original count, we'll use 0 as placeholder
                    // In production, we'd track this in state
                    let now = env.clock.now();
                    let completed = EventInventorySagaAction::EventCreationCompleted {
                        event_id,
                        sections_initialized: 0, // TODO: Track in state
                        completed_at: now,
                    };
                    let expected_version_2 = state.version;
                    Self::apply_event(state, &completed);

                    // Persist and publish the completion event
                    effects.extend(Self::create_effects(completed.clone(), expected_version_2, env));

                    // Emit as observable action for send_and_wait_for
                    let completed_clone = completed;
                    effects.push(Effect::Future(Box::pin(async move {
                        Some(completed_clone)
                    })));

                    tracing::info!(
                        event_id = %event_id.as_uuid(),
                        "Saga: Event creation completed - Event + all Inventory initialized"
                    );
                }

                effects
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
    use crate::types::{Money, SeatType, VenueSection};
    use chrono::Duration;
    use composable_rust_core::environment::SystemClock;
    use composable_rust_testing::{
        assertions,
        mocks::{InMemoryEventBus, InMemoryEventStore},
        ReducerTest,
    };

    fn create_test_env() -> EventInventorySagaEnvironment {
        EventInventorySagaEnvironment::new(
            Arc::new(SystemClock),
            Arc::new(InMemoryEventStore::new()),
            Arc::new(InMemoryEventBus::new()),
            StreamId::new("test-saga"),
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
                // - 2 effects for EventCreationInitiated (AppendEvents + PublishEvent)
                // - 1 effect for publishing CreateEvent command to events topic
                assert_eq!(effects.len(), 3);
            })
            .run();
    }

    #[test]
    fn test_event_created_triggers_inventory_initialization() {
        let event_id = EventId::new();

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = EventInventorySagaState::new();
                state.event_id = Some(event_id);
                state
            })
            .when_action(EventInventorySagaAction::EventCreated {
                event_id,
                sections: vec!["VIP".to_string(), "General".to_string()],
                created_at: Utc::now(),
            })
            .then_state(move |state| {
                assert_eq!(state.event_id, Some(event_id));
                assert!(state.event_created);
                assert_eq!(state.pending_sections.len(), 2);
                assert!(state.pending_sections.contains("VIP"));
                assert!(state.pending_sections.contains("General"));
            })
            .then_effects(|effects| {
                // Should return:
                // - 2 effects for EventCreated (AppendEvents + PublishEvent)
                // - 2 effects for publishing InitializeInventory commands (one per section)
                assert_eq!(effects.len(), 4);
            })
            .run();
    }

    #[test]
    fn test_section_initialized_removes_from_pending() {
        let event_id = EventId::new();

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = EventInventorySagaState::new();
                state.event_id = Some(event_id);
                state.event_created = true;
                state.pending_sections.insert("VIP".to_string());
                state.pending_sections.insert("General".to_string());
                state
            })
            .when_action(EventInventorySagaAction::SectionInventoryInitialized {
                event_id,
                section: "VIP".to_string(),
                initialized_at: Utc::now(),
            })
            .then_state(move |state| {
                assert_eq!(state.pending_sections.len(), 1);
                assert!(state.pending_sections.contains("General"));
                assert!(!state.pending_sections.contains("VIP"));
                assert!(!state.completed); // Still have General pending
            })
            .then_effects(|effects| {
                // Should return 2 effects for SectionInventoryInitialized
                assert_eq!(effects.len(), 2);
            })
            .run();
    }

    #[test]
    fn test_last_section_initialized_completes_saga() {
        let event_id = EventId::new();

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = EventInventorySagaState::new();
                state.event_id = Some(event_id);
                state.event_created = true;
                state.pending_sections.insert("VIP".to_string()); // Only one left
                state
            })
            .when_action(EventInventorySagaAction::SectionInventoryInitialized {
                event_id,
                section: "VIP".to_string(),
                initialized_at: Utc::now(),
            })
            .then_state(move |state| {
                assert!(state.pending_sections.is_empty());
                assert!(state.inventory_complete);
                assert!(state.completed);
            })
            .then_effects(|effects| {
                // Should return:
                // - 2 effects for SectionInventoryInitialized (persist + publish)
                // - 2 effects for EventCreationCompleted (persist + publish)
                // - 1 effect for observable completion (Future for send_and_wait_for)
                assert_eq!(effects.len(), 5);
            })
            .run();
    }

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

// ============================================================================
// Saga Consumer Infrastructure
// ============================================================================

/// Spawns background consumers that translate child aggregate events to saga actions.
///
/// This infrastructure wires up the saga's event-driven choreography:
/// - Event aggregate publishes `EventCreated` → Saga receives `EventCreated`
/// - Inventory aggregate publishes `InventoryInitialized` → Saga receives `SectionInventoryInitialized`
///
/// **Why this is needed**: Unlike TCA's Scope pattern where parent owns child state
/// and actions flow synchronously, our sagas coordinate independent aggregates via
/// EventBus. This helper provides the explicit wiring for distributed choreography.
///
/// # Usage
///
/// ```no_run
/// # use std::sync::Arc;
/// # use composable_rust_runtime::Store;
/// # use ticketing::aggregates::event_inventory_saga::*;
/// # async fn example(event_bus: Arc<dyn composable_rust_core::event_bus::EventBus>, saga_store: Arc<Store<EventInventorySagaState, EventInventorySagaAction, EventInventorySagaEnvironment, EventInventorySaga>>) {
/// // After creating saga store
/// spawn_event_inventory_saga_consumers(event_bus.clone(), saga_store.clone());
/// # }
/// ```
pub fn spawn_event_inventory_saga_consumers(
    event_bus: Arc<dyn composable_rust_core::event_bus::EventBus>,
    saga_store: Arc<
        composable_rust_runtime::Store<
            EventInventorySagaState,
            EventInventorySagaAction,
            EventInventorySagaEnvironment,
            EventInventorySaga,
        >,
    >,
) {
    use crate::projections::TicketingEvent;
    use futures::StreamExt;

    // Consumer 1: Event.EventCreated → Saga.EventCreated
    // Listens to "events" topic and translates EventAction::EventCreated to EventInventorySagaAction::EventCreated
    let event_to_saga_bus = event_bus.clone();
    let event_to_saga_store = saga_store.clone();
    tokio::spawn(async move {
        if let Ok(mut stream) = event_to_saga_bus.subscribe(&["events"]).await {
            while let Some(result) = stream.next().await {
                if let Ok(serialized) = result {
                    if let Ok(TicketingEvent::Event(crate::aggregates::EventAction::EventCreated {
                        id,
                        venue,
                        created_at,
                        ..
                    })) = TicketingEvent::deserialize(&serialized)
                    {
                        let section_names: Vec<String> =
                            venue.sections.iter().map(|s| s.name.clone()).collect();

                        let saga_action = EventInventorySagaAction::EventCreated {
                            event_id: id,
                            sections: section_names,
                            created_at,
                        };

                        if let Err(e) = event_to_saga_store.send(saga_action).await {
                            tracing::error!(
                                event_id = %id.as_uuid(),
                                error = %e,
                                "Failed to send EventCreated to saga"
                            );
                        }
                    }
                }
            }
        }
    });

    // Consumer 2: Inventory.InventoryInitialized → Saga.SectionInventoryInitialized
    // Listens to "inventory" topic and translates InventoryAction::InventoryInitialized to EventInventorySagaAction::SectionInventoryInitialized
    let inventory_to_saga_bus = event_bus;
    let inventory_to_saga_store = saga_store;
    tokio::spawn(async move {
        if let Ok(mut stream) = inventory_to_saga_bus.subscribe(&["inventory"]).await {
            while let Some(result) = stream.next().await {
                if let Ok(serialized) = result {
                    if let Ok(TicketingEvent::Inventory(
                        crate::aggregates::InventoryAction::InventoryInitialized {
                            event_id,
                            section,
                            initialized_at,
                            ..
                        },
                    )) = TicketingEvent::deserialize(&serialized)
                    {
                        let saga_action = EventInventorySagaAction::SectionInventoryInitialized {
                            event_id,
                            section: section.clone(),
                            initialized_at,
                        };

                        if let Err(e) = inventory_to_saga_store.send(saga_action).await {
                            tracing::error!(
                                event_id = %event_id.as_uuid(),
                                section = %section,
                                error = %e,
                                "Failed to send SectionInventoryInitialized to saga"
                            );
                        }
                    }
                }
            }
        }
    });
}
