//! Event-Inventory Saga
//!
//! **Responsibility**: Coordinate event creation with automatic inventory initialization.
//!
//! **Why a saga?**: Creating an event REQUIRES inventory. Without inventory, customers
//! can't make reservations. This is a multi-aggregate transaction that must be coordinated.
//!
//! **Pattern**: This saga uses the **feedback loop** pattern where each step in the workflow
//! generates an action that feeds back into the reducer, making the state machine explicit.
//!
//! **Flow** (Feedback Loop):
//! ```text
//! 1. CreateEventWithInventory command
//!    → Emit EventCreationInitiated
//!    → Effect: Call Event aggregate
//!    → Returns: EventCreated action (feeds back to reducer)
//!
//! 2. EventCreated action (from effect callback)
//!    → Emit EventCreated to event store
//!    → Effects: N inventory calls (one per section, in parallel)
//!    → Each returns: SectionInventoryInitialized action
//!
//! 3. SectionInventoryInitialized action (for each section)
//!    → Emit SectionInventoryInitialized to event store
//!    → Track completion in state
//!    → When all done: returns EventCreationCompleted action
//!
//! 4. EventCreationCompleted action
//!    → Emit EventCreationCompleted to event store
//!    → Saga complete!
//! ```
//!
//! **Benefits of this pattern**:
//! - Each step is explicitly modeled in the reducer
//! - State machine transitions are visible and testable
//! - Event history shows full trace of what happened
//! - Crash recovery: replay events to restore state

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
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// ============================================================================
// State
// ============================================================================

/// Saga state for a SINGLE event creation workflow
///
/// Each saga instance handles ONE event creation. When the workflow completes,
/// the saga is done. State is persisted via events in the saga's event stream.
#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)] // Saga state machines need multiple status flags
pub struct EventInventorySagaState {
    /// The event being created (None if not yet initiated)
    pub event_id: Option<EventId>,
    /// Sections that still need inventory initialized
    pub pending_sections: HashSet<String>,
    /// Section capacities from the venue (`section_name` -> capacity)
    pub section_capacities: HashMap<String, Capacity>,
    /// Whether the Event aggregate has created the event
    pub event_created: bool,
    /// Whether all inventory has been initialized
    pub inventory_complete: bool,
    /// Whether the entire saga completed
    pub completed: bool,
    /// Whether the saga failed
    pub failed: bool,
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
            section_capacities: HashMap::new(),
            event_created: false,
            inventory_complete: false,
            completed: false,
            failed: false,
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
        self.event_id.is_some() && !self.completed && !self.failed
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
/// Each event in this enum represents either a command (input) or an event
/// (state transition) in the saga's lifecycle.
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

    // ========== EVENTS (saga lifecycle - each feeds back through the reducer) ==========

    /// Saga initiated - event creation started
    #[event]
    EventCreationInitiated {
        /// Event ID
        event_id: EventId,
        /// Event name
        name: String,
        /// Number of sections that need inventory
        section_count: u32,
        /// Section capacities from the venue (`section_name` -> capacity)
        section_capacities: HashMap<String, Capacity>,
        /// When initiated
        initiated_at: DateTime<Utc>,
    },

    /// Event aggregate created the event successfully (feedback from Effect)
    #[event]
    EventCreated {
        /// Event ID
        event_id: EventId,
        /// Sections that need inventory initialization
        sections: Vec<String>,
        /// When event was created
        created_at: DateTime<Utc>,
    },

    /// Inventory initialized for a section (feedback from Effect)
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

    /// Serialization failed
    #[event]
    SerializationFailed {
        /// Error message
        error: String,
    },

    /// Stream version updated (internal bookkeeping)
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
/// and sends commands directly. Effects return actions that feed back to reducer.
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
/// This saga uses the **feedback loop** pattern:
/// - Each step processes an action and returns effects
/// - Effects call external services and return NEW actions
/// - Those actions feed back into the reducer
/// - This makes every state transition explicit and testable
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
    /// Uses `broadcast_on_success` to broadcast the event AFTER successful persistence.
    /// This allows `send_and_wait_for` to detect completion WITHOUT re-entering the reducer.
    fn create_persist_effects(
        event: EventInventorySagaAction,
        expected_version: Version,
        env: &EventInventorySagaEnvironment,
    ) -> Effect<EventInventorySagaAction> {
        let ticketing_event = TicketingEvent::EventInventorySaga(event.clone());
        let serialized = match ticketing_event.serialize() {
            Ok(s) => s,
            Err(e) => {
                return Effect::Future(Box::pin(async move {
                    Some(EventInventorySagaAction::SerializationFailed {
                        error: format!("Failed to serialize saga event: {e}"),
                    })
                }));
            }
        };

        append_events! {
            store: env.event_store,
            stream: env.stream_id.as_str(),
            expected_version: Some(expected_version),
            events: vec![serialized],
            broadcast_on_success: event,  // Broadcast AFTER persist, NO re-entry
            on_success: |version| Some(EventInventorySagaAction::VersionUpdated { version }),
            on_error: |error| Some(EventInventorySagaAction::ValidationFailed {
                error: error.to_string()
            })
        }
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

    /// Applies an event to state (pure state transition)
    fn apply_event(state: &mut EventInventorySagaState, action: &EventInventorySagaAction) {
        match action {
            EventInventorySagaAction::EventCreationInitiated {
                event_id,
                section_capacities,
                ..
            } => {
                state.event_id = Some(*event_id);
                state.section_capacities.clone_from(section_capacities);
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

            EventInventorySagaAction::EventCreationFailed { error, .. } => {
                state.failed = true;
                state.last_error = Some(error.clone());
            }

            EventInventorySagaAction::InventoryInitializationFailed { error, section, .. } => {
                state.failed = true;
                state.pending_sections.remove(section); // Mark as handled (failed)
                state.last_error = Some(error.clone());
            }

            EventInventorySagaAction::ValidationFailed { error }
            | EventInventorySagaAction::SerializationFailed { error } => {
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

    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)] // Saga state machine has inherently complex branching
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ================================================================
            // Step 1: COMMAND - Create Event With Inventory
            // ================================================================
            // This is the entry point. We validate, emit EventCreationInitiated,
            // and return an effect that calls the Event aggregate.
            // The effect returns EventCreated on success (feeds back to reducer).
            // ================================================================
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
                    "Saga Step 1: CreateEventWithInventory command received"
                );

                // Validate
                if let Err(error) = Self::validate_create_event(state, &name, &venue) {
                    Self::apply_event(state, &EventInventorySagaAction::ValidationFailed { error });
                    return SmallVec::new();
                }

                // Build section capacities for state
                let section_capacities: HashMap<String, Capacity> = venue
                    .sections
                    .iter()
                    .map(|s| (s.name.clone(), s.capacity))
                    .collect();

                // Create saga initiation event
                #[allow(clippy::cast_possible_truncation)] // sections.len() won't exceed u32::MAX
                let section_count = venue.sections.len() as u32;
                let initiated = EventInventorySagaAction::EventCreationInitiated {
                    event_id,
                    name: name.clone(),
                    section_count,
                    section_capacities,
                    initiated_at: env.clock.now(),
                };

                let expected_version = state.version;
                Self::apply_event(state, &initiated);

                // Persist saga event (uses broadcast_on_success for send_and_wait_for)
                let mut effects: SmallVec<[Effect<Self::Action>; 4]> = smallvec![Self::create_persist_effects(initiated, expected_version, env)];

                // Create effect to call Event aggregate
                // On completion, this returns EventCreated which feeds back to reducer
                let create_event_store = env.create_event_store.clone();
                let now = env.clock.now();
                let sections: Vec<String> = venue.sections.iter().map(|s| s.name.clone()).collect();

                effects.push(Effect::Future(Box::pin(async move {
                    tracing::debug!(
                        event_id = %event_id.as_uuid(),
                        "Saga: Calling Event aggregate"
                    );

                    let event_store = create_event_store(event_id);

                    let create_action = EventAction::CreateEvent {
                        id: event_id,
                        name,
                        owner_id,
                        venue,
                        date,
                        pricing_tiers,
                        respond_to: ResponseChannel::none(),
                    };

                    match event_store.send(create_action).await {
                        Ok(_handle) => {
                            tracing::info!(
                                event_id = %event_id.as_uuid(),
                                "Saga: Event aggregate succeeded, returning EventCreated"
                            );
                            // This action feeds back into the reducer!
                            Some(EventInventorySagaAction::EventCreated {
                                event_id,
                                sections,
                                created_at: now,
                            })
                        }
                        Err(e) => {
                            tracing::error!(
                                event_id = %event_id.as_uuid(),
                                error = %e,
                                "Saga: Event aggregate failed"
                            );
                            Some(EventInventorySagaAction::EventCreationFailed {
                                event_id,
                                error: e.to_string(),
                                failed_at: now,
                            })
                        }
                    }
                })));

                effects
            }

            // ================================================================
            // EVENT: EventCreationInitiated (persisted in Step 1, replayed on recovery)
            // ================================================================
            // When replaying events, we need to handle this. It's already applied
            // to state during Step 1, so during replay we just apply it again.
            // No effects needed since it's already been persisted.
            // ================================================================
            ref action @ EventInventorySagaAction::EventCreationInitiated { ref event_id, .. } => {
                tracing::debug!(
                    event_id = %event_id.as_uuid(),
                    "Saga: EventCreationInitiated replayed (recovery)"
                );

                // Apply original event to state (idempotent)
                Self::apply_event(state, action);

                // No effects - this event has already been persisted
                SmallVec::new()
            }

            // ================================================================
            // Step 2: EVENT - Event Created (Feedback from Step 1)
            // ================================================================
            // The Event aggregate succeeded. Now we kick off inventory
            // initialization for each section IN PARALLEL.
            // Each effect returns SectionInventoryInitialized (feeds back).
            // ================================================================
            EventInventorySagaAction::EventCreated {
                event_id,
                sections,
                created_at,
            } => {
                tracing::info!(
                    event_id = %event_id.as_uuid(),
                    sections = ?sections,
                    "Saga Step 2: EventCreated - initializing inventory for {} sections",
                    sections.len()
                );

                let expected_version = state.version;

                // Apply to state
                let event = EventInventorySagaAction::EventCreated {
                    event_id,
                    sections: sections.clone(),
                    created_at,
                };
                Self::apply_event(state, &event);

                // Persist the EventCreated event (uses broadcast_on_success)
                let mut effects: SmallVec<[Effect<Self::Action>; 4]> = smallvec![Self::create_persist_effects(event, expected_version, env)];

                // Create one effect per section to initialize inventory (in parallel)
                let create_inventory_store = env.create_inventory_store.clone();
                let now = env.clock.now();

                for section_name in &sections {
                    let section = section_name.clone();
                    let capacity = state.section_capacities.get(&section).copied()
                        .unwrap_or(Capacity::new(0));
                    let store_factory = create_inventory_store.clone();
                    let eid = event_id;
                    let ts = now;

                    tracing::debug!(
                        event_id = %eid.as_uuid(),
                        section = %section,
                        capacity = capacity.value(),
                        "Saga: Creating inventory initialization effect"
                    );

                    effects.push(Effect::Future(Box::pin(async move {
                        let inventory_store = store_factory(eid);

                        let init_action = InventoryAction::InitializeInventory {
                            event_id: eid,
                            section: section.clone(),
                            capacity,
                            seat_numbers: None,
                            respond_to: ResponseChannel::none(),
                        };

                        match inventory_store.send(init_action).await {
                            Ok(_handle) => {
                                tracing::info!(
                                    event_id = %eid.as_uuid(),
                                    section = %section,
                                    "Saga: Inventory initialized, returning SectionInventoryInitialized"
                                );
                                // This action feeds back into the reducer!
                                Some(EventInventorySagaAction::SectionInventoryInitialized {
                                    event_id: eid,
                                    section,
                                    initialized_at: ts,
                                })
                            }
                            Err(e) => {
                                tracing::error!(
                                    event_id = %eid.as_uuid(),
                                    section = %section,
                                    error = %e,
                                    "Saga: Inventory initialization failed"
                                );
                                Some(EventInventorySagaAction::InventoryInitializationFailed {
                                    event_id: eid,
                                    section,
                                    error: e.to_string(),
                                    failed_at: ts,
                                })
                            }
                        }
                    })));
                }

                effects
            }

            // ================================================================
            // Step 3: EVENT - Section Inventory Initialized (Feedback from Step 2)
            // ================================================================
            // One section's inventory is ready. Track progress.
            // When ALL sections are done, return EventCreationCompleted.
            // ================================================================
            EventInventorySagaAction::SectionInventoryInitialized {
                event_id,
                section,
                initialized_at,
            } => {
                // Idempotency: skip if section not in pending
                if !state.pending_sections.contains(&section) {
                    tracing::debug!(
                        event_id = %event_id.as_uuid(),
                        section = %section,
                        "Saga: Section already processed, skipping"
                    );
                    return SmallVec::new();
                }

                tracing::info!(
                    event_id = %event_id.as_uuid(),
                    section = %section,
                    remaining = state.pending_sections.len() - 1,
                    "Saga Step 3: SectionInventoryInitialized"
                );

                let expected_version = state.version;

                // Apply to state
                let event = EventInventorySagaAction::SectionInventoryInitialized {
                    event_id,
                    section,
                    initialized_at,
                };
                Self::apply_event(state, &event);

                // Persist the event (uses broadcast_on_success)
                let mut effects: SmallVec<[Effect<Self::Action>; 4]> = smallvec![Self::create_persist_effects(event, expected_version, env)];

                // Check if ALL sections are done
                if state.pending_sections.is_empty() && state.event_created && !state.failed {
                    tracing::info!(
                        event_id = %event_id.as_uuid(),
                        "Saga: All sections initialized, triggering completion"
                    );

                    // Return completion action (feeds back to reducer)
                    #[allow(clippy::cast_possible_truncation)]
                    let sections_initialized = state.section_capacities.len() as u32;
                    let completed_at = env.clock.now();

                    effects.push(Effect::Future(Box::pin(async move {
                        Some(EventInventorySagaAction::EventCreationCompleted {
                            event_id,
                            sections_initialized,
                            completed_at,
                        })
                    })));
                }

                effects
            }

            // ================================================================
            // Step 4: EVENT - Event Creation Completed (Final Step)
            // ================================================================
            // All inventory initialized. Saga is complete!
            // Uses broadcast_on_success to signal completion without re-entry.
            // ================================================================
            EventInventorySagaAction::EventCreationCompleted {
                event_id,
                sections_initialized,
                completed_at,
            } => {
                tracing::info!(
                    event_id = %event_id.as_uuid(),
                    sections = sections_initialized,
                    "Saga Step 4: EventCreationCompleted - saga finished!"
                );

                let expected_version = state.version;

                let event = EventInventorySagaAction::EventCreationCompleted {
                    event_id,
                    sections_initialized,
                    completed_at,
                };
                Self::apply_event(state, &event);

                // Persist and broadcast for send_and_wait_for (no re-entry)
                smallvec![Self::create_persist_effects(event, expected_version, env)]
            }

            // ================================================================
            // ERROR: Event Creation Failed
            // ================================================================
            EventInventorySagaAction::EventCreationFailed {
                event_id,
                error,
                failed_at,
            } => {
                tracing::error!(
                    event_id = %event_id.as_uuid(),
                    error = %error,
                    "Saga: Event creation failed"
                );

                let expected_version = state.version;

                let event = EventInventorySagaAction::EventCreationFailed {
                    event_id,
                    error,
                    failed_at,
                };
                Self::apply_event(state, &event);

                // Persist and broadcast for send_and_wait_for (no re-entry)
                smallvec![Self::create_persist_effects(event, expected_version, env)]
            }

            // ================================================================
            // ERROR: Inventory Initialization Failed
            // ================================================================
            EventInventorySagaAction::InventoryInitializationFailed {
                event_id,
                section,
                error,
                failed_at,
            } => {
                tracing::error!(
                    event_id = %event_id.as_uuid(),
                    section = %section,
                    error = %error,
                    "Saga: Inventory initialization failed"
                );

                let expected_version = state.version;

                let event = EventInventorySagaAction::InventoryInitializationFailed {
                    event_id,
                    section,
                    error,
                    failed_at,
                };
                Self::apply_event(state, &event);

                // Persist and broadcast for send_and_wait_for (no re-entry)
                smallvec![Self::create_persist_effects(event, expected_version, env)]
            }

            // ================================================================
            // UTILITY: Internal bookkeeping events (no effects needed)
            // ================================================================
            ref action @ (EventInventorySagaAction::VersionUpdated { .. }
            | EventInventorySagaAction::ValidationFailed { .. }
            | EventInventorySagaAction::SerializationFailed { .. }) => {
                Self::apply_event(state, action);
                SmallVec::new()
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::test_utils::{
        create_test_global_channels, MockEventProjectionQuery, MockInventoryQuery,
    };
    use crate::types::{Money, SeatType, TierType, VenueSection};
    use chrono::Duration;
    use composable_rust_core::environment::SystemClock;
    use composable_rust_testing::{
        assertions,
        mocks::InMemoryEventStore,
        ReducerTest,
    };

    fn create_test_env() -> EventInventorySagaEnvironment {
        // Factory functions for creating child aggregate stores
        let create_event_store = Arc::new(|_event_id: EventId| {
            let event_env = EventEnvironment::new(
                Arc::new(SystemClock),
                Arc::new(InMemoryEventStore::new()),
                StreamId::new("test-event"),
                Arc::new(MockEventProjectionQuery),
                create_test_global_channels(),
            );
            Store::new(EventState::new(), EventReducer::new(), event_env)
        });

        let create_inventory_store = Arc::new(|_event_id: EventId| {
            let inventory_env = InventoryEnvironment::new(
                Arc::new(SystemClock),
                Arc::new(InMemoryEventStore::new()),
                StreamId::new("test-inventory"),
                Arc::new(MockInventoryQuery),
                Arc::new(MockEventProjectionQuery),
                create_test_global_channels(),
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

    /// Returns a fixed test time for deterministic tests
    fn test_time() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp")
    }

    fn create_test_pricing_tiers() -> Vec<PricingTier> {
        vec![PricingTier::new(
            TierType::Regular,
            "General".to_string(),
            Money::from_dollars(50),
            test_time(),
            None,
        )]
    }

    // ========================================================================
    // Step 1: CreateEventWithInventory Tests
    // ========================================================================

    #[test]
    fn test_step1_create_event_initiates_saga() {
        let event_id = EventId::new();
        let venue = create_test_venue();

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state(EventInventorySagaState::new())
            .when_action(EventInventorySagaAction::CreateEventWithInventory {
                event_id,
                name: "Test Event".to_string(),
                owner_id: UserId::new(),
                venue: venue.clone(),
                date: crate::types::EventDate::new(test_time() + Duration::days(30)),
                pricing_tiers: create_test_pricing_tiers(),
            })
            .then_state(move |state| {
                // State should have event_id and section_capacities
                assert_eq!(state.event_id, Some(event_id));
                assert!(!state.event_created); // Not yet - waiting for EventCreated
                assert!(!state.completed);
                assert!(state.pending_sections.is_empty()); // Populated by EventCreated
                assert_eq!(state.section_capacities.len(), 2); // VIP + General
            })
            .then_effects(|effects| {
                // Should have:
                // - 1 persist effect for EventCreationInitiated (uses broadcast_on_success)
                // - 1 future effect to call Event aggregate
                assert_eq!(effects.len(), 2);
            })
            .run();
    }

    // ========================================================================
    // Step 2: EventCreated Tests
    // ========================================================================

    #[test]
    fn test_step2_event_created_starts_inventory_initialization() {
        let event_id = EventId::new();

        // Start with state after Step 1 (EventCreationInitiated applied)
        let mut initial_state = EventInventorySagaState::new();
        initial_state.event_id = Some(event_id);
        initial_state.section_capacities.insert("VIP".to_string(), Capacity::new(50));
        initial_state.section_capacities.insert("General".to_string(), Capacity::new(150));

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state(initial_state)
            .when_action(EventInventorySagaAction::EventCreated {
                event_id,
                sections: vec!["VIP".to_string(), "General".to_string()],
                created_at: test_time(),
            })
            .then_state(move |state| {
                assert!(state.event_created);
                assert_eq!(state.pending_sections.len(), 2);
                assert!(state.pending_sections.contains("VIP"));
                assert!(state.pending_sections.contains("General"));
                assert!(!state.completed);
            })
            .then_effects(|effects| {
                // Should have:
                // - 1 persist effect for EventCreated (uses broadcast_on_success)
                // - 2 future effects (one per section) for inventory initialization
                assert_eq!(effects.len(), 3);
            })
            .run();
    }

    // ========================================================================
    // Step 3: SectionInventoryInitialized Tests
    // ========================================================================

    #[test]
    fn test_step3_section_initialized_not_last_continues() {
        let event_id = EventId::new();

        // Start with state after EventCreated
        let mut initial_state = EventInventorySagaState::new();
        initial_state.event_id = Some(event_id);
        initial_state.event_created = true;
        initial_state.pending_sections.insert("VIP".to_string());
        initial_state.pending_sections.insert("General".to_string());
        initial_state.section_capacities.insert("VIP".to_string(), Capacity::new(50));
        initial_state.section_capacities.insert("General".to_string(), Capacity::new(150));

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state(initial_state)
            .when_action(EventInventorySagaAction::SectionInventoryInitialized {
                event_id,
                section: "VIP".to_string(),
                initialized_at: test_time(),
            })
            .then_state(|state| {
                // VIP removed, General still pending
                assert!(!state.pending_sections.contains("VIP"));
                assert!(state.pending_sections.contains("General"));
                assert_eq!(state.pending_sections.len(), 1);
                assert!(!state.completed); // Not yet!
                assert!(!state.inventory_complete);
            })
            .then_effects(|effects| {
                // 1 persist effect (uses broadcast_on_success) - no completion yet
                assert_eq!(effects.len(), 1);
            })
            .run();
    }

    #[test]
    fn test_step3_last_section_triggers_completion() {
        let event_id = EventId::new();

        // Start with state where only one section is left
        let mut initial_state = EventInventorySagaState::new();
        initial_state.event_id = Some(event_id);
        initial_state.event_created = true;
        initial_state.pending_sections.insert("General".to_string()); // Only one left
        initial_state.section_capacities.insert("VIP".to_string(), Capacity::new(50));
        initial_state.section_capacities.insert("General".to_string(), Capacity::new(150));

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state(initial_state)
            .when_action(EventInventorySagaAction::SectionInventoryInitialized {
                event_id,
                section: "General".to_string(),
                initialized_at: test_time(),
            })
            .then_state(|state| {
                assert!(state.pending_sections.is_empty());
                assert!(state.inventory_complete);
                assert!(!state.completed); // Completion happens in next step
            })
            .then_effects(|effects| {
                // Should have:
                // - 1 persist effect (uses broadcast_on_success)
                // - 1 future effect that returns EventCreationCompleted
                assert_eq!(effects.len(), 2);
            })
            .run();
    }

    // ========================================================================
    // Step 4: EventCreationCompleted Tests
    // ========================================================================

    #[test]
    fn test_step4_completion_finishes_saga() {
        let event_id = EventId::new();

        let mut initial_state = EventInventorySagaState::new();
        initial_state.event_id = Some(event_id);
        initial_state.event_created = true;
        initial_state.inventory_complete = true;
        initial_state.section_capacities.insert("VIP".to_string(), Capacity::new(50));

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state(initial_state)
            .when_action(EventInventorySagaAction::EventCreationCompleted {
                event_id,
                sections_initialized: 2,
                completed_at: test_time(),
            })
            .then_state(|state| {
                assert!(state.completed);
                assert!(state.is_complete());
            })
            .then_effects(|effects| {
                // 1 persist effect (uses broadcast_on_success)
                assert_eq!(effects.len(), 1);
            })
            .run();
    }

    // ========================================================================
    // Error Handling Tests
    // ========================================================================

    #[test]
    fn test_event_creation_failed_marks_saga_failed() {
        let event_id = EventId::new();

        let mut initial_state = EventInventorySagaState::new();
        initial_state.event_id = Some(event_id);

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state(initial_state)
            .when_action(EventInventorySagaAction::EventCreationFailed {
                event_id,
                error: "Database error".to_string(),
                failed_at: test_time(),
            })
            .then_state(|state| {
                assert!(state.failed);
                assert!(!state.completed);
                assert!(state.last_error.is_some());
                assert!(state.last_error.as_ref().unwrap().contains("Database"));
            })
            .run();
    }

    #[test]
    fn test_inventory_failed_marks_saga_failed() {
        let event_id = EventId::new();

        let mut initial_state = EventInventorySagaState::new();
        initial_state.event_id = Some(event_id);
        initial_state.event_created = true;
        initial_state.pending_sections.insert("VIP".to_string());
        initial_state.pending_sections.insert("General".to_string());

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state(initial_state)
            .when_action(EventInventorySagaAction::InventoryInitializationFailed {
                event_id,
                section: "VIP".to_string(),
                error: "Capacity exceeded".to_string(),
                failed_at: test_time(),
            })
            .then_state(|state| {
                assert!(state.failed);
                assert!(!state.completed);
                // Failed section is removed from pending (marked as handled)
                assert!(!state.pending_sections.contains("VIP"));
                assert!(state.last_error.is_some());
            })
            .run();
    }

    // ========================================================================
    // Validation Tests
    // ========================================================================

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
                date: crate::types::EventDate::new(test_time() + Duration::days(30)),
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
                date: crate::types::EventDate::new(test_time() + Duration::days(30)),
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
                date: crate::types::EventDate::new(test_time() + Duration::days(30)),
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

    // ========================================================================
    // Idempotency Tests
    // ========================================================================

    #[test]
    fn test_duplicate_section_initialized_is_idempotent() {
        let event_id = EventId::new();

        // State where VIP is already processed (not in pending)
        let mut initial_state = EventInventorySagaState::new();
        initial_state.event_id = Some(event_id);
        initial_state.event_created = true;
        initial_state.pending_sections.insert("General".to_string()); // Only General pending

        ReducerTest::new(EventInventorySaga::new())
            .with_env(create_test_env())
            .given_state(initial_state)
            .when_action(EventInventorySagaAction::SectionInventoryInitialized {
                event_id,
                section: "VIP".to_string(), // Already processed!
                initialized_at: test_time(),
            })
            .then_state(|state| {
                // State unchanged - VIP was already not in pending
                assert!(!state.pending_sections.contains("VIP"));
                assert!(state.pending_sections.contains("General"));
            })
            .then_effects(assertions::assert_no_effects) // No effects for duplicate
            .run();
    }
}
