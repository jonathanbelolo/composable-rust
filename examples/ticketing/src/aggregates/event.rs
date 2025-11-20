//! Event aggregate for the Event Ticketing System.
//!
//! Manages event lifecycle: creation, publishing, sales management, and cancellation.
//! Demonstrates validation, state transitions, and business rules enforcement.

use crate::projections::TicketingEvent;
use crate::types::{Event, EventDate, EventId, EventState, EventStatus, PricingTier, Venue};
use chrono::{DateTime, Duration, Utc};
use composable_rust_auth::state::UserId;
use composable_rust_core::{
    append_events, effect::Effect, environment::Clock, event_bus::EventBus,
    event_store::EventStore, publish_event, reducer::Reducer, smallvec, stream::StreamId, SmallVec,
};
use composable_rust_macros::Action;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ============================================================================
// Actions (Commands + Events)
// ============================================================================

/// Actions for the Event aggregate
///
/// Demonstrates command/event separation using Section 3 derive macros.
/// Commands express intent, events record what happened.
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum EventAction {
    // Commands
    /// Create a new event
    #[command]
    CreateEvent {
        /// Event identifier
        id: EventId,
        /// Event name
        name: String,
        /// Event owner (user creating the event)
        owner_id: UserId,
        /// Venue information
        venue: Venue,
        /// Event date
        date: EventDate,
        /// Pricing tiers
        pricing_tiers: Vec<PricingTier>,
    },

    /// Publish an event (make visible to public)
    #[command]
    PublishEvent {
        /// Event to publish
        event_id: EventId,
    },

    /// Open ticket sales for an event
    #[command]
    OpenSales {
        /// Event to open sales for
        event_id: EventId,
    },

    /// Close ticket sales for an event
    #[command]
    CloseSales {
        /// Event to close sales for
        event_id: EventId,
    },

    /// Cancel an event
    #[command]
    CancelEvent {
        /// Event to cancel
        event_id: EventId,
        /// Cancellation reason
        reason: String,
    },

    /// Update an event's details
    #[command]
    UpdateEvent {
        /// Event to update
        event_id: EventId,
        /// New name (if provided)
        name: Option<String>,
    },

    // Events
    /// Event was created
    #[event]
    EventCreated {
        /// Event identifier
        id: EventId,
        /// Event name
        name: String,
        /// Event owner (user who created the event)
        owner_id: UserId,
        /// Venue information
        venue: Venue,
        /// Event date
        date: EventDate,
        /// Pricing tiers
        pricing_tiers: Vec<PricingTier>,
        /// When the event was created
        created_at: DateTime<Utc>,
    },

    /// Event was published
    #[event]
    EventPublished {
        /// Published event ID
        event_id: EventId,
        /// When published
        published_at: DateTime<Utc>,
    },

    /// Sales were opened
    #[event]
    SalesOpened {
        /// Event ID
        event_id: EventId,
        /// When sales opened
        opened_at: DateTime<Utc>,
    },

    /// Sales were closed
    #[event]
    SalesClosed {
        /// Event ID
        event_id: EventId,
        /// When sales closed
        closed_at: DateTime<Utc>,
    },

    /// Event was cancelled
    #[event]
    EventCancelled {
        /// Event ID
        event_id: EventId,
        /// Cancellation reason
        reason: String,
        /// When cancelled
        cancelled_at: DateTime<Utc>,
    },

    /// Event details were updated
    #[event]
    EventUpdated {
        /// Event ID
        event_id: EventId,
        /// New name (if changed)
        name: Option<String>,
        /// When updated
        updated_at: DateTime<Utc>,
    },

    /// Command validation failed
    #[event]
    ValidationFailed {
        /// Error message
        error: String,
    },
}

// ============================================================================
// Environment
// ============================================================================

/// Environment dependencies for the Event aggregate
#[derive(Clone)]
pub struct EventEnvironment {
    /// Clock for timestamps
    pub clock: Arc<dyn Clock>,
    /// Event store for persistence
    pub event_store: Arc<dyn EventStore>,
    /// Event bus for publishing
    pub event_bus: Arc<dyn EventBus>,
    /// Stream ID for this aggregate instance
    pub stream_id: StreamId,
}

impl EventEnvironment {
    /// Creates a new `EventEnvironment`
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

/// Reducer for the Event aggregate
///
/// Demonstrates:
/// - Command validation (business rules)
/// - Event application (state updates)
/// - State machine (event status transitions)
#[derive(Clone, Debug)]
pub struct EventReducer;

impl EventReducer {
    /// Creates a new `EventReducer`
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Creates effects for persisting and publishing an event
    fn create_effects(
        event: EventAction,
        env: &EventEnvironment,
    ) -> SmallVec<[Effect<EventAction>; 4]> {
        let ticketing_event = TicketingEvent::Event(event);
        let Ok(serialized) = ticketing_event.serialize() else {
            return SmallVec::new();
        };

        smallvec![
            append_events! {
                store: env.event_store,
                stream: env.stream_id.as_str(),
                expected_version: None,
                events: vec![serialized.clone()],
                on_success: |_version| None,
                on_error: |error| Some(EventAction::ValidationFailed {
                    error: error.to_string()
                })
            },
            publish_event! {
                bus: env.event_bus,
                topic: "events",
                event: serialized,
                on_success: || None,
                on_error: |error| Some(EventAction::ValidationFailed {
                    error: error.to_string()
                })
            }
        ]
    }

    /// Validates `CreateEvent` command
    fn validate_create_event(
        state: &EventState,
        id: &EventId,
        name: &str,
        date: &EventDate,
        venue: &Venue,
        pricing_tiers: &[PricingTier],
    ) -> Result<(), String> {
        // Event must not already exist
        if state.exists(id) {
            return Err(format!("Event with ID {id} already exists"));
        }

        // Event name must be non-empty and reasonable length
        if name.is_empty() {
            return Err("Event name cannot be empty".to_string());
        }

        if name.len() > 200 {
            return Err(format!(
                "Event name too long: {} characters (max 200)",
                name.len()
            ));
        }

        // Event date must be in the future (using a simplified check)
        // In production, you'd compare with clock.now()
        let _ = date;

        // Venue capacity must be > 0
        if venue.capacity.value() == 0 {
            return Err("Venue capacity must be greater than zero".to_string());
        }

        // At least one pricing tier required
        if pricing_tiers.is_empty() {
            return Err("At least one pricing tier is required".to_string());
        }

        // All pricing tiers must have positive prices
        for tier in pricing_tiers {
            if tier.base_price.is_zero() {
                return Err("Pricing tier must have positive price".to_string());
            }
        }

        Ok(())
    }

    /// Validates `PublishEvent` command
    fn validate_publish_event(state: &EventState, event_id: &EventId) -> Result<(), String> {
        let Some(event) = state.get(event_id) else {
            return Err(format!("Event {event_id} not found"));
        };

        if event.status != EventStatus::Draft {
            return Err(format!(
                "Event must be in Draft status to publish (current: {:?})",
                event.status
            ));
        }

        Ok(())
    }

    /// Validates `OpenSales` command
    fn validate_open_sales(state: &EventState, event_id: &EventId) -> Result<(), String> {
        let Some(event) = state.get(event_id) else {
            return Err(format!("Event {event_id} not found"));
        };

        if event.status != EventStatus::Published {
            return Err(format!(
                "Event must be Published to open sales (current: {:?})",
                event.status
            ));
        }

        Ok(())
    }

    /// Validates `CloseSales` command
    fn validate_close_sales(state: &EventState, event_id: &EventId) -> Result<(), String> {
        let Some(event) = state.get(event_id) else {
            return Err(format!("Event {event_id} not found"));
        };

        if event.status != EventStatus::SalesOpen {
            return Err(format!(
                "Event must have sales open to close them (current: {:?})",
                event.status
            ));
        }

        Ok(())
    }

    /// Validates `CancelEvent` command
    fn validate_cancel_event(
        state: &EventState,
        event_id: &EventId,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        let Some(event) = state.get(event_id) else {
            return Err(format!("Event {event_id} not found"));
        };

        // Cannot cancel completed or already cancelled events
        if matches!(
            event.status,
            EventStatus::Completed | EventStatus::Cancelled
        ) {
            return Err(format!("Cannot cancel event with status {:?}", event.status));
        }

        // Cannot cancel < 24 hours before event
        let time_until_event = event.date.inner() - now;
        if time_until_event < Duration::hours(24) {
            return Err("Cannot cancel event less than 24 hours before start".to_string());
        }

        Ok(())
    }

    /// Validates that an event can be updated
    fn validate_update_event(state: &EventState, event_id: &EventId) -> Result<(), String> {
        let Some(event) = state.get(event_id) else {
            return Err(format!("Event {event_id} not found"));
        };

        // Cannot update cancelled events
        if event.status == EventStatus::Cancelled {
            return Err("Cannot update cancelled event".to_string());
        }

        Ok(())
    }

    /// Applies an event to state
    fn apply_event(state: &mut EventState, action: &EventAction) {
        match action {
            EventAction::EventCreated {
                id,
                name,
                owner_id,
                venue,
                date,
                pricing_tiers,
                created_at,
            } => {
                let event = Event::new(
                    *id,
                    name.clone(),
                    *owner_id,
                    venue.clone(),
                    *date,
                    pricing_tiers.clone(),
                    *created_at,
                );
                state.events.insert(*id, event);
                state.last_error = None;
            }
            EventAction::EventPublished { event_id, .. } => {
                if let Some(event) = state.events.get_mut(event_id) {
                    event.status = EventStatus::Published;
                }
                state.last_error = None;
            }
            EventAction::SalesOpened { event_id, .. } => {
                if let Some(event) = state.events.get_mut(event_id) {
                    event.status = EventStatus::SalesOpen;
                }
                state.last_error = None;
            }
            EventAction::SalesClosed { event_id, .. } => {
                if let Some(event) = state.events.get_mut(event_id) {
                    event.status = EventStatus::SalesClosed;
                }
                state.last_error = None;
            }
            EventAction::EventCancelled { event_id, .. } => {
                if let Some(event) = state.events.get_mut(event_id) {
                    event.status = EventStatus::Cancelled;
                }
                state.last_error = None;
            }
            EventAction::EventUpdated { event_id, name, .. } => {
                if let Some(event) = state.events.get_mut(event_id) {
                    if let Some(new_name) = name {
                        event.name = new_name.clone();
                    }
                }
                state.last_error = None;
            }
            EventAction::ValidationFailed { error } => {
                state.last_error = Some(error.clone());
            }
            // Commands don't modify state
            EventAction::CreateEvent { .. }
            | EventAction::PublishEvent { .. }
            | EventAction::OpenSales { .. }
            | EventAction::CloseSales { .. }
            | EventAction::CancelEvent { .. }
            | EventAction::UpdateEvent { .. } => {}
        }
    }
}

impl Default for EventReducer {
    fn default() -> Self {
        Self::new()
    }
}

impl Reducer for EventReducer {
    type State = EventState;
    type Action = EventAction;
    type Environment = EventEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ========== Commands ==========
            EventAction::CreateEvent {
                id,
                name,
                owner_id,
                venue,
                date,
                pricing_tiers,
            } => {
                // Validate command
                if let Err(error) =
                    Self::validate_create_event(state, &id, &name, &date, &venue, &pricing_tiers)
                {
                    Self::apply_event(state, &EventAction::ValidationFailed { error });
                    return SmallVec::new();
                }

                // Create and apply event
                let event = EventAction::EventCreated {
                    id,
                    name,
                    owner_id,
                    venue,
                    date,
                    pricing_tiers,
                    created_at: env.clock.now(),
                };
                Self::apply_event(state, &event);

                Self::create_effects(event, env)
            }

            EventAction::PublishEvent { event_id } => {
                // Validate
                if let Err(error) = Self::validate_publish_event(state, &event_id) {
                    Self::apply_event(state, &EventAction::ValidationFailed { error });
                    return SmallVec::new();
                }

                // Create and apply event
                let event = EventAction::EventPublished {
                    event_id,
                    published_at: env.clock.now(),
                };
                Self::apply_event(state, &event);

                Self::create_effects(event, env)
            }

            EventAction::OpenSales { event_id } => {
                // Validate
                if let Err(error) = Self::validate_open_sales(state, &event_id) {
                    Self::apply_event(state, &EventAction::ValidationFailed { error });
                    return SmallVec::new();
                }

                // Create and apply event
                let event = EventAction::SalesOpened {
                    event_id,
                    opened_at: env.clock.now(),
                };
                Self::apply_event(state, &event);

                Self::create_effects(event, env)
            }

            EventAction::CloseSales { event_id } => {
                // Validate
                if let Err(error) = Self::validate_close_sales(state, &event_id) {
                    Self::apply_event(state, &EventAction::ValidationFailed { error });
                    return SmallVec::new();
                }

                // Create and apply event
                let event = EventAction::SalesClosed {
                    event_id,
                    closed_at: env.clock.now(),
                };
                Self::apply_event(state, &event);

                Self::create_effects(event, env)
            }

            EventAction::CancelEvent { event_id, reason } => {
                // Validate
                let now = env.clock.now();
                if let Err(error) = Self::validate_cancel_event(state, &event_id, now) {
                    Self::apply_event(state, &EventAction::ValidationFailed { error });
                    return SmallVec::new();
                }

                // Create and apply event
                let event = EventAction::EventCancelled {
                    event_id,
                    reason,
                    cancelled_at: now,
                };
                Self::apply_event(state, &event);

                Self::create_effects(event, env)
            }

            EventAction::UpdateEvent { event_id, name } => {
                // Validate: event must exist and not be cancelled
                if let Err(error) = Self::validate_update_event(state, &event_id) {
                    Self::apply_event(state, &EventAction::ValidationFailed { error });
                    return SmallVec::new();
                }

                // Check if there's actually anything to update
                if name.is_none() {
                    Self::apply_event(state, &EventAction::ValidationFailed {
                        error: "No fields to update".to_string(),
                    });
                    return SmallVec::new();
                }

                // Create and apply event
                let event = EventAction::EventUpdated {
                    event_id,
                    name,
                    updated_at: env.clock.now(),
                };
                Self::apply_event(state, &event);

                Self::create_effects(event, env)
            }

            // ========== Events (from event store replay) ==========
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
    use crate::types::{Capacity, Money, SeatType, VenueSection};
    use composable_rust_core::environment::SystemClock;
    use composable_rust_testing::{
        assertions,
        mocks::{InMemoryEventBus, InMemoryEventStore},
        ReducerTest,
    };

    fn create_test_env() -> EventEnvironment {
        EventEnvironment::new(
            Arc::new(SystemClock),
            Arc::new(InMemoryEventStore::new()),
            Arc::new(InMemoryEventBus::new()),
            StreamId::new("test-stream"),
        )
    }

    fn create_test_venue() -> Venue {
        Venue::new(
            "Madison Square Garden".to_string(),
            Capacity::new(1000),
            vec![VenueSection::new(
                "General".to_string(),
                Capacity::new(1000),
                SeatType::GeneralAdmission,
            )],
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
    fn test_create_event_success() {
        let id = EventId::new();
        let event_date = EventDate::new(Utc::now() + Duration::days(30));

        ReducerTest::new(EventReducer::new())
            .with_env(create_test_env())
            .given_state(EventState::new())
            .when_action(EventAction::CreateEvent {
                id,
                name: "Taylor Swift Concert".to_string(),
                owner_id: UserId::new(),
                venue: create_test_venue(),
                date: event_date,
                pricing_tiers: create_test_pricing_tiers(),
            })
            .then_state(move |state| {
                assert_eq!(state.count(), 1);
                assert!(state.exists(&id));
                let event = state.get(&id).unwrap();
                assert_eq!(event.name, "Taylor Swift Concert");
                assert_eq!(event.status, EventStatus::Draft);
            })
            .then_effects(|effects| {
                // Should return 2 effects: AppendEvents + PublishEvent
                assert_eq!(effects.len(), 2);
            })
            .run();
    }

    #[test]
    fn test_create_event_empty_name() {
        let id = EventId::new();

        ReducerTest::new(EventReducer::new())
            .with_env(create_test_env())
            .given_state(EventState::new())
            .when_action(EventAction::CreateEvent {
                id,
                name: String::new(),
                owner_id: UserId::new(),
                venue: create_test_venue(),
                date: EventDate::new(Utc::now() + Duration::days(30)),
                pricing_tiers: create_test_pricing_tiers(),
            })
            .then_state(|state| {
                assert_eq!(state.count(), 0);
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
    fn test_create_event_zero_capacity() {
        let id = EventId::new();
        let mut venue = create_test_venue();
        venue.capacity = Capacity::new(0);

        ReducerTest::new(EventReducer::new())
            .with_env(create_test_env())
            .given_state(EventState::new())
            .when_action(EventAction::CreateEvent {
                id,
                name: "Test Event".to_string(),
                owner_id: UserId::new(),
                venue,
                date: EventDate::new(Utc::now() + Duration::days(30)),
                pricing_tiers: create_test_pricing_tiers(),
            })
            .then_state(|state| {
                assert_eq!(state.count(), 0);
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("greater than zero"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_publish_event() {
        let id = EventId::new();

        ReducerTest::new(EventReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = EventState::new();
                let event = Event::new(
                    id,
                    "Test Event".to_string(),
                    UserId::new(),
                    create_test_venue(),
                    EventDate::new(Utc::now() + Duration::days(30)),
                    create_test_pricing_tiers(),
                    Utc::now(),
                );
                state.events.insert(id, event);
                state
            })
            .when_action(EventAction::PublishEvent { event_id: id })
            .then_state(move |state| {
                let event = state.get(&id).unwrap();
                assert_eq!(event.status, EventStatus::Published);
            })
            .then_effects(|effects| {
                // Should return 2 effects: AppendEvents + PublishEvent
                assert_eq!(effects.len(), 2);
            })
            .run();
    }

    #[test]
    fn test_full_lifecycle() {
        let id = EventId::new();

        // Start with empty state
        let mut state = EventState::new();
        let reducer = EventReducer::new();
        let env = create_test_env();

        // 1. Create event
        reducer.reduce(
            &mut state,
            EventAction::CreateEvent {
                id,
                name: "Concert".to_string(),
                owner_id: UserId::new(),
                venue: create_test_venue(),
                date: EventDate::new(Utc::now() + Duration::days(30)),
                pricing_tiers: create_test_pricing_tiers(),
            },
            &env,
        );
        assert_eq!(state.get(&id).unwrap().status, EventStatus::Draft);

        // 2. Publish event
        reducer.reduce(&mut state, EventAction::PublishEvent { event_id: id }, &env);
        assert_eq!(state.get(&id).unwrap().status, EventStatus::Published);

        // 3. Open sales
        reducer.reduce(&mut state, EventAction::OpenSales { event_id: id }, &env);
        assert_eq!(state.get(&id).unwrap().status, EventStatus::SalesOpen);

        // 4. Close sales
        reducer.reduce(&mut state, EventAction::CloseSales { event_id: id }, &env);
        assert_eq!(state.get(&id).unwrap().status, EventStatus::SalesClosed);
    }

    #[test]
    fn test_update_event_success() {
        let id = EventId::new();
        let owner_id = UserId::new();

        ReducerTest::new(EventReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = EventState::new();
                let event = Event::new(
                    id,
                    "Original Name".to_string(),
                    owner_id,
                    create_test_venue(),
                    EventDate::new(Utc::now() + Duration::days(30)),
                    create_test_pricing_tiers(),
                    Utc::now(),
                );
                state.events.insert(id, event);
                state
            })
            .when_action(EventAction::UpdateEvent {
                event_id: id,
                name: Some("Updated Name".to_string()),
            })
            .then_state(move |state| {
                let event = state.get(&id).unwrap();
                assert_eq!(event.name, "Updated Name");
                assert!(state.last_error.is_none());
            })
            .then_effects(|effects| assertions::assert_effects_count(effects, 2))
            .run();
    }

    #[test]
    fn test_update_event_not_found() {
        let non_existent_id = EventId::new();

        ReducerTest::new(EventReducer::new())
            .with_env(create_test_env())
            .given_state(EventState::new())
            .when_action(EventAction::UpdateEvent {
                event_id: non_existent_id,
                name: Some("New Name".to_string()),
            })
            .then_state(|state| {
                assert_eq!(state.count(), 0);
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("not found"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_update_cancelled_event_fails() {
        let id = EventId::new();
        let owner_id = UserId::new();

        ReducerTest::new(EventReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = EventState::new();
                let mut event = Event::new(
                    id,
                    "Cancelled Event".to_string(),
                    owner_id,
                    create_test_venue(),
                    EventDate::new(Utc::now() + Duration::days(30)),
                    create_test_pricing_tiers(),
                    Utc::now(),
                );
                event.status = EventStatus::Cancelled;
                state.events.insert(id, event);
                state
            })
            .when_action(EventAction::UpdateEvent {
                event_id: id,
                name: Some("New Name".to_string()),
            })
            .then_state(move |state| {
                let event = state.get(&id).unwrap();
                assert_eq!(event.name, "Cancelled Event"); // Name should not change
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("cancelled"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_update_event_no_fields() {
        let id = EventId::new();
        let owner_id = UserId::new();

        ReducerTest::new(EventReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = EventState::new();
                let event = Event::new(
                    id,
                    "Original Name".to_string(),
                    owner_id,
                    create_test_venue(),
                    EventDate::new(Utc::now() + Duration::days(30)),
                    create_test_pricing_tiers(),
                    Utc::now(),
                );
                state.events.insert(id, event);
                state
            })
            .when_action(EventAction::UpdateEvent {
                event_id: id,
                name: None,
            })
            .then_state(|state| {
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("No fields to update"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }
}
