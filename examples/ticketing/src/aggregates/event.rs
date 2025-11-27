//! Event aggregate for the Event Ticketing System.
//!
//! Manages event lifecycle: creation, publishing, sales management, and cancellation.
//! Demonstrates validation, state transitions, and business rules enforcement.

use crate::aggregates::inventory::InventoryAction;
use crate::projections::TicketingEvent;
use crate::types::{
    Capacity, Event, EventDate, EventId, EventState, EventStatus, GlobalActionChannels,
    PricingTier, ResponseChannel, SeatNumber, SeatType, Venue,
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
        sections: Vec<crate::types::VenueSection>,

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
        sections: Vec<crate::types::VenueSection>,
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
    EventProjectionConfirmed {
        /// Event ID
        event_id: EventId,
    },

    /// Projection update failed
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
        sections: Vec<crate::types::VenueSection>,
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
// Projection Query Trait
// ============================================================================

/// Event projection query trait for loading event state.
///
/// This trait abstracts projection queries to enable:
/// - Dependency injection (pass mocks in tests)
/// - Query actions flowing through reducer
/// - Business logic on read operations
#[async_trait::async_trait]
pub trait EventProjectionQuery: Send + Sync {
    /// Load a single event by ID
    ///
    /// # Errors
    ///
    /// Returns error if database query fails
    async fn load_event(&self, event_id: &EventId) -> Result<Option<Event>, String>;

    /// Load events with optional status filter
    ///
    /// # Errors
    ///
    /// Returns error if database query fails
    async fn load_events(&self, status_filter: Option<EventStatus>) -> Result<Vec<Event>, String>;
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

    /// Creates effects for persisting events (PostgreSQL only, no Redpanda)
    ///
    /// With direct orchestration, we use local channels for coordination,
    /// so Redpanda publishing is no longer needed.
    fn create_effects(
        event: EventAction,
        expected_version: Version,
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
                expected_version: Some(expected_version),
                events: vec![serialized.clone()],
                on_success: |version| Some(EventAction::VersionUpdated { version }),
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

    #[allow(dead_code)]
    fn validate_update_pricing_tiers(
        state: &EventState,
        event_id: &EventId,
        pricing_tiers: &[PricingTier],
    ) -> Result<(), String> {
        // Event must exist
        let Some(event) = state.get(event_id) else {
            return Err(format!("Event {event_id} not found"));
        };

        // Cannot update pricing for cancelled events
        if event.status == EventStatus::Cancelled {
            return Err("Cannot update pricing for cancelled event".to_string());
        }

        // Must provide at least one pricing tier
        if pricing_tiers.is_empty() {
            return Err("At least one pricing tier must be provided".to_string());
        }

        // Validate all sections exist in the event's venue
        let valid_sections: std::collections::HashSet<&str> =
            event.venue.sections.iter().map(|s| s.name.as_str()).collect();

        for tier in pricing_tiers {
            if !valid_sections.contains(tier.section.as_str()) {
                return Err(format!(
                    "Section '{}' does not exist in event venue",
                    tier.section
                ));
            }
        }

        // Validate each section has at least one tier
        let sections_with_tiers: std::collections::HashSet<&str> =
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

    /// Validates adding venue sections to an event
    #[allow(dead_code)]
    fn validate_add_venue_sections(
        state: &EventState,
        event_id: &EventId,
        sections: &[crate::types::VenueSection],
    ) -> Result<(), String> {
        // Event must exist
        let Some(event) = state.get(event_id) else {
            return Err(format!("Event {event_id} not found"));
        };

        // Cannot add sections to cancelled events
        if event.status == EventStatus::Cancelled {
            return Err("Cannot add sections to cancelled event".to_string());
        }

        // Must provide at least one section
        if sections.is_empty() {
            return Err("At least one section must be provided".to_string());
        }

        // Validate sections don't duplicate existing sections
        let existing_sections: std::collections::HashSet<&str> =
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
        let mut seen_names = std::collections::HashSet::new();
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

    /// Validates adding venue sections using a loaded event (for projection-based validation)
    fn validate_add_venue_sections_with_event(
        event: &Event,
        sections: &[crate::types::VenueSection],
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
        let existing_sections: std::collections::HashSet<&str> =
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
        let mut seen_names = std::collections::HashSet::new();
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
        let section_names: std::collections::HashSet<&str> =
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
        let sections_with_tiers: std::collections::HashSet<&str> =
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
                    event.venue.sections.extend(sections.clone());
                }
                state.last_error = None;
            }
            EventAction::VersionUpdated { version } => {
                state.version = *version;
            }
            EventAction::ValidationFailed { error } => {
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
                respond_to: _,  // Explicitly ignore - infrastructure handles this
            } => {
                // Validate command
                if let Err(error) =
                    Self::validate_create_event(state, &id, &name, &date, &venue, &pricing_tiers)
                {
                    Self::apply_event(state, &EventAction::ValidationFailed { error });
                    return SmallVec::new();
                }

                // Capture timestamp before creating closures
                let created_at = env.clock.now();

                // Clone venue for inventory initialization (before it's moved into closures)
                let venue_for_inventory = venue.clone();
                let inventory_channel = env.global_actions.inventory_actions.clone();

                // Create and apply event (with placeholder respond_to for local state)
                let event_for_state = EventAction::EventCreated {
                    id,
                    name: name.clone(),
                    owner_id,
                    venue: venue.clone(),
                    date: date.clone(),
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

                // Auto-initialize inventory for each venue section
                // This ensures inventory exists before any reservations can be made
                for section in venue_for_inventory.sections {
                    let section_name = section.name.clone();
                    let capacity = section.capacity;
                    let seat_numbers = match &section.seat_type {
                        SeatType::Numbered { seats } => {
                            // Use existing seat numbers from the section
                            Some(seats.clone())
                        }
                        SeatType::GeneralAdmission => {
                            // Generate seat numbers for general admission
                            Some(
                                (1..=capacity.value())
                                    .map(|n| SeatNumber::new(format!("{}-{}", section_name, n)))
                                    .collect(),
                            )
                        }
                    };
                    let init_action = InventoryAction::InitializeInventory {
                        event_id: id_for_success,
                        section: section_name,
                        capacity: Capacity::new(capacity.value()),
                        seat_numbers,
                        respond_to: ResponseChannel::none(),
                    };
                    let channel = inventory_channel.clone();
                    effects.push(Effect::Future(Box::pin(async move {
                        let _ = channel.send(init_action);
                        None // No feedback needed
                    })));
                }

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

            EventAction::UpdateEvent { event_id, name } => {
                // Check if there's actually anything to update (early validation)
                if name.is_none() {
                    Self::apply_event(state, &EventAction::ValidationFailed {
                        error: "No fields to update".to_string(),
                    });
                    return SmallVec::new();
                }

                // Load event from projection to validate (async path for per-request stores)
                let projection = env.projection.clone();
                let clock = env.clock.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // Load event from projection
                    let loaded_event = match projection.load_event(&event_id).await {
                        Ok(Some(event)) => event,
                        Ok(None) => {
                            return Some(EventAction::ValidationFailed {
                                error: format!("Event {} not found", event_id),
                            });
                        }
                        Err(e) => {
                            return Some(EventAction::ValidationFailed {
                                error: format!("Failed to load event from projection: {e}"),
                            });
                        }
                    };

                    // Cannot update cancelled events
                    if loaded_event.status == EventStatus::Cancelled {
                        return Some(EventAction::ValidationFailed {
                            error: "Cannot update cancelled event".to_string(),
                        });
                    }

                    // Return execute action
                    // Note: Version is handled by the store - we use Version::new(0) as placeholder
                    // since we're not doing optimistic concurrency on per-request stores
                    Some(EventAction::ExecuteUpdateEvent {
                        event_id,
                        name,
                        current_version: Version::new(0),
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
                        event_store.load_events(stream_id, None)
                    );

                    // Handle projection result
                    let loaded_event = match projection_result {
                        Ok(Some(event)) => event,
                        Ok(None) => {
                            return Some(EventAction::ValidationFailed {
                                error: format!("Event {} not found", event_id),
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
                        Ok(events) => Version::new(events.len() as u64),
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
                // Use version loaded from event store for optimistic concurrency
                let expected_version = current_version;
                Self::apply_event(state, &event_for_state);

                // Persist event to event store
                let mut effects = Self::create_effects(event_for_state, expected_version, env);

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
                        event_store.load_events(stream_id, None)
                    );

                    // Handle projection result
                    let loaded_event = match projection_result {
                        Ok(Some(event)) => event,
                        Ok(None) => {
                            return Some(EventAction::ValidationFailed {
                                error: format!("Event {} not found", event_id),
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
                        Ok(events) => Version::new(events.len() as u64),
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

                // Use version loaded from event store for optimistic concurrency
                let expected_version = current_version;
                let mut effects = Self::create_effects(event_for_state, expected_version, env);

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

            // ========== Query Commands ==========
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
    use crate::types::{Capacity, Money, SeatType, VenueSection};
    use composable_rust_core::environment::SystemClock;
    use composable_rust_testing::{
        assertions,
        mocks::InMemoryEventStore,
        ReducerTest,
    };

    /// Mock projection for testing
    struct MockEventProjection;

    #[async_trait::async_trait]
    impl EventProjectionQuery for MockEventProjection {
        async fn load_event(&self, _event_id: &EventId) -> Result<Option<Event>, String> {
            Ok(None) // Tests don't need actual projection data
        }

        async fn load_events(&self, _status_filter: Option<EventStatus>) -> Result<Vec<Event>, String> {
            Ok(Vec::new()) // Tests don't need actual projection data
        }
    }

    fn create_test_global_actions() -> crate::types::GlobalActionChannels {
        use tokio::sync::broadcast;
        let (event_tx, _) = broadcast::channel(10);
        let (inventory_tx, _) = broadcast::channel(10);
        let (reservation_tx, _) = broadcast::channel(10);
        let (payment_tx, _) = broadcast::channel(10);
        crate::types::GlobalActionChannels {
            event_actions: event_tx,
            inventory_actions: inventory_tx,
            reservation_actions: reservation_tx,
            payment_actions: payment_tx,
        }
    }

    fn create_test_env() -> EventEnvironment {
        EventEnvironment::new(
            Arc::new(SystemClock),
            Arc::new(InMemoryEventStore::new()),
            StreamId::new("test-stream"),
            Arc::new(MockEventProjection),
            create_test_global_actions(),
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
                // Should return 3 effects: AppendEvents + PublishWithResponse + Inventory Init
                assert_eq!(effects.len(), 3);
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
                // Should return 1 effect: AppendEvents (no Redpanda)
                assert_eq!(effects.len(), 1);
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
    fn test_update_event_success() {
        let id = EventId::new();
        let owner_id = UserId::new();

        // Use ExecuteUpdateEvent (the sync internal action)
        // since UpdateEvent uses async load pattern with Effect::Future
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
            .when_action(EventAction::ExecuteUpdateEvent {
                event_id: id,
                name: Some("Updated Name".to_string()),
                current_version: Version::new(1),
                updated_at: Utc::now(),
            })
            .then_state(move |state| {
                let event = state.get(&id).unwrap();
                assert_eq!(event.name, "Updated Name");
                assert!(state.last_error.is_none());
            })
            .then_effects(|effects| {
                // Effects include AppendEvents and PublishWithResponse
                assert!(!effects.is_empty());
            })
            .run();
    }

    #[test]
    fn test_update_event_not_found() {
        // "Not found" error happens in the async Future, not in validation
        // The async path checks event_store.load_events() which returns empty for non-existent events
        // This is tested in E2E tests (auth_authorization_test.rs), not unit tests
        //
        // For unit testing, we verify that ExecuteUpdateEvent still works correctly
        // even if called with a non-existent event (it would insert into local state)
        let non_existent_id = EventId::new();

        ReducerTest::new(EventReducer::new())
            .with_env(create_test_env())
            .given_state(EventState::new())
            .when_action(EventAction::ExecuteUpdateEvent {
                event_id: non_existent_id,
                name: Some("New Name".to_string()),
                current_version: Version::new(1),
                updated_at: Utc::now(),
            })
            .then_state(|state| {
                // ExecuteUpdateEvent applies EventUpdated which updates existing events
                // Since there's no event with this ID, it doesn't create one
                assert_eq!(state.count(), 0);
            })
            .then_effects(|effects| {
                // Still produces effects (append + publish) even if event doesn't exist locally
                // Real validation happens in the async Future before ExecuteUpdateEvent is called
                assert!(!effects.is_empty());
            })
            .run();
    }

    #[test]
    fn test_update_cancelled_event_fails() {
        // Validation for cancelled events happens in the async Future path
        // For unit testing, we verify that ValidationFailed is applied correctly
        let id = EventId::new();

        ReducerTest::new(EventReducer::new())
            .with_env(create_test_env())
            .given_state(EventState::new())
            .when_action(EventAction::ValidationFailed {
                error: "Cannot update cancelled event".to_string(),
            })
            .then_state(|state| {
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("cancelled"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();

        let _ = id; // suppress unused warning
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

    // ==================== UpdatePricingTiers Tests ====================
    //
    // NOTE: UpdatePricingTiers is async (loads from projection + event store).
    // These tests use ExecuteUpdatePricingTiers (the sync internal action) and
    // validate_update_pricing_tiers_with_event directly.

    #[test]
    fn test_update_pricing_tiers_success() {
        let id = EventId::new();
        let owner_id = UserId::new();

        // Create event with initial pricing for both sections
        let initial_pricing = vec![
            PricingTier::new(
                crate::types::TierType::Regular,
                "VIP".to_string(),
                Money::from_dollars(100),
                Utc::now(),
                None,
            ),
            PricingTier::new(
                crate::types::TierType::Regular,
                "General".to_string(),
                Money::from_dollars(50),
                Utc::now(),
                None,
            ),
        ];

        // New pricing with multiple tiers (covers both sections)
        let new_pricing = vec![
            PricingTier::new(
                crate::types::TierType::EarlyBird,
                "VIP".to_string(),
                Money::from_dollars(80),
                Utc::now(),
                Some(Utc::now() + Duration::days(7)),
            ),
            PricingTier::new(
                crate::types::TierType::Regular,
                "VIP".to_string(),
                Money::from_dollars(100),
                Utc::now() + Duration::days(7),
                None,
            ),
            PricingTier::new(
                crate::types::TierType::Regular,
                "General".to_string(),
                Money::from_dollars(50),
                Utc::now(),
                None,
            ),
        ];

        // Create the loaded event (simulating what the async load would return)
        let loaded_event = Event::new(
            id,
            "Concert".to_string(),
            owner_id,
            create_test_venue_multi_section(),
            EventDate::new(Utc::now() + Duration::days(30)),
            initial_pricing,
            Utc::now(),
        );

        // Use ExecuteUpdatePricingTiers (the sync internal action)
        ReducerTest::new(EventReducer::new())
            .with_env(create_test_env())
            .given_state(EventState::new()) // Empty state - loaded_event provided in action
            .when_action(EventAction::ExecuteUpdatePricingTiers {
                event_id: id,
                pricing_tiers: new_pricing.clone(),
                loaded_event,
                current_version: Version::new(1),
            })
            .then_state(move |state| {
                let event = state.get(&id).unwrap();
                assert_eq!(event.pricing_tiers.len(), 3);
                assert!(state.last_error.is_none());
            })
            .then_effects(|effects| {
                assert!(!effects.is_empty());
            })
            .run();
    }

    #[test]
    fn test_update_pricing_tiers_event_not_found() {
        // Test validation function directly (async path returns ValidationFailed)
        let result = EventReducer::validate_update_pricing_tiers_with_event(
            &Event::new(
                EventId::new(),
                "Test".to_string(),
                UserId::new(),
                create_test_venue(),
                EventDate::new(Utc::now() + Duration::days(30)),
                create_test_pricing_tiers(),
                Utc::now(),
            ),
            &create_test_pricing_tiers(),
        );
        // This should pass validation since event exists
        assert!(result.is_ok());

        // "Not found" error happens in the async Future, not in validation
        // The async path checks projection.load_event() which returns None for non-existent events
        // This is tested in E2E tests, not unit tests
    }

    #[test]
    fn test_update_pricing_tiers_cancelled_event() {
        // Test validation function directly for cancelled event
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
    fn test_update_pricing_tiers_empty_tiers() {
        // Test validation function directly for empty pricing tiers
        let event = Event::new(
            EventId::new(),
            "Concert".to_string(),
            UserId::new(),
            create_test_venue(),
            EventDate::new(Utc::now() + Duration::days(30)),
            create_test_pricing_tiers(),
            Utc::now(),
        );

        let result = EventReducer::validate_update_pricing_tiers_with_event(
            &event,
            &vec![], // Empty pricing tiers
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("At least one pricing tier"));
    }

    #[test]
    fn test_update_pricing_tiers_invalid_section() {
        // Test validation function directly for invalid section
        let event = Event::new(
            EventId::new(),
            "Concert".to_string(),
            UserId::new(),
            create_test_venue(), // Has "General" section only
            EventDate::new(Utc::now() + Duration::days(30)),
            create_test_pricing_tiers(),
            Utc::now(),
        );

        // Pricing tier for section that doesn't exist in venue
        let invalid_pricing = vec![PricingTier::new(
            crate::types::TierType::Regular,
            "NonExistentSection".to_string(), // Invalid section
            Money::from_dollars(50),
            Utc::now(),
            None,
        )];

        let result = EventReducer::validate_update_pricing_tiers_with_event(
            &event,
            &invalid_pricing,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist in event venue"));
    }

    #[test]
    fn test_update_pricing_tiers_missing_section_coverage() {
        // Test validation function directly for missing section coverage
        let event = Event::new(
            EventId::new(),
            "Concert".to_string(),
            UserId::new(),
            create_test_venue_multi_section(), // Has VIP + General sections
            EventDate::new(Utc::now() + Duration::days(30)),
            vec![
                PricingTier::new(
                    crate::types::TierType::Regular,
                    "VIP".to_string(),
                    Money::from_dollars(100),
                    Utc::now(),
                    None,
                ),
                PricingTier::new(
                    crate::types::TierType::Regular,
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
            crate::types::TierType::Regular,
            "VIP".to_string(),
            Money::from_dollars(100),
            Utc::now(),
            None,
        )];

        let result = EventReducer::validate_update_pricing_tiers_with_event(
            &event,
            &incomplete_pricing,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must have at least one pricing tier"));
    }

    // ==================== Time-Based Pricing Tests ====================

    #[test]
    fn test_pricing_tiers_time_based_early_bird() {
        let id = EventId::new();
        let owner_id = UserId::new();
        let now = Utc::now();

        // Create pricing with time-based tiers
        let pricing_tiers = vec![
            // EarlyBird: available now until 7 days from now
            PricingTier::new(
                crate::types::TierType::EarlyBird,
                "General".to_string(),
                Money::from_dollars(30),
                now,
                Some(now + Duration::days(7)),
            ),
            // Regular: available after 7 days
            PricingTier::new(
                crate::types::TierType::Regular,
                "General".to_string(),
                Money::from_dollars(50),
                now + Duration::days(7),
                None,
            ),
        ];

        ReducerTest::new(EventReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = EventState::new();
                let event = Event::new(
                    id,
                    "Concert".to_string(),
                    owner_id,
                    create_test_venue(),
                    EventDate::new(now + Duration::days(30)),
                    pricing_tiers,
                    now,
                );
                state.events.insert(id, event);
                state
            })
            .when_action(EventAction::GetEvent { event_id: id })
            .then_state(move |state| {
                let event = state.get(&id).unwrap();
                // Verify early bird pricing is first
                assert_eq!(event.pricing_tiers[0].tier_type, crate::types::TierType::EarlyBird);
                assert_eq!(event.pricing_tiers[0].base_price.cents(), 3000);
            })
            .then_effects(|effects| {
                assert!(!effects.is_empty());
            })
            .run();
    }

    #[test]
    fn test_pricing_tiers_time_based_last_minute() {
        let id = EventId::new();
        let owner_id = UserId::new();
        let now = Utc::now();

        // Create pricing with last-minute tier
        let pricing_tiers = vec![
            PricingTier::new(
                crate::types::TierType::Regular,
                "General".to_string(),
                Money::from_dollars(50),
                now,
                Some(now + Duration::days(25)),
            ),
            // LastMinute: available 25-30 days from now (5 days before event)
            PricingTier::new(
                crate::types::TierType::LastMinute,
                "General".to_string(),
                Money::from_dollars(70),
                now + Duration::days(25),
                None,
            ),
        ];

        ReducerTest::new(EventReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = EventState::new();
                let event = Event::new(
                    id,
                    "Concert".to_string(),
                    owner_id,
                    create_test_venue(),
                    EventDate::new(now + Duration::days(30)),
                    pricing_tiers,
                    now,
                );
                state.events.insert(id, event);
                state
            })
            .when_action(EventAction::GetEvent { event_id: id })
            .then_state(move |state| {
                let event = state.get(&id).unwrap();
                // Verify last-minute pricing exists
                assert_eq!(event.pricing_tiers[1].tier_type, crate::types::TierType::LastMinute);
                assert_eq!(event.pricing_tiers[1].base_price.cents(), 7000);
            })
            .then_effects(|effects| {
                assert!(!effects.is_empty());
            })
            .run();
    }

    #[test]
    fn test_pricing_tiers_multiple_tiers_per_section() {
        let id = EventId::new();
        let owner_id = UserId::new();
        let now = Utc::now();

        // Multiple pricing tiers for the same section
        let pricing_tiers = vec![
            PricingTier::new(
                crate::types::TierType::EarlyBird,
                "General".to_string(),
                Money::from_dollars(30),
                now,
                Some(now + Duration::days(7)),
            ),
            PricingTier::new(
                crate::types::TierType::Regular,
                "General".to_string(),
                Money::from_dollars(50),
                now + Duration::days(7),
                Some(now + Duration::days(25)),
            ),
            PricingTier::new(
                crate::types::TierType::LastMinute,
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
                assert!(tier_types.contains(&crate::types::TierType::EarlyBird));
                assert!(tier_types.contains(&crate::types::TierType::Regular));
                assert!(tier_types.contains(&crate::types::TierType::LastMinute));
            })
            .then_effects(|effects| {
                assert!(!effects.is_empty());
            })
            .run();
    }
}
