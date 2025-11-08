//! Payment aggregate for the Event Ticketing System.
//!
//! Handles payment processing and refunds. In production, this would integrate with
//! real payment gateways (Stripe, `PayPal`, etc.). For this demo, we simulate success
//! and provide a command to simulate failures for testing compensation flows.

use crate::types::{CustomerId, Money, Payment, PaymentId, PaymentMethod, PaymentState, PaymentStatus, ReservationId};
use chrono::{DateTime, Utc};
use composable_rust_core::{effect::Effect, environment::Clock, reducer::Reducer, SmallVec};
use composable_rust_macros::Action;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// Note: ReservationAction would be imported in a real system for cross-aggregate events
// For now, we'll keep it simple and focus on the Payment aggregate itself

// ============================================================================
// Actions (Commands + Events)
// ============================================================================

/// Actions for the Payment aggregate
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum PaymentAction {
    // Commands
    /// Process a payment
    #[command]
    ProcessPayment {
        /// Payment ID
        payment_id: PaymentId,
        /// Reservation this payment is for
        reservation_id: ReservationId,
        /// Amount to charge
        amount: Money,
        /// Payment method
        payment_method: PaymentMethod,
    },

    /// Refund a payment
    #[command]
    RefundPayment {
        /// Payment to refund
        payment_id: PaymentId,
        /// Refund amount
        amount: Money,
        /// Refund reason
        reason: String,
    },

    /// Simulate payment failure (for testing)
    #[command]
    SimulatePaymentFailure {
        /// Payment ID
        payment_id: PaymentId,
        /// Reservation ID
        reservation_id: ReservationId,
        /// Failure reason
        reason: String,
    },

    // Events
    /// Payment was processed
    #[event]
    PaymentProcessed {
        /// Payment ID
        payment_id: PaymentId,
        /// Reservation ID
        reservation_id: ReservationId,
        /// Amount
        amount: Money,
        /// Payment method
        payment_method: PaymentMethod,
        /// When processed
        processed_at: DateTime<Utc>,
    },

    /// Payment succeeded
    #[event]
    PaymentSucceeded {
        /// Payment ID
        payment_id: PaymentId,
        /// Transaction ID from gateway
        transaction_id: String,
    },

    /// Payment failed
    #[event]
    PaymentFailed {
        /// Payment ID
        payment_id: PaymentId,
        /// Failure reason
        reason: String,
        /// When failed
        failed_at: DateTime<Utc>,
    },

    /// Payment was refunded
    #[event]
    PaymentRefunded {
        /// Payment ID
        payment_id: PaymentId,
        /// Refund amount
        amount: Money,
        /// Refund reason
        reason: String,
        /// When refunded
        refunded_at: DateTime<Utc>,
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

/// Environment dependencies for the Payment aggregate
#[derive(Clone)]
pub struct PaymentEnvironment {
    /// Clock for timestamps
    pub clock: Arc<dyn Clock>,
}

impl PaymentEnvironment {
    /// Creates a new `PaymentEnvironment`
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock }
    }
}

// ============================================================================
// Reducer
// ============================================================================

/// Reducer for the Payment aggregate
///
/// Simulates payment processing. In production, would integrate with Stripe/PayPal/etc.
#[derive(Clone, Debug)]
pub struct PaymentReducer;

impl PaymentReducer {
    /// Creates a new `PaymentReducer`
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Applies an event to state
    fn apply_event(state: &mut PaymentState, action: &PaymentAction) {
        match action {
            PaymentAction::PaymentProcessed {
                payment_id,
                reservation_id,
                amount,
                payment_method,
                processed_at,
            } => {
                let payment = Payment::new(
                    *payment_id,
                    *reservation_id,
                    CustomerId::new(), // Simplified
                    *amount,
                    payment_method.clone(),
                );
                let mut payment = payment;
                payment.processed_at = Some(*processed_at);
                state.payments.insert(*payment_id, payment);
                state.last_error = None;
            }

            PaymentAction::PaymentSucceeded {
                payment_id,
                transaction_id: _,
            } => {
                if let Some(payment) = state.payments.get_mut(payment_id) {
                    payment.status = PaymentStatus::Captured;
                }
                state.last_error = None;
            }

            PaymentAction::PaymentFailed {
                payment_id,
                reason,
                ..
            } => {
                if let Some(payment) = state.payments.get_mut(payment_id) {
                    payment.status = PaymentStatus::Failed {
                        reason: reason.clone(),
                    };
                }
                state.last_error = Some(reason.clone());
            }

            PaymentAction::PaymentRefunded {
                payment_id,
                amount,
                ..
            } => {
                if let Some(payment) = state.payments.get_mut(payment_id) {
                    payment.status = PaymentStatus::Refunded { amount: *amount };
                }
                state.last_error = None;
            }

            PaymentAction::ValidationFailed { error } => {
                state.last_error = Some(error.clone());
            }

            // Commands don't modify state
            PaymentAction::ProcessPayment { .. }
            | PaymentAction::RefundPayment { .. }
            | PaymentAction::SimulatePaymentFailure { .. } => {}
        }
    }
}

impl Default for PaymentReducer {
    fn default() -> Self {
        Self::new()
    }
}

impl Reducer for PaymentReducer {
    type State = PaymentState;
    type Action = PaymentAction;
    type Environment = PaymentEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ========== Process Payment (Happy Path) ==========
            PaymentAction::ProcessPayment {
                payment_id,
                reservation_id,
                amount,
                payment_method,
            } => {
                // Record payment attempt
                let processed = PaymentAction::PaymentProcessed {
                    payment_id,
                    reservation_id,
                    amount,
                    payment_method,
                    processed_at: env.clock.now(),
                };
                Self::apply_event(state, &processed);

                // Simulate payment processing
                // In production: This would call Stripe/PayPal/etc.
                // For demo: Always succeed to show happy path

                let success = PaymentAction::PaymentSucceeded {
                    payment_id,
                    transaction_id: format!("txn_{}", Uuid::new_v4()),
                };
                Self::apply_event(state, &success);

                // In a real system with event bus, would publish ReservationAction::PaymentSucceeded
                // For now, we return no effects (the saga would be notified via event bus)

                SmallVec::new()
            }

            // ========== Simulate Failure (For Testing) ==========
            PaymentAction::SimulatePaymentFailure {
                payment_id,
                reason,
                reservation_id: _,
            } => {
                // Emit PaymentFailed event
                let failure = PaymentAction::PaymentFailed {
                    payment_id,
                    reason,
                    failed_at: env.clock.now(),
                };
                Self::apply_event(state, &failure);

                // In a real system, would publish ReservationAction::PaymentFailed
                SmallVec::new()
            }

            // ========== Refund Payment ==========
            PaymentAction::RefundPayment {
                payment_id,
                amount,
                reason,
            } => {
                // Validate payment exists and is captured
                if let Some(payment) = state.payments.get(&payment_id) {
                    if !matches!(payment.status, PaymentStatus::Captured) {
                        Self::apply_event(
                            state,
                            &PaymentAction::ValidationFailed {
                                error: "Cannot refund uncaptured payment".to_string(),
                            },
                        );
                        return SmallVec::new();
                    }
                } else {
                    Self::apply_event(
                        state,
                        &PaymentAction::ValidationFailed {
                            error: format!("Payment {payment_id} not found"),
                        },
                    );
                    return SmallVec::new();
                }

                // Process refund
                let refund = PaymentAction::PaymentRefunded {
                    payment_id,
                    amount,
                    reason,
                    refunded_at: env.clock.now(),
                };
                Self::apply_event(state, &refund);

                SmallVec::new()
            }

            // ========== Events (from event store) ==========
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

    fn create_test_env() -> PaymentEnvironment {
        PaymentEnvironment::new(Arc::new(SystemClock))
    }

    #[test]
    fn test_process_payment_success() {
        let payment_id = PaymentId::new();
        let reservation_id = ReservationId::new();

        ReducerTest::new(PaymentReducer::new())
            .with_env(create_test_env())
            .given_state(PaymentState::new())
            .when_action(PaymentAction::ProcessPayment {
                payment_id,
                reservation_id,
                amount: Money::from_dollars(100),
                payment_method: PaymentMethod::CreditCard {
                    last_four: "4242".to_string(),
                },
            })
            .then_state(move |state| {
                assert_eq!(state.count(), 1);
                let payment = state.get(&payment_id).unwrap();
                assert_eq!(payment.status, PaymentStatus::Captured);
                assert_eq!(payment.amount, Money::from_dollars(100));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_simulate_payment_failure() {
        let payment_id = PaymentId::new();
        let reservation_id = ReservationId::new();

        ReducerTest::new(PaymentReducer::new())
            .with_env(create_test_env())
            .given_state(PaymentState::new())
            .when_action(PaymentAction::SimulatePaymentFailure {
                payment_id,
                reservation_id,
                reason: "Card declined".to_string(),
            })
            .then_state(move |state| {
                assert!(state.last_error.is_some());
                assert!(state.last_error.as_ref().unwrap().contains("declined"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_refund_payment() {
        let payment_id = PaymentId::new();

        ReducerTest::new(PaymentReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = PaymentState::new();
                let mut payment = Payment::new(
                    payment_id,
                    ReservationId::new(),
                    CustomerId::new(),
                    Money::from_dollars(100),
                    PaymentMethod::CreditCard {
                        last_four: "4242".to_string(),
                    },
                );
                payment.status = PaymentStatus::Captured;
                state.payments.insert(payment_id, payment);
                state
            })
            .when_action(PaymentAction::RefundPayment {
                payment_id,
                amount: Money::from_dollars(100),
                reason: "Event cancelled".to_string(),
            })
            .then_state(move |state| {
                let payment = state.get(&payment_id).unwrap();
                assert!(matches!(
                    payment.status,
                    PaymentStatus::Refunded { .. }
                ));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_cannot_refund_uncaptured_payment() {
        let payment_id = PaymentId::new();

        ReducerTest::new(PaymentReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = PaymentState::new();
                let payment = Payment::new(
                    payment_id,
                    ReservationId::new(),
                    CustomerId::new(),
                    Money::from_dollars(100),
                    PaymentMethod::CreditCard {
                        last_four: "4242".to_string(),
                    },
                );
                // Payment is Pending, not Captured
                state.payments.insert(payment_id, payment);
                state
            })
            .when_action(PaymentAction::RefundPayment {
                payment_id,
                amount: Money::from_dollars(100),
                reason: "Test".to_string(),
            })
            .then_state(move |state| {
                let payment = state.get(&payment_id).unwrap();
                // Should still be Pending
                assert_eq!(payment.status, PaymentStatus::Pending);
                // Should have error
                assert!(state.last_error.is_some());
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("Cannot refund"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }
}
