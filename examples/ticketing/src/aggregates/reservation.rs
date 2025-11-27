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
    CustomerId, EventId, GlobalActionChannels, Money, Reservation, ReservationExpiry, ReservationId, ReservationState,
    ReservationStatus, ResponseChannel, SeatId, SeatNumber, TicketId,
};
use chrono::{DateTime, Duration, Utc};
use composable_rust_core::{
    append_events, delay, effect::Effect, environment::Clock,
    event_store::EventStore, reducer::Reducer, smallvec,
    stream::{StreamId, Version},
    SmallVec,
};
use composable_rust_macros::Action;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::event::EventProjectionQuery;
use super::inventory::{InventoryAction, InventoryProjectionQuery};
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

        /// Response channel for projection completion
        #[serde(skip)]
        respond_to: ResponseChannel,
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

    /// Projection update confirmed
    ReservationProjectionConfirmed {
        /// Reservation ID
        reservation_id: ReservationId,
    },

    /// Projection update failed
    ReservationProjectionFailed {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Failure reason
        reason: String,
    },

    /// Stream version was updated after successful event append
    #[event]
    VersionUpdated {
        /// New version number
        version: Version,
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
    /// Stream ID for this aggregate instance
    pub stream_id: StreamId,
    /// Projection query for loading state on-demand
    pub projection: Arc<dyn ReservationProjectionQuery>,
    /// Global action channels for cross-aggregate coordination
    pub global_actions: GlobalActionChannels,
    // ===== Dependencies for Saga Orchestration =====
    /// Inventory projection query for creating inventory stores in saga
    pub inventory_query: Arc<dyn InventoryProjectionQuery>,
    /// Event projection query for pricing lookup in inventory stores
    pub event_query: Arc<dyn EventProjectionQuery>,
}

impl ReservationEnvironment {
    /// Creates a new `ReservationEnvironment`
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        clock: Arc<dyn Clock>,
        event_store: Arc<dyn EventStore>,
        stream_id: StreamId,
        projection: Arc<dyn ReservationProjectionQuery>,
        global_actions: GlobalActionChannels,
        inventory_query: Arc<dyn InventoryProjectionQuery>,
        event_query: Arc<dyn EventProjectionQuery>,
    ) -> Self {
        Self {
            clock,
            event_store,
            stream_id,
            projection,
            global_actions,
            inventory_query,
            event_query,
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

    /// Creates effects for persisting events (PostgreSQL only, no Redpanda)
    ///
    /// With direct orchestration, we use local channels for coordination,
    /// so Redpanda publishing is no longer needed.
    ///
    /// # Arguments
    ///
    /// - `event`: The event to persist
    /// - `expected_version`: Expected version for optimistic concurrency control
    /// - `env`: Environment for event store
    /// - `correlation_id`: Optional correlation ID for request tracking
    fn create_effects(
        event: ReservationAction,
        expected_version: Version,
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
                expected_version: Some(expected_version),
                events: vec![serialized.clone()],
                on_success: |version| Some(ReservationAction::VersionUpdated { version }),
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

            ReservationAction::VersionUpdated { version } => {
                state.version = *version;
            }

            ReservationAction::ValidationFailed { error } => {
                state.last_error = Some(error.clone());
            }

            // Commands and queries don't modify state
            // Response events also don't modify state (they're for API handlers)
            // Projection confirmation actions are logged but don't modify aggregate state
            ReservationAction::InitiateReservation { .. }
            | ReservationAction::CompletePayment { .. }
            | ReservationAction::CancelReservation { .. }
            | ReservationAction::ExpireReservation { .. }
            | ReservationAction::GetReservation { .. }
            | ReservationAction::ListReservations { .. }
            | ReservationAction::ReservationQueried { .. }
            | ReservationAction::ReservationsListed { .. }
            | ReservationAction::ReservationProjectionConfirmed { .. }
            | ReservationAction::ReservationProjectionFailed { .. } => {}
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
                respond_to: _,  // Explicitly ignore - infrastructure handles this
            } => {
                let _span = tracing::info_span!(
                    "saga.reservation.initiate",
                    reservation_id = %reservation_id.as_uuid(),
                    event_id = %event_id.as_uuid(),
                    step = "1_initiate"
                ).entered();

                tracing::info!(
                    reservation_id = %reservation_id.as_uuid(),
                    event_id = %event_id.as_uuid(),
                    "Processing InitiateReservation command"
                );

                // Clone fields for publishing to global channel
                let reservation_id_clone = reservation_id;
                let event_id_clone = event_id;
                let customer_id_clone = customer_id;
                let section_clone = section.clone();
                let quantity_clone = quantity;
                let specific_seats_clone = specific_seats.clone();
                let correlation_id_clone = correlation_id;

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
                let expected_version = state.version;
                Self::apply_event(state, &event);

                // Persist and publish our event with correlation_id
                let mut effects = Self::create_effects(event, expected_version, env, correlation_id);

                // Direct orchestration: Create inventory store and dispatch ReserveSeats
                // Then return SeatsAllocated action for saga feedback loop
                let clock_clone = env.clock.clone();
                let event_store_clone = env.event_store.clone();
                let inventory_query_clone = env.inventory_query.clone();
                let event_query_clone = env.event_query.clone();
                let global_actions_clone = env.global_actions.clone();
                let section_for_inventory = section.clone();

                effects.push(Effect::Future(Box::pin(async move {
                    use crate::aggregates::inventory::{InventoryEnvironment, InventoryReducer};
                    use crate::types::InventoryState;
                    use composable_rust_core::stream::StreamId;
                    use composable_rust_runtime::Store;
                    use std::time::Duration;

                    // Create inventory store for this event
                    let stream_id = StreamId::new(&format!("inventory-{}", event_id.as_uuid()));
                    let inv_env = InventoryEnvironment::new(
                        clock_clone,
                        event_store_clone,
                        stream_id,
                        inventory_query_clone,
                        event_query_clone,
                        global_actions_clone.clone(),
                    );
                    let inventory_store = Store::new(
                        InventoryState::new(),
                        InventoryReducer::new(),
                        inv_env,
                    );

                    // Send ReserveSeats action to inventory
                    let reserve_action = InventoryAction::ReserveSeats {
                        reservation_id,
                        event_id,
                        section: section_for_inventory.clone(),
                        quantity,
                        specific_seats,
                        expires_at,
                    };

                    // Use send_and_wait_for to wait for SeatsReserved or ValidationFailed
                    let result = inventory_store.send_and_wait_for(
                        reserve_action,
                        |action| {
                            matches!(
                                action,
                                InventoryAction::SeatsReserved { reservation_id: rid, .. } if *rid == reservation_id
                            ) || matches!(action, InventoryAction::ValidationFailed { .. })
                        },
                        Duration::from_secs(30),
                    ).await;

                    match result {
                        Ok(InventoryAction::SeatsReserved { seats, .. }) => {
                            // Calculate total amount (simplified - $50 per seat)
                            #[allow(clippy::cast_possible_truncation)]
                            let total_amount = Money::from_dollars(50).multiply(quantity);

                            tracing::info!(
                                reservation_id = %reservation_id.as_uuid(),
                                seat_count = seats.len(),
                                total_amount_cents = total_amount.cents(),
                                "Inventory reservation succeeded, returning SeatsAllocated"
                            );

                            Some(ReservationAction::SeatsAllocated {
                                reservation_id,
                                seats,
                                total_amount,
                            })
                        }
                        Ok(InventoryAction::ValidationFailed { error }) => {
                            tracing::warn!(
                                reservation_id = %reservation_id.as_uuid(),
                                error = %error,
                                "Inventory reservation failed"
                            );
                            Some(ReservationAction::ValidationFailed {
                                error: format!("Inventory: {error}"),
                            })
                        }
                        Ok(other) => {
                            tracing::error!(
                                reservation_id = %reservation_id.as_uuid(),
                                action = ?other,
                                "Unexpected action received from inventory store"
                            );
                            Some(ReservationAction::ValidationFailed {
                                error: "Unexpected inventory response".to_string(),
                            })
                        }
                        Err(e) => {
                            tracing::error!(
                                reservation_id = %reservation_id.as_uuid(),
                                error = %e,
                                "Inventory store error"
                            );
                            Some(ReservationAction::ValidationFailed {
                                error: format!("Inventory store error: {e}"),
                            })
                        }
                    }
                })));

                // Schedule expiration timeout (5 minutes)
                effects.push(delay! {
                    duration: std::time::Duration::from_secs(5 * 60),
                    action: ReservationAction::ExpireReservation { reservation_id }
                });

                // Publish to global channel for projections and wait for completion
                let reservation_id_for_success = reservation_id;
                let reservation_id_for_error = reservation_id;
                effects.push(Effect::PublishWithResponse {
                    channel: env.global_actions.reservation_actions.clone(),
                    create_action: Box::new(move |respond_to| {
                        ReservationAction::InitiateReservation {
                            reservation_id: reservation_id_clone,
                            event_id: event_id_clone,
                            customer_id: customer_id_clone,
                            section: section_clone,
                            quantity: quantity_clone,
                            specific_seats: specific_seats_clone,
                            correlation_id: correlation_id_clone,
                            respond_to,
                        }
                    }),
                    on_success: Box::new(move || {
                        Some(ReservationAction::ReservationProjectionConfirmed {
                            reservation_id: reservation_id_for_success,
                        })
                    }),
                    on_error: Box::new(move |reason| {
                        Some(ReservationAction::ReservationProjectionFailed {
                            reservation_id: reservation_id_for_error,
                            reason,
                        })
                    }),
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
                let _span = tracing::info_span!(
                    "saga.reservation.seats_allocated",
                    reservation_id = %reservation_id.as_uuid(),
                    seat_count = seats.len(),
                    step = "2_seats_allocated"
                ).entered();

                // Apply event
                let expected_version = state.version;
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
                let expected_version_2 = state.version;
                Self::apply_event(state, &payment_requested);

                // Persist and publish our events
                let mut effects = Self::create_effects(action, expected_version, env, None);
                effects.extend(Self::create_effects(payment_requested, expected_version_2, env, None));

                // Get customer_id from the reservation state
                let customer_id = state.reservations
                    .get(&reservation_id)
                    .map(|r| r.customer_id)
                    .unwrap_or_else(CustomerId::new);

                // Direct orchestration: Send command to Payment via global channel
                let process_payment = PaymentAction::ProcessPayment {
                    payment_id,
                    reservation_id,
                    customer_id,
                    amount: total_amount,
                    payment_method: crate::types::PaymentMethod::CreditCard {
                        last_four: "4242".to_string(),
                    },
                    respond_to: ResponseChannel::none(),
                };
                let payment_channel = env.global_actions.payment_actions.clone();
                effects.push(Effect::Future(Box::pin(async move {
                    if let Err(e) = payment_channel.send(process_payment) {
                        tracing::error!(error = %e, "Failed to send ProcessPayment command to payment channel");
                    }
                    None
                })));

                // Broadcast PaymentRequested to reservation_actions channel for projection
                let reservation_channel = env.global_actions.reservation_actions.clone();
                let payment_requested_for_projection = ReservationAction::PaymentRequested {
                    reservation_id,
                    payment_id,
                    amount: total,
                };
                effects.push(Effect::Future(Box::pin(async move {
                    if let Err(e) = reservation_channel.send(payment_requested_for_projection.clone()) {
                        tracing::error!(error = %e, "Failed to broadcast PaymentRequested to reservation channel");
                    }
                    // Also return the action for the saga to observe
                    Some(payment_requested_for_projection)
                })));

                effects
            }

            // ========== Step 3a: Payment Succeeded ==========
            ReservationAction::PaymentSucceeded {
                reservation_id,
                payment_id: _,
            } => {
                let _span = tracing::info_span!(
                    "saga.reservation.payment_succeeded",
                    reservation_id = %reservation_id.as_uuid(),
                    step = "3a_payment_succeeded"
                ).entered();

                // Apply event
                let expected_version = state.version;
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
                let expected_version_2 = state.version;
                Self::apply_event(state, &completion);

                // Persist and publish our events
                let mut effects = Self::create_effects(action, expected_version, env, None);
                effects.extend(Self::create_effects(completion.clone(), expected_version_2, env, None));

                // Emit as observable action for send_and_wait_for
                let completion_clone = completion;
                effects.push(Effect::Future(Box::pin(async move {
                    Some(completion_clone)
                })));

                // Direct orchestration: Send confirm command to Inventory via global channel
                let confirm_seats = InventoryAction::ConfirmReservation {
                    reservation_id,
                    customer_id,
                };
                let inventory_channel = env.global_actions.inventory_actions.clone();
                effects.push(Effect::Future(Box::pin(async move {
                    if let Err(e) = inventory_channel.send(confirm_seats) {
                        tracing::error!(error = %e, "Failed to send ConfirmReservation command to inventory channel");
                    }
                    None
                })));

                effects
            }

            // ========== Step 3b: Payment Failed (COMPENSATION) ==========
            ReservationAction::PaymentFailed {
                reservation_id,
                ref reason,
                payment_id: _,
            } => {
                let _span = tracing::info_span!(
                    "saga.reservation.compensation",
                    reservation_id = %reservation_id.as_uuid(),
                    reason = %reason,
                    step = "3b_compensation"
                ).entered();

                tracing::warn!(
                    reservation_id = %reservation_id.as_uuid(),
                    reason = %reason,
                    "Payment failed, triggering saga compensation"
                );

                // Apply event
                let expected_version = state.version;
                Self::apply_event(state, &action);

                let compensation = ReservationAction::ReservationCompensated {
                    reservation_id,
                    reason: reason.clone(),
                    compensated_at: env.clock.now(),
                };
                let expected_version_2 = state.version;
                Self::apply_event(state, &compensation);

                // Persist and publish our events
                let mut effects = Self::create_effects(action, expected_version, env, None);
                effects.extend(Self::create_effects(compensation, expected_version_2, env, None));

                // Direct orchestration: Send release command to Inventory (compensation)
                let release_seats = InventoryAction::ReleaseReservation { reservation_id };
                let inventory_channel = env.global_actions.inventory_actions.clone();
                effects.push(Effect::Future(Box::pin(async move {
                    if let Err(e) = inventory_channel.send(release_seats) {
                        tracing::error!(error = %e, "Failed to send ReleaseReservation command to inventory channel");
                    }
                    None
                })));

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
                        let expected_version = state.version;
                        Self::apply_event(state, &expiration);

                        // Persist and publish expiration event
                        let mut effects = Self::create_effects(expiration, expected_version, env, None);

                        // Direct orchestration: Send release command to Inventory (compensation)
                        let release_seats =
                            InventoryAction::ReleaseReservation { reservation_id };
                        let inventory_channel = env.global_actions.inventory_actions.clone();
                        effects.push(Effect::Future(Box::pin(async move {
                            if let Err(e) = inventory_channel.send(release_seats) {
                                tracing::error!(error = %e, "Failed to send ReleaseReservation command to inventory channel");
                            }
                            None
                        })));

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
                        let expected_version = state.version;
                        Self::apply_event(state, &cancellation);

                        // Persist and publish cancellation event
                        let mut effects = Self::create_effects(cancellation, expected_version, env, None);

                        // Direct orchestration: Send release command to Inventory (compensation)
                        let release_seats =
                            InventoryAction::ReleaseReservation { reservation_id };
                        let inventory_channel = env.global_actions.inventory_actions.clone();
                        effects.push(Effect::Future(Box::pin(async move {
                            if let Err(e) = inventory_channel.send(release_seats) {
                                tracing::error!(error = %e, "Failed to send ReleaseReservation command to inventory channel");
                            }
                            None
                        })));

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
    use composable_rust_testing::{assertions, mocks::InMemoryEventStore, ReducerTest};
    use crate::types::{CustomerId, EventId, Money, Reservation, ReservationExpiry, ReservationId};

    // Mock projection queries for tests
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

    // Mock inventory query for tests
    #[derive(Clone)]
    struct MockInventoryQuery;

    impl InventoryProjectionQuery for MockInventoryQuery {
        fn load_inventory(
            &self,
            _event_id: &EventId,
            _section: &str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<((u32, u32, u32, u32), Vec<crate::types::SeatAssignment>)>, String>> + Send + '_>> {
            Box::pin(async move { Ok(None) })
        }

        fn get_all_sections(
            &self,
            _event_id: &EventId,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<crate::aggregates::inventory::SectionAvailabilityData>, String>> + Send + '_>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn get_section_availability(
            &self,
            _event_id: &EventId,
            _section: &str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<crate::aggregates::inventory::SectionAvailabilityData>, String>> + Send + '_>> {
            Box::pin(async move { Ok(None) })
        }

        fn get_total_available(
            &self,
            _event_id: &EventId,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u32, String>> + Send + '_>> {
            Box::pin(async move { Ok(0) })
        }
    }

    // Mock event query for tests
    #[derive(Clone)]
    struct MockEventQuery;

    #[async_trait::async_trait]
    impl EventProjectionQuery for MockEventQuery {
        async fn load_event(&self, _event_id: &EventId) -> Result<Option<crate::types::Event>, String> {
            Ok(None)
        }

        async fn load_events(&self, _status_filter: Option<crate::types::EventStatus>) -> Result<Vec<crate::types::Event>, String> {
            Ok(Vec::new())
        }
    }

    fn create_test_global_actions() -> GlobalActionChannels {
        use tokio::sync::broadcast;
        let (event_tx, _) = broadcast::channel(10);
        let (inventory_tx, _) = broadcast::channel(10);
        let (reservation_tx, _) = broadcast::channel(10);
        let (payment_tx, _) = broadcast::channel(10);
        GlobalActionChannels {
            event_actions: event_tx,
            inventory_actions: inventory_tx,
            reservation_actions: reservation_tx,
            payment_actions: payment_tx,
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
            StreamId::new("reservation-test"),
            Arc::new(MockReservationQuery),
            create_test_global_actions(),
            Arc::new(MockInventoryQuery),
            Arc::new(MockEventQuery),
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
                respond_to: ResponseChannel::none(),
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
                // 2 for ReservationInitiated (AppendEvents + Channel Send to reservation_actions, no Redpanda)
                // 1 for sending ReserveSeats command to inventory_actions channel (direct orchestration)
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
