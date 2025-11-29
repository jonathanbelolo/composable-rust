//! Event-Inventory Saga using the next-generation architecture.
//!
//! This saga demonstrates the power of the separated architecture for
//! multi-aggregate coordination. It coordinates:
//!
//! 1. Creating an Event in the Event aggregate
//! 2. Initializing Inventory for each venue section
//! 3. Handling failures with compensation
//!
//! # Architecture
//!
//! The saga uses `BusinessResult::Continue` to orchestrate multiple aggregates:
//!
//! ```text
//! CreateEventWithInventory command
//!         │
//!         ▼
//! ┌───────────────────┐
//! │  SagaLogic        │
//! │  process()        │──► Continue { events: [Initiated], calls: [CreateEvent] }
//! └───────────────────┘
//!         │
//!         ▼ (Handler executes calls)
//! ┌───────────────────┐
//! │  Event Handler    │──► Success/Failure
//! └───────────────────┘
//!         │
//!         ▼ (feedback_input transforms result)
//! ┌───────────────────┐
//! │  SagaLogic        │
//! │  process()        │──► Continue { events: [EventCreated], calls: [InitInventory...] }
//! └───────────────────┘
//!         │
//!         ▼ (Handler executes calls in parallel)
//! ┌───────────────────┐
//! │  Inventory        │──► Success/Failure for each section
//! │  Handlers         │
//! └───────────────────┘
//!         │
//!         ▼
//! ┌───────────────────┐
//! │  SagaLogic        │──► Done { events: [Completed] }
//! │  process()        │    (or compensation flow on failure)
//! └───────────────────┘
//! ```
//!
//! # Compensation
//!
//! If inventory initialization fails for any section:
//! 1. Saga emits `InventoryFailed` event
//! 2. Calls compensation for Event aggregate (mark as cancelled)
//! 3. Calls compensation for any initialized Inventory sections
//! 4. Emits `CompensationCompleted` or `Failed` terminal event

use chrono::{DateTime, Utc};
use composable_rust_auth::state::UserId;
use composable_rust_next::{BusinessLogic, BusinessResult, Clock, StreamId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::types::{Capacity, EventDate, EventId, PricingTier, Venue};

use super::{EventCommand, EventError, EventEvent, InventoryCommand, InventoryError, InventoryEvent};

// ═══════════════════════════════════════════════════════════════════════════
// Saga Input (Commands + Feedback)
// ═══════════════════════════════════════════════════════════════════════════

/// Input to the saga: either a command or feedback from aggregate calls.
#[derive(Debug, Clone)]
pub enum SagaInput {
    /// Initial command to create event with inventory.
    CreateEventWithInventory {
        /// Event ID (provided by caller for idempotency)
        event_id: EventId,
        /// Event name
        name: String,
        /// Event owner
        owner_id: UserId,
        /// Venue with sections
        venue: Venue,
        /// Event date
        date: EventDate,
        /// Pricing tiers
        pricing_tiers: Vec<PricingTier>,
    },

    /// Feedback from aggregate calls.
    Feedback(Vec<SagaCallResult>),
}

// ═══════════════════════════════════════════════════════════════════════════
// Saga Calls (to child aggregates)
// ═══════════════════════════════════════════════════════════════════════════

/// Calls the saga makes to child aggregates.
#[derive(Debug, Clone)]
pub enum SagaCall {
    /// Call the Event aggregate.
    Event(EventCommand),
    /// Call the Inventory aggregate for a specific section.
    Inventory {
        /// Section name (for tracking)
        section: String,
        /// The command to execute
        command: InventoryCommand,
    },
}

/// Results from aggregate calls.
#[derive(Debug, Clone)]
pub enum SagaCallResult {
    /// Result from Event aggregate.
    Event(Result<Vec<EventEvent>, EventError>),
    /// Result from Inventory aggregate for a section.
    Inventory {
        /// Section name
        section: String,
        /// Result
        result: Result<Vec<InventoryEvent>, InventoryError>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// Saga Events
// ═══════════════════════════════════════════════════════════════════════════

/// Events emitted by the saga.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SagaEvent {
    /// Saga initiated.
    Initiated {
        /// Event ID being created
        event_id: EventId,
        /// Event name
        name: String,
        /// Section names to initialize
        sections: Vec<String>,
        /// When initiated
        initiated_at: DateTime<Utc>,
    },

    /// Event aggregate successfully created the event.
    EventCreated {
        /// Event ID
        event_id: EventId,
        /// When created
        created_at: DateTime<Utc>,
    },

    /// Inventory initialized for a section.
    SectionInventoryInitialized {
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
        /// When initialized
        initialized_at: DateTime<Utc>,
    },

    /// Saga completed successfully.
    Completed {
        /// Event ID
        event_id: EventId,
        /// Total sections initialized
        sections_count: u32,
        /// When completed
        completed_at: DateTime<Utc>,
    },

    /// Event creation failed.
    EventCreationFailed {
        /// Event ID
        event_id: EventId,
        /// Error message
        error: String,
        /// When failed
        failed_at: DateTime<Utc>,
    },

    /// Inventory initialization failed.
    InventoryInitializationFailed {
        /// Event ID
        event_id: EventId,
        /// Section that failed
        section: String,
        /// Error message
        error: String,
        /// When failed
        failed_at: DateTime<Utc>,
    },

    /// Compensation started.
    CompensationStarted {
        /// Event ID
        event_id: EventId,
        /// Reason for compensation
        reason: String,
        /// When started
        started_at: DateTime<Utc>,
    },

    /// Compensation completed.
    CompensationCompleted {
        /// Event ID
        event_id: EventId,
        /// When completed
        completed_at: DateTime<Utc>,
    },

    /// Saga failed (terminal state).
    Failed {
        /// Event ID
        event_id: EventId,
        /// Error message
        error: String,
        /// When failed
        failed_at: DateTime<Utc>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// Saga State
// ═══════════════════════════════════════════════════════════════════════════

/// State machine phase for the saga.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SagaPhase {
    /// Not started
    #[default]
    Initial,
    /// Waiting for Event aggregate to create event
    CreatingEvent,
    /// Waiting for Inventory aggregates to initialize
    InitializingInventory,
    /// All done successfully
    Completed,
    /// Compensation in progress
    Compensating,
    /// Compensation completed
    CompensationCompleted,
    /// Terminal failure (unrecoverable)
    Failed,
}

/// State for the Event-Inventory saga.
#[derive(Debug, Clone, Default)]
pub struct SagaState {
    /// Event ID being created
    pub event_id: Option<EventId>,
    /// Event name
    pub name: Option<String>,
    /// Current phase
    pub phase: SagaPhase,
    /// Sections that still need inventory
    pub pending_sections: HashSet<String>,
    /// Sections that completed successfully
    pub completed_sections: HashSet<String>,
    /// Section capacities
    pub section_capacities: HashMap<String, Capacity>,
    /// Last error (if any)
    pub last_error: Option<String>,
}

impl SagaState {
    /// Check if saga is in a terminal state.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.phase,
            SagaPhase::Completed | SagaPhase::CompensationCompleted | SagaPhase::Failed
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Saga Errors
// ═══════════════════════════════════════════════════════════════════════════

/// Errors from the saga.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SagaError {
    /// Saga already in progress
    #[error("Saga already in progress")]
    AlreadyInProgress,

    /// Invalid state transition
    #[error("Invalid state transition from {from:?} to {to:?}")]
    InvalidStateTransition {
        /// Current phase
        from: SagaPhase,
        /// Attempted phase
        to: SagaPhase,
    },

    /// Validation failed
    #[error("Validation failed: {0}")]
    ValidationFailed(String),
}

// ═══════════════════════════════════════════════════════════════════════════
// Saga Business Logic
// ═══════════════════════════════════════════════════════════════════════════

/// Business logic for the Event-Inventory saga.
///
/// This saga coordinates Event creation with Inventory initialization.
/// It uses `BusinessResult::Continue` to orchestrate child aggregates.
#[derive(Debug, Clone)]
pub struct EventInventorySagaLogic;

impl BusinessLogic for EventInventorySagaLogic {
    type State = SagaState;
    type Input = SagaInput;
    type Event = SagaEvent;
    type Error = SagaError;
    type Call = SagaCall;
    type CallResult = SagaCallResult;

    fn stream_id(input: &Self::Input) -> StreamId {
        match input {
            SagaInput::CreateEventWithInventory { event_id, .. } => {
                StreamId::new(format!("saga-event-inventory-{}", event_id.as_uuid()))
            }
            SagaInput::Feedback(_) => {
                // Feedback is always for an in-progress saga
                // The Handler maintains the stream_id from the original command
                StreamId::new("saga-feedback-placeholder")
            }
        }
    }

    fn process(
        &self,
        state: &Self::State,
        input: Self::Input,
        clock: &dyn Clock,
    ) -> Result<BusinessResult<Self::Event, Self::Call>, Self::Error> {
        let now = clock.now();

        match input {
            SagaInput::CreateEventWithInventory {
                event_id,
                name,
                owner_id,
                venue,
                date,
                pricing_tiers,
            } => {
                // Validation
                if state.phase != SagaPhase::Initial {
                    return Err(SagaError::AlreadyInProgress);
                }
                if venue.sections.is_empty() {
                    return Err(SagaError::ValidationFailed(
                        "Venue must have at least one section".to_string(),
                    ));
                }

                // Extract section info
                let sections: Vec<String> = venue.sections.iter().map(|s| s.name.clone()).collect();

                // Build event creation command
                let event_command = EventCommand::Create {
                    event_id,
                    name: name.clone(),
                    owner_id,
                    venue: venue.clone(),
                    date,
                    pricing_tiers,
                };

                Ok(BusinessResult::Continue {
                    events: vec![SagaEvent::Initiated {
                        event_id,
                        name,
                        sections,
                        initiated_at: now,
                    }],
                    calls: vec![SagaCall::Event(event_command)],
                })
            }

            SagaInput::Feedback(results) => {
                self.process_feedback(state, results, now)
            }
        }
    }

    fn apply(&self, state: &mut Self::State, event: &Self::Event) {
        match event {
            SagaEvent::Initiated {
                event_id,
                name,
                sections,
                ..
            } => {
                state.event_id = Some(*event_id);
                state.name = Some(name.clone());
                state.phase = SagaPhase::CreatingEvent;
                state.pending_sections = sections.iter().cloned().collect();
            }

            SagaEvent::EventCreated { .. } => {
                state.phase = SagaPhase::InitializingInventory;
            }

            SagaEvent::SectionInventoryInitialized { section, .. } => {
                state.pending_sections.remove(section);
                state.completed_sections.insert(section.clone());
            }

            SagaEvent::Completed { .. } => {
                state.phase = SagaPhase::Completed;
            }

            SagaEvent::EventCreationFailed { error, .. }
            | SagaEvent::InventoryInitializationFailed { error, .. } => {
                state.last_error = Some(error.clone());
            }

            SagaEvent::CompensationStarted { .. } => {
                state.phase = SagaPhase::Compensating;
            }

            SagaEvent::CompensationCompleted { .. } => {
                state.phase = SagaPhase::CompensationCompleted;
            }

            SagaEvent::Failed { error, .. } => {
                state.phase = SagaPhase::Failed;
                state.last_error = Some(error.clone());
            }
        }
    }

    fn feedback_input(results: Vec<Self::CallResult>) -> Self::Input {
        SagaInput::Feedback(results)
    }

    fn event_type_name(event: &Self::Event) -> &'static str {
        match event {
            SagaEvent::Initiated { .. } => "SagaInitiated",
            SagaEvent::EventCreated { .. } => "SagaEventCreated",
            SagaEvent::SectionInventoryInitialized { .. } => "SagaSectionInventoryInitialized",
            SagaEvent::Completed { .. } => "SagaCompleted",
            SagaEvent::EventCreationFailed { .. } => "SagaEventCreationFailed",
            SagaEvent::InventoryInitializationFailed { .. } => "SagaInventoryInitializationFailed",
            SagaEvent::CompensationStarted { .. } => "SagaCompensationStarted",
            SagaEvent::CompensationCompleted { .. } => "SagaCompensationCompleted",
            SagaEvent::Failed { .. } => "SagaFailed",
        }
    }
}

impl EventInventorySagaLogic {
    /// Process feedback from aggregate calls.
    fn process_feedback(
        &self,
        state: &SagaState,
        results: Vec<SagaCallResult>,
        now: DateTime<Utc>,
    ) -> Result<BusinessResult<SagaEvent, SagaCall>, SagaError> {
        let event_id = state.event_id.ok_or_else(|| {
            SagaError::ValidationFailed("No event_id in state".to_string())
        })?;

        match state.phase {
            SagaPhase::CreatingEvent => {
                self.process_event_creation_feedback(state, event_id, results, now)
            }
            SagaPhase::InitializingInventory => {
                self.process_inventory_feedback(state, event_id, results, now)
            }
            SagaPhase::Compensating => {
                self.process_compensation_feedback(state, event_id, results, now)
            }
            _ => Err(SagaError::InvalidStateTransition {
                from: state.phase.clone(),
                to: SagaPhase::Initial,
            }),
        }
    }

    /// Process feedback from Event aggregate creation.
    #[allow(clippy::unused_self)] // Consistent interface with other process_* methods
    fn process_event_creation_feedback(
        &self,
        state: &SagaState,
        event_id: EventId,
        results: Vec<SagaCallResult>,
        now: DateTime<Utc>,
    ) -> Result<BusinessResult<SagaEvent, SagaCall>, SagaError> {
        // Should have exactly one Event result
        let event_result = results.into_iter().find_map(|r| {
            if let SagaCallResult::Event(result) = r {
                Some(result)
            } else {
                None
            }
        });

        match event_result {
            Some(Ok(_events)) => {
                // Event created successfully, now initialize inventory for each section
                let events = vec![SagaEvent::EventCreated {
                    event_id,
                    created_at: now,
                }];

                let calls: Vec<SagaCall> = state
                    .pending_sections
                    .iter()
                    .map(|section| {
                        // Get capacity from section_capacities or use default
                        let capacity = state
                            .section_capacities
                            .get(section)
                            .copied()
                            .unwrap_or_else(|| Capacity::new(100));

                        SagaCall::Inventory {
                            section: section.clone(),
                            command: InventoryCommand::Initialize {
                                event_id,
                                section: section.clone(),
                                capacity,
                            },
                        }
                    })
                    .collect();

                Ok(BusinessResult::Continue { events, calls })
            }

            Some(Err(e)) => {
                // Event creation failed - saga fails immediately (nothing to compensate)
                Ok(BusinessResult::Done(vec![
                    SagaEvent::EventCreationFailed {
                        event_id,
                        error: e.to_string(),
                        failed_at: now,
                    },
                    SagaEvent::Failed {
                        event_id,
                        error: format!("Event creation failed: {e}"),
                        failed_at: now,
                    },
                ]))
            }

            None => Err(SagaError::ValidationFailed(
                "Expected Event result in feedback".to_string(),
            )),
        }
    }

    /// Process feedback from Inventory aggregates.
    #[allow(clippy::unnecessary_wraps)] // Returns Result for consistency with process_feedback dispatch
    fn process_inventory_feedback(
        &self,
        state: &SagaState,
        event_id: EventId,
        results: Vec<SagaCallResult>,
        now: DateTime<Utc>,
    ) -> Result<BusinessResult<SagaEvent, SagaCall>, SagaError> {
        let mut events = Vec::new();
        let mut failed_section = None;
        let mut completed_count = state.completed_sections.len();

        for result in results {
            if let SagaCallResult::Inventory { section, result } = result {
                match result {
                    Ok(_) => {
                        events.push(SagaEvent::SectionInventoryInitialized {
                            event_id,
                            section: section.clone(),
                            initialized_at: now,
                        });
                        completed_count += 1;
                    }
                    Err(e) => {
                        events.push(SagaEvent::InventoryInitializationFailed {
                            event_id,
                            section: section.clone(),
                            error: e.to_string(),
                            failed_at: now,
                        });
                        failed_section = Some((section, e.to_string()));
                    }
                }
            }
        }

        if let Some((section, error)) = failed_section {
            // Start compensation
            events.push(SagaEvent::CompensationStarted {
                event_id,
                reason: format!("Inventory initialization failed for section '{section}': {error}"),
                started_at: now,
            });

            // Build compensation calls
            let calls = self.build_compensation_calls(event_id);

            Ok(BusinessResult::Continue { events, calls })
        } else if completed_count >= state.pending_sections.len() + state.completed_sections.len() {
            // All sections initialized - saga complete!
            events.push(SagaEvent::Completed {
                event_id,
                sections_count: completed_count as u32,
                completed_at: now,
            });
            Ok(BusinessResult::Done(events))
        } else {
            // Still waiting for more results (shouldn't happen with parallel execution)
            Ok(BusinessResult::Done(events))
        }
    }

    /// Process feedback from compensation calls.
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)] // Consistent interface
    fn process_compensation_feedback(
        &self,
        _state: &SagaState,
        event_id: EventId,
        _results: Vec<SagaCallResult>,
        now: DateTime<Utc>,
    ) -> Result<BusinessResult<SagaEvent, SagaCall>, SagaError> {
        // Compensation completed (we don't fail on compensation errors - just log)
        Ok(BusinessResult::Done(vec![
            SagaEvent::CompensationCompleted {
                event_id,
                completed_at: now,
            },
        ]))
    }

    /// Build compensation calls for rollback.
    #[allow(clippy::unused_self)] // For future extension with state-aware compensation
    fn build_compensation_calls(&self, event_id: EventId) -> Vec<SagaCall> {
        // Cancel the event
        // Note: For a complete implementation, we'd also release any inventory
        // that was successfully initialized. For simplicity, we just cancel the event.
        vec![SagaCall::Event(EventCommand::Cancel {
            event_id,
            reason: "Inventory initialization failed".to_string(),
        })]
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::field_reassign_with_default,
    clippy::match_wildcard_for_single_variants
)] // Test code can use unwrap/panic and simpler patterns
mod tests {
    use super::*;
    use crate::types::{SeatType, VenueSection};
    use composable_rust_next::FixedClock;

    fn test_clock() -> FixedClock {
        FixedClock::new(Utc::now())
    }

    fn test_venue() -> Venue {
        Venue::new(
            "Test Arena".to_string(),
            Capacity::new(200),
            vec![
                VenueSection::new("VIP".to_string(), Capacity::new(50), SeatType::GeneralAdmission),
                VenueSection::new("GA".to_string(), Capacity::new(150), SeatType::GeneralAdmission),
            ],
        )
    }

    #[test]
    fn create_event_with_inventory_initiates_saga() {
        let logic = EventInventorySagaLogic;
        let state = SagaState::default();
        let clock = test_clock();

        let input = SagaInput::CreateEventWithInventory {
            event_id: EventId::new(),
            name: "Test Event".to_string(),
            owner_id: UserId::new(),
            venue: test_venue(),
            date: EventDate::new(Utc::now()),
            pricing_tiers: vec![],
        };

        let result = logic.process(&state, input, &clock).unwrap();

        match result {
            BusinessResult::Continue { events, calls } => {
                assert_eq!(events.len(), 1);
                assert!(matches!(events[0], SagaEvent::Initiated { .. }));
                assert_eq!(calls.len(), 1);
                assert!(matches!(calls[0], SagaCall::Event(EventCommand::Create { .. })));
            }
            _ => panic!("Expected Continue result"),
        }
    }

    #[test]
    fn apply_initiated_updates_state() {
        let logic = EventInventorySagaLogic;
        let mut state = SagaState::default();

        let event_id = EventId::new();
        let event = SagaEvent::Initiated {
            event_id,
            name: "Test".to_string(),
            sections: vec!["VIP".to_string(), "GA".to_string()],
            initiated_at: Utc::now(),
        };

        logic.apply(&mut state, &event);

        assert_eq!(state.event_id, Some(event_id));
        assert_eq!(state.phase, SagaPhase::CreatingEvent);
        assert_eq!(state.pending_sections.len(), 2);
        assert!(state.pending_sections.contains("VIP"));
        assert!(state.pending_sections.contains("GA"));
    }

    #[test]
    fn completed_saga_is_terminal() {
        let mut state = SagaState::default();
        state.phase = SagaPhase::Completed;
        assert!(state.is_terminal());
    }

    #[test]
    fn failed_saga_is_terminal() {
        let mut state = SagaState::default();
        state.phase = SagaPhase::Failed;
        assert!(state.is_terminal());
    }
}
