//! Payment aggregate for the Event Ticketing System.
//!
//! Handles payment processing and refunds. In production, this would integrate with
//! real payment gateways (Stripe, `PayPal`, etc.). For this demo, we simulate success
//! and provide a command to simulate failures for testing compensation flows.

use crate::projections::TicketingEvent;
use crate::types::{CustomerId, Money, Payment, PaymentId, PaymentMethod, PaymentState, PaymentStatus, ReservationId};
use chrono::{DateTime, Utc};
use composable_rust_core::{
    append_events, effect::Effect, environment::Clock, event_bus::EventBus,
    event_store::EventStore, publish_event, reducer::Reducer, smallvec, stream::StreamId, SmallVec,
};
use composable_rust_macros::Action;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// Note: ReservationAction would be imported in a real system for cross-aggregate events
// For now, we'll keep it simple and focus on the Payment aggregate itself

// ============================================================================
// Projection Query Trait
// ============================================================================

/// Trait for querying payment projection data.
///
/// This trait defines the read operations needed by the Payment aggregate
/// to load state from the projection when processing commands.
///
/// # Pattern: State Loading from Projections
///
/// According to the state-loading-patterns spec, aggregates load state on-demand
/// by querying projections. This trait is injected via the Environment to enable
/// the reducer to trigger state loading effects.
#[async_trait::async_trait]
pub trait PaymentProjectionQuery: Send + Sync {
    /// Load payment data for a specific payment.
    ///
    /// Returns payment details if found.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    async fn load_payment(&self, payment_id: &PaymentId) -> Result<Option<Payment>, String>;

    /// Load payments for a specific customer.
    ///
    /// Returns list of payments for the customer, with pagination support.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    async fn load_customer_payments(&self, customer_id: &CustomerId, limit: usize, offset: usize) -> Result<Vec<Payment>, String>;
}

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

    /// Query a single payment by ID
    #[command]
    GetPayment {
        /// Payment ID to query
        payment_id: PaymentId,
    },

    /// Query payments for a customer
    #[command]
    ListCustomerPayments {
        /// Customer ID to query payments for
        customer_id: CustomerId,
        /// Maximum number of results
        limit: usize,
        /// Offset for pagination
        offset: usize,
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

    /// Payment was queried (query result)
    #[event]
    PaymentQueried {
        /// Payment ID that was queried
        payment_id: PaymentId,
        /// Payment data (None if not found)
        payment: Option<Payment>,
    },

    /// Customer payments were listed (query result)
    #[event]
    CustomerPaymentsListed {
        /// Customer ID that was queried
        customer_id: CustomerId,
        /// List of payments for the customer
        payments: Vec<Payment>,
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
    /// Event store for persistence
    pub event_store: Arc<dyn EventStore>,
    /// Event bus for publishing
    pub event_bus: Arc<dyn EventBus>,
    /// Stream ID for this aggregate instance
    pub stream_id: StreamId,
    /// Projection query for loading state on-demand
    pub projection: Arc<dyn PaymentProjectionQuery>,
}

impl PaymentEnvironment {
    /// Creates a new `PaymentEnvironment`
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        event_store: Arc<dyn EventStore>,
        event_bus: Arc<dyn EventBus>,
        stream_id: StreamId,
        projection: Arc<dyn PaymentProjectionQuery>,
    ) -> Self {
        Self {
            clock,
            event_store,
            event_bus,
            stream_id,
            projection,
        }
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

    /// Creates effects for persisting and publishing an event
    fn create_effects(
        event: PaymentAction,
        env: &PaymentEnvironment,
    ) -> SmallVec<[Effect<PaymentAction>; 4]> {
        let ticketing_event = TicketingEvent::Payment(event);
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
                on_error: |error| Some(PaymentAction::ValidationFailed {
                    error: error.to_string()
                })
            },
            publish_event! {
                bus: env.event_bus,
                topic: "payment",
                event: serialized,
                on_success: || None,
                on_error: |error| Some(PaymentAction::ValidationFailed {
                    error: error.to_string()
                })
            }
        ]
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

            // Commands and query results don't modify state
            PaymentAction::ProcessPayment { .. }
            | PaymentAction::RefundPayment { .. }
            | PaymentAction::SimulatePaymentFailure { .. }
            | PaymentAction::GetPayment { .. }
            | PaymentAction::ListCustomerPayments { .. }
            | PaymentAction::PaymentQueried { .. }
            | PaymentAction::CustomerPaymentsListed { .. } => {}
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
                    payment_method: payment_method.clone(),
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

                // Persist and publish both events
                let mut effects = Self::create_effects(processed, env);
                effects.extend(Self::create_effects(success, env));
                effects
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

                Self::create_effects(failure, env)
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

                Self::create_effects(refund, env)
            }

            // ========== Query: Get Payment ==========
            PaymentAction::GetPayment { payment_id } => {
                let projection = env.projection.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    match projection.load_payment(&payment_id).await {
                        Ok(payment) => Some(PaymentAction::PaymentQueried {
                            payment_id,
                            payment,
                        }),
                        Err(e) => Some(PaymentAction::ValidationFailed {
                            error: format!("Failed to load payment: {e}"),
                        }),
                    }
                }))]
            }

            // ========== Query: List Customer Payments ==========
            PaymentAction::ListCustomerPayments { customer_id, limit, offset } => {
                let projection = env.projection.clone();
                let customer_id_clone = customer_id;
                smallvec![Effect::Future(Box::pin(async move {
                    match projection.load_customer_payments(&customer_id_clone, limit, offset).await {
                        Ok(payments) => Some(PaymentAction::CustomerPaymentsListed {
                            customer_id: customer_id_clone,
                            payments,
                        }),
                        Err(e) => Some(PaymentAction::ValidationFailed {
                            error: format!("Failed to load customer payments: {e}"),
                        }),
                    }
                }))]
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
    use composable_rust_testing::{assertions, mocks::{InMemoryEventBus, InMemoryEventStore}, ReducerTest};

    // Mock projection query for tests
    #[derive(Clone)]
    struct MockPaymentQuery;

    #[async_trait::async_trait]
    impl PaymentProjectionQuery for MockPaymentQuery {
        async fn load_payment(&self, _payment_id: &PaymentId) -> Result<Option<Payment>, String> {
            // Return None for tests - state will be built from events
            Ok(None)
        }

        async fn load_customer_payments(&self, _customer_id: &CustomerId, _limit: usize, _offset: usize) -> Result<Vec<Payment>, String> {
            // Return empty for tests - state will be built from events
            Ok(Vec::new())
        }
    }

    fn create_test_env() -> PaymentEnvironment {
        PaymentEnvironment::new(
            Arc::new(SystemClock),
            Arc::new(InMemoryEventStore::new()),
            Arc::new(InMemoryEventBus::new()),
            StreamId::new("payment-test"),
            Arc::new(MockPaymentQuery),
        )
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
            .then_effects(|effects| {
                // Should return 4 effects:
                // 2 for PaymentProcessed (AppendEvents + PublishEvent)
                // 2 for PaymentSucceeded (AppendEvents + PublishEvent)
                assert_eq!(effects.len(), 4);
            })
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
            .then_effects(|effects| {
                // Should return 2 effects: AppendEvents + PublishEvent
                assert_eq!(effects.len(), 2);
            })
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
            .then_effects(|effects| {
                // Should return 2 effects: AppendEvents + PublishEvent
                assert_eq!(effects.len(), 2);
            })
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
