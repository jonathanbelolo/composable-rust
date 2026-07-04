//! Event aggregate business logic.
//!
//! This module contains the pure business logic for event management,
//! implementing the [`BusinessLogic`] trait from the `next` framework.
//!
//! # Architecture
//!
//! The business logic is completely separated from infrastructure:
//! - No database access
//! - No serialization concerns
//! - No version tracking
//! - Just pure domain logic: validate → decide → emit events

use chrono::{DateTime, Utc};
use composable_rust_next::{BusinessLogic, BusinessResult, Clock, StreamId};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;

use crate::types::{Capacity, EventDate, EventId, EventStatus, PricingTier, Venue, VenueSection};
use composable_rust_auth::state::UserId;

// ═══════════════════════════════════════════════════════════════════════════
// Commands (Input)
// ═══════════════════════════════════════════════════════════════════════════

/// Commands that can be sent to the Event aggregate.
///
/// These represent user intentions - what someone wants to do with an event.
///
/// # CQRS Pattern
///
/// Write commands include a `fetched` field that contains validation data
/// pre-loaded from projections by the `QueryFetcher`. This follows the CQRS
/// principle: validation data comes from projections, not from loading state
/// from the event store.
#[derive(Debug, Clone)]
pub enum EventCommand {
    /// Create a new event
    ///
    /// `fetched: None` means the event doesn't exist (can create).
    /// `fetched: Some(_)` means the event already exists (reject as duplicate).
    Create {
        /// Unique identifier for the event
        event_id: EventId,
        /// Event name
        name: String,
        /// User creating the event
        owner_id: UserId,
        /// Venue details
        venue: Venue,
        /// Event date and time
        date: EventDate,
        /// Pricing tiers for tickets
        pricing_tiers: Vec<PricingTier>,
        /// Pre-fetched event data (None = doesn't exist, Some = already exists)
        fetched: Option<EventDto>,
    },

    /// Update an existing event (only allowed in Draft status)
    ///
    /// `fetched` contains the current event state for validation.
    Update {
        /// Event to update
        event_id: EventId,
        /// User making the request (for authorization)
        requesting_user_id: UserId,
        /// New name (if changing)
        name: Option<String>,
        /// New venue (if changing)
        venue: Option<Venue>,
        /// New date (if changing)
        date: Option<EventDate>,
        /// Pre-fetched event data for validation
        fetched: Option<EventDto>,
    },

    /// Publish an event (Draft → Published)
    ///
    /// `fetched` contains the current event state for validation.
    Publish {
        /// Event to publish
        event_id: EventId,
        /// Pre-fetched event data for validation
        fetched: Option<EventDto>,
    },

    /// Cancel an event
    ///
    /// `fetched` contains the current event state for validation.
    Cancel {
        /// Event to cancel
        event_id: EventId,
        /// Reason for cancellation
        reason: String,
        /// Pre-fetched event data for validation
        fetched: Option<EventDto>,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Query Commands
    // ═══════════════════════════════════════════════════════════════════════
    /// Get a single event by ID
    ///
    /// The `fetched` field is populated by the Handler before calling process().
    /// Business logic only does authorization and returns the data.
    GetEvent {
        /// Event to retrieve
        event_id: EventId,
        /// User making the request (for authorization)
        requesting_user_id: UserId,
        /// Pre-fetched event data (populated by Handler)
        fetched: Option<EventDto>,
    },

    /// List events owned by a user
    ///
    /// The `fetched` field is populated by the Handler before calling process().
    ListMyEvents {
        /// User whose events to list
        user_id: UserId,
        /// Pre-fetched events (populated by Handler)
        fetched: Vec<EventDto>,
    },

    /// List all events with optional filtering and pagination
    ///
    /// The `fetched` field is populated by the Handler before calling process().
    ListEvents {
        /// Filter by status (optional)
        status_filter: Option<EventStatus>,
        /// Page number (0-indexed)
        page: usize,
        /// Page size
        page_size: usize,
        /// Pre-fetched events and total count (populated by Handler)
        fetched: (Vec<EventDto>, usize),
    },

    /// Get pricing tiers for an event
    ///
    /// The `fetched` field is populated by the Handler before calling process().
    GetEventPricing {
        /// Event to get pricing for
        event_id: EventId,
        /// Pre-fetched event with pricing (populated by Handler)
        fetched: Option<EventDto>,
    },

    /// Update pricing tiers for an event (only allowed in Draft status)
    ///
    /// `fetched` contains the current event state for validation.
    UpdatePricing {
        /// Event to update
        event_id: EventId,
        /// User making the request (for authorization)
        requesting_user_id: UserId,
        /// New pricing tiers
        pricing_tiers: Vec<PricingTier>,
        /// Pre-fetched event data for validation
        fetched: Option<EventDto>,
    },

    /// Add venue sections to an event (only allowed in Draft status)
    ///
    /// `fetched` contains the current event state for validation.
    AddVenueSections {
        /// Event to update
        event_id: EventId,
        /// User making the request (for authorization)
        requesting_user_id: UserId,
        /// Sections to add
        sections: Vec<VenueSection>,
        /// Pre-fetched event data for validation
        fetched: Option<EventDto>,
    },

    /// Delete (cancel) an event
    ///
    /// `fetched` contains the current event state for validation.
    Delete {
        /// Event to delete
        event_id: EventId,
        /// User making the request (for authorization)
        requesting_user_id: UserId,
        /// Pre-fetched event data for validation
        fetched: Option<EventDto>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// Query Response Types
// ═══════════════════════════════════════════════════════════════════════════

/// Response types for Event queries.
///
/// These are returned via `BusinessResult::Respond` for query commands.
#[derive(Debug, Clone)]
pub enum EventResponse {
    /// Single event details
    Single(EventDto),
    /// List of events (for ListMyEvents - no pagination)
    List(Vec<EventDto>),
    /// Paginated list of events (for ListEvents)
    Paginated {
        /// Events on this page
        events: Vec<EventDto>,
        /// Total count of matching events
        total: usize,
        /// Current page number
        page: usize,
        /// Page size
        page_size: usize,
    },
    /// Pricing tiers for an event
    Pricing {
        /// Event ID
        event_id: EventId,
        /// Pricing tiers
        pricing_tiers: Vec<PricingTier>,
    },
}

/// Event data transfer object for query responses.
///
/// This is the shape of event data returned to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventDto {
    /// Event identifier
    pub id: EventId,
    /// Event name
    pub name: String,
    /// Owner of the event
    pub owner_id: UserId,
    /// Venue details
    pub venue: Venue,
    /// Scheduled date
    pub date: EventDate,
    /// Current status
    pub status: EventStatus,
    /// Pricing tiers
    pub pricing_tiers: Vec<PricingTier>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Events (Output)
// ═══════════════════════════════════════════════════════════════════════════

/// Domain events emitted by the Event aggregate.
///
/// These are the facts that get persisted to the event store.
/// They represent what actually happened, not what was requested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventEvent {
    /// A new event was created
    Created {
        /// Event identifier
        event_id: EventId,
        /// Event name
        name: String,
        /// Owner who created it
        owner_id: UserId,
        /// Venue details
        venue: Venue,
        /// Scheduled date
        date: EventDate,
        /// Ticket pricing
        pricing_tiers: Vec<PricingTier>,
        /// When it was created
        created_at: DateTime<Utc>,
    },

    /// Event details were updated
    Updated {
        /// Event identifier
        event_id: EventId,
        /// New name (if changed)
        name: Option<String>,
        /// New venue (if changed)
        venue: Option<Venue>,
        /// New date (if changed)
        date: Option<EventDate>,
        /// When it was updated
        updated_at: DateTime<Utc>,
    },

    /// Event was published and is now open for ticket sales
    Published {
        /// Event identifier
        event_id: EventId,
        /// When it was published
        published_at: DateTime<Utc>,
    },

    /// Event was cancelled
    Cancelled {
        /// Event identifier
        event_id: EventId,
        /// Reason for cancellation
        reason: String,
        /// When it was cancelled
        cancelled_at: DateTime<Utc>,
    },

    /// Pricing tiers were updated
    PricingUpdated {
        /// Event identifier
        event_id: EventId,
        /// New pricing tiers
        pricing_tiers: Vec<PricingTier>,
        /// When updated
        updated_at: DateTime<Utc>,
    },

    /// Venue sections were added
    VenueSectionsAdded {
        /// Event identifier
        event_id: EventId,
        /// Sections that were added
        sections: Vec<VenueSection>,
        /// When added
        added_at: DateTime<Utc>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// Errors
// ═══════════════════════════════════════════════════════════════════════════

/// Business errors from the Event aggregate.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EventError {
    /// Attempted to create an event that already exists
    #[error("event already exists")]
    AlreadyExists,

    /// Event not found (stream is empty)
    #[error("event not found")]
    NotFound,

    /// Invalid state transition
    #[error("invalid state transition from {from:?} to {to:?}")]
    InvalidStateTransition {
        /// Current status
        from: EventStatus,
        /// Attempted status
        to: EventStatus,
    },

    /// Validation failed
    #[error("validation failed: {message}")]
    ValidationFailed {
        /// What failed
        message: String,
    },

    /// User is not authorized to access this resource
    #[error("forbidden: not authorized to access this resource")]
    Forbidden,
}

// ═══════════════════════════════════════════════════════════════════════════
// State
// ═══════════════════════════════════════════════════════════════════════════

/// Domain state for an Event aggregate.
///
/// This state is reconstructed by replaying events from the event store.
#[derive(Debug, Clone)]
pub struct EventState {
    /// Event identifier (None if no events yet)
    pub event_id: Option<EventId>,
    /// Event name
    pub name: String,
    /// Owner who created the event
    pub owner_id: Option<UserId>,
    /// Venue details
    pub venue: Option<Venue>,
    /// Scheduled date
    pub date: Option<EventDate>,
    /// Current status
    pub status: EventStatus,
    /// Ticket pricing tiers
    pub pricing_tiers: Vec<PricingTier>,
}

impl Default for EventState {
    fn default() -> Self {
        Self {
            event_id: None,
            name: String::new(),
            owner_id: None,
            venue: None,
            date: None,
            status: EventStatus::Draft,
            pricing_tiers: Vec::new(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Business Logic Implementation
// ═══════════════════════════════════════════════════════════════════════════

/// Pure business logic for the Event aggregate.
///
/// This struct implements [`BusinessLogic`] and contains only domain logic:
/// - Validation rules
/// - State transition rules
/// - Event generation
///
/// No infrastructure concerns (persistence, serialization, versioning).
#[derive(Debug, Clone, Default)]
pub struct EventBusinessLogic;

impl BusinessLogic for EventBusinessLogic {
    type State = EventState;
    type Input = EventCommand;
    type Event = EventEvent;
    type Error = EventError;

    // Aggregates never make calls to other aggregates
    type Call = Infallible;
    type CallResult = Infallible;

    // Query responses
    type Response = EventResponse;

    fn stream_id(input: &Self::Input) -> StreamId {
        let event_id = match input {
            EventCommand::Create { event_id, .. }
            | EventCommand::Update { event_id, .. }
            | EventCommand::Publish { event_id, .. }
            | EventCommand::Cancel { event_id, .. }
            | EventCommand::GetEvent { event_id, .. }
            | EventCommand::GetEventPricing { event_id, .. }
            | EventCommand::UpdatePricing { event_id, .. }
            | EventCommand::AddVenueSections { event_id, .. }
            | EventCommand::Delete { event_id, .. } => event_id,
            // Query-only commands don't load a single aggregate - uses placeholder
            EventCommand::ListMyEvents { user_id, .. } => {
                return StreamId::new(format!("query-events-user-{}", user_id.0));
            },
            EventCommand::ListEvents { .. } => {
                return StreamId::new("query-events-all".to_string());
            },
        };
        StreamId::new(format!("event-{event_id}"))
    }

    #[allow(clippy::too_many_lines)] // Match arms for each command variant
    fn process(
        &self,
        input: Self::Input,
        clock: &dyn Clock,
    ) -> Result<BusinessResult<Self::Event, Self::Call, Self::Response>, Self::Error> {
        let now = clock.now();

        match input {
            EventCommand::Create {
                event_id,
                name,
                owner_id,
                venue,
                date,
                pricing_tiers,
                fetched,
            } => {
                // Validate: event must not already exist (fetched = None means doesn't exist)
                if fetched.is_some() {
                    return Err(EventError::AlreadyExists);
                }

                // Validate: name must not be empty
                if name.trim().is_empty() {
                    return Err(EventError::ValidationFailed {
                        message: "event name cannot be empty".to_string(),
                    });
                }

                // Validate: must have at least one pricing tier
                if pricing_tiers.is_empty() {
                    return Err(EventError::ValidationFailed {
                        message: "event must have at least one pricing tier".to_string(),
                    });
                }

                Ok(BusinessResult::Done(vec![EventEvent::Created {
                    event_id,
                    name,
                    owner_id,
                    venue,
                    date,
                    pricing_tiers,
                    created_at: now,
                }]))
            },

            EventCommand::Update {
                event_id,
                requesting_user_id,
                name,
                venue,
                date,
                fetched,
            } => {
                // Validate: event must exist (fetched = Some means it exists)
                let event_data = fetched.ok_or(EventError::NotFound)?;

                // Authorize: only owner can update
                if event_data.owner_id != requesting_user_id {
                    return Err(EventError::Forbidden);
                }

                // Validate: can only update in Draft status
                if event_data.status != EventStatus::Draft {
                    return Err(EventError::InvalidStateTransition {
                        from: event_data.status,
                        to: event_data.status, // Not actually transitioning, just updating
                    });
                }

                // Validate: at least one field must be updated
                if name.is_none() && venue.is_none() && date.is_none() {
                    return Err(EventError::ValidationFailed {
                        message: "at least one field must be updated".to_string(),
                    });
                }

                // Validate: name cannot be empty if provided
                if let Some(ref n) = name {
                    if n.trim().is_empty() {
                        return Err(EventError::ValidationFailed {
                            message: "event name cannot be empty".to_string(),
                        });
                    }
                }

                Ok(BusinessResult::Done(vec![EventEvent::Updated {
                    event_id,
                    name,
                    venue,
                    date,
                    updated_at: now,
                }]))
            },

            EventCommand::Publish { event_id, fetched } => {
                // Validate: event must exist (fetched = Some means it exists)
                let event_data = fetched.ok_or(EventError::NotFound)?;

                // Validate: can only publish from Draft status
                if event_data.status != EventStatus::Draft {
                    return Err(EventError::InvalidStateTransition {
                        from: event_data.status,
                        to: EventStatus::Published,
                    });
                }

                Ok(BusinessResult::Done(vec![EventEvent::Published {
                    event_id,
                    published_at: now,
                }]))
            },

            EventCommand::Cancel {
                event_id,
                reason,
                fetched,
            } => {
                // Validate: event must exist (fetched = Some means it exists)
                let event_data = fetched.ok_or(EventError::NotFound)?;

                // Validate: cannot cancel already cancelled events
                if event_data.status == EventStatus::Cancelled {
                    return Err(EventError::InvalidStateTransition {
                        from: EventStatus::Cancelled,
                        to: EventStatus::Cancelled,
                    });
                }

                // Validate: reason must not be empty
                if reason.trim().is_empty() {
                    return Err(EventError::ValidationFailed {
                        message: "cancellation reason cannot be empty".to_string(),
                    });
                }

                Ok(BusinessResult::Done(vec![EventEvent::Cancelled {
                    event_id,
                    reason,
                    cancelled_at: now,
                }]))
            },

            // ═══════════════════════════════════════════════════════════════
            // Query Commands
            // ═══════════════════════════════════════════════════════════════
            EventCommand::GetEvent {
                event_id: _,
                requesting_user_id: _,
                fetched,
            } => {
                // Data was pre-fetched by Handler - just return it
                let event = fetched.ok_or(EventError::NotFound)?;

                // Events are publicly viewable - no authorization required
                // (Write operations like Update/Delete still require ownership)
                Ok(BusinessResult::Respond(EventResponse::Single(event)))
            },

            EventCommand::ListMyEvents {
                user_id: _,
                fetched,
            } => {
                // Data was pre-fetched by Handler, already scoped to user
                // No additional authorization needed - query was already scoped
                Ok(BusinessResult::Respond(EventResponse::List(fetched)))
            },

            EventCommand::ListEvents {
                status_filter: _,
                page,
                page_size,
                fetched,
            } => {
                // Data was pre-fetched by Handler with filtering applied
                let (events, total) = fetched;
                Ok(BusinessResult::Respond(EventResponse::Paginated {
                    events,
                    total,
                    page,
                    page_size,
                }))
            },

            EventCommand::GetEventPricing { event_id, fetched } => {
                // Data was pre-fetched by Handler
                let event = fetched.ok_or(EventError::NotFound)?;
                Ok(BusinessResult::Respond(EventResponse::Pricing {
                    event_id,
                    pricing_tiers: event.pricing_tiers,
                }))
            },

            // ═══════════════════════════════════════════════════════════════
            // Write Commands (validation data from fetched projections)
            // ═══════════════════════════════════════════════════════════════
            EventCommand::UpdatePricing {
                event_id,
                requesting_user_id,
                pricing_tiers,
                fetched,
            } => {
                // Validate: event must exist (fetched = Some means it exists)
                let event_data = fetched.ok_or(EventError::NotFound)?;

                // Authorize: only owner can update pricing
                if event_data.owner_id != requesting_user_id {
                    return Err(EventError::Forbidden);
                }

                // Validate: can only update pricing in Draft status
                if event_data.status != EventStatus::Draft {
                    return Err(EventError::InvalidStateTransition {
                        from: event_data.status,
                        to: event_data.status,
                    });
                }

                // Validate: at least one pricing tier required
                if pricing_tiers.is_empty() {
                    return Err(EventError::ValidationFailed {
                        message: "at least one pricing tier is required".to_string(),
                    });
                }

                Ok(BusinessResult::Done(vec![EventEvent::PricingUpdated {
                    event_id,
                    pricing_tiers,
                    updated_at: now,
                }]))
            },

            EventCommand::AddVenueSections {
                event_id,
                requesting_user_id,
                sections,
                fetched,
            } => {
                // Validate: event must exist (fetched = Some means it exists)
                let event_data = fetched.ok_or(EventError::NotFound)?;

                // Authorize: only owner can add sections
                if event_data.owner_id != requesting_user_id {
                    return Err(EventError::Forbidden);
                }

                // Validate: can only add sections in Draft status
                if event_data.status != EventStatus::Draft {
                    return Err(EventError::InvalidStateTransition {
                        from: event_data.status,
                        to: event_data.status,
                    });
                }

                // Validate: at least one section required
                if sections.is_empty() {
                    return Err(EventError::ValidationFailed {
                        message: "at least one section is required".to_string(),
                    });
                }

                Ok(BusinessResult::Done(vec![EventEvent::VenueSectionsAdded {
                    event_id,
                    sections,
                    added_at: now,
                }]))
            },

            EventCommand::Delete {
                event_id,
                requesting_user_id,
                fetched,
            } => {
                // Validate: event must exist (fetched = Some means it exists)
                let event_data = fetched.ok_or(EventError::NotFound)?;

                // Authorize: only owner can delete
                if event_data.owner_id != requesting_user_id {
                    return Err(EventError::Forbidden);
                }

                // Validate: cannot delete already cancelled events
                if event_data.status == EventStatus::Cancelled {
                    return Err(EventError::InvalidStateTransition {
                        from: EventStatus::Cancelled,
                        to: EventStatus::Cancelled,
                    });
                }

                Ok(BusinessResult::Done(vec![EventEvent::Cancelled {
                    event_id,
                    reason: "Deleted by owner".to_string(),
                    cancelled_at: now,
                }]))
            },
        }
    }

    fn apply(&self, state: &mut Self::State, event: &Self::Event) {
        match event {
            EventEvent::Created {
                event_id,
                name,
                owner_id,
                venue,
                date,
                pricing_tiers,
                created_at: _,
            } => {
                state.event_id = Some(*event_id);
                state.name.clone_from(name);
                state.owner_id = Some(*owner_id);
                state.venue = Some(venue.clone());
                state.date = Some(*date);
                state.pricing_tiers.clone_from(pricing_tiers);
                state.status = EventStatus::Draft;
            },

            EventEvent::Updated {
                event_id: _,
                name,
                venue,
                date,
                updated_at: _,
            } => {
                if let Some(n) = name {
                    state.name.clone_from(n);
                }
                if let Some(v) = venue {
                    state.venue = Some(v.clone());
                }
                if let Some(d) = date {
                    state.date = Some(*d);
                }
            },

            EventEvent::Published {
                event_id: _,
                published_at: _,
            } => {
                state.status = EventStatus::Published;
            },

            EventEvent::Cancelled {
                event_id: _,
                reason: _,
                cancelled_at: _,
            } => {
                state.status = EventStatus::Cancelled;
            },

            EventEvent::PricingUpdated {
                event_id: _,
                pricing_tiers,
                updated_at: _,
            } => {
                state.pricing_tiers.clone_from(pricing_tiers);
            },

            EventEvent::VenueSectionsAdded {
                event_id: _,
                sections,
                added_at: _,
            } => {
                // Add new sections to the venue
                if let Some(ref mut venue) = state.venue {
                    venue.sections.extend(sections.iter().cloned());
                    // Update total capacity
                    let additional_capacity: u32 =
                        sections.iter().map(|s| s.capacity.value()).sum();
                    venue.capacity = Capacity::new(venue.capacity.value() + additional_capacity);
                }
            },
        }
    }

    fn event_type_name(event: &Self::Event) -> &'static str {
        match event {
            EventEvent::Created { .. } => "EventCreated",
            EventEvent::Updated { .. } => "EventUpdated",
            EventEvent::Published { .. } => "EventPublished",
            EventEvent::Cancelled { .. } => "EventCancelled",
            EventEvent::PricingUpdated { .. } => "EventPricingUpdated",
            EventEvent::VenueSectionsAdded { .. } => "EventVenueSectionsAdded",
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::types::{Capacity, Money, TierType};
    use composable_rust_next::FixedClock;

    fn test_clock() -> FixedClock {
        FixedClock::new(Utc::now())
    }

    fn sample_venue() -> Venue {
        Venue {
            name: "Test Arena".to_string(),
            capacity: Capacity::new(1000),
            sections: vec![],
        }
    }

    fn sample_pricing_tier() -> PricingTier {
        PricingTier {
            tier_type: TierType::Regular,
            section: "General".to_string(),
            base_price: Money::from_cents(5000),
            available_from: Utc::now(),
            available_until: None,
        }
    }

    /// Helper to create a sample EventDto for testing
    fn sample_event_dto(event_id: EventId, owner_id: UserId, status: EventStatus) -> EventDto {
        EventDto {
            id: event_id,
            name: "Test Event".to_string(),
            owner_id,
            venue: sample_venue(),
            date: EventDate::new(Utc::now()),
            status,
            pricing_tiers: vec![sample_pricing_tier()],
        }
    }

    #[test]
    fn create_event_succeeds() {
        let logic = EventBusinessLogic;
        let clock = test_clock();

        // fetched = None means event doesn't exist, so we can create
        let result = logic.process(
            EventCommand::Create {
                event_id: EventId::new(),
                name: "Concert".to_string(),
                owner_id: UserId::new(),
                venue: sample_venue(),
                date: EventDate::new(Utc::now()),
                pricing_tiers: vec![sample_pricing_tier()],
                fetched: None, // Event doesn't exist
            },
            &clock,
        );

        assert!(result.is_ok());
        let BusinessResult::Done(events) = result.unwrap() else {
            panic!("expected Done");
        };
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], EventEvent::Created { .. }));
    }

    #[test]
    fn create_duplicate_event_fails() {
        let logic = EventBusinessLogic;
        let clock = test_clock();

        let event_id = EventId::new();
        let owner_id = UserId::new();

        // fetched = Some means event already exists
        let result = logic.process(
            EventCommand::Create {
                event_id,
                name: "Concert".to_string(),
                owner_id,
                venue: sample_venue(),
                date: EventDate::new(Utc::now()),
                pricing_tiers: vec![sample_pricing_tier()],
                fetched: Some(sample_event_dto(event_id, owner_id, EventStatus::Draft)),
            },
            &clock,
        );

        assert!(matches!(result, Err(EventError::AlreadyExists)));
    }

    #[test]
    fn publish_draft_event_succeeds() {
        let logic = EventBusinessLogic;
        let event_id = EventId::new();
        let owner_id = UserId::new();
        let clock = test_clock();

        // Provide fetched data showing event exists in Draft status
        let result = logic.process(
            EventCommand::Publish {
                event_id,
                fetched: Some(sample_event_dto(event_id, owner_id, EventStatus::Draft)),
            },
            &clock,
        );

        assert!(result.is_ok());
        let BusinessResult::Done(events) = result.unwrap() else {
            panic!("expected Done");
        };
        assert!(matches!(events[0], EventEvent::Published { .. }));
    }

    #[test]
    fn publish_already_published_fails() {
        let logic = EventBusinessLogic;
        let event_id = EventId::new();
        let owner_id = UserId::new();
        let clock = test_clock();

        // Provide fetched data showing event is already Published
        let result = logic.process(
            EventCommand::Publish {
                event_id,
                fetched: Some(sample_event_dto(event_id, owner_id, EventStatus::Published)),
            },
            &clock,
        );

        assert!(matches!(
            result,
            Err(EventError::InvalidStateTransition { .. })
        ));
    }

    #[test]
    fn apply_created_event() {
        let logic = EventBusinessLogic;
        let mut state = EventState::default();
        let event_id = EventId::new();

        logic.apply(
            &mut state,
            &EventEvent::Created {
                event_id,
                name: "Concert".to_string(),
                owner_id: UserId::new(),
                venue: sample_venue(),
                date: EventDate::new(Utc::now()),
                pricing_tiers: vec![sample_pricing_tier()],
                created_at: Utc::now(),
            },
        );

        assert_eq!(state.event_id, Some(event_id));
        assert_eq!(state.name, "Concert");
        assert_eq!(state.status, EventStatus::Draft);
    }
}

/// Handler-level integration tests using in-memory dependencies.
///
/// These tests demonstrate the full flow: command → Handler → events persisted.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod handler_tests {
    use super::*;
    use crate::next::environment::{NoOpEventBus, NoOpProjector, TicketingEnvironment};
    use crate::types::{Capacity, Money, TierType};
    use composable_rust_next::testing::{InMemoryEventBus, InMemoryEventStore, InMemoryProjector};
    use composable_rust_next::{FixedClock, Handler, NoOpCallExecutor, NoOpQueryFetcher};

    fn sample_venue() -> Venue {
        Venue {
            name: "Test Arena".to_string(),
            capacity: Capacity::new(1000),
            sections: vec![],
        }
    }

    fn sample_pricing_tier() -> PricingTier {
        PricingTier {
            tier_type: TierType::Regular,
            section: "General".to_string(),
            base_price: Money::from_cents(5000),
            available_from: Utc::now(),
            available_until: None,
        }
    }

    /// Test environment type with all in-memory dependencies.
    type TestEnv =
        TicketingEnvironment<FixedClock, InMemoryEventStore, InMemoryProjector, InMemoryEventBus>;

    /// Type alias for a test handler using in-memory dependencies.
    type TestHandler = Handler<EventBusinessLogic, NoOpCallExecutor, NoOpQueryFetcher, TestEnv>;

    /// Creates a test environment with shared in-memory stores.
    fn create_test_env() -> (
        TestEnv,
        InMemoryEventStore,
        InMemoryProjector,
        InMemoryEventBus,
    ) {
        let clock = FixedClock::new(Utc::now());
        let event_store = InMemoryEventStore::new();
        let projector = InMemoryProjector::new();
        let event_bus = InMemoryEventBus::new();

        let env = TicketingEnvironment::new(
            clock,
            event_store.clone(),
            Some(projector.clone()),
            Some(event_bus.clone()),
            "test-events",
        );

        (env, event_store, projector, event_bus)
    }

    /// Helper to create a sample EventDto for handler tests
    fn sample_event_dto(event_id: EventId, owner_id: UserId, status: EventStatus) -> EventDto {
        EventDto {
            id: event_id,
            name: "Test Event".to_string(),
            owner_id,
            venue: sample_venue(),
            date: EventDate::new(Utc::now()),
            status,
            pricing_tiers: vec![sample_pricing_tier()],
        }
    }

    #[tokio::test]
    async fn handler_creates_event_and_persists() {
        // Arrange: Create handler with in-memory environment
        let (env, event_store, projector, event_bus) = create_test_env();
        let handler = TestHandler::new(EventBusinessLogic, NoOpCallExecutor, NoOpQueryFetcher, env);

        let event_id = EventId::new();

        // Act: Handle a create event command
        // fetched: None means event doesn't exist (can create)
        let result = handler
            .handle(EventCommand::Create {
                event_id,
                name: "Test Concert".to_string(),
                owner_id: UserId::new(),
                venue: sample_venue(),
                date: EventDate::new(Utc::now()),
                pricing_tiers: vec![sample_pricing_tier()],
                fetched: None,
            })
            .await;

        // Assert: Command succeeded
        assert!(result.is_ok());

        // Assert: Events were persisted to the in-memory store
        let stream_id = format!("event-{}", event_id.as_uuid());
        let stored_events = event_store.events_for_stream(&stream_id);
        assert_eq!(stored_events.len(), 1);
        assert_eq!(stored_events[0].event_type, "EventCreated");

        // Assert: Events were projected
        let projected = projector.projected_events();
        assert_eq!(projected.len(), 1);

        // Assert: Events were broadcast
        let published = event_bus.published_events();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].0, "test-events"); // topic
    }

    #[tokio::test]
    async fn handler_publishes_event_after_create() {
        // Arrange: Create handler
        let (env, event_store, _projector, _event_bus) = create_test_env();
        let handler = TestHandler::new(EventBusinessLogic, NoOpCallExecutor, NoOpQueryFetcher, env);

        let event_id = EventId::new();
        let owner_id = UserId::new();

        // First create the event
        handler
            .handle(EventCommand::Create {
                event_id,
                name: "Test Concert".to_string(),
                owner_id,
                venue: sample_venue(),
                date: EventDate::new(Utc::now()),
                pricing_tiers: vec![sample_pricing_tier()],
                fetched: None,
            })
            .await
            .unwrap();

        // Act: Publish the event (provide fetched data showing it exists in Draft)
        let result = handler
            .handle(EventCommand::Publish {
                event_id,
                fetched: Some(sample_event_dto(event_id, owner_id, EventStatus::Draft)),
            })
            .await;

        // Assert: Command succeeded
        assert!(result.is_ok());

        // Assert: Both events persisted (Created + Published)
        let stream_id = format!("event-{}", event_id.as_uuid());
        let stored_events = event_store.events_for_stream(&stream_id);
        assert_eq!(stored_events.len(), 2);
        assert_eq!(stored_events[0].event_type, "EventCreated");
        assert_eq!(stored_events[1].event_type, "EventPublished");
    }

    #[tokio::test]
    async fn handler_rejects_duplicate_create() {
        // Arrange: Create handler and first event
        let (env, _event_store, _projector, _event_bus) = create_test_env();
        let handler = TestHandler::new(EventBusinessLogic, NoOpCallExecutor, NoOpQueryFetcher, env);

        let event_id = EventId::new();
        let owner_id = UserId::new();

        // Create first event
        handler
            .handle(EventCommand::Create {
                event_id,
                name: "Test Concert".to_string(),
                owner_id,
                venue: sample_venue(),
                date: EventDate::new(Utc::now()),
                pricing_tiers: vec![sample_pricing_tier()],
                fetched: None,
            })
            .await
            .unwrap();

        // Act: Try to create duplicate (fetched = Some means it already exists)
        let result = handler
            .handle(EventCommand::Create {
                event_id,
                name: "Another Concert".to_string(),
                owner_id: UserId::new(),
                venue: sample_venue(),
                date: EventDate::new(Utc::now()),
                pricing_tiers: vec![sample_pricing_tier()],
                fetched: Some(sample_event_dto(event_id, owner_id, EventStatus::Draft)),
            })
            .await;

        // Assert: Business error returned
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.is_business());
    }

    #[tokio::test]
    async fn handler_rejects_publish_nonexistent_event() {
        // Arrange: Create handler (no events created) - use minimal env
        let clock = FixedClock::new(Utc::now());
        let event_store = InMemoryEventStore::new();
        let env: TicketingEnvironment<_, _, NoOpProjector, NoOpEventBus> =
            TicketingEnvironment::new(clock, event_store, None, None, "");
        let handler = Handler::new(EventBusinessLogic, NoOpCallExecutor, NoOpQueryFetcher, env);

        // Act: Try to publish non-existent event (fetched: None means not found)
        let result = handler
            .handle(EventCommand::Publish {
                event_id: EventId::new(),
                fetched: None,
            })
            .await;

        // Assert: Business error (NotFound)
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.is_business());
    }
}
