//! Payment aggregate for the Event Ticketing System.
//!
//! Handles payment processing and refunds. In production, this would integrate with
//! real payment gateways (Stripe, `PayPal`, etc.). For this demo, we simulate success
//! and provide a command to simulate failures for testing compensation flows.

use crate::projections::TicketingEvent;
use crate::types::{
    CustomerId, GlobalActionChannels, Money, Payment, PaymentId, PaymentMethod, PaymentState, PaymentStatus,
    ReservationId, ResponseChannel,
};
use chrono::{DateTime, Utc};
use composable_rust_core::{
    append_events, effect::Effect, environment::Clock,
    event_store::EventStore, reducer::Reducer, smallvec,
    stream::{StreamId, Version},
    SmallVec,
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
#[derive(Action, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PaymentAction {
    // Commands
    /// Process a payment
    #[command]
    ProcessPayment {
        /// Payment ID
        payment_id: PaymentId,
        /// Reservation this payment is for
        reservation_id: ReservationId,
        /// Customer making the payment
        customer_id: CustomerId,
        /// Amount to charge
        amount: Money,
        /// Payment method
        payment_method: PaymentMethod,

        /// Response channel for projection completion
        #[serde(skip)]
        respond_to: ResponseChannel,
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
        /// Customer ID
        customer_id: CustomerId,
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

    /// Serialization failed
    #[event]
    SerializationFailed {
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

    /// Projection completed (infrastructure event from projection system)
    #[event]
    ProjectionCompleted {
        /// Correlation ID from original request
        correlation_id: String,
        /// Name of projection that completed
        projection_name: String,
    },

    /// Payment confirmed - projection updated successfully (business domain event)
    #[event]
    PaymentConfirmed {
        /// Payment ID
        payment_id: PaymentId,
    },

    /// Payment projection failed (business domain event)
    #[event]
    PaymentProjectionFailed {
        /// Payment ID
        payment_id: PaymentId,
        /// Failure reason
        reason: String,
    },

    /// Stream version was updated after successful event append
    #[event]
    VersionUpdated {
        /// New version number
        version: Version,
    },

    // ─────────────────────────────────────────────────────────────────────
    // INTERNAL ACTIONS: Two-phase pattern (async load → sync execute)
    // ─────────────────────────────────────────────────────────────────────

    /// Execute process payment after async version query (internal)
    #[doc(hidden)]
    ExecuteProcessPayment {
        /// Payment ID
        payment_id: PaymentId,
        /// Reservation this payment is for
        reservation_id: ReservationId,
        /// Customer making the payment
        customer_id: CustomerId,
        /// Amount to charge
        amount: Money,
        /// Payment method
        payment_method: PaymentMethod,
        /// Stream version from database query
        current_version: Version,
        /// Timestamp captured in Phase 1
        processed_at: DateTime<Utc>,
    },

    /// Execute simulate payment failure after async version query (internal)
    #[doc(hidden)]
    ExecuteSimulatePaymentFailure {
        /// Payment ID
        payment_id: PaymentId,
        /// Failure reason
        reason: String,
        /// Stream version from database query
        current_version: Version,
        /// Timestamp captured in Phase 1
        failed_at: DateTime<Utc>,
    },

    /// Execute refund payment after async version query (internal)
    #[doc(hidden)]
    ExecuteRefundPayment {
        /// Payment to refund
        payment_id: PaymentId,
        /// Refund amount
        amount: Money,
        /// Refund reason
        reason: String,
        /// Loaded payment for validation
        loaded_payment: Payment,
        /// Stream version from database query
        current_version: Version,
        /// Timestamp captured in Phase 1
        refunded_at: DateTime<Utc>,
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
    /// Stream ID for this aggregate instance
    pub stream_id: StreamId,
    /// Projection query for loading state on-demand
    pub projection: Arc<dyn PaymentProjectionQuery>,
    /// Global action channels for cross-aggregate coordination
    pub global_actions: GlobalActionChannels,
}

impl PaymentEnvironment {
    /// Creates a new `PaymentEnvironment`
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        event_store: Arc<dyn EventStore>,
        stream_id: StreamId,
        projection: Arc<dyn PaymentProjectionQuery>,
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

    /// Creates effects for persisting events (`PostgreSQL` only, no Redpanda)
    ///
    /// With direct orchestration, we use local channels for coordination,
    /// so Redpanda publishing is no longer needed.
    ///
    /// # Returns
    ///
    /// Effects to persist the event. On serialization failure, returns a
    /// `SerializationFailed` effect instead of silently failing.
    fn create_effects(
        event: PaymentAction,
        expected_version: Version,
        env: &PaymentEnvironment,
    ) -> SmallVec<[Effect<PaymentAction>; 4]> {
        let ticketing_event = TicketingEvent::Payment(event.clone());
        let serialized = match ticketing_event.serialize() {
            Ok(s) => s,
            Err(e) => {
                return smallvec![Effect::Future(Box::pin(async move {
                    Some(PaymentAction::SerializationFailed {
                        error: format!("Failed to serialize payment event: {e}"),
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
                broadcast_on_success: event,
                on_success: |version| Some(PaymentAction::VersionUpdated { version }),
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
                customer_id,
                amount,
                payment_method,
                processed_at,
            } => {
                let payment = Payment::new(
                    *payment_id,
                    *reservation_id,
                    *customer_id,
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

            PaymentAction::ValidationFailed { error }
            | PaymentAction::SerializationFailed { error } => {
                state.last_error = Some(error.clone());
            }

            // Version updated callback from append_events! macro
            PaymentAction::VersionUpdated { version } => {
                state.version = *version;
            }

            // Commands, Execute actions, query results, and infrastructure events
            // don't modify state (or are handled above)
            PaymentAction::ProcessPayment { .. }
            | PaymentAction::RefundPayment { .. }
            | PaymentAction::SimulatePaymentFailure { .. }
            | PaymentAction::ExecuteProcessPayment { .. }
            | PaymentAction::ExecuteSimulatePaymentFailure { .. }
            | PaymentAction::ExecuteRefundPayment { .. }
            | PaymentAction::GetPayment { .. }
            | PaymentAction::ListCustomerPayments { .. }
            | PaymentAction::PaymentQueried { .. }
            | PaymentAction::CustomerPaymentsListed { .. }
            | PaymentAction::ProjectionCompleted { .. }
            | PaymentAction::PaymentConfirmed { .. }
            | PaymentAction::PaymentProjectionFailed { .. } => {}
            // Note: ValidationFailed and SerializationFailed are handled above
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

    #[allow(clippy::too_many_lines)] // Payment workflow has many state transitions
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ========== Process Payment - Phase 1 (Async Load) ==========
            PaymentAction::ProcessPayment {
                payment_id,
                reservation_id,
                customer_id,
                amount,
                payment_method,
                respond_to: _,
            } => {
                // ┌─────────────────────────────────────────────────────────────────┐
                // │ STEP 1: Early sync validation (before async load)              │
                // │ Fail fast for obviously invalid requests                       │
                // └─────────────────────────────────────────────────────────────────┘

                // Validate: payment doesn't already exist (idempotency)
                if state.payments.contains_key(&payment_id) {
                    Self::apply_event(
                        state,
                        &PaymentAction::ValidationFailed {
                            error: format!("Payment {payment_id} already exists"),
                        },
                    );
                    return SmallVec::new();
                }

                // Validate: amount must be positive
                if amount.cents() == 0 {
                    Self::apply_event(
                        state,
                        &PaymentAction::ValidationFailed {
                            error: "Payment amount must be greater than zero".to_string(),
                        },
                    );
                    return SmallVec::new();
                }

                // ┌─────────────────────────────────────────────────────────────────┐
                // │ STEP 2: Clone dependencies for async block                     │
                // └─────────────────────────────────────────────────────────────────┘
                let event_store = env.event_store.clone();
                let stream_id = env.stream_id.clone();
                let clock = env.clock.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // ┌─────────────────────────────────────────────────────────────┐
                    // │ STEP 3: Query database for authoritative stream version    │
                    // └─────────────────────────────────────────────────────────────┘
                    let current_version = match event_store.get_stream_version(stream_id).await {
                        Ok(version) => version,
                        Err(e) => {
                            return Some(PaymentAction::ValidationFailed {
                                error: format!("Failed to get stream version: {e}"),
                            });
                        }
                    };

                    // ┌─────────────────────────────────────────────────────────────┐
                    // │ STEP 4: Return Execute action with loaded data             │
                    // └─────────────────────────────────────────────────────────────┘
                    Some(PaymentAction::ExecuteProcessPayment {
                        payment_id,
                        reservation_id,
                        customer_id,
                        amount,
                        payment_method,
                        current_version,
                        processed_at: clock.now(),
                    })
                }))]
            }

            // ========== Process Payment - Phase 2 (Sync Execute) ==========
            PaymentAction::ExecuteProcessPayment {
                payment_id,
                reservation_id,
                customer_id,
                amount,
                payment_method,
                current_version,
                processed_at,
            } => {
                // Record payment attempt
                let processed = PaymentAction::PaymentProcessed {
                    payment_id,
                    reservation_id,
                    customer_id,
                    amount,
                    payment_method: payment_method.clone(),
                    processed_at,
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

                // Persist both events sequentially
                // Use current_version from database query for first event
                // Second event expects version to be one higher
                let effects1 = Self::create_effects(processed, current_version, env);
                let effects2 = Self::create_effects(success, current_version.next(), env);

                let mut effects = smallvec![Effect::Sequential(
                    effects1.into_iter().chain(effects2).collect()
                )];

                // Clone data for PublishWithResponse closure
                let payment_id_clone_for_publish = payment_id;
                let reservation_id_clone_for_publish = reservation_id;
                let customer_id_clone_for_publish = customer_id;
                let amount_clone_for_publish = amount;
                let payment_method_clone_for_publish = payment_method.clone();

                // Clone IDs for callbacks (closures need separate owned values)
                let payment_id_for_success = payment_id;
                let payment_id_for_error = payment_id;

                // Publish ProcessPayment command to global channel for projections
                // Note: We publish the command (ProcessPayment) not the event (PaymentProcessed)
                // because projections need the respond_to field for completion tracking
                effects.push(Effect::PublishWithResponse {
                    channel: env.global_actions.payment_actions.clone(),
                    create_action: Box::new(move |respond_to| PaymentAction::ProcessPayment {
                        payment_id: payment_id_clone_for_publish,
                        reservation_id: reservation_id_clone_for_publish,
                        customer_id: customer_id_clone_for_publish,
                        amount: amount_clone_for_publish,
                        payment_method: payment_method_clone_for_publish,
                        respond_to,
                    }),
                    on_success: Box::new(move || {
                        Some(PaymentAction::PaymentConfirmed {
                            payment_id: payment_id_for_success,
                        })
                    }),
                    on_error: Box::new(move |reason| {
                        Some(PaymentAction::PaymentProjectionFailed {
                            payment_id: payment_id_for_error,
                            reason,
                        })
                    }),
                });

                // Note: The reservation saga receives payment results via send_and_wait_for()
                // on the payment store's action_broadcast channel. We don't need to send
                // to the global reservation_actions channel - that would be dead code since
                // nothing listens to it for cross-aggregate responses.

                effects
            }

            // ========== Simulate Failure - Phase 1 (Async Load) ==========
            //
            // NOTE: This command intentionally has NO validation. It's designed for
            // testing compensation flows in the reservation saga without requiring
            // a full payment setup. The saga needs to test what happens when payment
            // fails, and this command allows simulating that failure for any payment_id.
            //
            // If the payment_id doesn't exist in state, apply_event gracefully handles
            // it (no-op on the payments map, but still sets last_error with the reason).
            // This is acceptable because:
            // 1. This is a TEST-ONLY command (not exposed in production APIs)
            // 2. The saga's compensation logic triggers on PaymentFailed event
            // 3. The event is still persisted and broadcast for saga coordination
            PaymentAction::SimulatePaymentFailure {
                payment_id,
                reason,
            } => {
                let event_store = env.event_store.clone();
                let stream_id = env.stream_id.clone();
                let clock = env.clock.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    let current_version = match event_store.get_stream_version(stream_id).await {
                        Ok(version) => version,
                        Err(e) => {
                            return Some(PaymentAction::ValidationFailed {
                                error: format!("Failed to get stream version: {e}"),
                            });
                        }
                    };

                    Some(PaymentAction::ExecuteSimulatePaymentFailure {
                        payment_id,
                        reason,
                        current_version,
                        failed_at: clock.now(),
                    })
                }))]
            }

            // ========== Simulate Failure - Phase 2 (Sync Execute) ==========
            PaymentAction::ExecuteSimulatePaymentFailure {
                payment_id,
                reason,
                current_version,
                failed_at,
            } => {
                let failure = PaymentAction::PaymentFailed {
                    payment_id,
                    reason,
                    failed_at,
                };
                Self::apply_event(state, &failure);

                Self::create_effects(failure, current_version, env)
            }

            // ========== Refund Payment - Phase 1 (Async Load) ==========
            PaymentAction::RefundPayment {
                payment_id,
                amount,
                reason,
            } => {
                // ┌─────────────────────────────────────────────────────────────────┐
                // │ STEP 1: Early sync validation (before async load)              │
                // └─────────────────────────────────────────────────────────────────┘

                // Validate: refund amount must be positive
                if amount.cents() == 0 {
                    Self::apply_event(
                        state,
                        &PaymentAction::ValidationFailed {
                            error: "Refund amount must be greater than zero".to_string(),
                        },
                    );
                    return SmallVec::new();
                }

                // Validate: reason must not be empty
                if reason.trim().is_empty() {
                    Self::apply_event(
                        state,
                        &PaymentAction::ValidationFailed {
                            error: "Refund reason is required".to_string(),
                        },
                    );
                    return SmallVec::new();
                }

                // ┌─────────────────────────────────────────────────────────────────┐
                // │ STEP 2: Clone dependencies for async block                     │
                // └─────────────────────────────────────────────────────────────────┘
                let projection = env.projection.clone();
                let event_store = env.event_store.clone();
                let stream_id = env.stream_id.clone();
                let clock = env.clock.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // ┌─────────────────────────────────────────────────────────────┐
                    // │ STEP 3: Load data in parallel                              │
                    // └─────────────────────────────────────────────────────────────┘
                    let (payment_result, version_result) = tokio::join!(
                        projection.load_payment(&payment_id),
                        event_store.get_stream_version(stream_id)
                    );

                    let loaded_payment = match payment_result {
                        Ok(Some(payment)) => payment,
                        Ok(None) => {
                            return Some(PaymentAction::ValidationFailed {
                                error: format!("Payment {payment_id} not found"),
                            });
                        }
                        Err(e) => {
                            return Some(PaymentAction::ValidationFailed {
                                error: format!("Failed to load payment: {e}"),
                            });
                        }
                    };

                    let current_version = match version_result {
                        Ok(version) => version,
                        Err(e) => {
                            return Some(PaymentAction::ValidationFailed {
                                error: format!("Failed to get stream version: {e}"),
                            });
                        }
                    };

                    // ┌─────────────────────────────────────────────────────────────┐
                    // │ STEP 4: Validate with loaded data                          │
                    // └─────────────────────────────────────────────────────────────┘
                    if !matches!(loaded_payment.status, PaymentStatus::Captured) {
                        return Some(PaymentAction::ValidationFailed {
                            error: "Cannot refund uncaptured payment".to_string(),
                        });
                    }

                    if amount.cents() > loaded_payment.amount.cents() {
                        return Some(PaymentAction::ValidationFailed {
                            error: format!(
                                "Refund amount ({}) exceeds payment amount ({})",
                                amount, loaded_payment.amount
                            ),
                        });
                    }

                    // ┌─────────────────────────────────────────────────────────────┐
                    // │ STEP 5: Return Execute action with loaded data             │
                    // └─────────────────────────────────────────────────────────────┘
                    Some(PaymentAction::ExecuteRefundPayment {
                        payment_id,
                        amount,
                        reason,
                        loaded_payment,
                        current_version,
                        refunded_at: clock.now(),
                    })
                }))]
            }

            // ========== Refund Payment - Phase 2 (Sync Execute) ==========
            PaymentAction::ExecuteRefundPayment {
                payment_id,
                amount,
                reason,
                loaded_payment,
                current_version,
                refunded_at,
            } => {
                // Insert loaded payment into state for tracking
                state.payments.insert(payment_id, loaded_payment);

                // Process refund
                let refund = PaymentAction::PaymentRefunded {
                    payment_id,
                    amount,
                    reason,
                    refunded_at,
                };
                Self::apply_event(state, &refund);

                Self::create_effects(refund, current_version, env)
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

            // ========== Infrastructure: Projection Completed ==========
            //
            // NOTE: This handler exists for an alternative projection completion pattern
            // where projections send completion events through the aggregate. The current
            // architecture uses `PublishWithResponse` in ProcessPayment which handles
            // projection completion via the `respond_to` channel instead.
            //
            // This handler is kept for backwards compatibility and potential future use,
            // but is not actively used in the current monolith/channel-based architecture.
            PaymentAction::ProjectionCompleted {
                correlation_id,
                projection_name,
            } => {
                // Check if this is the payments projection completing
                if projection_name == "payments_projection" {
                    // Parse correlation_id to payment_id
                    // The HTTP handler sets correlation_id = payment_id.to_string()
                    if let Ok(uuid) = uuid::Uuid::parse_str(&correlation_id) {
                        let payment_id = PaymentId::from_uuid(uuid);

                        // Verify this payment exists in state
                        if state.payments.contains_key(&payment_id) {
                            let confirmed = PaymentAction::PaymentConfirmed { payment_id };
                            let expected_version = state.version;
                            Self::apply_event(state, &confirmed);
                            return Self::create_effects(confirmed, expected_version, env);
                        }

                        // Payment not found - emit failure event
                        let failed = PaymentAction::PaymentProjectionFailed {
                            payment_id,
                            reason: format!("Payment {payment_id:?} not found in state"),
                        };
                        let expected_version = state.version;
                        Self::apply_event(state, &failed);
                        return Self::create_effects(failed, expected_version, env);
                    }

                    // Failed to parse correlation_id as UUID
                    state.last_error = Some(format!(
                        "Invalid correlation_id format: {correlation_id}"
                    ));
                }
                // Not our projection or parsing failed - just apply event
                Self::apply_event(state, &PaymentAction::ProjectionCompleted {
                    correlation_id: correlation_id.clone(),
                    projection_name: projection_name.clone(),
                });
                SmallVec::new()
            }

            // ========== Echoed Events (terminal actions for send_and_wait_for) ==========
            //
            // These events are persisted and then echoed back as actions so they appear
            // on the action_broadcast channel. This allows send_and_wait_for to detect
            // completion. They apply the event to state and return NO effects to prevent
            // infinite loops.
            //
            // CRITICAL: Each echoed event MUST have an explicit handler here.
            // Do NOT use a catch-all - it hides bugs and is architecturally sloppy.
            PaymentAction::PaymentProcessed { .. } => {
                Self::apply_event(state, &action);
                SmallVec::new()
            }

            PaymentAction::PaymentSucceeded { .. } => {
                Self::apply_event(state, &action);
                SmallVec::new()
            }

            PaymentAction::PaymentFailed { .. } => {
                Self::apply_event(state, &action);
                SmallVec::new()
            }

            PaymentAction::PaymentRefunded { .. } => {
                Self::apply_event(state, &action);
                SmallVec::new()
            }

            PaymentAction::PaymentConfirmed { .. } => {
                Self::apply_event(state, &action);
                SmallVec::new()
            }

            PaymentAction::PaymentProjectionFailed { .. } => {
                Self::apply_event(state, &action);
                SmallVec::new()
            }

            // ========== Infrastructure Events (from effect system) ==========
            //
            // These are produced by the effect system (e.g., append_events macro)
            // and feed back into the reducer. They update state but produce no
            // further effects.
            PaymentAction::VersionUpdated { version } => {
                state.version = version;
                SmallVec::new()
            }

            PaymentAction::ValidationFailed { ref error } => {
                state.last_error = Some(error.clone());
                SmallVec::new()
            }

            PaymentAction::SerializationFailed { ref error } => {
                state.last_error = Some(error.clone());
                SmallVec::new()
            }

            // ========== Query Results (terminal actions) ==========
            //
            // Query results are returned to the caller via send_and_wait_for.
            // They don't modify aggregate state (queries are side-effect free).
            PaymentAction::PaymentQueried { .. } => {
                SmallVec::new()
            }

            PaymentAction::CustomerPaymentsListed { .. } => {
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
    use composable_rust_testing::{assertions, mocks::InMemoryEventStore, ReducerTest, TestStore};
    use std::collections::HashMap;
    use std::sync::RwLock;
    use std::time::Duration;

    // =========================================================================
    // Configurable Mock Projection Query
    // =========================================================================
    //
    // This mock allows tests to configure what data should be returned,
    // enabling comprehensive testing of async validation logic.

    /// Configurable mock for testing different payment scenarios
    #[derive(Clone, Default)]
    struct ConfigurableMockPaymentQuery {
        /// Payments to return, keyed by `PaymentId`
        payments: Arc<RwLock<HashMap<PaymentId, Payment>>>,
        /// If set, `load_payment` will return this error
        load_error: Arc<RwLock<Option<String>>>,
    }

    impl ConfigurableMockPaymentQuery {
        fn new() -> Self {
            Self::default()
        }

        /// Add a payment to be returned by `load_payment`
        fn with_payment(self, payment: Payment) -> Self {
            self.payments.write().unwrap().insert(payment.id, payment);
            self
        }

        /// Configure `load_payment` to return an error
        #[allow(dead_code)]
        fn with_error(self, error: &str) -> Self {
            *self.load_error.write().unwrap() = Some(error.to_string());
            self
        }
    }

    #[async_trait::async_trait]
    impl PaymentProjectionQuery for ConfigurableMockPaymentQuery {
        async fn load_payment(&self, payment_id: &PaymentId) -> Result<Option<Payment>, String> {
            // Check for configured error
            if let Some(ref error) = *self.load_error.read().unwrap() {
                return Err(error.clone());
            }
            // Return configured payment if present
            Ok(self.payments.read().unwrap().get(payment_id).cloned())
        }

        async fn load_customer_payments(
            &self,
            customer_id: &CustomerId,
            limit: usize,
            _offset: usize,
        ) -> Result<Vec<Payment>, String> {
            let payments: Vec<Payment> = self
                .payments
                .read()
                .unwrap()
                .values()
                .filter(|p| p.customer_id == *customer_id)
                .take(limit)
                .cloned()
                .collect();
            Ok(payments)
        }
    }

    // Simple mock that always returns None (for ReducerTest)
    #[derive(Clone)]
    struct MockPaymentQuery;

    #[async_trait::async_trait]
    impl PaymentProjectionQuery for MockPaymentQuery {
        async fn load_payment(&self, _payment_id: &PaymentId) -> Result<Option<Payment>, String> {
            Ok(None)
        }

        async fn load_customer_payments(
            &self,
            _customer_id: &CustomerId,
            _limit: usize,
            _offset: usize,
        ) -> Result<Vec<Payment>, String> {
            Ok(Vec::new())
        }
    }

    fn create_test_global_channels() -> GlobalActionChannels {
        use tokio::sync::broadcast;

        let (event_actions, _) = broadcast::channel(1000);
        let (inventory_actions, _) = broadcast::channel(1000);
        let (reservation_actions, _) = broadcast::channel(1000);
        let (payment_actions, _) = broadcast::channel(1000);

        GlobalActionChannels {
            event_actions,
            inventory_actions,
            reservation_actions,
            payment_actions,
        }
    }

    fn create_test_env() -> PaymentEnvironment {
        PaymentEnvironment::new(
            Arc::new(SystemClock),
            Arc::new(InMemoryEventStore::new()),
            StreamId::new("payment-test"),
            Arc::new(MockPaymentQuery),
            create_test_global_channels(),
        )
    }

    /// Create test environment with configurable projection query
    fn create_test_env_with_projection(
        projection: Arc<dyn PaymentProjectionQuery>,
    ) -> PaymentEnvironment {
        PaymentEnvironment::new(
            Arc::new(SystemClock),
            Arc::new(InMemoryEventStore::new()),
            StreamId::new("payment-test"),
            projection,
            create_test_global_channels(),
        )
    }

    // =========================================================================
    // Sync Validation Tests (ReducerTest)
    // =========================================================================
    //
    // Fast, pure tests that verify commands with invalid inputs are rejected
    // immediately without triggering async effects. These test the COMMAND
    // actions (ProcessPayment, RefundPayment), NOT the internal Execute actions.
    //
    // The two-phase async pattern works as:
    //   Phase 1: Command → sync validation → if OK, Effect::Future (async load)
    //   Phase 2: Execute action (internal) → state changes → effects
    //
    // We test sync validation here. Full async flows are tested with TestStore below.

    #[test]
    fn test_process_payment_duplicate_rejected() {
        let payment_id = PaymentId::new();

        ReducerTest::new(PaymentReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = PaymentState::new();
                // Payment already exists
                let payment = Payment::new(
                    payment_id,
                    ReservationId::new(),
                    CustomerId::new(),
                    Money::from_dollars(50),
                    PaymentMethod::CreditCard {
                        last_four: "1234".to_string(),
                    },
                );
                state.payments.insert(payment_id, payment);
                state
            })
            .when_action(PaymentAction::ProcessPayment {
                payment_id,
                reservation_id: ReservationId::new(),
                customer_id: CustomerId::new(),
                amount: Money::from_dollars(100),
                payment_method: PaymentMethod::CreditCard {
                    last_four: "4242".to_string(),
                },
                respond_to: ResponseChannel::none(),
            })
            .then_state(move |state| {
                // Original payment should be unchanged
                let payment = state.get(&payment_id).unwrap();
                assert_eq!(payment.amount, Money::from_dollars(50));
                // Should have error
                assert!(state.last_error.is_some());
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("already exists"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_process_payment_zero_amount_rejected() {
        ReducerTest::new(PaymentReducer::new())
            .with_env(create_test_env())
            .given_state(PaymentState::new())
            .when_action(PaymentAction::ProcessPayment {
                payment_id: PaymentId::new(),
                reservation_id: ReservationId::new(),
                customer_id: CustomerId::new(),
                amount: Money::from_cents(0),
                payment_method: PaymentMethod::CreditCard {
                    last_four: "4242".to_string(),
                },
                respond_to: ResponseChannel::none(),
            })
            .then_state(|state| {
                assert_eq!(state.count(), 0);
                assert!(state.last_error.is_some());
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
    fn test_refund_zero_amount_rejected() {
        // Zero amount is sync validation - happens before async block
        ReducerTest::new(PaymentReducer::new())
            .with_env(create_test_env())
            .given_state(PaymentState::new())
            .when_action(PaymentAction::RefundPayment {
                payment_id: PaymentId::new(),
                amount: Money::from_cents(0),
                reason: "Test refund".to_string(),
            })
            .then_state(|state| {
                // Should have error
                assert!(state.last_error.is_some());
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
    fn test_refund_empty_reason_rejected() {
        // Empty reason is sync validation - happens before async block
        ReducerTest::new(PaymentReducer::new())
            .with_env(create_test_env())
            .given_state(PaymentState::new())
            .when_action(PaymentAction::RefundPayment {
                payment_id: PaymentId::new(),
                amount: Money::from_dollars(50),
                reason: "   ".to_string(), // Whitespace only
            })
            .then_state(|state| {
                // Should have error
                assert!(state.last_error.is_some());
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("reason is required"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    // =========================================================================
    // Full Flow Tests (TestStore)
    // =========================================================================
    //
    // Test complete async behavior from command to terminal event.
    // These test the REAL behavior without knowing about internal Execute actions.
    //
    // The TestStore executes the full async flow:
    //   Command → Effect::Future (async load) → Execute action → Terminal event
    //
    // We wait for TERMINAL actions (PaymentSucceeded, PaymentFailed, PaymentRefunded)
    // which indicate the flow completed and state is updated.

    // -------------------------------------------------------------------------
    // Happy Paths
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_process_payment_success() {
        // Create TestStore with simple mock (no payment exists)
        let env = create_test_env_with_projection(Arc::new(ConfigurableMockPaymentQuery::new()));
        let store = TestStore::new(PaymentReducer::new(), env, PaymentState::new());

        let payment_id = PaymentId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

        // Send ProcessPayment command and wait for TERMINAL action (PaymentSucceeded)
        // This tests the full async flow:
        // ProcessPayment → get_stream_version() → ExecuteProcessPayment → PaymentSucceeded
        //
        // We wait for PaymentSucceeded (not ExecuteProcessPayment) because that's the
        // terminal action indicating the full flow completed and state is updated.
        let result = store
            .send_and_wait_for(
                PaymentAction::ProcessPayment {
                    payment_id,
                    reservation_id,
                    customer_id,
                    amount: Money::from_dollars(100),
                    payment_method: PaymentMethod::CreditCard {
                        last_four: "4242".to_string(),
                    },
                    respond_to: ResponseChannel::none(),
                },
                |action| {
                    matches!(
                        action,
                        PaymentAction::PaymentSucceeded { .. }
                            | PaymentAction::ValidationFailed { .. }
                    )
                },
                Duration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive PaymentSucceeded");
        let action = result.unwrap();
        assert!(
            matches!(action, PaymentAction::PaymentSucceeded { .. }),
            "Expected PaymentSucceeded, got {action:?}"
        );

        // No sleep needed - when PaymentSucceeded is broadcast, state is already updated
        let state = store.state(|s| s.clone()).await;
        assert_eq!(state.count(), 1, "Should have one payment");
        let payment = state.get(&payment_id).unwrap();
        assert_eq!(payment.status, PaymentStatus::Captured);
        assert_eq!(payment.amount, Money::from_dollars(100));

        // Clean up queued actions to avoid drop panic
        store.clear_queue();
    }

    #[tokio::test]
    async fn test_simulate_payment_failure() {
        let env = create_test_env_with_projection(Arc::new(ConfigurableMockPaymentQuery::new()));
        let store = TestStore::new(PaymentReducer::new(), env, PaymentState::new());

        let payment_id = PaymentId::new();

        // Send SimulatePaymentFailure and wait for TERMINAL action (PaymentFailed)
        // This tests the full async flow:
        // SimulatePaymentFailure → get_stream_version() → ExecuteSimulatePaymentFailure → PaymentFailed
        //
        // We wait for PaymentFailed (not ExecuteSimulatePaymentFailure) because that's the
        // terminal action indicating the full flow completed and state is updated.
        let result = store
            .send_and_wait_for(
                PaymentAction::SimulatePaymentFailure {
                    payment_id,
                    reason: "Card declined".to_string(),
                },
                |action| {
                    matches!(
                        action,
                        PaymentAction::PaymentFailed { .. }
                            | PaymentAction::ValidationFailed { .. }
                    )
                },
                Duration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive PaymentFailed");
        let action = result.unwrap();
        assert!(
            matches!(action, PaymentAction::PaymentFailed { .. }),
            "Expected PaymentFailed, got {action:?}"
        );

        // No sleep needed - when PaymentFailed is broadcast, state is already updated
        let state = store.state(|s| s.clone()).await;
        assert!(state.last_error.is_some());
        assert!(state.last_error.as_ref().unwrap().contains("declined"));

        // Clean up
        store.clear_queue();
    }

    #[tokio::test]
    async fn test_refund_payment_success() {
        let payment_id = PaymentId::new();
        let customer_id = CustomerId::new();

        // Create a captured payment in the mock projection
        let mut captured_payment = Payment::new(
            payment_id,
            ReservationId::new(),
            customer_id,
            Money::from_dollars(100),
            PaymentMethod::CreditCard {
                last_four: "4242".to_string(),
            },
        );
        captured_payment.status = PaymentStatus::Captured;

        let mock_projection = ConfigurableMockPaymentQuery::new().with_payment(captured_payment);
        let env = create_test_env_with_projection(Arc::new(mock_projection));
        let store = TestStore::new(PaymentReducer::new(), env, PaymentState::new());

        // Send RefundPayment and wait for TERMINAL action (PaymentRefunded)
        // This tests the full async flow:
        // RefundPayment → (load_payment + get_stream_version) → ExecuteRefundPayment → PaymentRefunded
        //
        // We wait for PaymentRefunded (not ExecuteRefundPayment) because that's the
        // terminal action indicating the full flow completed and state is updated.
        let result = store
            .send_and_wait_for(
                PaymentAction::RefundPayment {
                    payment_id,
                    amount: Money::from_dollars(50),
                    reason: "Customer requested refund".to_string(),
                },
                |action| {
                    matches!(
                        action,
                        PaymentAction::PaymentRefunded { .. }
                            | PaymentAction::ValidationFailed { .. }
                    )
                },
                Duration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive PaymentRefunded");
        let action = result.unwrap();
        assert!(
            matches!(action, PaymentAction::PaymentRefunded { .. }),
            "Expected PaymentRefunded, got {action:?}"
        );

        // No sleep needed - when PaymentRefunded is broadcast, state is already updated
        let state = store.state(|s| s.clone()).await;
        let payment = state.get(&payment_id).unwrap();
        assert!(
            matches!(payment.status, PaymentStatus::Refunded { .. }),
            "Payment should be refunded"
        );

        // Clean up
        store.clear_queue();
    }

    // -------------------------------------------------------------------------
    // Async Validation Failures
    // -------------------------------------------------------------------------
    //
    // These test validation that happens in the async Phase 1 block after
    // loading data from the projection. The async block returns ValidationFailed
    // when the loaded data doesn't meet requirements.

    #[tokio::test]
    async fn test_refund_payment_not_found() {
        // Empty projection - no payment exists
        let env = create_test_env_with_projection(Arc::new(ConfigurableMockPaymentQuery::new()));
        let store = TestStore::new(PaymentReducer::new(), env, PaymentState::new());

        let payment_id = PaymentId::new();

        // Try to refund a payment that doesn't exist
        let result = store
            .send_and_wait_for(
                PaymentAction::RefundPayment {
                    payment_id,
                    amount: Money::from_dollars(50),
                    reason: "Customer requested refund".to_string(),
                },
                |action| {
                    matches!(
                        action,
                        PaymentAction::ExecuteRefundPayment { .. }
                            | PaymentAction::ValidationFailed { .. }
                    )
                },
                Duration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive ValidationFailed");
        let action = result.unwrap();

        // Should get ValidationFailed because payment wasn't found
        match action {
            PaymentAction::ValidationFailed { error } => {
                assert!(
                    error.contains("not found"),
                    "Error should mention 'not found': {error}"
                );
            }
            other => panic!("Expected ValidationFailed, got {other:?}"),
        }

        // Clean up
        store.clear_queue();
    }

    #[tokio::test]
    async fn test_refund_uncaptured_payment() {
        let payment_id = PaymentId::new();

        // Create a pending (not captured) payment
        let pending_payment = Payment::new(
            payment_id,
            ReservationId::new(),
            CustomerId::new(),
            Money::from_dollars(100),
            PaymentMethod::CreditCard {
                last_four: "4242".to_string(),
            },
        );
        // Note: default status is Pending, not Captured

        let mock_projection = ConfigurableMockPaymentQuery::new().with_payment(pending_payment);
        let env = create_test_env_with_projection(Arc::new(mock_projection));
        let store = TestStore::new(PaymentReducer::new(), env, PaymentState::new());

        // Try to refund an uncaptured payment
        let result = store
            .send_and_wait_for(
                PaymentAction::RefundPayment {
                    payment_id,
                    amount: Money::from_dollars(50),
                    reason: "Customer requested refund".to_string(),
                },
                |action| {
                    matches!(
                        action,
                        PaymentAction::ExecuteRefundPayment { .. }
                            | PaymentAction::ValidationFailed { .. }
                    )
                },
                Duration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive ValidationFailed");
        let action = result.unwrap();

        // Should get ValidationFailed because payment is not captured
        match action {
            PaymentAction::ValidationFailed { error } => {
                assert!(
                    error.contains("Cannot refund") || error.contains("uncaptured"),
                    "Error should mention cannot refund uncaptured: {error}"
                );
            }
            other => panic!("Expected ValidationFailed, got {other:?}"),
        }

        // Clean up
        store.clear_queue();
    }

    #[tokio::test]
    async fn test_refund_exceeds_payment_amount() {
        let payment_id = PaymentId::new();

        // Create a captured payment for $100
        let mut captured_payment = Payment::new(
            payment_id,
            ReservationId::new(),
            CustomerId::new(),
            Money::from_dollars(100),
            PaymentMethod::CreditCard {
                last_four: "4242".to_string(),
            },
        );
        captured_payment.status = PaymentStatus::Captured;

        let mock_projection = ConfigurableMockPaymentQuery::new().with_payment(captured_payment);
        let env = create_test_env_with_projection(Arc::new(mock_projection));
        let store = TestStore::new(PaymentReducer::new(), env, PaymentState::new());

        // Try to refund more than the payment amount
        let result = store
            .send_and_wait_for(
                PaymentAction::RefundPayment {
                    payment_id,
                    amount: Money::from_dollars(150), // More than $100!
                    reason: "Customer requested refund".to_string(),
                },
                |action| {
                    matches!(
                        action,
                        PaymentAction::ExecuteRefundPayment { .. }
                            | PaymentAction::ValidationFailed { .. }
                    )
                },
                Duration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive ValidationFailed");
        let action = result.unwrap();

        // Should get ValidationFailed because refund exceeds payment
        match action {
            PaymentAction::ValidationFailed { error } => {
                assert!(
                    error.contains("exceeds"),
                    "Error should mention 'exceeds': {error}"
                );
            }
            other => panic!("Expected ValidationFailed, got {other:?}"),
        }

        // Clean up
        store.clear_queue();
    }

    // -------------------------------------------------------------------------
    // Query Operations
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_get_payment() {
        let payment_id = PaymentId::new();
        let customer_id = CustomerId::new();

        // Create a payment in the projection
        let payment = Payment::new(
            payment_id,
            ReservationId::new(),
            customer_id,
            Money::from_dollars(75),
            PaymentMethod::CreditCard {
                last_four: "9999".to_string(),
            },
        );

        let mock_projection = ConfigurableMockPaymentQuery::new().with_payment(payment.clone());
        let env = create_test_env_with_projection(Arc::new(mock_projection));
        let store = TestStore::new(PaymentReducer::new(), env, PaymentState::new());

        // Query the payment
        let result = store
            .send_and_wait_for(
                PaymentAction::GetPayment { payment_id },
                |action| {
                    matches!(
                        action,
                        PaymentAction::PaymentQueried { .. }
                            | PaymentAction::ValidationFailed { .. }
                    )
                },
                Duration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive PaymentQueried");
        let action = result.unwrap();

        match action {
            PaymentAction::PaymentQueried {
                payment_id: queried_id,
                payment: queried_payment,
            } => {
                assert_eq!(queried_id, payment_id);
                assert!(queried_payment.is_some());
                let p = queried_payment.unwrap();
                assert_eq!(p.amount, Money::from_dollars(75));
            }
            other => panic!("Expected PaymentQueried, got {other:?}"),
        }

        // Clean up
        store.clear_queue();
    }

    #[tokio::test]
    async fn test_get_payment_not_found() {
        let payment_id = PaymentId::new();

        // Empty projection - no payments
        let env = create_test_env_with_projection(Arc::new(ConfigurableMockPaymentQuery::new()));
        let store = TestStore::new(PaymentReducer::new(), env, PaymentState::new());

        // Query a non-existent payment
        let result = store
            .send_and_wait_for(
                PaymentAction::GetPayment { payment_id },
                |action| {
                    matches!(
                        action,
                        PaymentAction::PaymentQueried { .. }
                            | PaymentAction::ValidationFailed { .. }
                    )
                },
                Duration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive PaymentQueried");
        let action = result.unwrap();

        match action {
            PaymentAction::PaymentQueried {
                payment_id: queried_id,
                payment: queried_payment,
            } => {
                assert_eq!(queried_id, payment_id);
                assert!(queried_payment.is_none(), "Payment should not exist");
            }
            other => panic!("Expected PaymentQueried, got {other:?}"),
        }

        // Clean up
        store.clear_queue();
    }

    #[tokio::test]
    async fn test_list_customer_payments() {
        let customer_id = CustomerId::new();

        // Create multiple payments for the same customer
        let payment1 = Payment::new(
            PaymentId::new(),
            ReservationId::new(),
            customer_id,
            Money::from_dollars(100),
            PaymentMethod::CreditCard {
                last_four: "1111".to_string(),
            },
        );
        let payment2 = Payment::new(
            PaymentId::new(),
            ReservationId::new(),
            customer_id,
            Money::from_dollars(200),
            PaymentMethod::CreditCard {
                last_four: "2222".to_string(),
            },
        );
        // Different customer - should not be returned
        let other_customer_payment = Payment::new(
            PaymentId::new(),
            ReservationId::new(),
            CustomerId::new(), // Different customer!
            Money::from_dollars(300),
            PaymentMethod::CreditCard {
                last_four: "3333".to_string(),
            },
        );

        let mock_projection = ConfigurableMockPaymentQuery::new()
            .with_payment(payment1)
            .with_payment(payment2)
            .with_payment(other_customer_payment);
        let env = create_test_env_with_projection(Arc::new(mock_projection));
        let store = TestStore::new(PaymentReducer::new(), env, PaymentState::new());

        // List customer payments
        let result = store
            .send_and_wait_for(
                PaymentAction::ListCustomerPayments {
                    customer_id,
                    limit: 10,
                    offset: 0,
                },
                |action| {
                    matches!(
                        action,
                        PaymentAction::CustomerPaymentsListed { .. }
                            | PaymentAction::ValidationFailed { .. }
                    )
                },
                Duration::from_secs(5),
            )
            .await;

        assert!(result.is_ok(), "Should receive CustomerPaymentsListed");
        let action = result.unwrap();

        match action {
            PaymentAction::CustomerPaymentsListed {
                customer_id: listed_customer,
                payments,
            } => {
                assert_eq!(listed_customer, customer_id);
                assert_eq!(payments.len(), 2, "Should have 2 payments for this customer");
                // All payments should belong to the requested customer
                assert!(payments.iter().all(|p| p.customer_id == customer_id));
            }
            other => panic!("Expected CustomerPaymentsListed, got {other:?}"),
        }

        // Clean up
        store.clear_queue();
    }
}
