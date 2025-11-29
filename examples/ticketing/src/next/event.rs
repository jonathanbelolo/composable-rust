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

use crate::types::{EventDate, EventId, EventStatus, PricingTier, Venue};
use composable_rust_auth::state::UserId;

// ═══════════════════════════════════════════════════════════════════════════
// Commands (Input)
// ═══════════════════════════════════════════════════════════════════════════

/// Commands that can be sent to the Event aggregate.
///
/// These represent user intentions - what someone wants to do with an event.
#[derive(Debug, Clone)]
pub enum EventCommand {
    /// Create a new event
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
    },

    /// Update an existing event (only allowed in Draft status)
    Update {
        /// Event to update
        event_id: EventId,
        /// New name (if changing)
        name: Option<String>,
        /// New venue (if changing)
        venue: Option<Venue>,
        /// New date (if changing)
        date: Option<EventDate>,
    },

    /// Publish an event (Draft → Published)
    Publish {
        /// Event to publish
        event_id: EventId,
    },

    /// Cancel an event
    Cancel {
        /// Event to cancel
        event_id: EventId,
        /// Reason for cancellation
        reason: String,
    },
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

    fn stream_id(input: &Self::Input) -> StreamId {
        let event_id = match input {
            EventCommand::Create { event_id, .. }
            | EventCommand::Update { event_id, .. }
            | EventCommand::Publish { event_id }
            | EventCommand::Cancel { event_id, .. } => event_id,
        };
        StreamId::new(format!("event-{event_id}"))
    }

    #[allow(clippy::too_many_lines)] // Match arms for each command variant
    fn process(
        &self,
        state: &Self::State,
        input: Self::Input,
        clock: &dyn Clock,
    ) -> Result<BusinessResult<Self::Event, Self::Call>, Self::Error> {
        let now = clock.now();

        match input {
            EventCommand::Create {
                event_id,
                name,
                owner_id,
                venue,
                date,
                pricing_tiers,
            } => {
                // Validate: event must not already exist
                if state.event_id.is_some() {
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
            }

            EventCommand::Update {
                event_id: _,
                name,
                venue,
                date,
            } => {
                // Validate: event must exist
                if state.event_id.is_none() {
                    return Err(EventError::NotFound);
                }

                // Validate: can only update in Draft status
                if state.status != EventStatus::Draft {
                    return Err(EventError::InvalidStateTransition {
                        from: state.status,
                        to: state.status, // Not actually transitioning, just updating
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

                // Use state's event_id since we validated it exists
                let event_id = state.event_id.ok_or(EventError::NotFound)?;

                Ok(BusinessResult::Done(vec![EventEvent::Updated {
                    event_id,
                    name,
                    venue,
                    date,
                    updated_at: now,
                }]))
            }

            EventCommand::Publish { event_id: _ } => {
                // Validate: event must exist
                if state.event_id.is_none() {
                    return Err(EventError::NotFound);
                }

                // Validate: can only publish from Draft status
                if state.status != EventStatus::Draft {
                    return Err(EventError::InvalidStateTransition {
                        from: state.status,
                        to: EventStatus::Published,
                    });
                }

                let event_id = state.event_id.ok_or(EventError::NotFound)?;

                Ok(BusinessResult::Done(vec![EventEvent::Published {
                    event_id,
                    published_at: now,
                }]))
            }

            EventCommand::Cancel { event_id: _, reason } => {
                // Validate: event must exist
                if state.event_id.is_none() {
                    return Err(EventError::NotFound);
                }

                // Validate: cannot cancel already cancelled events
                if state.status == EventStatus::Cancelled {
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

                let event_id = state.event_id.ok_or(EventError::NotFound)?;

                Ok(BusinessResult::Done(vec![EventEvent::Cancelled {
                    event_id,
                    reason,
                    cancelled_at: now,
                }]))
            }
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
            }

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
            }

            EventEvent::Published {
                event_id: _,
                published_at: _,
            } => {
                state.status = EventStatus::Published;
            }

            EventEvent::Cancelled {
                event_id: _,
                reason: _,
                cancelled_at: _,
            } => {
                state.status = EventStatus::Cancelled;
            }
        }
    }

    fn event_type_name(event: &Self::Event) -> &'static str {
        match event {
            EventEvent::Created { .. } => "EventCreated",
            EventEvent::Updated { .. } => "EventUpdated",
            EventEvent::Published { .. } => "EventPublished",
            EventEvent::Cancelled { .. } => "EventCancelled",
        }
    }
}

#[cfg(test)]
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

    #[test]
    fn create_event_succeeds() {
        let logic = EventBusinessLogic;
        let state = EventState::default();
        let clock = test_clock();

        let result = logic.process(
            &state,
            EventCommand::Create {
                event_id: EventId::new(),
                name: "Concert".to_string(),
                owner_id: UserId::new(),
                venue: sample_venue(),
                date: EventDate::new(Utc::now()),
                pricing_tiers: vec![sample_pricing_tier()],
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
        let mut state = EventState::default();
        state.event_id = Some(EventId::new()); // Already exists
        let clock = test_clock();

        let result = logic.process(
            &state,
            EventCommand::Create {
                event_id: EventId::new(),
                name: "Concert".to_string(),
                owner_id: UserId::new(),
                venue: sample_venue(),
                date: EventDate::new(Utc::now()),
                pricing_tiers: vec![sample_pricing_tier()],
            },
            &clock,
        );

        assert!(matches!(result, Err(EventError::AlreadyExists)));
    }

    #[test]
    fn publish_draft_event_succeeds() {
        let logic = EventBusinessLogic;
        let event_id = EventId::new();
        let state = EventState {
            event_id: Some(event_id),
            status: EventStatus::Draft,
            ..Default::default()
        };
        let clock = test_clock();

        let result = logic.process(&state, EventCommand::Publish { event_id }, &clock);

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
        let state = EventState {
            event_id: Some(event_id),
            status: EventStatus::Published,
            ..Default::default()
        };
        let clock = test_clock();

        let result = logic.process(&state, EventCommand::Publish { event_id }, &clock);

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
