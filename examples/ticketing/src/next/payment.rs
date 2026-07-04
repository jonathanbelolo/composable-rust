//! Payment aggregate business logic using next-generation architecture.
//!
//! Pure domain logic for payment processing. This aggregate handles payment
//! processing, success/failure handling, and refunds.
//!
//! # Commands
//!
//! - `ProcessPayment`: Process a payment for a reservation
//! - `RefundPayment`: Refund a captured payment
//! - `SimulatePaymentFailure`: Test command for saga compensation flows
//!
//! # Payment Flow
//!
//! ```text
//! ProcessPayment → PaymentProcessed → PaymentSucceeded/PaymentFailed
//!                                            │
//!                                            ▼
//!                                    RefundPayment (if captured)
//!                                            │
//!                                            ▼
//!                                    PaymentRefunded
//! ```

use chrono::{DateTime, Utc};
use composable_rust_next::{BusinessLogic, BusinessResult, Clock, StreamId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use uuid::Uuid;

use crate::types::{CustomerId, Money, PaymentId, PaymentMethod, ReservationId};

// ═══════════════════════════════════════════════════════════════════════════
// Commands
// ═══════════════════════════════════════════════════════════════════════════

/// Commands for the Payment aggregate.
///
/// # CQRS Pattern
///
/// Write commands include a `fetched` field that contains validation data
/// pre-loaded from projections by the `QueryFetcher`. This follows the CQRS
/// principle: validation data comes from projections, not from loading state
/// from the event store.
#[derive(Debug, Clone)]
pub enum PaymentCommand {
    /// Process a new payment.
    ///
    /// `fetched: None` means payment doesn't exist (can create).
    /// `fetched: Some(_)` means payment already exists (reject as duplicate).
    ProcessPayment {
        /// Payment ID (provided for idempotency)
        payment_id: PaymentId,
        /// Reservation this payment is for
        reservation_id: ReservationId,
        /// Customer making the payment
        customer_id: CustomerId,
        /// Amount to charge
        amount: Money,
        /// Payment method
        payment_method: PaymentMethod,
        /// Pre-fetched payment data (None = doesn't exist)
        fetched: Option<PaymentDto>,
    },

    /// Refund a captured payment.
    ///
    /// `fetched` contains the payment data for validation (must exist, must be captured).
    RefundPayment {
        /// Payment to refund
        payment_id: PaymentId,
        /// Refund amount (must be <= original amount)
        amount: Money,
        /// Reason for refund
        reason: String,
        /// Pre-fetched payment data for validation
        fetched: Option<PaymentDto>,
    },

    /// Simulate payment failure (for testing compensation flows).
    SimulatePaymentFailure {
        /// Payment ID
        payment_id: PaymentId,
        /// Failure reason
        reason: String,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Query Commands
    // ═══════════════════════════════════════════════════════════════════════

    /// Get a single payment by ID.
    ///
    /// The `fetched` field is populated by the Handler before calling process().
    GetPayment {
        /// Payment to retrieve
        payment_id: PaymentId,
        /// Pre-fetched payment data (populated by Handler)
        fetched: Option<PaymentDto>,
    },

    /// List all payments for a customer.
    ///
    /// The `fetched` field is populated by the Handler before calling process().
    ListCustomerPayments {
        /// Customer to list payments for
        customer_id: CustomerId,
        /// Pre-fetched payments (populated by Handler)
        fetched: Vec<PaymentDto>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// Query DTOs and Response Types
// ═══════════════════════════════════════════════════════════════════════════

/// Payment data transfer object for query responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentDto {
    /// Payment ID
    pub id: PaymentId,
    /// Reservation ID
    pub reservation_id: ReservationId,
    /// Customer ID
    pub customer_id: CustomerId,
    /// Amount
    pub amount: Money,
    /// Payment method
    pub payment_method: String,
    /// Payment status
    pub status: PaymentDtoStatus,
    /// Transaction ID (if succeeded)
    pub transaction_id: Option<String>,
    /// Failure reason (if failed)
    pub failure_reason: Option<String>,
    /// Refund amount (if refunded)
    pub refund_amount: Option<Money>,
    /// Refund reason (if refunded)
    pub refund_reason: Option<String>,
}

/// Payment status for DTOs (simplified, no embedded data).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaymentDtoStatus {
    /// Payment is being processed
    Processing,
    /// Payment succeeded (captured)
    Succeeded,
    /// Payment failed
    Failed,
    /// Payment was refunded
    Refunded,
}

/// Response types for Payment queries.
#[derive(Debug, Clone)]
pub enum PaymentResponse {
    /// Single payment details
    Single(PaymentDto),
    /// List of payments
    List(Vec<PaymentDto>),
}

// ═══════════════════════════════════════════════════════════════════════════
// Events
// ═══════════════════════════════════════════════════════════════════════════

/// Domain events for the Payment aggregate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PaymentEvent {
    /// Payment processing started.
    PaymentProcessed {
        /// Payment ID
        payment_id: PaymentId,
        /// Reservation ID
        reservation_id: ReservationId,
        /// Customer ID
        customer_id: CustomerId,
        /// Amount
        amount: Money,
        /// Payment method
        payment_method: PaymentMethod,
        /// When processed
        processed_at: DateTime<Utc>,
    },

    /// Payment succeeded (captured).
    PaymentSucceeded {
        /// Payment ID
        payment_id: PaymentId,
        /// Transaction ID from gateway
        transaction_id: String,
        /// When succeeded
        succeeded_at: DateTime<Utc>,
    },

    /// Payment failed.
    PaymentFailed {
        /// Payment ID
        payment_id: PaymentId,
        /// Failure reason
        reason: String,
        /// When failed
        failed_at: DateTime<Utc>,
    },

    /// Payment was refunded.
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
}

// ═══════════════════════════════════════════════════════════════════════════
// State
// ═══════════════════════════════════════════════════════════════════════════

/// Payment status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentStatus {
    /// Payment is being processed
    Processing,
    /// Payment was captured successfully
    Captured,
    /// Payment failed
    Failed {
        /// Failure reason
        reason: String,
    },
    /// Payment was refunded
    Refunded {
        /// Refund amount
        amount: Money,
    },
}

/// Individual payment record.
#[derive(Debug, Clone)]
pub struct Payment {
    /// Payment ID
    pub id: PaymentId,
    /// Reservation this payment is for
    pub reservation_id: ReservationId,
    /// Customer who made the payment
    pub customer_id: CustomerId,
    /// Payment amount
    pub amount: Money,
    /// Payment method
    pub payment_method: PaymentMethod,
    /// Current status
    pub status: PaymentStatus,
    /// When processed
    pub processed_at: Option<DateTime<Utc>>,
    /// Transaction ID (if captured)
    pub transaction_id: Option<String>,
}

/// State for the Payment aggregate.
#[derive(Debug, Clone, Default)]
pub struct PaymentState {
    /// All payments indexed by ID
    pub payments: HashMap<PaymentId, Payment>,
}

impl PaymentState {
    /// Get a payment by ID.
    #[must_use]
    pub fn get(&self, payment_id: &PaymentId) -> Option<&Payment> {
        self.payments.get(payment_id)
    }

    /// Check if a payment exists.
    #[must_use]
    pub fn exists(&self, payment_id: &PaymentId) -> bool {
        self.payments.contains_key(payment_id)
    }

    /// Count total payments.
    #[must_use]
    pub fn count(&self) -> usize {
        self.payments.len()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Errors
// ═══════════════════════════════════════════════════════════════════════════

/// Business errors for the Payment aggregate.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PaymentError {
    /// Payment already exists
    #[error("Payment already exists: {0}")]
    AlreadyExists(PaymentId),

    /// Payment not found
    #[error("Payment not found: {0}")]
    NotFound(PaymentId),

    /// Payment not found (query context)
    #[error("Payment not found")]
    PaymentNotFound,

    /// Invalid amount
    #[error("Invalid amount: {0}")]
    InvalidAmount(String),

    /// Payment cannot be refunded
    #[error("Cannot refund payment: {0}")]
    CannotRefund(String),

    /// Validation failed
    #[error("Validation failed: {0}")]
    ValidationFailed(String),

    /// Payment was declined by the payment processor
    #[error("Payment declined: {0}")]
    PaymentDeclined(String),
}

// ═══════════════════════════════════════════════════════════════════════════
// Business Logic
// ═══════════════════════════════════════════════════════════════════════════

/// Pure business logic for the Payment aggregate.
///
/// Simulates payment processing. In production, would integrate with
/// payment gateways like Stripe, PayPal, etc.
#[derive(Debug, Clone)]
pub struct PaymentBusinessLogic;

impl BusinessLogic for PaymentBusinessLogic {
    type State = PaymentState;
    type Input = PaymentCommand;
    type Event = PaymentEvent;
    type Error = PaymentError;
    type Call = Infallible;
    type CallResult = Infallible;
    type Response = PaymentResponse;

    fn stream_id(input: &Self::Input) -> StreamId {
        match input {
            PaymentCommand::ProcessPayment { payment_id, .. }
            | PaymentCommand::RefundPayment { payment_id, .. }
            | PaymentCommand::SimulatePaymentFailure { payment_id, .. }
            | PaymentCommand::GetPayment { payment_id, .. } => {
                StreamId::new(format!("payment-{}", payment_id.as_uuid()))
            }
            PaymentCommand::ListCustomerPayments { customer_id, .. } => {
                StreamId::new(format!("query-payments-customer-{}", customer_id.as_uuid()))
            }
        }
    }

    fn process(
        &self,
        input: Self::Input,
        clock: &dyn Clock,
    ) -> Result<BusinessResult<Self::Event, Self::Call, Self::Response>, Self::Error> {
        let now = clock.now();

        match input {
            PaymentCommand::ProcessPayment {
                payment_id,
                reservation_id,
                customer_id,
                amount,
                payment_method,
                fetched,
            } => {
                // Validation: fetched = Some means payment already exists
                if fetched.is_some() {
                    return Err(PaymentError::AlreadyExists(payment_id));
                }

                if amount.cents() == 0 {
                    return Err(PaymentError::InvalidAmount(
                        "Payment amount must be greater than zero".to_string(),
                    ));
                }

                // In the next-gen architecture, we emit both processed and success
                // events atomically. In production, you'd have async gateway calls.
                let transaction_id = format!("txn_{}", Uuid::new_v4());

                Ok(BusinessResult::Done(vec![
                    PaymentEvent::PaymentProcessed {
                        payment_id,
                        reservation_id,
                        customer_id,
                        amount,
                        payment_method,
                        processed_at: now,
                    },
                    PaymentEvent::PaymentSucceeded {
                        payment_id,
                        transaction_id,
                        succeeded_at: now,
                    },
                ]))
            }

            PaymentCommand::RefundPayment {
                payment_id,
                amount,
                reason,
                fetched,
            } => {
                // Validation: payment must exist
                let payment = fetched.ok_or(PaymentError::NotFound(payment_id))?;

                if amount.cents() == 0 {
                    return Err(PaymentError::InvalidAmount(
                        "Refund amount must be greater than zero".to_string(),
                    ));
                }

                if reason.trim().is_empty() {
                    return Err(PaymentError::ValidationFailed(
                        "Refund reason is required".to_string(),
                    ));
                }

                // Can only refund captured payments
                if payment.status != PaymentDtoStatus::Succeeded {
                    return Err(PaymentError::CannotRefund(
                        "Can only refund captured payments".to_string(),
                    ));
                }

                // Refund cannot exceed original amount
                if amount.cents() > payment.amount.cents() {
                    return Err(PaymentError::InvalidAmount(format!(
                        "Refund amount ({}) exceeds payment amount ({})",
                        amount, payment.amount
                    )));
                }

                Ok(BusinessResult::Done(vec![PaymentEvent::PaymentRefunded {
                    payment_id,
                    amount,
                    reason,
                    refunded_at: now,
                }]))
            }

            PaymentCommand::SimulatePaymentFailure { payment_id, reason } => {
                // This is a test command that simulates payment failure.
                // It's used to test saga compensation flows.
                // In the next-gen architecture, we emit both processed and failed events.

                // Don't validate existence - this is for testing compensation
                // where we may want to simulate failure for any payment_id.

                Ok(BusinessResult::Done(vec![PaymentEvent::PaymentFailed {
                    payment_id,
                    reason,
                    failed_at: now,
                }]))
            }

            // ═══════════════════════════════════════════════════════════════
            // Query Commands (data pre-fetched by Handler)
            // ═══════════════════════════════════════════════════════════════

            PaymentCommand::GetPayment { payment_id: _, fetched } => {
                let payment = fetched.ok_or(PaymentError::PaymentNotFound)?;
                Ok(BusinessResult::Respond(PaymentResponse::Single(payment)))
            }

            PaymentCommand::ListCustomerPayments { customer_id: _, fetched } => {
                Ok(BusinessResult::Respond(PaymentResponse::List(fetched)))
            }
        }
    }

    fn apply(&self, state: &mut Self::State, event: &Self::Event) {
        match event {
            PaymentEvent::PaymentProcessed {
                payment_id,
                reservation_id,
                customer_id,
                amount,
                payment_method,
                processed_at,
            } => {
                let payment = Payment {
                    id: *payment_id,
                    reservation_id: *reservation_id,
                    customer_id: *customer_id,
                    amount: *amount,
                    payment_method: payment_method.clone(),
                    status: PaymentStatus::Processing,
                    processed_at: Some(*processed_at),
                    transaction_id: None,
                };
                state.payments.insert(*payment_id, payment);
            }

            PaymentEvent::PaymentSucceeded {
                payment_id,
                transaction_id,
                ..
            } => {
                if let Some(payment) = state.payments.get_mut(payment_id) {
                    payment.status = PaymentStatus::Captured;
                    payment.transaction_id = Some(transaction_id.clone());
                }
            }

            PaymentEvent::PaymentFailed {
                payment_id, reason, ..
            } => {
                if let Some(payment) = state.payments.get_mut(payment_id) {
                    payment.status = PaymentStatus::Failed {
                        reason: reason.clone(),
                    };
                }
            }

            PaymentEvent::PaymentRefunded {
                payment_id, amount, ..
            } => {
                if let Some(payment) = state.payments.get_mut(payment_id) {
                    payment.status = PaymentStatus::Refunded { amount: *amount };
                }
            }
        }
    }

    fn event_type_name(event: &Self::Event) -> &'static str {
        match event {
            PaymentEvent::PaymentProcessed { .. } => "PaymentProcessed",
            PaymentEvent::PaymentSucceeded { .. } => "PaymentSucceeded",
            PaymentEvent::PaymentFailed { .. } => "PaymentFailed",
            PaymentEvent::PaymentRefunded { .. } => "PaymentRefunded",
        }
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

    /// Helper to create a PaymentDto for testing
    fn payment_dto(payment_id: PaymentId, amount: Money, status: PaymentDtoStatus) -> PaymentDto {
        PaymentDto {
            id: payment_id,
            reservation_id: ReservationId::new(),
            customer_id: CustomerId::new(),
            amount,
            payment_method: "CreditCard".to_string(),
            status,
            transaction_id: if status == PaymentDtoStatus::Succeeded { Some("txn_123".to_string()) } else { None },
            failure_reason: None,
            refund_amount: None,
            refund_reason: None,
        }
    }

    #[test]
    fn process_payment_succeeds() {
        let logic = PaymentBusinessLogic;
        let clock = test_clock();

        // fetched: None means payment doesn't exist yet
        let command = PaymentCommand::ProcessPayment {
            payment_id: PaymentId::new(),
            reservation_id: ReservationId::new(),
            customer_id: CustomerId::new(),
            amount: Money::from_dollars(100),
            payment_method: PaymentMethod::CreditCard {
                last_four: "4242".to_string(),
            },
            fetched: None,
        };

        let result = logic.process(command, &clock).unwrap();

        match result {
            BusinessResult::Done(events) => {
                assert_eq!(events.len(), 2);
                assert!(matches!(events[0], PaymentEvent::PaymentProcessed { .. }));
                assert!(matches!(events[1], PaymentEvent::PaymentSucceeded { .. }));
            }
            _ => panic!("Expected Done result"),
        }
    }

    #[test]
    fn process_payment_duplicate_fails() {
        let logic = PaymentBusinessLogic;
        let clock = test_clock();

        let payment_id = PaymentId::new();

        // fetched: Some means payment already exists
        let command = PaymentCommand::ProcessPayment {
            payment_id,
            reservation_id: ReservationId::new(),
            customer_id: CustomerId::new(),
            amount: Money::from_dollars(100),
            payment_method: PaymentMethod::CreditCard {
                last_four: "4242".to_string(),
            },
            fetched: Some(payment_dto(payment_id, Money::from_dollars(50), PaymentDtoStatus::Succeeded)),
        };

        let result = logic.process(command, &clock);
        assert!(matches!(result, Err(PaymentError::AlreadyExists(_))));
    }

    #[test]
    fn process_payment_zero_amount_fails() {
        let logic = PaymentBusinessLogic;
        let clock = test_clock();

        let command = PaymentCommand::ProcessPayment {
            payment_id: PaymentId::new(),
            reservation_id: ReservationId::new(),
            customer_id: CustomerId::new(),
            amount: Money::from_cents(0),
            payment_method: PaymentMethod::CreditCard {
                last_four: "4242".to_string(),
            },
            fetched: None,
        };

        let result = logic.process(command, &clock);
        assert!(matches!(result, Err(PaymentError::InvalidAmount(_))));
    }

    #[test]
    fn refund_captured_payment_succeeds() {
        let logic = PaymentBusinessLogic;
        let clock = test_clock();

        let payment_id = PaymentId::new();

        // Provide fetched data showing captured payment
        let command = PaymentCommand::RefundPayment {
            payment_id,
            amount: Money::from_dollars(50),
            reason: "Customer requested refund".to_string(),
            fetched: Some(payment_dto(payment_id, Money::from_dollars(100), PaymentDtoStatus::Succeeded)),
        };

        let result = logic.process(command, &clock).unwrap();

        match result {
            BusinessResult::Done(events) => {
                assert_eq!(events.len(), 1);
                match &events[0] {
                    PaymentEvent::PaymentRefunded { amount, reason, .. } => {
                        assert_eq!(*amount, Money::from_dollars(50));
                        assert_eq!(reason, "Customer requested refund");
                    }
                    _ => panic!("Expected PaymentRefunded event"),
                }
            }
            _ => panic!("Expected Done result"),
        }
    }

    #[test]
    fn refund_uncaptured_payment_fails() {
        let logic = PaymentBusinessLogic;
        let clock = test_clock();

        let payment_id = PaymentId::new();

        // Provide fetched data showing processing payment (not captured)
        let command = PaymentCommand::RefundPayment {
            payment_id,
            amount: Money::from_dollars(50),
            reason: "Customer requested refund".to_string(),
            fetched: Some(payment_dto(payment_id, Money::from_dollars(100), PaymentDtoStatus::Processing)),
        };

        let result = logic.process(command, &clock);
        assert!(matches!(result, Err(PaymentError::CannotRefund(_))));
    }

    #[test]
    fn refund_exceeds_amount_fails() {
        let logic = PaymentBusinessLogic;
        let clock = test_clock();

        let payment_id = PaymentId::new();

        // Provide fetched data for $100 payment
        let command = PaymentCommand::RefundPayment {
            payment_id,
            amount: Money::from_dollars(150), // More than original!
            reason: "Customer requested refund".to_string(),
            fetched: Some(payment_dto(payment_id, Money::from_dollars(100), PaymentDtoStatus::Succeeded)),
        };

        let result = logic.process(command, &clock);
        assert!(matches!(result, Err(PaymentError::InvalidAmount(_))));
    }

    #[test]
    fn simulate_payment_failure_succeeds() {
        let logic = PaymentBusinessLogic;
        let clock = test_clock();

        let command = PaymentCommand::SimulatePaymentFailure {
            payment_id: PaymentId::new(),
            reason: "Card declined".to_string(),
        };

        let result = logic.process(command, &clock).unwrap();

        match result {
            BusinessResult::Done(events) => {
                assert_eq!(events.len(), 1);
                match &events[0] {
                    PaymentEvent::PaymentFailed { reason, .. } => {
                        assert_eq!(reason, "Card declined");
                    }
                    _ => panic!("Expected PaymentFailed event"),
                }
            }
            _ => panic!("Expected Done result"),
        }
    }

    #[test]
    fn apply_payment_processed_updates_state() {
        let logic = PaymentBusinessLogic;
        let mut state = PaymentState::default();

        let payment_id = PaymentId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

        let event = PaymentEvent::PaymentProcessed {
            payment_id,
            reservation_id,
            customer_id,
            amount: Money::from_dollars(100),
            payment_method: PaymentMethod::CreditCard {
                last_four: "4242".to_string(),
            },
            processed_at: Utc::now(),
        };

        logic.apply(&mut state, &event);

        assert_eq!(state.count(), 1);
        let payment = state.get(&payment_id).unwrap();
        assert_eq!(payment.status, PaymentStatus::Processing);
        assert_eq!(payment.amount, Money::from_dollars(100));
    }

    #[test]
    fn apply_payment_succeeded_updates_state() {
        let logic = PaymentBusinessLogic;
        let mut state = PaymentState::default();

        let payment_id = PaymentId::new();

        // First add a payment in processing state
        state.payments.insert(
            payment_id,
            Payment {
                id: payment_id,
                reservation_id: ReservationId::new(),
                customer_id: CustomerId::new(),
                amount: Money::from_dollars(100),
                payment_method: PaymentMethod::CreditCard {
                    last_four: "4242".to_string(),
                },
                status: PaymentStatus::Processing,
                processed_at: Some(Utc::now()),
                transaction_id: None,
            },
        );

        let event = PaymentEvent::PaymentSucceeded {
            payment_id,
            transaction_id: "txn_123".to_string(),
            succeeded_at: Utc::now(),
        };

        logic.apply(&mut state, &event);

        let payment = state.get(&payment_id).unwrap();
        assert_eq!(payment.status, PaymentStatus::Captured);
        assert_eq!(payment.transaction_id, Some("txn_123".to_string()));
    }
}
