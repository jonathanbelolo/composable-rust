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

use crate::projections::{CorrelationId, TicketingEvent};
use crate::types::{
    CustomerId, EventId, Money, Reservation, ReservationExpiry, ReservationId, ReservationState,
    ReservationStatus, SeatId, SeatNumber, TicketId,
};
use chrono::{DateTime, Duration, Utc};
use composable_rust_core::{
    append_events, delay, effect::Effect, environment::Clock, event_bus::EventBus,
    event_store::EventStore, publish_event, reducer::Reducer, smallvec, stream::StreamId,
    SmallVec,
};
use composable_rust_macros::Action;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::inventory::InventoryAction;
use super::payment::PaymentAction;
use crate::types::PaymentId;

// ============================================================================
// Projection Query Trait
// ============================================================================

/// Trait for querying reservation projection data.
///
/// This trait defines the read operations needed by the Reservation saga
/// to load state from the projection when processing commands.
///
/// # Pattern: State Loading from Projections
///
/// According to the state-loading-patterns spec, aggregates load state on-demand
/// by querying projections. This trait is injected via the Environment to enable
/// the reducer to trigger state loading effects.
///
/// Note: Returns `BoxFuture` instead of async fn to be dyn-compatible (object-safe).
pub trait ReservationProjectionQuery: Send + Sync {
    /// Load reservation data for a specific reservation.
    ///
    /// Returns reservation details if found.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn load_reservation(
        &self,
        reservation_id: &ReservationId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Reservation>, String>> + Send + '_>>;

    /// List all reservations for a specific customer.
    ///
    /// Returns all reservations (across all states) for the given customer.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn list_by_customer(
        &self,
        customer_id: &CustomerId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Reservation>, String>> + Send + '_>>;
}

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
        /// Optional correlation ID for request tracking
        #[serde(skip_serializing_if = "Option::is_none")]
        correlation_id: Option<CorrelationId>,
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

    /// Query a single reservation by ID
    #[command]
    GetReservation {
        /// Reservation ID to query
        reservation_id: ReservationId,
    },

    /// List all reservations for a customer
    #[command]
    ListReservations {
        /// Customer ID to query reservations for
        customer_id: CustomerId,
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

    /// Reservation was queried (query result)
    #[event]
    ReservationQueried {
        /// Reservation ID that was queried
        reservation_id: ReservationId,
        /// Reservation data (None if not found)
        reservation: Option<Reservation>,
    },

    /// Reservations were listed (query result)
    #[event]
    ReservationsListed {
        /// Customer ID that was queried
        customer_id: CustomerId,
        /// List of reservations for this customer
        reservations: Vec<Reservation>,
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
///
/// Contains ONLY side effect dependencies. Child stores are held in `ReservationState`.
#[derive(Clone)]
pub struct ReservationEnvironment {
    // ===== Side Effect Dependencies ONLY =====
    /// Clock for timestamps and timeout calculation
    pub clock: Arc<dyn Clock>,
    /// Event store for persistence of reservation events
    pub event_store: Arc<dyn EventStore>,
    /// Event bus for publishing reservation events
    pub event_bus: Arc<dyn EventBus>,
    /// Stream ID for this aggregate instance
    pub stream_id: StreamId,
    /// Projection query for loading state on-demand
    pub projection: Arc<dyn ReservationProjectionQuery>,
}

impl ReservationEnvironment {
    /// Creates a new `ReservationEnvironment`
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        event_store: Arc<dyn EventStore>,
        event_bus: Arc<dyn EventBus>,
        stream_id: StreamId,
        projection: Arc<dyn ReservationProjectionQuery>,
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

    /// Creates effects for persisting and publishing an event
    ///
    /// # Arguments
    ///
    /// - `event`: The event to persist and publish
    /// - `env`: Environment for event store and bus
    /// - `correlation_id`: Optional correlation ID for request tracking
    fn create_effects(
        event: ReservationAction,
        env: &ReservationEnvironment,
        correlation_id: Option<CorrelationId>,
    ) -> SmallVec<[Effect<ReservationAction>; 4]> {
        let ticketing_event = TicketingEvent::Reservation(event);
        let Ok(mut serialized) = ticketing_event.serialize() else {
            return SmallVec::new();
        };

        // Add correlation_id to metadata if present
        if let Some(cid) = correlation_id {
            let metadata = serialized.metadata.get_or_insert_with(composable_rust_core::event::EventMetadata::new);
            metadata.correlation_id = Some(cid.to_string());
        }

        smallvec![
            append_events! {
                store: env.event_store,
                stream: env.stream_id.as_str(),
                expected_version: None,
                events: vec![serialized.clone()],
                on_success: |_version| None,
                on_error: |error| Some(ReservationAction::ValidationFailed {
                    error: error.to_string()
                })
            },
            publish_event! {
                bus: env.event_bus,
                topic: "reservation",
                event: serialized,
                on_success: || None,
                on_error: |error| Some(ReservationAction::ValidationFailed {
                    error: error.to_string()
                })
            }
        ]
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

            // Commands and queries don't modify state
            // Response events also don't modify state (they're for API handlers)
            ReservationAction::InitiateReservation { .. }
            | ReservationAction::CompletePayment { .. }
            | ReservationAction::CancelReservation { .. }
            | ReservationAction::ExpireReservation { .. }
            | ReservationAction::GetReservation { .. }
            | ReservationAction::ListReservations { .. }
            | ReservationAction::ReservationQueried { .. }
            | ReservationAction::ReservationsListed { .. } => {}
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
                correlation_id,
            } => {
                tracing::info!(
                    reservation_id = %reservation_id.as_uuid(),
                    event_id = %event_id.as_uuid(),
                    "Processing InitiateReservation command"
                );
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

                // Persist and publish our event with correlation_id
                let mut effects = Self::create_effects(event, env, correlation_id);

                // TCA Pattern: Publish command to event bus for Inventory child aggregate
                // The Inventory aggregate subscribes to its topic and will process this command
                let reserve_seats_cmd = InventoryAction::ReserveSeats {
                    reservation_id,
                    event_id,
                    section,
                    quantity,
                    specific_seats,
                    expires_at,
                };
                let ticketing_event = crate::projections::TicketingEvent::Inventory(reserve_seats_cmd);
                match ticketing_event.serialize() {
                    Ok(mut serialized) => {
                        // Propagate correlation_id to cross-aggregate event for projection tracking
                        if let Some(cid) = correlation_id {
                            let metadata = serialized.metadata.get_or_insert_with(composable_rust_core::event::EventMetadata::new);
                            metadata.correlation_id = Some(cid.to_string());
                        }

                        effects.push(publish_event! {
                            bus: env.event_bus,
                            topic: "inventory",
                            event: serialized,
                            on_success: || None,
                            on_error: |error| Some(ReservationAction::ValidationFailed {
                                error: error.to_string()
                            })
                        });
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to serialize ReserveSeats command");
                        // Emit ValidationFailed event to indicate serialization failure
                        let failed_event = ReservationAction::ValidationFailed {
                            error: format!("Failed to serialize ReserveSeats command: {e}")
                        };
                        Self::apply_event(state, &failed_event);
                        return Self::create_effects(failed_event, env, correlation_id);
                    }
                }

                // Schedule expiration timeout (5 minutes)
                effects.push(delay! {
                    duration: std::time::Duration::from_secs(5 * 60),
                    action: ReservationAction::ExpireReservation { reservation_id }
                });

                tracing::info!(
                    reservation_id = %reservation_id.as_uuid(),
                    effects_count = effects.len(),
                    "Returning effects from InitiateReservation"
                );
                effects
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

                // Persist and publish our events
                let mut effects = Self::create_effects(action, env, None);
                effects.extend(Self::create_effects(payment_requested, env, None));

                // TCA Pattern: Publish command to event bus for Payment child aggregate
                let process_payment = PaymentAction::ProcessPayment {
                    payment_id,
                    reservation_id,
                    amount: total_amount,
                    payment_method: crate::types::PaymentMethod::CreditCard {
                        last_four: "4242".to_string(),
                    },
                };
                let ticketing_event = crate::projections::TicketingEvent::Payment(process_payment);
                if let Ok(serialized) = ticketing_event.serialize() {
                    effects.push(publish_event! {
                        bus: env.event_bus,
                        topic: "payment",
                        event: serialized,
                        on_success: || None,
                        on_error: |error| Some(ReservationAction::ValidationFailed {
                            error: error.to_string()
                        })
                    });
                }

                effects
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

                // Persist and publish our events
                let mut effects = Self::create_effects(action, env, None);
                effects.extend(Self::create_effects(completion, env, None));

                // TCA Pattern: Publish confirm command to event bus for Inventory
                let confirm_seats = InventoryAction::ConfirmReservation {
                    reservation_id,
                    customer_id,
                };
                let ticketing_event = crate::projections::TicketingEvent::Inventory(confirm_seats);
                if let Ok(serialized) = ticketing_event.serialize() {
                    effects.push(publish_event! {
                        bus: env.event_bus,
                        topic: "inventory",
                        event: serialized,
                        on_success: || None,
                        on_error: |error| Some(ReservationAction::ValidationFailed {
                            error: error.to_string()
                        })
                    });
                }

                effects
            }

            // ========== Step 3b: Payment Failed (COMPENSATION) ==========
            ReservationAction::PaymentFailed {
                reservation_id,
                ref reason,
                payment_id: _,
            } => {
                // Apply event
                Self::apply_event(state, &action);

                let compensation = ReservationAction::ReservationCompensated {
                    reservation_id,
                    reason: reason.clone(),
                    compensated_at: env.clock.now(),
                };
                Self::apply_event(state, &compensation);

                // Persist and publish our events
                let mut effects = Self::create_effects(action, env, None);
                effects.extend(Self::create_effects(compensation, env, None));

                // TCA Pattern: Publish release command to event bus (compensation)
                let release_seats = InventoryAction::ReleaseReservation { reservation_id };
                let ticketing_event = crate::projections::TicketingEvent::Inventory(release_seats);
                if let Ok(serialized) = ticketing_event.serialize() {
                    effects.push(publish_event! {
                        bus: env.event_bus,
                        topic: "inventory",
                        event: serialized,
                        on_success: || None,
                        on_error: |error| Some(ReservationAction::ValidationFailed {
                            error: error.to_string()
                        })
                    });
                }

                effects
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

                        // Persist and publish expiration event
                        let mut effects = Self::create_effects(expiration, env, None);

                        // TCA Pattern: Publish release command to event bus (compensation)
                        let release_seats =
                            InventoryAction::ReleaseReservation { reservation_id };
                        let ticketing_event = crate::projections::TicketingEvent::Inventory(release_seats);
                        if let Ok(serialized) = ticketing_event.serialize() {
                            effects.push(publish_event! {
                                bus: env.event_bus,
                                topic: "inventory",
                                event: serialized,
                                on_success: || None,
                                on_error: |error| Some(ReservationAction::ValidationFailed {
                                    error: error.to_string()
                                })
                            });
                        }

                        return effects;
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

                        // Persist and publish cancellation event
                        let mut effects = Self::create_effects(cancellation, env, None);

                        // TCA Pattern: Publish release command to event bus (compensation)
                        let release_seats =
                            InventoryAction::ReleaseReservation { reservation_id };
                        let ticketing_event = crate::projections::TicketingEvent::Inventory(release_seats);
                        if let Ok(serialized) = ticketing_event.serialize() {
                            effects.push(publish_event! {
                                bus: env.event_bus,
                                topic: "inventory",
                                event: serialized,
                                on_success: || None,
                                on_error: |error| Some(ReservationAction::ValidationFailed {
                                    error: error.to_string()
                                })
                            });
                        }

                        return effects;
                    }
                }

                SmallVec::new()
            }

            // ========== Query: Get Reservation ==========
            ReservationAction::GetReservation { reservation_id } => {
                // Use projection to load reservation data
                let projection = env.projection.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    match projection.load_reservation(&reservation_id).await {
                        Ok(reservation) => Some(ReservationAction::ReservationQueried {
                            reservation_id,
                            reservation,
                        }),
                        Err(error) => Some(ReservationAction::ValidationFailed { error }),
                    }
                }))]
            }

            // ========== Query: List Reservations ==========
            ReservationAction::ListReservations { customer_id } => {
                // Use projection to load all reservations for customer
                let projection = env.projection.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    match projection.list_by_customer(&customer_id).await {
                        Ok(reservations) => Some(ReservationAction::ReservationsListed {
                            customer_id,
                            reservations,
                        }),
                        Err(error) => Some(ReservationAction::ValidationFailed { error }),
                    }
                }))]
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
    use super::*;  // Brings in ReservationEnvironment, ReservationState, etc.
    use std::sync::Arc;
    use composable_rust_core::environment::SystemClock;
    use composable_rust_core::stream::StreamId;
    use composable_rust_testing::{assertions, mocks::{InMemoryEventBus, InMemoryEventStore}, ReducerTest};
    use crate::types::{CustomerId, EventId, Money, Reservation, ReservationExpiry, ReservationId};

    // Mock projection queries for tests
    #[derive(Clone)]
    struct MockInventoryQuery;

    impl crate::aggregates::inventory::InventoryProjectionQuery for MockInventoryQuery {
        fn load_inventory(
            &self,
            _event_id: &EventId,
            _section: &str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<((u32, u32, u32, u32), Vec<crate::types::SeatAssignment>)>, String>> + Send + '_>> {
            Box::pin(async move { Ok(None) })
        }
    }

    #[derive(Clone)]
    struct MockPaymentQuery;

    impl crate::aggregates::payment::PaymentProjectionQuery for MockPaymentQuery {
        fn load_payment(
            &self,
            _payment_id: &crate::types::PaymentId,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<crate::types::Payment>, String>> + Send + '_>> {
            Box::pin(async move { Ok(None) })
        }
    }

    #[derive(Clone)]
    struct MockReservationQuery;

    impl ReservationProjectionQuery for MockReservationQuery {
        fn load_reservation(
            &self,
            _reservation_id: &ReservationId,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Reservation>, String>> + Send + '_>> {
            Box::pin(async move { Ok(None) })
        }

        fn list_by_customer(
            &self,
            _customer_id: &CustomerId,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Reservation>, String>> + Send + '_>> {
            Box::pin(async move { Ok(Vec::new()) })
        }
    }

    fn create_test_env_and_state() -> (
        ReservationEnvironment,
        ReservationState,
    ) {
        // TCA pattern: Parent state holds child STATE, not child stores
        let env = ReservationEnvironment::new(
            Arc::new(SystemClock),
            Arc::new(InMemoryEventStore::new()),
            Arc::new(InMemoryEventBus::new()),
            StreamId::new("reservation-test"),
            Arc::new(MockReservationQuery),
        );

        let state = ReservationState::new();

        (env, state)
    }

    #[test]
    fn test_initiate_reservation() {
        let reservation_id = ReservationId::new();
        let event_id = EventId::new();
        let customer_id = CustomerId::new();

        ReducerTest::new(ReservationReducer::new())
            .with_env({
                let (env, _) = create_test_env_and_state();
                env
            })
            .given_state({
                let (_, state) = create_test_env_and_state();
                state
            })
            .when_action(ReservationAction::InitiateReservation {
                reservation_id,
                event_id,
                customer_id,
                section: "General".to_string(),
                quantity: 2,
                specific_seats: None,
                correlation_id: None,
            })
            .then_state(move |state| {
                assert_eq!(state.count(), 1);
                assert!(state.exists(&reservation_id));
                let reservation = state.get(&reservation_id).unwrap();
                assert_eq!(reservation.status, ReservationStatus::Initiated);
                assert_eq!(reservation.seats.len(), 0); // Not yet allocated
            })
            .then_effects(|effects| {
                // Should return 4 effects:
                // 2 for ReservationInitiated (AppendEvents + PublishEvent)
                // 1 for publishing ReserveSeats command to inventory topic
                // 1 for scheduling expiration timeout (Delay)
                assert_eq!(effects.len(), 4);
            })
            .run();
    }

    #[test]
    fn test_seats_allocated() {
        let reservation_id = ReservationId::new();

        ReducerTest::new(ReservationReducer::new())
            .with_env({
                let (env, _) = create_test_env_and_state();
                env
            })
            .given_state({
                let (_, mut state) = create_test_env_and_state();
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
            .with_env({
                let (env, _) = create_test_env_and_state();
                env
            })
            .given_state({
                let (_, mut state) = create_test_env_and_state();
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
            .with_env({
                let (env, _) = create_test_env_and_state();
                env
            })
            .given_state({
                let (_, mut state) = create_test_env_and_state();
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
            .with_env({
                let (env, _) = create_test_env_and_state();
                env
            })
            .given_state({
                let (_, mut state) = create_test_env_and_state();
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
            .with_env({
                let (env, _) = create_test_env_and_state();
                env
            })
            .given_state({
                let (_, mut state) = create_test_env_and_state();
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
