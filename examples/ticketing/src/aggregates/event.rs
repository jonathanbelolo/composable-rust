//! Event aggregate for the Event Ticketing System.
//!
//! Manages event lifecycle: creation, publishing, sales management, and cancellation.
//! Demonstrates validation, state transitions, and business rules enforcement.

use crate::projections::{EventProjectionQuery, TicketingEvent};
use crate::types::{
    Event, EventDate, EventId, EventState, EventStatus, GlobalActionChannels, PricingTier,
    ResponseChannel, Venue, VenueSection,
};
use chrono::{DateTime, Duration, Utc};
use composable_rust_auth::state::UserId;
use composable_rust_core::{
    append_events, effect::Effect, environment::Clock,
    event_store::EventStore, reducer::Reducer, smallvec,
    stream::{StreamId, Version},
    SmallVec,
};
use composable_rust_macros::Action;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

// ============================================================================
// Actions (Commands + Events)
// ============================================================================

/// Actions for the Event aggregate
///
/// Demonstrates command/event separation using Section 3 derive macros.
/// Commands express intent, events record what happened.
#[derive(Action, Clone, Debug, PartialEq, Serialize, Deserialize)]
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

        /// Response channel for projection completion
        #[serde(skip)]
        respond_to: ResponseChannel,
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

    /// Update pricing tiers for an event
    #[command]
    UpdatePricingTiers {
        /// Event to update
        event_id: EventId,
        /// New pricing tiers (replaces all existing tiers)
        pricing_tiers: Vec<PricingTier>,

        /// Response channel for projection completion
        #[serde(skip)]
        respond_to: ResponseChannel,
    },

    /// Add venue sections to an event
    #[command]
    AddVenueSections {
        /// Event to update
        event_id: EventId,
        /// Sections to add
        sections: Vec<VenueSection>,

        /// Response channel for projection completion
        #[serde(skip)]
        respond_to: ResponseChannel,
    },

    /// Query a single event by ID
    #[command]
    GetEvent {
        /// Event ID to query
        event_id: EventId,
    },

    /// Query events with optional status filter
    #[command]
    ListEvents {
        /// Optional status filter
        status_filter: Option<EventStatus>,
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
        /// Response channel for projection completion signaling
        #[serde(skip)]
        respond_to: ResponseChannel,
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
        /// Response channel for projection completion signaling
        #[serde(skip)]
        respond_to: ResponseChannel,
    },

    /// Pricing tiers were updated
    #[event]
    PricingTiersUpdated {
        /// Event ID
        event_id: EventId,
        /// New pricing tiers
        pricing_tiers: Vec<PricingTier>,
        /// When updated
        updated_at: DateTime<Utc>,
        /// Response channel for projection completion signaling
        #[serde(skip)]
        respond_to: ResponseChannel,
    },

    /// Venue sections were added
    #[event]
    VenueSectionsAdded {
        /// Event ID
        event_id: EventId,
        /// Sections that were added
        sections: Vec<VenueSection>,
        /// When updated
        updated_at: DateTime<Utc>,
        /// Response channel for projection completion signaling
        #[serde(skip)]
        respond_to: ResponseChannel,
    },

    /// Command validation failed
    #[event]
    ValidationFailed {
        /// Error message
        error: String,
    },

    /// Event serialization failed (internal error)
    #[event]
    SerializationFailed {
        /// Error message
        error: String,
    },

    /// Event was queried (query result)
    #[event]
    EventQueried {
        /// Event ID that was queried
        event_id: EventId,
        /// Event data (None if not found)
        event: Option<Event>,
    },

    /// Events were listed (query result)
    #[event]
    EventsListed {
        /// List of events
        events: Vec<Event>,
        /// Status filter that was applied
        status_filter: Option<EventStatus>,
    },

    /// Stream version was updated after successful event append
    #[event]
    VersionUpdated {
        /// New version number
        version: Version,
    },

    /// Projection update confirmed
    #[event]
    EventProjectionConfirmed {
        /// Event ID
        event_id: EventId,
    },

    /// Projection update failed
    #[event]
    EventProjectionFailed {
        /// Event ID
        event_id: EventId,
        /// Failure reason
        reason: String,
    },

    // Internal actions (post-validation execution)
    /// Execute add venue sections after validation (internal)
    #[doc(hidden)]
    ExecuteAddVenueSections {
        /// Event to update
        event_id: EventId,
        /// Sections to add
        sections: Vec<VenueSection>,
        /// Loaded event for state update
        loaded_event: Event,
        /// Current version from event store for optimistic concurrency
        current_version: Version,
    },

    /// Execute update pricing tiers after validation (internal)
    #[doc(hidden)]
    ExecuteUpdatePricingTiers {
        /// Event to update
        event_id: EventId,
        /// New pricing tiers
        pricing_tiers: Vec<PricingTier>,
        /// Loaded event for state update
        loaded_event: Event,
        /// Current version from event store for optimistic concurrency
        current_version: Version,
    },

    /// Execute update event after validation (internal)
    #[doc(hidden)]
    ExecuteUpdateEvent {
        /// Event to update
        event_id: EventId,
        /// New name (if provided)
        name: Option<String>,
        /// Current version from event store for optimistic concurrency
        current_version: Version,
        /// Update timestamp
        updated_at: DateTime<Utc>,
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
    /// Stream ID for this aggregate instance
    pub stream_id: StreamId,
    /// Projection for querying event state
    pub projection: Arc<dyn EventProjectionQuery>,
    /// Global action channels for cross-aggregate coordination
    pub global_actions: GlobalActionChannels,
}

impl EventEnvironment {
    /// Creates a new `EventEnvironment`
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        event_store: Arc<dyn EventStore>,
        stream_id: StreamId,
        projection: Arc<dyn EventProjectionQuery>,
        global_actions: GlobalActionChannels,
    ) -> Self {
        Self {
            clock,
            event_store,
            stream_id,
            projection,
            global_actions,
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

    /// Creates effects for persisting events (`PostgreSQL` only, no Redpanda)
    ///
    /// With direct orchestration, we use local channels for coordination,
    /// so Redpanda publishing is no longer needed.
    fn create_effects(
        event: EventAction,
        expected_version: Version,
        env: &EventEnvironment,
    ) -> SmallVec<[Effect<EventAction>; 4]> {
        let ticketing_event = TicketingEvent::Event(event.clone());
        let serialized = match ticketing_event.serialize() {
            Ok(s) => s,
            Err(e) => {
                // Return an effect that emits SerializationFailed action
                return smallvec![Effect::Future(Box::pin(async move {
                    Some(EventAction::SerializationFailed {
                        error: format!("Failed to serialize event: {e}"),
                    })
                }))];
            }
        };

        smallvec![
            append_events! {
                store: env.event_store,
                stream: env.stream_id.as_str(),
                expected_version: Some(expected_version),
                events: vec![serialized],
                on_success: |version| Some(EventAction::VersionUpdated { version }),
                on_error: |error| Some(EventAction::ValidationFailed {
                    error: error.to_string()
                })
            },
            // Echo the event back as an action so it broadcasts to action_broadcast channel
            // This allows send_and_wait_for to receive it (e.g., EventUpdated, PricingTiersUpdated)
            Effect::Future(Box::pin(async move {
                Some(event)
            }))
        ]
    }


    /// Validates `CreateEvent` command
    ///
    /// Note: Date validation (ensuring event is in the future) should be done
    /// at the caller level where the clock is available.
    fn validate_create_event(
        state: &EventState,
        id: &EventId,
        name: &str,
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

        // All pricing tier sections must exist in venue
        let section_names: HashSet<&str> =
            venue.sections.iter().map(|s| s.name.as_str()).collect();

        for tier in pricing_tiers {
            if !section_names.contains(tier.section.as_str()) {
                return Err(format!(
                    "Pricing tier references non-existent section '{}'",
                    tier.section
                ));
            }
        }

        // All venue sections must have at least one pricing tier
        let sections_with_tiers: HashSet<&str> =
            pricing_tiers.iter().map(|t| t.section.as_str()).collect();

        for section in &venue.sections {
            if !sections_with_tiers.contains(section.name.as_str()) {
                return Err(format!(
                    "Venue section '{}' must have at least one pricing tier",
                    section.name
                ));
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

    /// Validates adding venue sections using a loaded event (for projection-based validation)
    fn validate_add_venue_sections_with_event(
        event: &Event,
        sections: &[VenueSection],
    ) -> Result<(), String> {
        // Cannot add sections to cancelled events
        if event.status == EventStatus::Cancelled {
            return Err("Cannot add sections to cancelled event".to_string());
        }

        // Must provide at least one section
        if sections.is_empty() {
            return Err("At least one section must be provided".to_string());
        }

        // Validate sections don't duplicate existing sections
        let existing_sections: HashSet<&str> =
            event.venue.sections.iter().map(|s| s.name.as_str()).collect();

        for section in sections {
            if existing_sections.contains(section.name.as_str()) {
                return Err(format!(
                    "Section '{}' already exists in event venue",
                    section.name
                ));
            }
        }

        // Validate no duplicate section names within the new sections
        let mut seen_names = HashSet::new();
        for section in sections {
            if !seen_names.insert(section.name.as_str()) {
                return Err(format!(
                    "Duplicate section name '{}' in request",
                    section.name
                ));
            }
        }

        Ok(())
    }

    /// Validates updating pricing tiers using a loaded event (for projection-based validation)
    fn validate_update_pricing_tiers_with_event(
        event: &Event,
        pricing_tiers: &[PricingTier],
    ) -> Result<(), String> {
        // Cannot update cancelled events
        if event.status == EventStatus::Cancelled {
            return Err("Cannot update pricing for cancelled event".to_string());
        }

        // Must provide at least one pricing tier
        if pricing_tiers.is_empty() {
            return Err("At least one pricing tier must be provided".to_string());
        }

        // All sections referenced in pricing tiers must exist
        let section_names: HashSet<&str> =
            event.venue.sections.iter().map(|s| s.name.as_str()).collect();

        for tier in pricing_tiers {
            if !section_names.contains(tier.section.as_str()) {
                return Err(format!(
                    "Section '{}' does not exist in event venue",
                    tier.section
                ));
            }
        }

        // All sections must have at least one pricing tier
        let sections_with_tiers: HashSet<&str> =
            pricing_tiers.iter().map(|t| t.section.as_str()).collect();

        for section in &event.venue.sections {
            if !sections_with_tiers.contains(section.name.as_str()) {
                return Err(format!(
                    "Section '{}' must have at least one pricing tier",
                    section.name
                ));
            }
        }

        Ok(())
    }

    /// Applies an event to state
    #[allow(clippy::too_many_lines)]
    // Each match arm is a simple state update - no complex logic to extract.
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
                ..
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
            EventAction::EventUpdated {
                event_id, name, ..
            } => {
                if let Some(event) = state.events.get_mut(event_id) {
                    if let Some(new_name) = name {
                        event.name.clone_from(new_name);
                    }
                }
                state.last_error = None;
            }
            EventAction::PricingTiersUpdated {
                event_id,
                pricing_tiers,
                ..
            } => {
                if let Some(event) = state.events.get_mut(event_id) {
                    event.pricing_tiers.clone_from(pricing_tiers);
                }
                state.last_error = None;
            }
            EventAction::VenueSectionsAdded {
                event_id,
                sections,
                ..
            } => {
                if let Some(event) = state.events.get_mut(event_id) {
                    // Use iter().cloned() to avoid intermediate Vec allocation
                    event.venue.sections.extend(sections.iter().cloned());
                }
                state.last_error = None;
            }
            EventAction::VersionUpdated { version } => {
                state.version = *version;
            }
            EventAction::ValidationFailed { error }
            | EventAction::SerializationFailed { error } => {
                state.last_error = Some(error.clone());
            }
            // Commands and query results don't modify state
            // Projection confirmation actions are logged but don't modify aggregate state
            EventAction::CreateEvent { .. }
            | EventAction::PublishEvent { .. }
            | EventAction::OpenSales { .. }
            | EventAction::CloseSales { .. }
            | EventAction::CancelEvent { .. }
            | EventAction::UpdateEvent { .. }
            | EventAction::UpdatePricingTiers { .. }
            | EventAction::AddVenueSections { .. }
            | EventAction::GetEvent { .. }
            | EventAction::ListEvents { .. }
            | EventAction::EventQueried { .. }
            | EventAction::EventsListed { .. }
            | EventAction::EventProjectionConfirmed { .. }
            | EventAction::EventProjectionFailed { .. }
            | EventAction::ExecuteAddVenueSections { .. }
            | EventAction::ExecuteUpdatePricingTiers { .. }
            | EventAction::ExecuteUpdateEvent { .. } => {}
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

    #[allow(clippy::too_many_lines)]
    // This reducer handles 15+ distinct action types. Each match arm is self-contained
    // and follows a consistent validate→apply→effects pattern. Extracting to separate
    // handler methods would add parameter-passing boilerplate without improving clarity.
    // Navigate by searching for "EventAction::Foo" to find specific handlers.
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ═══════════════════════════════════════════════════════════════════════════
            // COMMANDS: External API requests that initiate state changes
            // ═══════════════════════════════════════════════════════════════════════════
            EventAction::CreateEvent {
                id,
                name,
                owner_id,
                venue,
                date,
                pricing_tiers,
                respond_to: _,  // Explicitly ignore - infrastructure handles this
            } => {
                // Validate command
                if let Err(error) =
                    Self::validate_create_event(state, &id, &name, &venue, &pricing_tiers)
                {
                    Self::apply_event(state, &EventAction::ValidationFailed { error });
                    return SmallVec::new();
                }

                // Capture timestamp before creating closures
                let created_at = env.clock.now();

                // Create and apply event (with placeholder respond_to for local state)
                let event_for_state = EventAction::EventCreated {
                    id,
                    name: name.clone(),
                    owner_id,
                    venue: venue.clone(),
                    date,
                    pricing_tiers: pricing_tiers.clone(),
                    created_at,
                    respond_to: ResponseChannel::none(),
                };
                let expected_version = state.version;
                Self::apply_event(state, &event_for_state);

                // Create base effects (append to event store)
                let mut effects = Self::create_effects(event_for_state, expected_version, env);

                // Publish EVENT to global channel for projections and wait for completion
                let id_for_success = id;
                let id_for_error = id;
                effects.push(Effect::PublishWithResponse {
                    channel: env.global_actions.event_actions.clone(),
                    create_action: Box::new(move |respond_to| EventAction::EventCreated {
                        id,
                        name,
                        owner_id,
                        venue,
                        date,
                        pricing_tiers,
                        created_at,
                        respond_to,
                    }),
                    on_success: Box::new(move || {
                        Some(EventAction::EventProjectionConfirmed {
                            event_id: id_for_success,
                        })
                    }),
                    on_error: Box::new(move |reason| {
                        Some(EventAction::EventProjectionFailed {
                            event_id: id_for_error,
                            reason,
                        })
                    }),
                });

                effects
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
                let expected_version = state.version;
                Self::apply_event(state, &event);

                Self::create_effects(event, expected_version, env)
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
                let expected_version = state.version;
                Self::apply_event(state, &event);

                Self::create_effects(event, expected_version, env)
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
                let expected_version = state.version;
                Self::apply_event(state, &event);

                Self::create_effects(event, expected_version, env)
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
                let expected_version = state.version;
                Self::apply_event(state, &event);

                Self::create_effects(event, expected_version, env)
            }

            // ═══════════════════════════════════════════════════════════════════════════
            // COMMANDS WITH ASYNC VALIDATION: Two-phase pattern (load → validate → execute)
            // These commands need to load data from projections before validation.
            // Phase 1: Return Effect::Future that loads data and returns Execute* action
            // Phase 2: Execute* action applies changes with loaded data
            // ═══════════════════════════════════════════════════════════════════════════
            EventAction::UpdateEvent { event_id, name } => {
                // Check if there's actually anything to update (early validation)
                if name.is_none() {
                    Self::apply_event(state, &EventAction::ValidationFailed {
                        error: "No fields to update".to_string(),
                    });
                    return SmallVec::new();
                }

                // Load event from projection and version from event store in parallel
                let projection = env.projection.clone();
                let event_store = env.event_store.clone();
                let stream_id = env.stream_id.clone();
                let clock = env.clock.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // Run both queries in parallel using tokio::join!
                    let (projection_result, version_result) = tokio::join!(
                        projection.load_event(&event_id),
                        event_store.get_stream_version(stream_id)
                    );

                    // Handle projection result
                    let loaded_event = match projection_result {
                        Ok(Some(event)) => event,
                        Ok(None) => {
                            return Some(EventAction::ValidationFailed {
                                error: format!("Event {event_id} not found"),
                            });
                        }
                        Err(e) => {
                            return Some(EventAction::ValidationFailed {
                                error: format!("Failed to load event from projection: {e}"),
                            });
                        }
                    };

                    // Handle version result
                    let current_version = match version_result {
                        Ok(version) => version,
                        Err(e) => {
                            return Some(EventAction::ValidationFailed {
                                error: format!("Failed to load version from event store: {e}"),
                            });
                        }
                    };

                    // Cannot update cancelled events
                    if loaded_event.status == EventStatus::Cancelled {
                        return Some(EventAction::ValidationFailed {
                            error: "Cannot update cancelled event".to_string(),
                        });
                    }

                    // Return execute action with loaded version for optimistic concurrency
                    Some(EventAction::ExecuteUpdateEvent {
                        event_id,
                        name,
                        current_version,
                        updated_at: clock.now(),
                    })
                }))]
            }

            EventAction::ExecuteUpdateEvent {
                event_id,
                name,
                current_version,
                updated_at,
            } => {
                // Create and apply event (with placeholder respond_to for local state)
                let event_for_state = EventAction::EventUpdated {
                    event_id,
                    name: name.clone(),
                    updated_at,
                    respond_to: ResponseChannel::none(),
                };
                Self::apply_event(state, &event_for_state);

                // Create base effects (append to event store with correct version)
                let mut effects = Self::create_effects(event_for_state, current_version, env);

                // Publish EVENT to global channel for projections and wait for completion
                let event_id_for_success = event_id;
                let event_id_for_error = event_id;
                effects.push(Effect::PublishWithResponse {
                    channel: env.global_actions.event_actions.clone(),
                    create_action: Box::new(move |respond_to| EventAction::EventUpdated {
                        event_id,
                        name,
                        updated_at,
                        respond_to,
                    }),
                    on_success: Box::new(move || {
                        Some(EventAction::EventProjectionConfirmed {
                            event_id: event_id_for_success,
                        })
                    }),
                    on_error: Box::new(move |reason| {
                        Some(EventAction::EventProjectionFailed {
                            event_id: event_id_for_error,
                            reason,
                        })
                    }),
                });

                effects
            }

            EventAction::UpdatePricingTiers {
                event_id,
                pricing_tiers,
                respond_to: _,  // Explicitly ignore - infrastructure handles this
            } => {
                // Load event from projection and version from event store in parallel
                let projection = env.projection.clone();
                let event_store = env.event_store.clone();
                let stream_id = env.stream_id.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // Run both queries in parallel using tokio::join!
                    let (projection_result, version_result) = tokio::join!(
                        projection.load_event(&event_id),
                        event_store.get_stream_version(stream_id)
                    );

                    // Handle projection result
                    let loaded_event = match projection_result {
                        Ok(Some(event)) => event,
                        Ok(None) => {
                            return Some(EventAction::ValidationFailed {
                                error: format!("Event {event_id} not found"),
                            });
                        }
                        Err(e) => {
                            return Some(EventAction::ValidationFailed {
                                error: format!("Failed to load event from projection: {e}"),
                            });
                        }
                    };

                    // Handle version result
                    let current_version = match version_result {
                        Ok(version) => version,
                        Err(e) => {
                            return Some(EventAction::ValidationFailed {
                                error: format!("Failed to load version from event store: {e}"),
                            });
                        }
                    };

                    // Validate pricing tiers against loaded event
                    if let Err(error) = Self::validate_update_pricing_tiers_with_event(&loaded_event, &pricing_tiers) {
                        return Some(EventAction::ValidationFailed { error });
                    }

                    // Return execute action with loaded data
                    Some(EventAction::ExecuteUpdatePricingTiers {
                        event_id,
                        pricing_tiers,
                        loaded_event,
                        current_version,
                    })
                }))]
            }

            EventAction::ExecuteUpdatePricingTiers {
                event_id,
                pricing_tiers,
                loaded_event,
                current_version,
            } => {
                // Insert loaded event into state for local tracking
                state.events.insert(event_id, loaded_event);

                // Capture timestamp before creating closures
                let updated_at = env.clock.now();

                // Create and apply event (with placeholder respond_to for local state)
                let event_for_state = EventAction::PricingTiersUpdated {
                    event_id,
                    pricing_tiers: pricing_tiers.clone(),
                    updated_at,
                    respond_to: ResponseChannel::none(),
                };
                Self::apply_event(state, &event_for_state);

                // Persist event to event store (use version loaded from event store for optimistic concurrency)
                let mut effects = Self::create_effects(event_for_state, current_version, env);

                // Publish EVENT to global channel for projections and wait for completion
                let event_id_for_success = event_id;
                let event_id_for_error = event_id;
                effects.push(Effect::PublishWithResponse {
                    channel: env.global_actions.event_actions.clone(),
                    create_action: Box::new(move |respond_to| {
                        EventAction::PricingTiersUpdated {
                            event_id,
                            pricing_tiers,
                            updated_at,
                            respond_to,
                        }
                    }),
                    on_success: Box::new(move || {
                        Some(EventAction::EventProjectionConfirmed {
                            event_id: event_id_for_success,
                        })
                    }),
                    on_error: Box::new(move |reason| {
                        Some(EventAction::EventProjectionFailed {
                            event_id: event_id_for_error,
                            reason,
                        })
                    }),
                });

                effects
            }

            EventAction::AddVenueSections {
                event_id,
                sections,
                respond_to: _,  // Explicitly ignore - infrastructure handles this
            } => {
                // Load event from projection and version from event store in parallel
                let projection = env.projection.clone();
                let event_store = env.event_store.clone();
                let stream_id = env.stream_id.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // Run both queries in parallel using tokio::join!
                    let (projection_result, version_result) = tokio::join!(
                        projection.load_event(&event_id),
                        event_store.get_stream_version(stream_id)
                    );

                    // Handle projection result
                    let loaded_event = match projection_result {
                        Ok(Some(event)) => event,
                        Ok(None) => {
                            return Some(EventAction::ValidationFailed {
                                error: format!("Event {event_id} not found"),
                            });
                        }
                        Err(e) => {
                            return Some(EventAction::ValidationFailed {
                                error: format!("Failed to load event from projection: {e}"),
                            });
                        }
                    };

                    // Handle version result
                    let current_version = match version_result {
                        Ok(version) => version,
                        Err(e) => {
                            return Some(EventAction::ValidationFailed {
                                error: format!("Failed to load version from event store: {e}"),
                            });
                        }
                    };

                    // Validate sections against loaded event
                    if let Err(error) = Self::validate_add_venue_sections_with_event(&loaded_event, &sections) {
                        return Some(EventAction::ValidationFailed { error });
                    }

                    // Return execute action with loaded data
                    Some(EventAction::ExecuteAddVenueSections {
                        event_id,
                        sections,
                        loaded_event,
                        current_version,
                    })
                }))]
            }

            EventAction::ExecuteAddVenueSections {
                event_id,
                sections,
                loaded_event,
                current_version,
            } => {
                // Insert loaded event into state for local tracking
                state.events.insert(event_id, loaded_event);

                // Capture timestamp before creating closures
                let updated_at = env.clock.now();

                // Create and apply event (with placeholder respond_to for local state)
                let event_for_state = EventAction::VenueSectionsAdded {
                    event_id,
                    sections: sections.clone(),
                    updated_at,
                    respond_to: ResponseChannel::none(),
                };
                Self::apply_event(state, &event_for_state);

                // Persist event to event store (use version loaded from event store for optimistic concurrency)
                let mut effects = Self::create_effects(event_for_state, current_version, env);

                // Publish EVENT to global channel for projections and wait for completion
                let event_id_for_success = event_id;
                let event_id_for_error = event_id;
                effects.push(Effect::PublishWithResponse {
                    channel: env.global_actions.event_actions.clone(),
                    create_action: Box::new(move |respond_to| {
                        EventAction::VenueSectionsAdded {
                            event_id,
                            sections,
                            updated_at,
                            respond_to,
                        }
                    }),
                    on_success: Box::new(move || {
                        Some(EventAction::EventProjectionConfirmed {
                            event_id: event_id_for_success,
                        })
                    }),
                    on_error: Box::new(move |reason| {
                        Some(EventAction::EventProjectionFailed {
                            event_id: event_id_for_error,
                            reason,
                        })
                    }),
                });

                effects
            }

            // ═══════════════════════════════════════════════════════════════════════════
            // QUERIES: Read-only operations that load data from projections
            // ═══════════════════════════════════════════════════════════════════════════
            EventAction::GetEvent { event_id } => {
                let projection = env.projection.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    match projection.load_event(&event_id).await {
                        Ok(event) => Some(EventAction::EventQueried { event_id, event }),
                        Err(e) => Some(EventAction::ValidationFailed {
                            error: format!("Failed to load event: {e}"),
                        }),
                    }
                }))]
            }

            EventAction::ListEvents { status_filter } => {
                let projection = env.projection.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    match projection.load_events(status_filter).await {
                        Ok(events) => Some(EventAction::EventsListed {
                            events,
                            status_filter,
                        }),
                        Err(e) => Some(EventAction::ValidationFailed {
                            error: format!("Failed to load events: {e}"),
                        }),
                    }
                }))]
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
    use crate::test_utils::create_test_global_channels;
    use crate::types::{Capacity, Money, SeatType, TierType};
    use composable_rust_core::environment::SystemClock;
    use composable_rust_testing::{assertions, mocks::InMemoryEventStore, ReducerTest, TestStore};
    use std::collections::HashMap;
    use std::sync::RwLock;
    use std::time::Duration as StdDuration;

    // =========================================================================
    // Test Infrastructure
    // =========================================================================

    fn create_test_env() -> EventEnvironment {
        EventEnvironment::new(
            Arc::new(SystemClock),
            Arc::new(InMemoryEventStore::new()),
            StreamId::new("test-stream"),
            Arc::new(ConfigurableMockEventQuery::new()),
            create_test_global_channels(),
        )
    }

    fn create_test_env_with_projection(
        projection: Arc<dyn EventProjectionQuery>,
    ) -> EventEnvironment {
        EventEnvironment::new(
            Arc::new(SystemClock),
            Arc::new(InMemoryEventStore::new()),
            StreamId::new("test-stream"),
            projection,
            create_test_global_channels(),
        )
    }

    /// Configurable mock for different test scenarios
    #[derive(Clone, Default)]
    struct ConfigurableMockEventQuery {
        events: Arc<RwLock<HashMap<EventId, Event>>>,
        load_error: Arc<RwLock<Option<String>>>,
    }

    impl ConfigurableMockEventQuery {
        fn new() -> Self {
            Self::default()
        }

        /// Add an event to be returned by load queries
        fn with_event(self, event: Event) -> Self {
            self.events.write().unwrap().insert(event.id, event);
            self
        }

        /// Configure load queries to return an error
        #[allow(dead_code)]
        fn with_error(self, error: &str) -> Self {
            *self.load_error.write().unwrap() = Some(error.to_string());
            self
        }
    }

    #[async_trait::async_trait]
    impl EventProjectionQuery for ConfigurableMockEventQuery {
        async fn load_event(&self, event_id: &EventId) -> Result<Option<Event>, String> {
            if let Some(ref error) = *self.load_error.read().unwrap() {
                return Err(error.clone());
            }
            Ok(self.events.read().unwrap().get(event_id).cloned())
        }

        async fn load_events(
            &self,
            status_filter: Option<EventStatus>,
        ) -> Result<Vec<Event>, String> {
            if let Some(ref error) = *self.load_error.read().unwrap() {
                return Err(error.clone());
            }
            let events: Vec<Event> = self
                .events
                .read()
                .unwrap()
                .values()
                .filter(|e| status_filter.is_none_or(|s| e.status == s))
                .cloned()
                .collect();
            Ok(events)
        }
    }

    // Local test venue with single section (for simpler tests)
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

    // Helper for creating a venue with multiple sections
    fn create_test_venue_multi_section() -> Venue {
        Venue::new(
            "Test Arena".to_string(),
            Capacity::new(2000),
            vec![
                VenueSection::new(
                    "VIP".to_string(),
                    Capacity::new(500),
                    SeatType::GeneralAdmission,
                ),
                VenueSection::new(
                    "General".to_string(),
                    Capacity::new(1500),
                    SeatType::GeneralAdmission,
                ),
            ],
        )
    }

    // Local test pricing matching the single-section venue
    fn create_test_pricing_tiers() -> Vec<PricingTier> {
        vec![PricingTier::new(
            TierType::Regular,
            "General".to_string(),
            Money::from_dollars(50),
            Utc::now(),
            None,
        )]
    }

    // =========================================================================
    // Sync Validation Tests (ReducerTest)
    // =========================================================================
    //
    // Fast, pure tests that verify commands with invalid inputs are rejected
    // immediately without triggering async effects. These test the COMMAND
    // actions, NOT the internal Execute actions.

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
                respond_to: ResponseChannel::none(),
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
                respond_to: ResponseChannel::none(),
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

    // =========================================================================
    // Sync Command Tests (ReducerTest)
    // =========================================================================
    //
    // These commands don't use the two-phase async pattern, so they can be
    // tested directly with ReducerTest.

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
                respond_to: ResponseChannel::none(),
            })
            .then_state(move |state| {
                assert_eq!(state.count(), 1);
                assert!(state.exists(&id));
                let event = state.get(&id).unwrap();
                assert_eq!(event.name, "Taylor Swift Concert");
                assert_eq!(event.status, EventStatus::Draft);
            })
            .then_effects(|effects| {
                // Should return 3 effects: AppendEvents + Echo + PublishWithResponse
                assert_eq!(effects.len(), 3);
            })
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
                // Should return 2 effects: AppendEvents + Echo
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
                respond_to: ResponseChannel::none(),
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
    fn test_pricing_tiers_multiple_tiers_per_section() {
        let id = EventId::new();
        let owner_id = UserId::new();
        let now = Utc::now();

        // Multiple pricing tiers for the same section
        let pricing_tiers = vec![
            PricingTier::new(
                TierType::EarlyBird,
                "General".to_string(),
                Money::from_dollars(30),
                now,
                Some(now + Duration::days(7)),
            ),
            PricingTier::new(
                TierType::Regular,
                "General".to_string(),
                Money::from_dollars(50),
                now + Duration::days(7),
                Some(now + Duration::days(25)),
            ),
            PricingTier::new(
                TierType::LastMinute,
                "General".to_string(),
                Money::from_dollars(70),
                now + Duration::days(25),
                None,
            ),
        ];

        ReducerTest::new(EventReducer::new())
            .with_env(create_test_env())
            .given_state(EventState::new())
            .when_action(EventAction::CreateEvent {
                id,
                name: "Concert with Dynamic Pricing".to_string(),
                owner_id,
                venue: create_test_venue(),
                date: EventDate::new(now + Duration::days(30)),
                pricing_tiers,
                respond_to: ResponseChannel::none(),
            })
            .then_state(move |state| {
                let event = state.get(&id).unwrap();
                assert_eq!(event.pricing_tiers.len(), 3);
                // Verify all three tier types exist
                let tier_types: Vec<_> = event
                    .pricing_tiers
                    .iter()
                    .map(|t| t.tier_type)
                    .collect();
                assert!(tier_types.contains(&TierType::EarlyBird));
                assert!(tier_types.contains(&TierType::Regular));
                assert!(tier_types.contains(&TierType::LastMinute));
            })
            .then_effects(|effects| {
                assert!(!effects.is_empty());
            })
            .run();
    }

    // =========================================================================
    // Validation Function Tests
    // =========================================================================
    //
    // Direct tests for validation functions. These test edge cases that are
    // easier to verify by calling the validation function directly.

    #[test]
    fn test_validate_pricing_tiers_cancelled_event() {
        let mut event = Event::new(
            EventId::new(),
            "Cancelled Concert".to_string(),
            UserId::new(),
            create_test_venue(),
            EventDate::new(Utc::now() + Duration::days(30)),
            create_test_pricing_tiers(),
            Utc::now(),
        );
        event.status = EventStatus::Cancelled;

        let result = EventReducer::validate_update_pricing_tiers_with_event(
            &event,
            &create_test_pricing_tiers(),
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cancelled"));
    }

    #[test]
    fn test_validate_pricing_tiers_empty_tiers() {
        let event = Event::new(
            EventId::new(),
            "Concert".to_string(),
            UserId::new(),
            create_test_venue(),
            EventDate::new(Utc::now() + Duration::days(30)),
            create_test_pricing_tiers(),
            Utc::now(),
        );

        let result = EventReducer::validate_update_pricing_tiers_with_event(&event, &[]);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("At least one pricing tier"));
    }

    #[test]
    fn test_validate_pricing_tiers_invalid_section() {
        let event = Event::new(
            EventId::new(),
            "Concert".to_string(),
            UserId::new(),
            create_test_venue(), // Has "General" section only
            EventDate::new(Utc::now() + Duration::days(30)),
            create_test_pricing_tiers(),
            Utc::now(),
        );

        let invalid_pricing = vec![PricingTier::new(
            TierType::Regular,
            "NonExistentSection".to_string(),
            Money::from_dollars(50),
            Utc::now(),
            None,
        )];

        let result =
            EventReducer::validate_update_pricing_tiers_with_event(&event, &invalid_pricing);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist in event venue"));
    }

    #[test]
    fn test_validate_pricing_tiers_missing_section_coverage() {
        let event = Event::new(
            EventId::new(),
            "Concert".to_string(),
            UserId::new(),
            create_test_venue_multi_section(), // Has VIP + General sections
            EventDate::new(Utc::now() + Duration::days(30)),
            vec![
                PricingTier::new(
                    TierType::Regular,
                    "VIP".to_string(),
                    Money::from_dollars(100),
                    Utc::now(),
                    None,
                ),
                PricingTier::new(
                    TierType::Regular,
                    "General".to_string(),
                    Money::from_dollars(50),
                    Utc::now(),
                    None,
                ),
            ],
            Utc::now(),
        );

        // Only provide pricing for VIP, missing General section
        let incomplete_pricing = vec![PricingTier::new(
            TierType::Regular,
            "VIP".to_string(),
            Money::from_dollars(100),
            Utc::now(),
            None,
        )];

        let result =
            EventReducer::validate_update_pricing_tiers_with_event(&event, &incomplete_pricing);

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("must have at least one pricing tier"));
    }

    // =========================================================================
    // Full Flow Tests (TestStore)
    // =========================================================================
    //
    // Test complete async behavior from command to terminal event.
    // These test the REAL behavior without knowing about internal Execute actions.

    // -------------------------------------------------------------------------
    // Happy Paths
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_update_event_success() {
        let event_id = EventId::new();
        let owner_id = UserId::new();

        // Create an existing event
        let existing = Event::new(
            event_id,
            "Original Name".to_string(),
            owner_id,
            create_test_venue(),
            EventDate::new(Utc::now() + Duration::days(30)),
            create_test_pricing_tiers(),
            Utc::now(),
        );

        // Pre-populate both projection (for async load) and state (for apply_event)
        let mock = ConfigurableMockEventQuery::new().with_event(existing.clone());
        let env = create_test_env_with_projection(Arc::new(mock));
        let mut initial_state = EventState::new();
        initial_state.events.insert(event_id, existing);
        let store = TestStore::new(EventReducer::new(), env, initial_state);

        // Send UpdateEvent and wait for TERMINAL action (EventUpdated)
        let result = store
            .send_and_wait_for(
                EventAction::UpdateEvent {
                    event_id,
                    name: Some("Updated Name".to_string()),
                },
                |action| {
                    matches!(
                        action,
                        EventAction::EventUpdated { .. } | EventAction::ValidationFailed { .. }
                    )
                },
                StdDuration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive EventUpdated");
        let action = result.unwrap();
        assert!(
            matches!(action, EventAction::EventUpdated { .. }),
            "Expected EventUpdated, got {action:?}"
        );

        // State is updated when terminal action is broadcast
        let state = store.state(|s| s.clone()).await;
        let event = state.get(&event_id).unwrap();
        assert_eq!(event.name, "Updated Name");

        store.clear_queue();
    }

    #[tokio::test]
    async fn test_update_pricing_tiers_success() {
        let event_id = EventId::new();
        let owner_id = UserId::new();

        // Create an existing event with initial pricing
        let existing = Event::new(
            event_id,
            "Concert".to_string(),
            owner_id,
            create_test_venue(),
            EventDate::new(Utc::now() + Duration::days(30)),
            create_test_pricing_tiers(),
            Utc::now(),
        );

        // Pre-populate both projection (for async load) and state (for apply_event)
        let mock = ConfigurableMockEventQuery::new().with_event(existing.clone());
        let env = create_test_env_with_projection(Arc::new(mock));
        let mut initial_state = EventState::new();
        initial_state.events.insert(event_id, existing);
        let store = TestStore::new(EventReducer::new(), env, initial_state);

        // New pricing tiers
        let new_pricing = vec![
            PricingTier::new(
                TierType::EarlyBird,
                "General".to_string(),
                Money::from_dollars(40),
                Utc::now(),
                Some(Utc::now() + Duration::days(7)),
            ),
            PricingTier::new(
                TierType::Regular,
                "General".to_string(),
                Money::from_dollars(60),
                Utc::now() + Duration::days(7),
                None,
            ),
        ];

        // Send UpdatePricingTiers and wait for TERMINAL action
        let result = store
            .send_and_wait_for(
                EventAction::UpdatePricingTiers {
                    event_id,
                    pricing_tiers: new_pricing,
                    respond_to: ResponseChannel::none(),
                },
                |action| {
                    matches!(
                        action,
                        EventAction::PricingTiersUpdated { .. }
                            | EventAction::ValidationFailed { .. }
                    )
                },
                StdDuration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive PricingTiersUpdated");
        let action = result.unwrap();
        assert!(
            matches!(action, EventAction::PricingTiersUpdated { .. }),
            "Expected PricingTiersUpdated, got {action:?}"
        );

        // State is updated when terminal action is broadcast
        let state = store.state(|s| s.clone()).await;
        let event = state.get(&event_id).unwrap();
        assert_eq!(event.pricing_tiers.len(), 2);

        store.clear_queue();
    }

    // -------------------------------------------------------------------------
    // Async Validation Failures
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_update_event_not_found() {
        // Empty projection - no events exist
        let env = create_test_env_with_projection(Arc::new(ConfigurableMockEventQuery::new()));
        let store = TestStore::new(EventReducer::new(), env, EventState::new());

        let result = store
            .send_and_wait_for(
                EventAction::UpdateEvent {
                    event_id: EventId::new(),
                    name: Some("New Name".to_string()),
                },
                |action| {
                    matches!(
                        action,
                        EventAction::EventUpdated { .. } | EventAction::ValidationFailed { .. }
                    )
                },
                StdDuration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive ValidationFailed");
        match result.unwrap() {
            EventAction::ValidationFailed { error } => {
                assert!(
                    error.contains("not found"),
                    "Error should mention 'not found': {error}"
                );
            }
            other => panic!("Expected ValidationFailed, got {other:?}"),
        }

        store.clear_queue();
    }

    #[tokio::test]
    async fn test_update_event_cancelled_rejected() {
        let event_id = EventId::new();

        // Create a cancelled event in the projection
        let mut cancelled = Event::new(
            event_id,
            "Cancelled Concert".to_string(),
            UserId::new(),
            create_test_venue(),
            EventDate::new(Utc::now() + Duration::days(30)),
            create_test_pricing_tiers(),
            Utc::now(),
        );
        cancelled.status = EventStatus::Cancelled;

        let mock = ConfigurableMockEventQuery::new().with_event(cancelled);
        let env = create_test_env_with_projection(Arc::new(mock));
        let store = TestStore::new(EventReducer::new(), env, EventState::new());

        let result = store
            .send_and_wait_for(
                EventAction::UpdateEvent {
                    event_id,
                    name: Some("New Name".to_string()),
                },
                |action| {
                    matches!(
                        action,
                        EventAction::EventUpdated { .. } | EventAction::ValidationFailed { .. }
                    )
                },
                StdDuration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive ValidationFailed");
        match result.unwrap() {
            EventAction::ValidationFailed { error } => {
                assert!(
                    error.contains("cancelled"),
                    "Error should mention 'cancelled': {error}"
                );
            }
            other => panic!("Expected ValidationFailed, got {other:?}"),
        }

        store.clear_queue();
    }

    #[tokio::test]
    async fn test_update_pricing_tiers_event_not_found() {
        // Empty projection - no events exist
        let env = create_test_env_with_projection(Arc::new(ConfigurableMockEventQuery::new()));
        let store = TestStore::new(EventReducer::new(), env, EventState::new());

        let result = store
            .send_and_wait_for(
                EventAction::UpdatePricingTiers {
                    event_id: EventId::new(),
                    pricing_tiers: create_test_pricing_tiers(),
                    respond_to: ResponseChannel::none(),
                },
                |action| {
                    matches!(
                        action,
                        EventAction::PricingTiersUpdated { .. }
                            | EventAction::ValidationFailed { .. }
                    )
                },
                StdDuration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive ValidationFailed");
        match result.unwrap() {
            EventAction::ValidationFailed { error } => {
                assert!(
                    error.contains("not found"),
                    "Error should mention 'not found': {error}"
                );
            }
            other => panic!("Expected ValidationFailed, got {other:?}"),
        }

        store.clear_queue();
    }

    // -------------------------------------------------------------------------
    // Query Operations
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_get_event() {
        let event_id = EventId::new();
        let event = Event::new(
            event_id,
            "Test Concert".to_string(),
            UserId::new(),
            create_test_venue(),
            EventDate::new(Utc::now() + Duration::days(30)),
            create_test_pricing_tiers(),
            Utc::now(),
        );

        let mock = ConfigurableMockEventQuery::new().with_event(event);
        let env = create_test_env_with_projection(Arc::new(mock));
        let store = TestStore::new(EventReducer::new(), env, EventState::new());

        let result = store
            .send_and_wait_for(
                EventAction::GetEvent { event_id },
                |action| {
                    matches!(
                        action,
                        EventAction::EventQueried { .. } | EventAction::ValidationFailed { .. }
                    )
                },
                StdDuration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive EventQueried");
        match result.unwrap() {
            EventAction::EventQueried {
                event_id: id,
                event: Some(e),
            } => {
                assert_eq!(id, event_id);
                assert_eq!(e.name, "Test Concert");
            }
            other => panic!("Expected EventQueried with event, got {other:?}"),
        }

        store.clear_queue();
    }

    #[tokio::test]
    async fn test_get_event_not_found() {
        // Empty projection - no events exist
        let env = create_test_env_with_projection(Arc::new(ConfigurableMockEventQuery::new()));
        let store = TestStore::new(EventReducer::new(), env, EventState::new());

        let result = store
            .send_and_wait_for(
                EventAction::GetEvent {
                    event_id: EventId::new(),
                },
                |action| {
                    matches!(
                        action,
                        EventAction::EventQueried { .. } | EventAction::ValidationFailed { .. }
                    )
                },
                StdDuration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive EventQueried");
        match result.unwrap() {
            EventAction::EventQueried { event: None, .. } => {
                // Expected - event not found returns None
            }
            other => panic!("Expected EventQueried with None, got {other:?}"),
        }

        store.clear_queue();
    }

    #[tokio::test]
    async fn test_list_events() {
        let event1 = Event::new(
            EventId::new(),
            "Concert 1".to_string(),
            UserId::new(),
            create_test_venue(),
            EventDate::new(Utc::now() + Duration::days(30)),
            create_test_pricing_tiers(),
            Utc::now(),
        );
        let mut event2 = Event::new(
            EventId::new(),
            "Concert 2".to_string(),
            UserId::new(),
            create_test_venue(),
            EventDate::new(Utc::now() + Duration::days(60)),
            create_test_pricing_tiers(),
            Utc::now(),
        );
        event2.status = EventStatus::Published;

        let mock = ConfigurableMockEventQuery::new()
            .with_event(event1)
            .with_event(event2);
        let env = create_test_env_with_projection(Arc::new(mock));
        let store = TestStore::new(EventReducer::new(), env, EventState::new());

        let result = store
            .send_and_wait_for(
                EventAction::ListEvents {
                    status_filter: None,
                },
                |action| {
                    matches!(
                        action,
                        EventAction::EventsListed { .. } | EventAction::ValidationFailed { .. }
                    )
                },
                StdDuration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive EventsListed");
        match result.unwrap() {
            EventAction::EventsListed { events, .. } => {
                assert_eq!(events.len(), 2, "Should have 2 events");
            }
            other => panic!("Expected EventsListed, got {other:?}"),
        }

        store.clear_queue();
    }

    #[tokio::test]
    async fn test_list_events_with_filter() {
        let mut draft_event = Event::new(
            EventId::new(),
            "Draft Concert".to_string(),
            UserId::new(),
            create_test_venue(),
            EventDate::new(Utc::now() + Duration::days(30)),
            create_test_pricing_tiers(),
            Utc::now(),
        );
        draft_event.status = EventStatus::Draft;

        let mut published_event = Event::new(
            EventId::new(),
            "Published Concert".to_string(),
            UserId::new(),
            create_test_venue(),
            EventDate::new(Utc::now() + Duration::days(60)),
            create_test_pricing_tiers(),
            Utc::now(),
        );
        published_event.status = EventStatus::Published;

        let mock = ConfigurableMockEventQuery::new()
            .with_event(draft_event)
            .with_event(published_event);
        let env = create_test_env_with_projection(Arc::new(mock));
        let store = TestStore::new(EventReducer::new(), env, EventState::new());

        // Filter to only Published events
        let result = store
            .send_and_wait_for(
                EventAction::ListEvents {
                    status_filter: Some(EventStatus::Published),
                },
                |action| {
                    matches!(
                        action,
                        EventAction::EventsListed { .. } | EventAction::ValidationFailed { .. }
                    )
                },
                StdDuration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive EventsListed");
        match result.unwrap() {
            EventAction::EventsListed { events, .. } => {
                assert_eq!(events.len(), 1, "Should have 1 published event");
                assert_eq!(events[0].status, EventStatus::Published);
            }
            other => panic!("Expected EventsListed, got {other:?}"),
        }

        store.clear_queue();
    }
}
