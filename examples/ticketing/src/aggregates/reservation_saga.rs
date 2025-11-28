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
    CustomerId, EventId, GlobalActionChannels, InventoryState, Money, PaymentState, Reservation,
    ReservationExpiry, ReservationId, ReservationState, ReservationStatus, ResponseChannel, SeatId,
    SeatNumber, TicketId,
};
use chrono::{DateTime, Duration, Utc};
use composable_rust_core::{
    append_events, delay, effect::Effect, environment::Clock,
    event_store::EventStore, reducer::Reducer, smallvec,
    stream::{StreamId, Version},
    SmallVec,
};
use composable_rust_macros::Action;
use composable_rust_runtime::Store;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::inventory::{InventoryAction, InventoryEnvironment, InventoryReducer};
use super::payment::{PaymentAction, PaymentEnvironment, PaymentReducer};
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

    // NOTE: CompletePayment was removed - payment is handled automatically via the saga
    // when seats are allocated. The saga orchestrates payment internally.

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

    /// Serialization failed
    #[event]
    SerializationFailed {
        /// Error message
        error: String,
    },

    /// Projection update confirmed
    #[event]
    ReservationProjectionConfirmed {
        /// Reservation ID
        reservation_id: ReservationId,
    },

    /// Projection update failed
    #[event]
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
/// Uses **factory functions** for child aggregate stores, following the pattern
/// established in `EventInventorySaga`. This enables proper dependency injection
/// and testability.
#[derive(Clone)]
pub struct ReservationEnvironment {
    // ===== Core Dependencies =====
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

    // ===== Factory Functions for Child Aggregate Stores =====
    /// Factory function to create Inventory aggregate stores
    pub create_inventory_store: Arc<
        dyn Fn(EventId) -> Store<InventoryState, InventoryAction, InventoryEnvironment, InventoryReducer>
            + Send
            + Sync,
    >,
    /// Factory function to create Payment aggregate stores
    pub create_payment_store: Arc<
        dyn Fn(PaymentId) -> Store<PaymentState, PaymentAction, PaymentEnvironment, PaymentReducer>
            + Send
            + Sync,
    >,
}

impl ReservationEnvironment {
    /// Creates a new `ReservationEnvironment` with factory functions for child stores
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        event_store: Arc<dyn EventStore>,
        stream_id: StreamId,
        projection: Arc<dyn ReservationProjectionQuery>,
        global_actions: GlobalActionChannels,
        create_inventory_store: Arc<
            dyn Fn(EventId) -> Store<InventoryState, InventoryAction, InventoryEnvironment, InventoryReducer>
                + Send
                + Sync,
        >,
        create_payment_store: Arc<
            dyn Fn(PaymentId) -> Store<PaymentState, PaymentAction, PaymentEnvironment, PaymentReducer>
                + Send
                + Sync,
        >,
    ) -> Self {
        Self {
            clock,
            event_store,
            stream_id,
            projection,
            global_actions,
            create_inventory_store,
            create_payment_store,
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

    /// Creates effects for persisting events (`PostgreSQL` only, no Redpanda)
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
        let ticketing_event = TicketingEvent::Reservation(event.clone());
        let mut serialized = match ticketing_event.serialize() {
            Ok(s) => s,
            Err(e) => {
                // Return error action instead of silent failure
                return smallvec![Effect::Future(Box::pin(async move {
                    Some(ReservationAction::SerializationFailed {
                        error: format!("Failed to serialize saga event: {e}"),
                    })
                }))];
            }
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
                events: vec![serialized],
                on_success: |version| Some(ReservationAction::VersionUpdated { version }),
                on_error: |error| Some(ReservationAction::ValidationFailed {
                    error: error.to_string()
                })
            },
            // Echo the event back as an action so it broadcasts to action_broadcast channel
            // This allows send_and_wait_for to receive it (e.g., ReservationCompleted)
            Effect::Future(Box::pin(async move {
                Some(event)
            }))
        ]
    }

    /// Creates effects for persisting multiple events atomically in a single append.
    ///
    /// This prevents race conditions when multiple events need to be persisted
    /// as part of the same state transition.
    fn create_batch_effects(
        events: Vec<ReservationAction>,
        expected_version: Version,
        env: &ReservationEnvironment,
    ) -> SmallVec<[Effect<ReservationAction>; 4]> {
        let mut serialized_events = Vec::with_capacity(events.len());
        let events_for_echo = events.clone();

        for event in events {
            let ticketing_event = TicketingEvent::Reservation(event);
            match ticketing_event.serialize() {
                Ok(s) => serialized_events.push(s),
                Err(e) => {
                    return smallvec![Effect::Future(Box::pin(async move {
                        Some(ReservationAction::SerializationFailed {
                            error: format!("Failed to serialize saga event: {e}"),
                        })
                    }))];
                }
            }
        }

        let mut effects = smallvec![
            append_events! {
                store: env.event_store,
                stream: env.stream_id.as_str(),
                expected_version: Some(expected_version),
                events: serialized_events,
                on_success: |version| Some(ReservationAction::VersionUpdated { version }),
                on_error: |error| Some(ReservationAction::ValidationFailed {
                    error: error.to_string()
                })
            }
        ];

        // Echo each event back as an action so it broadcasts to action_broadcast channel
        // This allows send_and_wait_for to receive them (e.g., ReservationCompleted)
        for event in events_for_echo {
            effects.push(Effect::Future(Box::pin(async move {
                Some(event)
            })));
        }

        effects
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

            ReservationAction::ValidationFailed { error }
            | ReservationAction::SerializationFailed { error } => {
                state.last_error = Some(error.clone());
            }

            // Commands and queries don't modify state
            // Response events also don't modify state (they're for API handlers)
            // Projection confirmation actions are logged but don't modify aggregate state
            ReservationAction::InitiateReservation { .. }
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

    #[allow(clippy::too_many_lines, clippy::cognitive_complexity)] // Complex saga orchestration required
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

                // Clone non-Copy fields for publishing to global channel
                let section_for_channel = section.clone();
                let specific_seats_for_channel = specific_seats.clone();

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

                // Direct orchestration: Use factory to create inventory store
                // Clone factory Arc to move into async block
                let create_inventory_store = env.create_inventory_store.clone();
                let section_for_inventory = section.clone();

                effects.push(Effect::Future(Box::pin(async move {
                    use std::time::Duration;

                    // Create inventory store using factory function
                    let inventory_store = create_inventory_store(event_id);

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
                effects.push(Effect::PublishWithResponse {
                    channel: env.global_actions.reservation_actions.clone(),
                    create_action: Box::new(move |respond_to| {
                        ReservationAction::InitiateReservation {
                            reservation_id,
                            event_id,
                            customer_id,
                            section: section_for_channel,
                            quantity,
                            specific_seats: specific_seats_for_channel,
                            correlation_id,
                            respond_to,
                        }
                    }),
                    on_success: Box::new(move || {
                        Some(ReservationAction::ReservationProjectionConfirmed {
                            reservation_id,
                        })
                    }),
                    on_error: Box::new(move |reason| {
                        Some(ReservationAction::ReservationProjectionFailed {
                            reservation_id,
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

                // Validate: Reservation must exist and be in Initiated state
                let Some(reservation) = state.reservations.get(&reservation_id) else {
                    tracing::error!(
                        reservation_id = %reservation_id.as_uuid(),
                        "SeatsAllocated received for non-existent reservation"
                    );
                    state.last_error = Some(format!(
                        "Reservation {} not found",
                        reservation_id.as_uuid()
                    ));
                    return smallvec![Effect::Future(Box::pin(async move {
                        Some(ReservationAction::ValidationFailed {
                            error: format!("Reservation {} not found", reservation_id.as_uuid()),
                        })
                    }))];
                };

                if !matches!(reservation.status, ReservationStatus::Initiated) {
                    tracing::warn!(
                        reservation_id = %reservation_id.as_uuid(),
                        current_status = ?reservation.status,
                        "SeatsAllocated received for reservation not in Initiated state"
                    );
                    // Idempotency: if already past this state, ignore
                    return SmallVec::new();
                }

                // Capture customer_id from the validated reservation before state changes
                let customer_id = reservation.customer_id;

                // Capture version BEFORE any state changes
                let expected_version = state.version;

                // Apply SeatsAllocated event
                Self::apply_event(state, &action);

                // Create payment request event with total_amount from SeatsAllocated
                // (calculated in the InitiateReservation async block based on seat count)
                let payment_id = PaymentId::new();
                let payment_requested = ReservationAction::PaymentRequested {
                    reservation_id,
                    payment_id,
                    amount: total_amount,
                };
                Self::apply_event(state, &payment_requested);

                // Batch persist both events atomically to avoid race condition
                let mut effects = Self::create_batch_effects(
                    vec![action, payment_requested.clone()],
                    expected_version,
                    env,
                );

                // Direct orchestration: Use factory to create payment store and wait for response
                let create_payment_store = env.create_payment_store.clone();

                effects.push(Effect::Future(Box::pin(async move {
                    use std::time::Duration;

                    // Create payment store using factory function
                    let payment_store = create_payment_store(payment_id);

                    // Send ProcessPayment action to payment store
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

                    // Use send_and_wait_for to wait for PaymentSucceeded or PaymentFailed
                    let result = payment_store.send_and_wait_for(
                        process_payment,
                        |action| {
                            matches!(
                                action,
                                PaymentAction::PaymentSucceeded { payment_id: pid, .. } if *pid == payment_id
                            ) || matches!(
                                action,
                                PaymentAction::PaymentFailed { payment_id: pid, .. } if *pid == payment_id
                            )
                        },
                        Duration::from_secs(30),
                    ).await;

                    match result {
                        Ok(PaymentAction::PaymentSucceeded { .. }) => {
                            tracing::info!(
                                reservation_id = %reservation_id.as_uuid(),
                                payment_id = %payment_id.as_uuid(),
                                "Payment succeeded, returning PaymentSucceeded"
                            );

                            Some(ReservationAction::PaymentSucceeded {
                                reservation_id,
                                payment_id,
                            })
                        }
                        Ok(PaymentAction::PaymentFailed { reason, .. }) => {
                            tracing::warn!(
                                reservation_id = %reservation_id.as_uuid(),
                                payment_id = %payment_id.as_uuid(),
                                reason = %reason,
                                "Payment failed"
                            );
                            Some(ReservationAction::PaymentFailed {
                                reservation_id,
                                payment_id,
                                reason,
                            })
                        }
                        Ok(other) => {
                            tracing::error!(
                                reservation_id = %reservation_id.as_uuid(),
                                action = ?other,
                                "Unexpected action received from payment store"
                            );
                            Some(ReservationAction::PaymentFailed {
                                reservation_id,
                                payment_id,
                                reason: "Unexpected payment response".to_string(),
                            })
                        }
                        Err(e) => {
                            tracing::error!(
                                reservation_id = %reservation_id.as_uuid(),
                                error = %e,
                                "Payment store error"
                            );
                            Some(ReservationAction::PaymentFailed {
                                reservation_id,
                                payment_id,
                                reason: format!("Payment store error: {e}"),
                            })
                        }
                    }
                })));

                // Broadcast PaymentRequested to reservation_actions channel for projection
                let reservation_channel = env.global_actions.reservation_actions.clone();
                let payment_requested_for_projection = ReservationAction::PaymentRequested {
                    reservation_id,
                    payment_id,
                    amount: total_amount,
                };
                effects.push(Effect::Future(Box::pin(async move {
                    if let Err(e) = reservation_channel.send(payment_requested_for_projection.clone()) {
                        tracing::error!(error = %e, "Failed to broadcast PaymentRequested to reservation channel");
                    }
                    None // Don't return action here - the payment store feedback loop handles it
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

                // Validate: Reservation must exist and be in PaymentPending state
                let Some(reservation) = state.reservations.get(&reservation_id) else {
                    tracing::error!(
                        reservation_id = %reservation_id.as_uuid(),
                        "PaymentSucceeded received for non-existent reservation"
                    );
                    state.last_error = Some(format!(
                        "Reservation {} not found",
                        reservation_id.as_uuid()
                    ));
                    return smallvec![Effect::Future(Box::pin(async move {
                        Some(ReservationAction::ValidationFailed {
                            error: format!("Reservation {} not found", reservation_id.as_uuid()),
                        })
                    }))];
                };

                if !matches!(reservation.status, ReservationStatus::PaymentPending) {
                    tracing::warn!(
                        reservation_id = %reservation_id.as_uuid(),
                        current_status = ?reservation.status,
                        "PaymentSucceeded received for reservation not in PaymentPending state"
                    );
                    // Idempotency: if already completed, ignore
                    if matches!(reservation.status, ReservationStatus::Completed | ReservationStatus::PaymentCompleted) {
                        return SmallVec::new();
                    }
                    // Otherwise this is an error - wrong state transition
                    state.last_error = Some(format!(
                        "Invalid state transition: cannot complete payment for reservation in {:?} state",
                        reservation.status
                    ));
                    return smallvec![Effect::Future(Box::pin(async move {
                        Some(ReservationAction::ValidationFailed {
                            error: "Invalid state: reservation not awaiting payment".to_string(),
                        })
                    }))];
                }

                // Capture data from reservation before applying events (which may mutate it)
                let customer_id = reservation.customer_id;
                let ticket_count = reservation.seats.len();
                let event_id = reservation.event_id;

                // Capture version BEFORE any state changes
                let expected_version = state.version;

                // Apply PaymentSucceeded event
                Self::apply_event(state, &action);

                let tickets: Vec<TicketId> =
                    (0..ticket_count).map(|_| TicketId::new()).collect();

                // Create completion event
                let completion = ReservationAction::ReservationCompleted {
                    reservation_id,
                    tickets_issued: tickets,
                    completed_at: env.clock.now(),
                };
                Self::apply_event(state, &completion);

                // Batch persist both events atomically to avoid race condition
                let mut effects = Self::create_batch_effects(
                    vec![action, completion.clone()],
                    expected_version,
                    env,
                );

                // Emit completion as observable action for send_and_wait_for
                effects.push(Effect::Future(Box::pin(async move {
                    Some(completion)
                })));

                // Direct orchestration: Confirm reservation in Inventory using factory
                // Use send_and_wait_for with short timeout - compensation is best-effort
                // since saga is already in Completed state
                let create_inventory_store = env.create_inventory_store.clone();
                effects.push(Effect::Future(Box::pin(async move {
                    use std::time::Duration;

                    let inventory_store = create_inventory_store(event_id);
                    let confirm_action = InventoryAction::ConfirmReservation {
                        reservation_id,
                        customer_id,
                    };

                    // Short timeout (5s) - if inventory is slow, don't block
                    // Saga is already complete, this is best-effort confirmation
                    let result = inventory_store.send_and_wait_for(
                        confirm_action,
                        |action| {
                            matches!(
                                action,
                                InventoryAction::SeatsConfirmed { reservation_id: rid, .. } if *rid == reservation_id
                            ) || matches!(action, InventoryAction::ValidationFailed { .. })
                        },
                        Duration::from_secs(5),
                    ).await;

                    match result {
                        Ok(InventoryAction::SeatsConfirmed { .. }) => {
                            tracing::info!(
                                reservation_id = %reservation_id.as_uuid(),
                                "Inventory confirmation succeeded"
                            );
                        }
                        Ok(InventoryAction::ValidationFailed { error }) => {
                            tracing::warn!(
                                reservation_id = %reservation_id.as_uuid(),
                                error = %error,
                                "Inventory confirmation failed (reservation already complete)"
                            );
                        }
                        Ok(other) => {
                            tracing::warn!(
                                reservation_id = %reservation_id.as_uuid(),
                                action = ?other,
                                "Unexpected inventory response during confirmation"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                reservation_id = %reservation_id.as_uuid(),
                                error = %e,
                                "Inventory confirmation timed out or failed (reservation already complete)"
                            );
                        }
                    }

                    None // Don't return action - saga is already complete
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

                // Validate: Reservation must exist and be in PaymentPending state
                let Some(reservation) = state.reservations.get(&reservation_id) else {
                    tracing::error!(
                        reservation_id = %reservation_id.as_uuid(),
                        "PaymentFailed received for non-existent reservation"
                    );
                    state.last_error = Some(format!(
                        "Reservation {} not found",
                        reservation_id.as_uuid()
                    ));
                    return smallvec![Effect::Future(Box::pin(async move {
                        Some(ReservationAction::ValidationFailed {
                            error: format!("Reservation {} not found", reservation_id.as_uuid()),
                        })
                    }))];
                };

                if !matches!(reservation.status, ReservationStatus::PaymentPending) {
                    tracing::warn!(
                        reservation_id = %reservation_id.as_uuid(),
                        current_status = ?reservation.status,
                        "PaymentFailed received for reservation not in PaymentPending state"
                    );
                    // Idempotency: if already compensated, ignore
                    if matches!(reservation.status, ReservationStatus::Compensated | ReservationStatus::Cancelled | ReservationStatus::Expired) {
                        return SmallVec::new();
                    }
                    // Otherwise this is an error - wrong state transition
                    state.last_error = Some(format!(
                        "Invalid state transition: cannot fail payment for reservation in {:?} state",
                        reservation.status
                    ));
                    return smallvec![Effect::Future(Box::pin(async move {
                        Some(ReservationAction::ValidationFailed {
                            error: "Invalid state: reservation not awaiting payment".to_string(),
                        })
                    }))];
                }

                tracing::warn!(
                    reservation_id = %reservation_id.as_uuid(),
                    reason = %reason,
                    "Payment failed, triggering saga compensation"
                );

                // Capture event_id from validated reservation before state changes
                let event_id = reservation.event_id;

                // Capture version BEFORE any state changes
                let expected_version = state.version;

                // Apply PaymentFailed event
                Self::apply_event(state, &action);

                let compensation = ReservationAction::ReservationCompensated {
                    reservation_id,
                    reason: reason.clone(),
                    compensated_at: env.clock.now(),
                };
                Self::apply_event(state, &compensation);

                // Batch persist both events atomically to avoid race condition
                let mut effects = Self::create_batch_effects(
                    vec![action, compensation],
                    expected_version,
                    env,
                );

                // Direct orchestration: Release seats in Inventory using factory
                // Use send_and_wait_for with short timeout - compensation is best-effort
                // since saga is already in Compensated state
                let create_inventory_store = env.create_inventory_store.clone();
                effects.push(Effect::Future(Box::pin(async move {
                    use std::time::Duration;

                    let inventory_store = create_inventory_store(event_id);
                    let release_action = InventoryAction::ReleaseReservation { reservation_id };

                    // Short timeout (5s) - if inventory is slow, don't block
                    // Saga is already compensated, this is best-effort release
                    let result = inventory_store.send_and_wait_for(
                        release_action,
                        |action| {
                            matches!(
                                action,
                                InventoryAction::SeatsReleased { reservation_id: rid, .. } if *rid == reservation_id
                            ) || matches!(action, InventoryAction::ValidationFailed { .. })
                        },
                        Duration::from_secs(5),
                    ).await;

                    match result {
                        Ok(InventoryAction::SeatsReleased { .. }) => {
                            tracing::info!(
                                reservation_id = %reservation_id.as_uuid(),
                                "Inventory release succeeded (payment failed compensation)"
                            );
                        }
                        Ok(InventoryAction::ValidationFailed { error }) => {
                            tracing::warn!(
                                reservation_id = %reservation_id.as_uuid(),
                                error = %error,
                                "Inventory release failed (may already be released)"
                            );
                        }
                        Ok(other) => {
                            tracing::warn!(
                                reservation_id = %reservation_id.as_uuid(),
                                action = ?other,
                                "Unexpected inventory response during release"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                reservation_id = %reservation_id.as_uuid(),
                                error = %e,
                                "Inventory release timed out or failed (compensation best-effort)"
                            );
                        }
                    }

                    None // Don't return action - saga is already compensated
                })));

                effects
            }

            // ========== Step 4: Timeout (COMPENSATION) ==========
            ReservationAction::ExpireReservation { reservation_id } => {
                // Check if reservation still exists and is in an expirable state
                if let Some(reservation) = state.reservations.get(&reservation_id) {
                    // Only expire if in SeatsReserved or PaymentPending state.
                    //
                    // NOTE: We intentionally do NOT expire reservations in Initiated state.
                    // In Initiated state, the inventory `send_and_wait_for` is still pending.
                    // If we expired here, we could have a race condition where:
                    //   1. Expiration fires, we set status to Expired
                    //   2. Inventory responds with SeatsAllocated
                    //   3. SeatsAllocated handler finds wrong state
                    //
                    // By only expiring SeatsReserved/PaymentPending, we ensure:
                    //   - If inventory is slow (>5 min), it will eventually fail or succeed
                    //   - If it fails, ValidationFailed will handle cleanup
                    //   - If it succeeds, a subsequent expiration check will handle it
                    if matches!(
                        reservation.status,
                        ReservationStatus::SeatsReserved | ReservationStatus::PaymentPending
                    ) {
                        // Capture event_id before state changes
                        let event_id = reservation.event_id;

                        // Apply expiration event
                        let expiration = ReservationAction::ReservationExpired {
                            reservation_id,
                            expired_at: env.clock.now(),
                        };
                        let expected_version = state.version;
                        Self::apply_event(state, &expiration);

                        // Persist and publish expiration event
                        let mut effects = Self::create_effects(expiration, expected_version, env, None);

                        // Direct orchestration: Release seats in Inventory using factory
                        // Use send_and_wait_for with short timeout - compensation is best-effort
                        // since saga is already in Expired state
                        let create_inventory_store = env.create_inventory_store.clone();
                        effects.push(Effect::Future(Box::pin(async move {
                            use std::time::Duration;

                            let inventory_store = create_inventory_store(event_id);
                            let release_action = InventoryAction::ReleaseReservation { reservation_id };

                            // Short timeout (5s) - if inventory is slow, don't block
                            // Saga is already expired, this is best-effort release
                            let result = inventory_store.send_and_wait_for(
                                release_action,
                                |action| {
                                    matches!(
                                        action,
                                        InventoryAction::SeatsReleased { reservation_id: rid, .. } if *rid == reservation_id
                                    ) || matches!(action, InventoryAction::ValidationFailed { .. })
                                },
                                Duration::from_secs(5),
                            ).await;

                            match result {
                                Ok(InventoryAction::SeatsReleased { .. }) => {
                                    tracing::info!(
                                        reservation_id = %reservation_id.as_uuid(),
                                        "Inventory release succeeded (expiration compensation)"
                                    );
                                }
                                Ok(InventoryAction::ValidationFailed { error }) => {
                                    tracing::warn!(
                                        reservation_id = %reservation_id.as_uuid(),
                                        error = %error,
                                        "Inventory release failed (may already be released)"
                                    );
                                }
                                Ok(other) => {
                                    tracing::warn!(
                                        reservation_id = %reservation_id.as_uuid(),
                                        action = ?other,
                                        "Unexpected inventory response during release"
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        reservation_id = %reservation_id.as_uuid(),
                                        error = %e,
                                        "Inventory release timed out or failed (compensation best-effort)"
                                    );
                                }
                            }

                            None // Don't return action - saga is already expired
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
                    // Only cancel if in SeatsReserved or PaymentPending state.
                    //
                    // NOTE: We intentionally do NOT cancel reservations in Initiated state.
                    // In Initiated state, the inventory `send_and_wait_for` is still pending.
                    // If we cancelled here, we could have a race condition where:
                    //   1. Cancellation fires, we set status to Cancelled, send release (no-op)
                    //   2. Inventory responds with SeatsAllocated
                    //   3. SeatsAllocated handler finds wrong state, ignores
                    //   4. Seats are orphaned in inventory!
                    //
                    // By only cancelling SeatsReserved/PaymentPending, we ensure:
                    //   - If inventory is slow, the user must wait for it to complete
                    //   - Once seats are reserved, cancellation works correctly
                    //
                    // Also don't cancel if already in a terminal state (Completed, Cancelled,
                    // Expired, Compensated) to avoid duplicate work.
                    if matches!(
                        reservation.status,
                        ReservationStatus::SeatsReserved | ReservationStatus::PaymentPending
                    ) {
                        // Capture event_id before state changes
                        let event_id = reservation.event_id;

                        let cancellation = ReservationAction::ReservationCancelled {
                            reservation_id,
                            reason: "Cancelled by customer".to_string(),
                            cancelled_at: env.clock.now(),
                        };
                        let expected_version = state.version;
                        Self::apply_event(state, &cancellation);

                        // Persist and publish cancellation event
                        let mut effects = Self::create_effects(cancellation, expected_version, env, None);

                        // Direct orchestration: Release seats in Inventory using factory
                        // Use send_and_wait_for with short timeout - compensation is best-effort
                        // since saga is already in Cancelled state
                        let create_inventory_store = env.create_inventory_store.clone();
                        effects.push(Effect::Future(Box::pin(async move {
                            use std::time::Duration;

                            let inventory_store = create_inventory_store(event_id);
                            let release_action = InventoryAction::ReleaseReservation { reservation_id };

                            // Short timeout (5s) - if inventory is slow, don't block
                            // Saga is already cancelled, this is best-effort release
                            let result = inventory_store.send_and_wait_for(
                                release_action,
                                |action| {
                                    matches!(
                                        action,
                                        InventoryAction::SeatsReleased { reservation_id: rid, .. } if *rid == reservation_id
                                    ) || matches!(action, InventoryAction::ValidationFailed { .. })
                                },
                                Duration::from_secs(5),
                            ).await;

                            match result {
                                Ok(InventoryAction::SeatsReleased { .. }) => {
                                    tracing::info!(
                                        reservation_id = %reservation_id.as_uuid(),
                                        "Inventory release succeeded (cancellation compensation)"
                                    );
                                }
                                Ok(InventoryAction::ValidationFailed { error }) => {
                                    tracing::warn!(
                                        reservation_id = %reservation_id.as_uuid(),
                                        error = %error,
                                        "Inventory release failed (may already be released)"
                                    );
                                }
                                Ok(other) => {
                                    tracing::warn!(
                                        reservation_id = %reservation_id.as_uuid(),
                                        action = ?other,
                                        "Unexpected inventory response during release"
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        reservation_id = %reservation_id.as_uuid(),
                                        error = %e,
                                        "Inventory release timed out or failed (compensation best-effort)"
                                    );
                                }
                            }

                            None // Don't return action - saga is already cancelled
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;  // Brings in ReservationEnvironment, ReservationState, Store, etc.
    use std::sync::Arc;
    use composable_rust_core::environment::SystemClock;
    use composable_rust_core::stream::StreamId;
    use composable_rust_testing::{assertions, mocks::InMemoryEventStore, ReducerTest};
    use crate::aggregates::inventory::InventoryProjectionQuery;
    use crate::aggregates::payment::PaymentProjectionQuery;
    use crate::projections::EventProjectionQuery;
    use crate::types::{CustomerId, EventId, Money, Payment, PaymentId, Reservation, ReservationExpiry, ReservationId};

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

    // Mock payment query for tests
    #[derive(Clone)]
    struct MockPaymentQuery;

    #[async_trait::async_trait]
    impl PaymentProjectionQuery for MockPaymentQuery {
        async fn load_payment(&self, _payment_id: &PaymentId) -> Result<Option<Payment>, String> {
            Ok(None)
        }

        async fn load_customer_payments(&self, _customer_id: &CustomerId, _limit: usize, _offset: usize) -> Result<Vec<Payment>, String> {
            Ok(Vec::new())
        }
    }

    /// Returns a fixed test time for deterministic tests.
    fn test_time() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp")
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
        // Factory functions for child stores
        let global_actions = create_test_global_actions();
        let global_actions_for_inventory = global_actions.clone();
        let global_actions_for_payment = global_actions.clone();

        let create_inventory_store: Arc<
            dyn Fn(EventId) -> Store<InventoryState, InventoryAction, InventoryEnvironment, InventoryReducer>
                + Send
                + Sync,
        > = Arc::new(move |event_id| {
            let stream_id = StreamId::new(format!("inventory-{}", event_id.as_uuid()));
            let inv_env = InventoryEnvironment::new(
                Arc::new(SystemClock),
                Arc::new(InMemoryEventStore::new()),
                stream_id,
                Arc::new(MockInventoryQuery),
                Arc::new(MockEventQuery),
                global_actions_for_inventory.clone(),
            );
            Store::new(InventoryState::new(), InventoryReducer::new(), inv_env)
        });

        let create_payment_store: Arc<
            dyn Fn(PaymentId) -> Store<PaymentState, PaymentAction, PaymentEnvironment, PaymentReducer>
                + Send
                + Sync,
        > = Arc::new(move |payment_id| {
            let stream_id = StreamId::new(format!("payment-{}", payment_id.as_uuid()));
            let pay_env = PaymentEnvironment::new(
                Arc::new(SystemClock),
                Arc::new(InMemoryEventStore::new()),
                stream_id,
                Arc::new(MockPaymentQuery),
                global_actions_for_payment.clone(),
            );
            Store::new(PaymentState::new(), PaymentReducer::new(), pay_env)
        });

        let env = ReservationEnvironment::new(
            Arc::new(SystemClock),
            Arc::new(InMemoryEventStore::new()),
            StreamId::new("reservation-test"),
            Arc::new(MockReservationQuery),
            global_actions,
            create_inventory_store,
            create_payment_store,
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
                // Should return 5 effects:
                // 2 for ReservationInitiated (AppendEvents + Echo)
                // 1 for sending ReserveSeats command to inventory_actions channel (direct orchestration)
                // 1 for scheduling expiration timeout (Delay)
                // 1 for PublishWithResponse to reservation_actions channel
                assert_eq!(effects.len(), 5);
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
                    ReservationExpiry::new(test_time() + Duration::minutes(5)),
                    test_time(),
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
                    ReservationExpiry::new(test_time() + Duration::minutes(5)),
                    test_time(),
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
                    ReservationExpiry::new(test_time() + Duration::minutes(5)),
                    test_time(),
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
                    ReservationExpiry::new(test_time() + Duration::minutes(5)),
                    test_time(),
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
                    ReservationExpiry::new(test_time() + Duration::minutes(5)),
                    test_time(),
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
