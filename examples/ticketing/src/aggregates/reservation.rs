//! Reservation saga for the Event Ticketing System.
//!
//! Orchestrates the multi-step ticket purchase workflow:
//! 1. Initiate reservation (5-minute timeout starts)
//! 2. Reserve seats in Inventory aggregate
//! 3. Request payment from Payment aggregate
//! 4. On success: Confirm seats, issue tickets
//! 5. On failure: Release seats (compensation)
//! 6. On timeout: Release seats (compensation)
//!
//! This demonstrates the **saga pattern** with time-based workflows and automatic compensation.

use crate::types::{
    CustomerId, EventId, Money, Reservation, ReservationExpiry, ReservationId, ReservationState,
    ReservationStatus, SeatId, SeatNumber, TicketId,
};
use chrono::{DateTime, Duration, Utc};
use composable_rust_core::{effect::Effect, environment::Clock, reducer::Reducer, smallvec, SmallVec};
use composable_rust_macros::Action;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::inventory::InventoryAction;
use super::payment::PaymentAction;
use crate::types::PaymentId;

// ============================================================================
// Actions (Commands + Events)
// ============================================================================

/// Actions for the Reservation saga
///
/// This is a **saga coordinator** that orchestrates multiple aggregates.
/// Demonstrates cross-aggregate communication via the event bus.
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum ReservationAction {
    // Commands
    /// Initiate a new reservation
    #[command]
    InitiateReservation {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Event to reserve tickets for
        event_id: EventId,
        /// Customer making reservation
        customer_id: CustomerId,
        /// Section to reserve from
        section: String,
        /// Number of tickets
        quantity: u32,
        /// Optional specific seat numbers
        specific_seats: Option<Vec<SeatNumber>>,
    },

    /// Complete payment for reservation
    #[command]
    CompletePayment {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Payment ID
        payment_id: PaymentId,
    },

    /// Cancel reservation
    #[command]
    CancelReservation {
        /// Reservation ID
        reservation_id: ReservationId,
    },

    /// Expire reservation (timeout reached)
    #[command]
    ExpireReservation {
        /// Reservation ID
        reservation_id: ReservationId,
    },

    // Events
    /// Reservation was initiated
    #[event]
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

    /// Seats were allocated from inventory
    #[event]
    SeatsAllocated {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Allocated seat IDs
        seats: Vec<SeatId>,
        /// Total amount to pay
        total_amount: Money,
    },

    /// Payment was requested
    #[event]
    PaymentRequested {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Payment ID
        payment_id: PaymentId,
        /// Amount
        amount: Money,
    },

    /// Payment succeeded
    #[event]
    PaymentSucceeded {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Payment ID
        payment_id: PaymentId,
    },

    /// Payment failed
    #[event]
    PaymentFailed {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Payment ID
        payment_id: PaymentId,
        /// Failure reason
        reason: String,
    },

    /// Reservation completed (tickets issued)
    #[event]
    ReservationCompleted {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Issued ticket IDs
        tickets_issued: Vec<TicketId>,
        /// When completed
        completed_at: DateTime<Utc>,
    },

    /// Reservation expired (timeout)
    #[event]
    ReservationExpired {
        /// Reservation ID
        reservation_id: ReservationId,
        /// When expired
        expired_at: DateTime<Utc>,
    },

    /// Reservation cancelled
    #[event]
    ReservationCancelled {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Cancellation reason
        reason: String,
        /// When cancelled
        cancelled_at: DateTime<Utc>,
    },

    /// Reservation compensated (rolled back)
    #[event]
    ReservationCompensated {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Compensation reason
        reason: String,
        /// When compensated
        compensated_at: DateTime<Utc>,
    },

    /// Validation failed
    #[event]
    ValidationFailed {
        /// Error message
        error: String,
    },
}

// ============================================================================
// Environment
// ============================================================================

/// Environment dependencies for the Reservation saga
#[derive(Clone)]
pub struct ReservationEnvironment {
    /// Clock for timestamps and timeout calculation
    pub clock: Arc<dyn Clock>,
}

impl ReservationEnvironment {
    /// Creates a new `ReservationEnvironment`
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock }
    }
}

// ============================================================================
// Reducer
// ============================================================================

/// Reducer for the Reservation saga
///
/// This is a **saga coordinator** that manages a multi-step workflow across
/// multiple aggregates (Inventory, Payment) with compensation on failures.
#[derive(Clone, Debug)]
pub struct ReservationReducer;

impl ReservationReducer {
    /// Creates a new `ReservationReducer`
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Validates `InitiateReservation` command
    fn validate_initiate_reservation(
        state: &ReservationState,
        reservation_id: &ReservationId,
        quantity: u32,
    ) -> Result<(), String> {
        // Reservation must not already exist
        if state.exists(reservation_id) {
            return Err(format!(
                "Reservation {reservation_id} already exists"
            ));
        }

        // Quantity must be valid (1-8)
        if quantity == 0 {
            return Err("Quantity must be greater than zero".to_string());
        }

        if quantity > 8 {
            return Err(format!(
                "Cannot reserve more than 8 tickets (requested: {quantity})"
            ));
        }

        Ok(())
    }

    /// Applies an event to state
    #[allow(clippy::too_many_lines)] // Complex saga state management
    fn apply_event(state: &mut ReservationState, action: &ReservationAction) {
        match action {
            ReservationAction::ReservationInitiated {
                reservation_id,
                event_id,
                customer_id,
                section: _,
                quantity: _,
                expires_at,
                initiated_at,
            } => {
                let reservation = Reservation::new(
                    *reservation_id,
                    *event_id,
                    *customer_id,
                    Vec::new(), // Seats not yet allocated
                    Money::from_cents(0), // Amount not yet calculated
                    ReservationExpiry::new(*expires_at),
                    *initiated_at,
                );
                state.reservations.insert(*reservation_id, reservation);
                state.last_error = None;
            }

            ReservationAction::SeatsAllocated {
                reservation_id,
                seats,
                total_amount,
            } => {
                if let Some(reservation) = state.reservations.get_mut(reservation_id) {
                    reservation.seats.clone_from(seats);
                    reservation.total_amount = *total_amount;
                    reservation.status = ReservationStatus::SeatsReserved;
                }
                state.last_error = None;
            }

            ReservationAction::PaymentRequested {
                reservation_id, ..
            } => {
                if let Some(reservation) = state.reservations.get_mut(reservation_id) {
                    reservation.status = ReservationStatus::PaymentPending;
                }
                state.last_error = None;
            }

            ReservationAction::PaymentSucceeded {
                reservation_id, ..
            } => {
                if let Some(reservation) = state.reservations.get_mut(reservation_id) {
                    reservation.status = ReservationStatus::PaymentCompleted;
                }
                state.last_error = None;
            }

            ReservationAction::PaymentFailed {
                reservation_id,
                reason,
                ..
            } => {
                if let Some(reservation) = state.reservations.get_mut(reservation_id) {
                    reservation.status = ReservationStatus::PaymentFailed {
                        reason: reason.clone(),
                    };
                }
                state.last_error = None;
            }

            ReservationAction::ReservationCompleted {
                reservation_id, ..
            } => {
                if let Some(reservation) = state.reservations.get_mut(reservation_id) {
                    reservation.status = ReservationStatus::Completed;
                }
                state.last_error = None;
            }

            ReservationAction::ReservationExpired {
                reservation_id, ..
            } => {
                if let Some(reservation) = state.reservations.get_mut(reservation_id) {
                    reservation.status = ReservationStatus::Expired;
                }
                state.last_error = None;
            }

            ReservationAction::ReservationCancelled {
                reservation_id, ..
            } => {
                if let Some(reservation) = state.reservations.get_mut(reservation_id) {
                    reservation.status = ReservationStatus::Cancelled;
                }
                state.last_error = None;
            }

            ReservationAction::ReservationCompensated {
                reservation_id, ..
            } => {
                if let Some(reservation) = state.reservations.get_mut(reservation_id) {
                    reservation.status = ReservationStatus::Compensated;
                }
                state.last_error = None;
            }

            ReservationAction::ValidationFailed { error } => {
                state.last_error = Some(error.clone());
            }

            // Commands don't modify state
            ReservationAction::InitiateReservation { .. }
            | ReservationAction::CompletePayment { .. }
            | ReservationAction::CancelReservation { .. }
            | ReservationAction::ExpireReservation { .. } => {}
        }
    }
}

impl Default for ReservationReducer {
    fn default() -> Self {
        Self::new()
    }
}

impl Reducer for ReservationReducer {
    type State = ReservationState;
    type Action = ReservationAction;
    type Environment = ReservationEnvironment;

    #[allow(clippy::too_many_lines)] // Complex saga orchestration required
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ========== Step 1: Initiate Reservation ==========
            ReservationAction::InitiateReservation {
                reservation_id,
                event_id,
                customer_id,
                section,
                quantity,
                specific_seats,
            } => {
                // Validate
                if let Err(error) =
                    Self::validate_initiate_reservation(state, &reservation_id, quantity)
                {
                    Self::apply_event(state, &ReservationAction::ValidationFailed { error });
                    return SmallVec::new();
                }

                // Calculate expiration (5 minutes from now)
                let now = env.clock.now();
                let expires_at = now + Duration::minutes(5);

                // Create and apply ReservationInitiated event
                let event = ReservationAction::ReservationInitiated {
                    reservation_id,
                    event_id,
                    customer_id,
                    section: section.clone(),
                    quantity,
                    expires_at,
                    initiated_at: now,
                };
                Self::apply_event(state, &event);

                // Effect 1: Reserve seats in Inventory aggregate (via event bus)
                let reserve_seats_cmd = InventoryAction::ReserveSeats {
                    reservation_id,
                    event_id,
                    section,
                    quantity,
                    specific_seats,
                    expires_at,
                };

                // Effect 2: Schedule expiration timeout (5 minutes)
                let expire_cmd = ReservationAction::ExpireReservation { reservation_id };

                smallvec![
                    // In a real system with event bus, this would be Effect::PublishEvent
                    // For now, we'll return as a note that cross-aggregate communication
                    // would happen here
                    Effect::Future(Box::pin(async move {
                        // Simulated: would publish to event bus
                        let _ = reserve_seats_cmd;
                        None // No immediate feedback
                    })),
                    Effect::Delay {
                        duration: std::time::Duration::from_secs(5 * 60),
                        action: Box::new(expire_cmd),
                    }
                ]
            }

            // ========== Step 2: Seats Allocated (from Inventory) ==========
            ReservationAction::SeatsAllocated {
                reservation_id,
                ref seats,
                total_amount,
            } => {
                // Apply event
                Self::apply_event(state, &action);

                // Calculate price (simplified - in production would look up pricing tiers)
                let price_per_ticket = Money::from_dollars(50);
                #[allow(clippy::cast_possible_truncation)]
                let total = price_per_ticket.multiply(seats.len() as u32);

                // Create payment request event
                let payment_id = PaymentId::new();
                let payment_requested = ReservationAction::PaymentRequested {
                    reservation_id,
                    payment_id,
                    amount: total,
                };
                Self::apply_event(state, &payment_requested);

                // Effect: Request payment from Payment aggregate
                let process_payment = PaymentAction::ProcessPayment {
                    payment_id,
                    reservation_id,
                    amount: total_amount,
                    payment_method: crate::types::PaymentMethod::CreditCard {
                        last_four: "4242".to_string(),
                    },
                };

                smallvec![Effect::Future(Box::pin(async move {
                    // Simulated: would publish to event bus
                    let _ = process_payment;
                    None
                }))]
            }

            // ========== Step 3a: Payment Succeeded ==========
            ReservationAction::PaymentSucceeded {
                reservation_id,
                payment_id: _,
            } => {
                // Apply event
                Self::apply_event(state, &action);

                // Get customer ID from reservation
                let customer_id = state
                    .reservations
                    .get(&reservation_id)
                    .map_or_else(CustomerId::new, |r| r.customer_id);

                // Effect 1: Confirm seats in inventory (mark as sold)
                let confirm_seats = InventoryAction::ConfirmReservation {
                    reservation_id,
                    customer_id,
                };

                // Generate ticket IDs
                let ticket_count = state
                    .reservations
                    .get(&reservation_id)
                    .map_or(0, |r| r.seats.len());

                let tickets: Vec<TicketId> =
                    (0..ticket_count).map(|_| TicketId::new()).collect();

                // Create completion event
                let completion = ReservationAction::ReservationCompleted {
                    reservation_id,
                    tickets_issued: tickets,
                    completed_at: env.clock.now(),
                };
                Self::apply_event(state, &completion);

                smallvec![Effect::Future(Box::pin(async move {
                    // Simulated: would publish to event bus
                    let _ = confirm_seats;
                    None
                }))]
            }

            // ========== Step 3b: Payment Failed (COMPENSATION) ==========
            ReservationAction::PaymentFailed {
                reservation_id,
                ref reason,
                payment_id: _,
            } => {
                // Apply event
                Self::apply_event(state, &action);

                // COMPENSATION: Release seats back to inventory
                let release_seats = InventoryAction::ReleaseReservation { reservation_id };

                let compensation = ReservationAction::ReservationCompensated {
                    reservation_id,
                    reason: reason.clone(),
                    compensated_at: env.clock.now(),
                };
                Self::apply_event(state, &compensation);

                smallvec![Effect::Future(Box::pin(async move {
                    // Simulated: would publish to event bus
                    let _ = release_seats;
                    None
                }))]
            }

            // ========== Step 4: Timeout (COMPENSATION) ==========
            ReservationAction::ExpireReservation { reservation_id } => {
                // Check if reservation still exists and is pending
                if let Some(reservation) = state.reservations.get(&reservation_id) {
                    // Only expire if still in a pending state
                    if matches!(
                        reservation.status,
                        ReservationStatus::SeatsReserved | ReservationStatus::PaymentPending
                    ) {
                        // Apply expiration event
                        let expiration = ReservationAction::ReservationExpired {
                            reservation_id,
                            expired_at: env.clock.now(),
                        };
                        Self::apply_event(state, &expiration);

                        // COMPENSATION: Release seats
                        let release_seats =
                            InventoryAction::ReleaseReservation { reservation_id };

                        return smallvec![Effect::Future(Box::pin(async move {
                            // Simulated: would publish to event bus
                            let _ = release_seats;
                            None
                        }))];
                    }
                }

                // Already completed or cancelled - ignore
                SmallVec::new()
            }

            // ========== Cancel ==========
            ReservationAction::CancelReservation { reservation_id } => {
                if let Some(reservation) = state.reservations.get(&reservation_id) {
                    // Can only cancel if not yet completed
                    if !matches!(reservation.status, ReservationStatus::Completed) {
                        let cancellation = ReservationAction::ReservationCancelled {
                            reservation_id,
                            reason: "Cancelled by customer".to_string(),
                            cancelled_at: env.clock.now(),
                        };
                        Self::apply_event(state, &cancellation);

                        let release_seats =
                            InventoryAction::ReleaseReservation { reservation_id };

                        return smallvec![Effect::Future(Box::pin(async move {
                            let _ = release_seats;
                            None
                        }))];
                    }
                }

                SmallVec::new()
            }

            // ========== Events (from event store or other aggregates) ==========
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
    use composable_rust_core::environment::SystemClock;
    use composable_rust_testing::{assertions, ReducerTest};

    fn create_test_env() -> ReservationEnvironment {
        ReservationEnvironment::new(Arc::new(SystemClock))
    }

    #[test]
    fn test_initiate_reservation() {
        let reservation_id = ReservationId::new();
        let event_id = EventId::new();
        let customer_id = CustomerId::new();

        ReducerTest::new(ReservationReducer::new())
            .with_env(create_test_env())
            .given_state(ReservationState::new())
            .when_action(ReservationAction::InitiateReservation {
                reservation_id,
                event_id,
                customer_id,
                section: "General".to_string(),
                quantity: 2,
                specific_seats: None,
            })
            .then_state(move |state| {
                assert_eq!(state.count(), 1);
                assert!(state.exists(&reservation_id));
                let reservation = state.get(&reservation_id).unwrap();
                assert_eq!(reservation.status, ReservationStatus::Initiated);
                assert_eq!(reservation.seats.len(), 0); // Not yet allocated
            })
            .then_effects(|effects| {
                assert_eq!(effects.len(), 2); // Reserve seats + timeout
            })
            .run();
    }

    #[test]
    fn test_seats_allocated() {
        let reservation_id = ReservationId::new();

        ReducerTest::new(ReservationReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = ReservationState::new();
                let reservation = Reservation::new(
                    reservation_id,
                    EventId::new(),
                    CustomerId::new(),
                    Vec::new(),
                    Money::from_cents(0),
                    ReservationExpiry::new(Utc::now() + Duration::minutes(5)),
                    Utc::now(),
                );
                state.reservations.insert(reservation_id, reservation);
                state
            })
            .when_action(ReservationAction::SeatsAllocated {
                reservation_id,
                seats: vec![SeatId::new(), SeatId::new()],
                total_amount: Money::from_dollars(100),
            })
            .then_state(move |state| {
                let reservation = state.get(&reservation_id).unwrap();
                // After seats allocated, saga immediately requests payment
                assert_eq!(reservation.status, ReservationStatus::PaymentPending);
                assert_eq!(reservation.seats.len(), 2);
                assert_eq!(reservation.total_amount, Money::from_dollars(100));
            })
            .then_effects(|effects| {
                assert!(!effects.is_empty(), "Expected payment request effect");
            })
            .run();
    }

    #[test]
    fn test_payment_succeeded_completes_reservation() {
        let reservation_id = ReservationId::new();

        ReducerTest::new(ReservationReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = ReservationState::new();
                let mut reservation = Reservation::new(
                    reservation_id,
                    EventId::new(),
                    CustomerId::new(),
                    vec![SeatId::new()],
                    Money::from_dollars(50),
                    ReservationExpiry::new(Utc::now() + Duration::minutes(5)),
                    Utc::now(),
                );
                reservation.status = ReservationStatus::PaymentPending;
                state.reservations.insert(reservation_id, reservation);
                state
            })
            .when_action(ReservationAction::PaymentSucceeded {
                reservation_id,
                payment_id: PaymentId::new(),
            })
            .then_state(move |state| {
                let reservation = state.get(&reservation_id).unwrap();
                assert_eq!(reservation.status, ReservationStatus::Completed);
            })
            .then_effects(|effects| {
                assert!(!effects.is_empty(), "Expected confirm seats effect");
            })
            .run();
    }

    #[test]
    fn test_payment_failed_compensates() {
        let reservation_id = ReservationId::new();

        ReducerTest::new(ReservationReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = ReservationState::new();
                let mut reservation = Reservation::new(
                    reservation_id,
                    EventId::new(),
                    CustomerId::new(),
                    vec![SeatId::new()],
                    Money::from_dollars(50),
                    ReservationExpiry::new(Utc::now() + Duration::minutes(5)),
                    Utc::now(),
                );
                reservation.status = ReservationStatus::PaymentPending;
                state.reservations.insert(reservation_id, reservation);
                state
            })
            .when_action(ReservationAction::PaymentFailed {
                reservation_id,
                payment_id: PaymentId::new(),
                reason: "Card declined".to_string(),
            })
            .then_state(move |state| {
                let reservation = state.get(&reservation_id).unwrap();
                assert_eq!(reservation.status, ReservationStatus::Compensated);
            })
            .then_effects(|effects| {
                assert!(!effects.is_empty(), "Expected release seats effect");
            })
            .run();
    }

    #[test]
    fn test_timeout_expires_reservation() {
        let reservation_id = ReservationId::new();

        ReducerTest::new(ReservationReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = ReservationState::new();
                let mut reservation = Reservation::new(
                    reservation_id,
                    EventId::new(),
                    CustomerId::new(),
                    vec![SeatId::new()],
                    Money::from_dollars(50),
                    ReservationExpiry::new(Utc::now() + Duration::minutes(5)),
                    Utc::now(),
                );
                reservation.status = ReservationStatus::SeatsReserved;
                state.reservations.insert(reservation_id, reservation);
                state
            })
            .when_action(ReservationAction::ExpireReservation { reservation_id })
            .then_state(move |state| {
                let reservation = state.get(&reservation_id).unwrap();
                assert_eq!(reservation.status, ReservationStatus::Expired);
            })
            .then_effects(|effects| {
                assert!(!effects.is_empty(), "Expected release seats effect");
            })
            .run();
    }

    #[test]
    fn test_completed_reservation_ignores_timeout() {
        let reservation_id = ReservationId::new();

        ReducerTest::new(ReservationReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = ReservationState::new();
                let mut reservation = Reservation::new(
                    reservation_id,
                    EventId::new(),
                    CustomerId::new(),
                    vec![SeatId::new()],
                    Money::from_dollars(50),
                    ReservationExpiry::new(Utc::now() + Duration::minutes(5)),
                    Utc::now(),
                );
                reservation.status = ReservationStatus::Completed; // Already completed
                state.reservations.insert(reservation_id, reservation);
                state
            })
            .when_action(ReservationAction::ExpireReservation { reservation_id })
            .then_state(move |state| {
                let reservation = state.get(&reservation_id).unwrap();
                // Should still be Completed, not Expired
                assert_eq!(reservation.status, ReservationStatus::Completed);
            })
            .then_effects(assertions::assert_no_effects) // No compensation needed
            .run();
    }
}
