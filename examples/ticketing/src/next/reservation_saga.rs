//! Reservation saga using next-generation architecture.
//!
//! This saga orchestrates the multi-step ticket purchase workflow:
//!
//! 1. Initiate reservation (5-minute timeout starts)
//! 2. Reserve seats in Inventory aggregate
//! 3. Request payment from Payment aggregate
//! 4. On success: Confirm seats, issue tickets
//! 5. On failure: Release seats (compensation)
//! 6. On timeout: Release seats (compensation)
//!
//! # Architecture
//!
//! The saga uses `BusinessResult::Continue` to orchestrate multiple aggregates:
//!
//! ```text
//! InitiateReservation command
//!         │
//!         ▼
//! ┌───────────────────┐
//! │  ReservationSaga  │
//! │  process()        │──► Continue { events: [Initiated], calls: [ReserveSeats] }
//! └───────────────────┘
//!         │
//!         ▼ (Handler executes calls)
//! ┌───────────────────┐
//! │  Inventory        │──► SeatsReserved / Error
//! └───────────────────┘
//!         │
//!         ▼ (feedback_input transforms result)
//! ┌───────────────────┐
//! │  ReservationSaga  │
//! │  process()        │──► Continue { events: [SeatsAllocated], calls: [ProcessPayment] }
//! └───────────────────┘
//!         │
//!         ▼ (Handler executes calls)
//! ┌───────────────────┐
//! │  Payment          │──► PaymentSucceeded / PaymentFailed
//! └───────────────────┘
//!         │
//!         ▼
//! ┌───────────────────┐
//! │  ReservationSaga  │──► Done { events: [Completed] }
//! │  process()        │    (or compensation flow on failure)
//! └───────────────────┘
//! ```
//!
//! # Compensation
//!
//! If payment fails or times out:
//! 1. Saga emits `PaymentFailed` event
//! 2. Calls `ReleaseReservation` on Inventory
//! 3. Emits `Compensated` terminal event

use chrono::{DateTime, Duration, Utc};
use composable_rust_next::{BusinessLogic, BusinessResult, Clock, StreamId};
use serde::{Deserialize, Serialize};

use crate::types::{CustomerId, EventId, Money, PaymentId, PaymentMethod, ReservationId, SeatId, TicketId};

use super::{
    InventoryCommand, InventoryError, InventoryEvent,
    PaymentCommand, PaymentError, PaymentEvent,
};

// ═══════════════════════════════════════════════════════════════════════════
// Saga Input (Commands + Feedback)
// ═══════════════════════════════════════════════════════════════════════════

/// Input to the saga: either a command or feedback from aggregate calls.
#[derive(Debug, Clone)]
pub enum ReservationSagaInput {
    /// Initiate a new reservation.
    InitiateReservation {
        /// Reservation ID (provided for idempotency)
        reservation_id: ReservationId,
        /// Event to reserve tickets for
        event_id: EventId,
        /// Customer making reservation
        customer_id: CustomerId,
        /// Section to reserve from
        section: String,
        /// Number of tickets
        quantity: u32,
    },

    /// Cancel a reservation.
    CancelReservation {
        /// Reservation ID
        reservation_id: ReservationId,
    },

    /// Expire a reservation (timeout reached).
    ExpireReservation {
        /// Reservation ID
        reservation_id: ReservationId,
    },

    /// Feedback from aggregate calls.
    Feedback(Vec<ReservationSagaCallResult>),
}

// ═══════════════════════════════════════════════════════════════════════════
// Saga Calls (to child aggregates)
// ═══════════════════════════════════════════════════════════════════════════

/// Calls the saga makes to child aggregates.
#[derive(Debug, Clone)]
pub enum ReservationSagaCall {
    /// Call the Inventory aggregate.
    Inventory(InventoryCommand),
    /// Call the Payment aggregate.
    Payment(PaymentCommand),
}

/// Results from aggregate calls.
#[derive(Debug, Clone)]
pub enum ReservationSagaCallResult {
    /// Result from Inventory aggregate.
    Inventory(Result<Vec<InventoryEvent>, InventoryError>),
    /// Result from Payment aggregate.
    Payment(Result<Vec<PaymentEvent>, PaymentError>),
}

// ═══════════════════════════════════════════════════════════════════════════
// Saga Events
// ═══════════════════════════════════════════════════════════════════════════

/// Events emitted by the saga.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReservationSagaEvent {
    /// Reservation was initiated.
    ReservationInitiated {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Event ID
        event_id: EventId,
        /// Customer ID
        customer_id: CustomerId,
        /// Section
        section: String,
        /// Quantity
        quantity: u32,
        /// Expiration time
        expires_at: DateTime<Utc>,
        /// When initiated
        initiated_at: DateTime<Utc>,
    },

    /// Seats were allocated from inventory.
    SeatsAllocated {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Allocated seat IDs
        seats: Vec<SeatId>,
        /// Total amount to pay
        total_amount: Money,
        /// When allocated
        allocated_at: DateTime<Utc>,
    },

    /// Payment was requested.
    PaymentRequested {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Payment ID
        payment_id: PaymentId,
        /// Amount
        amount: Money,
        /// When requested
        requested_at: DateTime<Utc>,
    },

    /// Payment succeeded.
    PaymentSucceeded {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Payment ID
        payment_id: PaymentId,
        /// When succeeded
        succeeded_at: DateTime<Utc>,
    },

    /// Payment failed.
    PaymentFailed {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Payment ID
        payment_id: PaymentId,
        /// Failure reason
        reason: String,
        /// When failed
        failed_at: DateTime<Utc>,
    },

    /// Reservation completed (tickets issued).
    ReservationCompleted {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Issued ticket IDs
        tickets_issued: Vec<TicketId>,
        /// When completed
        completed_at: DateTime<Utc>,
    },

    /// Reservation expired (timeout).
    ReservationExpired {
        /// Reservation ID
        reservation_id: ReservationId,
        /// When expired
        expired_at: DateTime<Utc>,
    },

    /// Reservation cancelled.
    ReservationCancelled {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Cancellation reason
        reason: String,
        /// When cancelled
        cancelled_at: DateTime<Utc>,
    },

    /// Reservation compensated (rolled back).
    ReservationCompensated {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Compensation reason
        reason: String,
        /// When compensated
        compensated_at: DateTime<Utc>,
    },

    /// Inventory reservation failed.
    InventoryReservationFailed {
        /// Reservation ID
        reservation_id: ReservationId,
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
pub enum ReservationSagaPhase {
    /// Not started
    #[default]
    Initial,
    /// Waiting for Inventory to reserve seats
    ReservingSeats,
    /// Seats reserved, waiting for payment
    AwaitingPayment,
    /// Payment in progress
    ProcessingPayment,
    /// All done successfully
    Completed,
    /// Compensation in progress
    Compensating,
    /// Terminal failure (cancelled, expired, or compensated)
    Failed,
}

/// State for the Reservation saga.
#[derive(Debug, Clone, Default)]
pub struct ReservationSagaState {
    /// Reservation ID
    pub reservation_id: Option<ReservationId>,
    /// Event ID
    pub event_id: Option<EventId>,
    /// Customer ID
    pub customer_id: Option<CustomerId>,
    /// Section
    pub section: Option<String>,
    /// Quantity
    pub quantity: u32,
    /// Current phase
    pub phase: ReservationSagaPhase,
    /// Allocated seat IDs
    pub seats: Vec<SeatId>,
    /// Total amount to pay
    pub total_amount: Option<Money>,
    /// Payment ID (if payment requested)
    pub payment_id: Option<PaymentId>,
    /// Issued ticket IDs (if completed)
    pub tickets: Vec<TicketId>,
    /// Expiration time
    pub expires_at: Option<DateTime<Utc>>,
    /// Last error (if any)
    pub last_error: Option<String>,
}

impl ReservationSagaState {
    /// Check if saga is in a terminal state.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.phase,
            ReservationSagaPhase::Completed | ReservationSagaPhase::Failed
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Saga Errors
// ═══════════════════════════════════════════════════════════════════════════

/// Errors from the saga.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ReservationSagaError {
    /// Saga already in progress
    #[error("Reservation already in progress")]
    AlreadyInProgress,

    /// Invalid state transition
    #[error("Invalid state transition from {from:?}")]
    InvalidStateTransition {
        /// Current phase
        from: ReservationSagaPhase,
    },

    /// Validation failed
    #[error("Validation failed: {0}")]
    ValidationFailed(String),

    /// Missing required data
    #[error("Missing required data: {0}")]
    MissingData(String),
}

// ═══════════════════════════════════════════════════════════════════════════
// Saga Business Logic
// ═══════════════════════════════════════════════════════════════════════════

/// Business logic for the Reservation saga.
///
/// This saga coordinates Inventory reservation with Payment processing.
/// It uses `BusinessResult::Continue` to orchestrate child aggregates.
#[derive(Debug, Clone)]
pub struct ReservationSagaLogic;

impl BusinessLogic for ReservationSagaLogic {
    type State = ReservationSagaState;
    type Input = ReservationSagaInput;
    type Event = ReservationSagaEvent;
    type Error = ReservationSagaError;
    type Call = ReservationSagaCall;
    type CallResult = ReservationSagaCallResult;

    fn stream_id(input: &Self::Input) -> StreamId {
        match input {
            ReservationSagaInput::InitiateReservation { reservation_id, .. }
            | ReservationSagaInput::CancelReservation { reservation_id }
            | ReservationSagaInput::ExpireReservation { reservation_id } => {
                StreamId::new(format!("saga-reservation-{}", reservation_id.as_uuid()))
            }
            ReservationSagaInput::Feedback(_) => {
                // Feedback is always for an in-progress saga
                // The Handler maintains the stream_id from the original command
                StreamId::new("saga-reservation-feedback-placeholder")
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
            ReservationSagaInput::InitiateReservation {
                reservation_id,
                event_id,
                customer_id,
                section,
                quantity,
            } => {
                // Validation
                if state.phase != ReservationSagaPhase::Initial {
                    return Err(ReservationSagaError::AlreadyInProgress);
                }

                if quantity == 0 {
                    return Err(ReservationSagaError::ValidationFailed(
                        "Quantity must be greater than zero".to_string(),
                    ));
                }

                if quantity > 8 {
                    return Err(ReservationSagaError::ValidationFailed(
                        format!("Cannot reserve more than 8 tickets (requested: {quantity})")
                    ));
                }

                // Calculate expiration (5 minutes from now)
                let expires_at = now + Duration::minutes(5);

                // Build inventory reservation command
                let reserve_cmd = InventoryCommand::Reserve {
                    reservation_id,
                    event_id,
                    section: section.clone(),
                    quantity,
                    expires_at,
                };

                Ok(BusinessResult::Continue {
                    events: vec![ReservationSagaEvent::ReservationInitiated {
                        reservation_id,
                        event_id,
                        customer_id,
                        section,
                        quantity,
                        expires_at,
                        initiated_at: now,
                    }],
                    calls: vec![ReservationSagaCall::Inventory(reserve_cmd)],
                })
            }

            ReservationSagaInput::CancelReservation { reservation_id } => {
                // Can only cancel if in appropriate state
                match state.phase {
                    ReservationSagaPhase::ReservingSeats | ReservationSagaPhase::AwaitingPayment => {
                        // Validate we have required data
                        let _event_id = state.event_id.ok_or_else(|| {
                            ReservationSagaError::MissingData("event_id".to_string())
                        })?;

                        // Release seats
                        let release_cmd = InventoryCommand::Release { reservation_id };

                        Ok(BusinessResult::Continue {
                            events: vec![ReservationSagaEvent::ReservationCancelled {
                                reservation_id,
                                reason: "Cancelled by customer".to_string(),
                                cancelled_at: now,
                            }],
                            calls: vec![ReservationSagaCall::Inventory(release_cmd)],
                        })
                    }
                    ReservationSagaPhase::Initial => {
                        // Nothing to cancel
                        Ok(BusinessResult::Done(vec![
                            ReservationSagaEvent::ReservationCancelled {
                                reservation_id,
                                reason: "Cancelled before seats reserved".to_string(),
                                cancelled_at: now,
                            },
                        ]))
                    }
                    _ => {
                        // Already completed, compensating, or failed - ignore
                        Ok(BusinessResult::Done(vec![]))
                    }
                }
            }

            ReservationSagaInput::ExpireReservation { reservation_id } => {
                // Can only expire if in appropriate state
                match state.phase {
                    ReservationSagaPhase::ReservingSeats
                    | ReservationSagaPhase::AwaitingPayment
                    | ReservationSagaPhase::ProcessingPayment => {
                        // Release seats
                        let release_cmd = InventoryCommand::Release { reservation_id };

                        Ok(BusinessResult::Continue {
                            events: vec![ReservationSagaEvent::ReservationExpired {
                                reservation_id,
                                expired_at: now,
                            }],
                            calls: vec![ReservationSagaCall::Inventory(release_cmd)],
                        })
                    }
                    _ => {
                        // Already completed, compensating, or failed - ignore
                        Ok(BusinessResult::Done(vec![]))
                    }
                }
            }

            ReservationSagaInput::Feedback(results) => {
                self.process_feedback(state, results, now)
            }
        }
    }

    fn apply(&self, state: &mut Self::State, event: &Self::Event) {
        match event {
            ReservationSagaEvent::ReservationInitiated {
                reservation_id,
                event_id,
                customer_id,
                section,
                quantity,
                expires_at,
                ..
            } => {
                state.reservation_id = Some(*reservation_id);
                state.event_id = Some(*event_id);
                state.customer_id = Some(*customer_id);
                state.section = Some(section.clone());
                state.quantity = *quantity;
                state.expires_at = Some(*expires_at);
                state.phase = ReservationSagaPhase::ReservingSeats;
            }

            ReservationSagaEvent::SeatsAllocated { seats, total_amount, .. } => {
                state.seats.clone_from(seats);
                state.total_amount = Some(*total_amount);
                state.phase = ReservationSagaPhase::AwaitingPayment;
            }

            ReservationSagaEvent::PaymentRequested { payment_id, .. } => {
                state.payment_id = Some(*payment_id);
                state.phase = ReservationSagaPhase::ProcessingPayment;
            }

            ReservationSagaEvent::PaymentSucceeded { .. } => {
                // Stay in ProcessingPayment until Completed
            }

            ReservationSagaEvent::PaymentFailed { reason, .. } => {
                state.last_error = Some(reason.clone());
                state.phase = ReservationSagaPhase::Compensating;
            }

            ReservationSagaEvent::ReservationCompleted { tickets_issued, .. } => {
                state.tickets.clone_from(tickets_issued);
                state.phase = ReservationSagaPhase::Completed;
            }

            ReservationSagaEvent::ReservationExpired { .. }
            | ReservationSagaEvent::ReservationCancelled { .. }
            | ReservationSagaEvent::ReservationCompensated { .. }
            | ReservationSagaEvent::InventoryReservationFailed { .. } => {
                state.phase = ReservationSagaPhase::Failed;
            }
        }
    }

    fn feedback_input(results: Vec<Self::CallResult>) -> Self::Input {
        ReservationSagaInput::Feedback(results)
    }

    fn event_type_name(event: &Self::Event) -> &'static str {
        match event {
            ReservationSagaEvent::ReservationInitiated { .. } => "ReservationInitiated",
            ReservationSagaEvent::SeatsAllocated { .. } => "SeatsAllocated",
            ReservationSagaEvent::PaymentRequested { .. } => "PaymentRequested",
            ReservationSagaEvent::PaymentSucceeded { .. } => "PaymentSucceeded",
            ReservationSagaEvent::PaymentFailed { .. } => "PaymentFailed",
            ReservationSagaEvent::ReservationCompleted { .. } => "ReservationCompleted",
            ReservationSagaEvent::ReservationExpired { .. } => "ReservationExpired",
            ReservationSagaEvent::ReservationCancelled { .. } => "ReservationCancelled",
            ReservationSagaEvent::ReservationCompensated { .. } => "ReservationCompensated",
            ReservationSagaEvent::InventoryReservationFailed { .. } => "InventoryReservationFailed",
        }
    }
}

impl ReservationSagaLogic {
    /// Process feedback from aggregate calls.
    fn process_feedback(
        &self,
        state: &ReservationSagaState,
        results: Vec<ReservationSagaCallResult>,
        now: DateTime<Utc>,
    ) -> Result<BusinessResult<ReservationSagaEvent, ReservationSagaCall>, ReservationSagaError> {
        let reservation_id = state.reservation_id.ok_or_else(|| {
            ReservationSagaError::MissingData("reservation_id".to_string())
        })?;

        match state.phase {
            ReservationSagaPhase::ReservingSeats => {
                self.process_inventory_feedback(state, reservation_id, results, now)
            }
            ReservationSagaPhase::ProcessingPayment => {
                self.process_payment_feedback(state, reservation_id, results, now)
            }
            ReservationSagaPhase::Compensating => {
                self.process_compensation_feedback(state, reservation_id, results, now)
            }
            _ => Err(ReservationSagaError::InvalidStateTransition {
                from: state.phase.clone(),
            }),
        }
    }

    /// Process feedback from Inventory aggregate.
    #[allow(clippy::unused_self)] // Consistent interface with other process_* methods
    fn process_inventory_feedback(
        &self,
        state: &ReservationSagaState,
        reservation_id: ReservationId,
        results: Vec<ReservationSagaCallResult>,
        now: DateTime<Utc>,
    ) -> Result<BusinessResult<ReservationSagaEvent, ReservationSagaCall>, ReservationSagaError> {
        // Find inventory result
        let inventory_result = results.into_iter().find_map(|r| {
            if let ReservationSagaCallResult::Inventory(result) = r {
                Some(result)
            } else {
                None
            }
        });

        let customer_id = state.customer_id.ok_or_else(|| {
            ReservationSagaError::MissingData("customer_id".to_string())
        })?;

        match inventory_result {
            Some(Ok(events)) => {
                // Find SeatsReserved event to extract seat IDs
                let seats: Vec<SeatId> = events
                    .iter()
                    .filter_map(|e| {
                        if let InventoryEvent::SeatsReserved { seats, .. } = e {
                            Some(seats.clone())
                        } else {
                            None
                        }
                    })
                    .flatten()
                    .collect();

                if seats.is_empty() {
                    return Ok(BusinessResult::Done(vec![
                        ReservationSagaEvent::InventoryReservationFailed {
                            reservation_id,
                            error: "No seats returned from inventory".to_string(),
                            failed_at: now,
                        },
                    ]));
                }

                // Calculate total amount ($50 per seat)
                #[allow(clippy::cast_possible_truncation)]
                let total_amount = Money::from_dollars(50).multiply(seats.len() as u32);

                // Create payment request
                let payment_id = PaymentId::new();
                let payment_cmd = PaymentCommand::ProcessPayment {
                    payment_id,
                    reservation_id,
                    customer_id,
                    amount: total_amount,
                    payment_method: PaymentMethod::CreditCard {
                        last_four: "4242".to_string(),
                    },
                };

                Ok(BusinessResult::Continue {
                    events: vec![
                        ReservationSagaEvent::SeatsAllocated {
                            reservation_id,
                            seats,
                            total_amount,
                            allocated_at: now,
                        },
                        ReservationSagaEvent::PaymentRequested {
                            reservation_id,
                            payment_id,
                            amount: total_amount,
                            requested_at: now,
                        },
                    ],
                    calls: vec![ReservationSagaCall::Payment(payment_cmd)],
                })
            }

            Some(Err(e)) => {
                // Inventory reservation failed - saga fails immediately (nothing to compensate)
                Ok(BusinessResult::Done(vec![
                    ReservationSagaEvent::InventoryReservationFailed {
                        reservation_id,
                        error: e.to_string(),
                        failed_at: now,
                    },
                ]))
            }

            None => Err(ReservationSagaError::ValidationFailed(
                "Expected Inventory result in feedback".to_string(),
            )),
        }
    }

    /// Process feedback from Payment aggregate.
    #[allow(clippy::unused_self)] // Consistent interface with other process_* methods
    fn process_payment_feedback(
        &self,
        state: &ReservationSagaState,
        reservation_id: ReservationId,
        results: Vec<ReservationSagaCallResult>,
        now: DateTime<Utc>,
    ) -> Result<BusinessResult<ReservationSagaEvent, ReservationSagaCall>, ReservationSagaError> {
        let payment_id = state.payment_id.ok_or_else(|| {
            ReservationSagaError::MissingData("payment_id".to_string())
        })?;

        let customer_id = state.customer_id.ok_or_else(|| {
            ReservationSagaError::MissingData("customer_id".to_string())
        })?;

        // Find payment result
        let payment_result = results.into_iter().find_map(|r| {
            if let ReservationSagaCallResult::Payment(result) = r {
                Some(result)
            } else {
                None
            }
        });

        match payment_result {
            Some(Ok(events)) => {
                // Check if payment succeeded
                let succeeded = events.iter().any(|e| {
                    matches!(e, PaymentEvent::PaymentSucceeded { .. })
                });

                let failed = events.iter().find_map(|e| {
                    if let PaymentEvent::PaymentFailed { reason, .. } = e {
                        Some(reason.clone())
                    } else {
                        None
                    }
                });

                if let Some(reason) = failed {
                    // Payment failed - compensate by releasing seats
                    let release_cmd = InventoryCommand::Release { reservation_id };

                    return Ok(BusinessResult::Continue {
                        events: vec![ReservationSagaEvent::PaymentFailed {
                            reservation_id,
                            payment_id,
                            reason,
                            failed_at: now,
                        }],
                        calls: vec![ReservationSagaCall::Inventory(release_cmd)],
                    });
                }

                if succeeded {
                    // Payment succeeded - confirm seats and issue tickets
                    let tickets: Vec<TicketId> = state
                        .seats
                        .iter()
                        .map(|_| TicketId::new())
                        .collect();

                    let confirm_cmd = InventoryCommand::Confirm {
                        reservation_id,
                        customer_id,
                    };

                    return Ok(BusinessResult::Continue {
                        events: vec![
                            ReservationSagaEvent::PaymentSucceeded {
                                reservation_id,
                                payment_id,
                                succeeded_at: now,
                            },
                            ReservationSagaEvent::ReservationCompleted {
                                reservation_id,
                                tickets_issued: tickets,
                                completed_at: now,
                            },
                        ],
                        calls: vec![ReservationSagaCall::Inventory(confirm_cmd)],
                    });
                }

                // No succeeded or failed event found - unexpected
                Err(ReservationSagaError::ValidationFailed(
                    "Payment returned no success or failure event".to_string(),
                ))
            }

            Some(Err(e)) => {
                // Payment call failed - compensate by releasing seats
                let release_cmd = InventoryCommand::Release { reservation_id };

                Ok(BusinessResult::Continue {
                    events: vec![ReservationSagaEvent::PaymentFailed {
                        reservation_id,
                        payment_id,
                        reason: e.to_string(),
                        failed_at: now,
                    }],
                    calls: vec![ReservationSagaCall::Inventory(release_cmd)],
                })
            }

            None => Err(ReservationSagaError::ValidationFailed(
                "Expected Payment result in feedback".to_string(),
            )),
        }
    }

    /// Process feedback from compensation calls.
    #[allow(clippy::unnecessary_wraps, clippy::unused_self)] // Consistent interface
    fn process_compensation_feedback(
        &self,
        state: &ReservationSagaState,
        reservation_id: ReservationId,
        _results: Vec<ReservationSagaCallResult>,
        now: DateTime<Utc>,
    ) -> Result<BusinessResult<ReservationSagaEvent, ReservationSagaCall>, ReservationSagaError> {
        // Compensation completed (we don't fail on compensation errors - just log)
        let reason = state
            .last_error
            .clone()
            .unwrap_or_else(|| "Unknown error".to_string());

        Ok(BusinessResult::Done(vec![
            ReservationSagaEvent::ReservationCompensated {
                reservation_id,
                reason,
                compensated_at: now,
            },
        ]))
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
    use composable_rust_next::FixedClock;

    fn test_clock() -> FixedClock {
        FixedClock::new(Utc::now())
    }

    #[test]
    fn initiate_reservation_creates_inventory_call() {
        let logic = ReservationSagaLogic;
        let state = ReservationSagaState::default();
        let clock = test_clock();

        let input = ReservationSagaInput::InitiateReservation {
            reservation_id: ReservationId::new(),
            event_id: EventId::new(),
            customer_id: CustomerId::new(),
            section: "VIP".to_string(),
            quantity: 2,
        };

        let result = logic.process(&state, input, &clock).unwrap();

        match result {
            BusinessResult::Continue { events, calls } => {
                assert_eq!(events.len(), 1);
                assert!(matches!(
                    events[0],
                    ReservationSagaEvent::ReservationInitiated { .. }
                ));
                assert_eq!(calls.len(), 1);
                assert!(matches!(
                    calls[0],
                    ReservationSagaCall::Inventory(InventoryCommand::Reserve { .. })
                ));
            }
            _ => panic!("Expected Continue result"),
        }
    }

    #[test]
    fn initiate_reservation_zero_quantity_fails() {
        let logic = ReservationSagaLogic;
        let state = ReservationSagaState::default();
        let clock = test_clock();

        let input = ReservationSagaInput::InitiateReservation {
            reservation_id: ReservationId::new(),
            event_id: EventId::new(),
            customer_id: CustomerId::new(),
            section: "VIP".to_string(),
            quantity: 0,
        };

        let result = logic.process(&state, input, &clock);
        assert!(matches!(
            result,
            Err(ReservationSagaError::ValidationFailed(_))
        ));
    }

    #[test]
    fn initiate_reservation_too_many_tickets_fails() {
        let logic = ReservationSagaLogic;
        let state = ReservationSagaState::default();
        let clock = test_clock();

        let input = ReservationSagaInput::InitiateReservation {
            reservation_id: ReservationId::new(),
            event_id: EventId::new(),
            customer_id: CustomerId::new(),
            section: "VIP".to_string(),
            quantity: 10, // More than 8
        };

        let result = logic.process(&state, input, &clock);
        assert!(matches!(
            result,
            Err(ReservationSagaError::ValidationFailed(_))
        ));
    }

    #[test]
    fn apply_initiated_updates_state() {
        let logic = ReservationSagaLogic;
        let mut state = ReservationSagaState::default();

        let reservation_id = ReservationId::new();
        let event_id = EventId::new();
        let customer_id = CustomerId::new();

        let event = ReservationSagaEvent::ReservationInitiated {
            reservation_id,
            event_id,
            customer_id,
            section: "VIP".to_string(),
            quantity: 2,
            expires_at: Utc::now() + Duration::minutes(5),
            initiated_at: Utc::now(),
        };

        logic.apply(&mut state, &event);

        assert_eq!(state.reservation_id, Some(reservation_id));
        assert_eq!(state.event_id, Some(event_id));
        assert_eq!(state.customer_id, Some(customer_id));
        assert_eq!(state.quantity, 2);
        assert_eq!(state.phase, ReservationSagaPhase::ReservingSeats);
    }

    #[test]
    fn completed_saga_is_terminal() {
        let mut state = ReservationSagaState::default();
        state.phase = ReservationSagaPhase::Completed;
        assert!(state.is_terminal());
    }

    #[test]
    fn failed_saga_is_terminal() {
        let mut state = ReservationSagaState::default();
        state.phase = ReservationSagaPhase::Failed;
        assert!(state.is_terminal());
    }

    #[test]
    fn inventory_success_requests_payment() {
        let logic = ReservationSagaLogic;
        let clock = test_clock();

        let reservation_id = ReservationId::new();
        let event_id = EventId::new();
        let customer_id = CustomerId::new();

        let mut state = ReservationSagaState::default();
        state.reservation_id = Some(reservation_id);
        state.event_id = Some(event_id);
        state.customer_id = Some(customer_id);
        state.phase = ReservationSagaPhase::ReservingSeats;

        // Simulate inventory success feedback
        let seats = vec![SeatId::new(), SeatId::new()];
        let feedback = ReservationSagaInput::Feedback(vec![
            ReservationSagaCallResult::Inventory(Ok(vec![
                InventoryEvent::SeatsReserved {
                    reservation_id,
                    event_id,
                    section: "VIP".to_string(),
                    seats: seats.clone(),
                    expires_at: Utc::now() + Duration::minutes(5),
                    reserved_at: Utc::now(),
                },
            ])),
        ]);

        let result = logic.process(&state, feedback, &clock).unwrap();

        match result {
            BusinessResult::Continue { events, calls } => {
                assert_eq!(events.len(), 2);
                assert!(matches!(
                    events[0],
                    ReservationSagaEvent::SeatsAllocated { .. }
                ));
                assert!(matches!(
                    events[1],
                    ReservationSagaEvent::PaymentRequested { .. }
                ));
                assert_eq!(calls.len(), 1);
                assert!(matches!(
                    calls[0],
                    ReservationSagaCall::Payment(PaymentCommand::ProcessPayment { .. })
                ));
            }
            _ => panic!("Expected Continue result"),
        }
    }

    #[test]
    fn payment_success_completes_saga() {
        let logic = ReservationSagaLogic;
        let clock = test_clock();

        let reservation_id = ReservationId::new();
        let payment_id = PaymentId::new();
        let customer_id = CustomerId::new();

        let mut state = ReservationSagaState::default();
        state.reservation_id = Some(reservation_id);
        state.payment_id = Some(payment_id);
        state.customer_id = Some(customer_id);
        state.seats = vec![SeatId::new(), SeatId::new()];
        state.phase = ReservationSagaPhase::ProcessingPayment;

        // Simulate payment success feedback
        let feedback = ReservationSagaInput::Feedback(vec![
            ReservationSagaCallResult::Payment(Ok(vec![
                PaymentEvent::PaymentProcessed {
                    payment_id,
                    reservation_id,
                    customer_id,
                    amount: Money::from_dollars(100),
                    payment_method: PaymentMethod::CreditCard {
                        last_four: "4242".to_string(),
                    },
                    processed_at: Utc::now(),
                },
                PaymentEvent::PaymentSucceeded {
                    payment_id,
                    transaction_id: "txn_123".to_string(),
                    succeeded_at: Utc::now(),
                },
            ])),
        ]);

        let result = logic.process(&state, feedback, &clock).unwrap();

        match result {
            BusinessResult::Continue { events, calls } => {
                assert_eq!(events.len(), 2);
                assert!(matches!(
                    events[0],
                    ReservationSagaEvent::PaymentSucceeded { .. }
                ));
                assert!(matches!(
                    events[1],
                    ReservationSagaEvent::ReservationCompleted { .. }
                ));
                // Should confirm seats in inventory
                assert_eq!(calls.len(), 1);
                assert!(matches!(
                    calls[0],
                    ReservationSagaCall::Inventory(InventoryCommand::Confirm { .. })
                ));
            }
            _ => panic!("Expected Continue result"),
        }
    }

    #[test]
    fn payment_failure_triggers_compensation() {
        let logic = ReservationSagaLogic;
        let clock = test_clock();

        let reservation_id = ReservationId::new();
        let payment_id = PaymentId::new();
        let customer_id = CustomerId::new();

        let mut state = ReservationSagaState::default();
        state.reservation_id = Some(reservation_id);
        state.payment_id = Some(payment_id);
        state.customer_id = Some(customer_id);
        state.seats = vec![SeatId::new()];
        state.phase = ReservationSagaPhase::ProcessingPayment;

        // Simulate payment failure feedback
        let feedback = ReservationSagaInput::Feedback(vec![
            ReservationSagaCallResult::Payment(Ok(vec![PaymentEvent::PaymentFailed {
                payment_id,
                reason: "Card declined".to_string(),
                failed_at: Utc::now(),
            }])),
        ]);

        let result = logic.process(&state, feedback, &clock).unwrap();

        match result {
            BusinessResult::Continue { events, calls } => {
                assert_eq!(events.len(), 1);
                match &events[0] {
                    ReservationSagaEvent::PaymentFailed { reason, .. } => {
                        assert_eq!(reason, "Card declined");
                    }
                    _ => panic!("Expected PaymentFailed event"),
                }
                // Should release seats in inventory (compensation)
                assert_eq!(calls.len(), 1);
                assert!(matches!(
                    calls[0],
                    ReservationSagaCall::Inventory(InventoryCommand::Release { .. })
                ));
            }
            _ => panic!("Expected Continue result"),
        }
    }
}
