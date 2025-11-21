//! Inventory aggregate for the Event Ticketing System.
//!
//! Manages seat availability and reservations. This aggregate is CRITICAL for preventing
//! double-booking in high-concurrency scenarios (the "last seat" problem).
//!
//! **Concurrency Strategy**: Optimistic concurrency control - check available seats including
//! reserved count to prevent overselling during concurrent reservation attempts.

use crate::projections::TicketingEvent;
use crate::types::{
    Capacity, CustomerId, EventId, Inventory, InventoryState, ReservationId, SeatAssignment,
    SeatId, SeatNumber, SeatStatus,
};
use chrono::{DateTime, Utc};
use composable_rust_core::{
    append_events, delay, effect::Effect, environment::Clock, event_bus::EventBus,
    event_store::EventStore, publish_event, reducer::Reducer, smallvec, stream::StreamId, SmallVec,
};
use composable_rust_macros::Action;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ============================================================================
// Data Structures
// ============================================================================

/// Section availability data for query results.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SectionAvailabilityData {
    /// Section identifier
    pub section: String,
    /// Total capacity
    pub total_capacity: u32,
    /// Currently reserved seats (pending payment)
    pub reserved: u32,
    /// Sold seats (payment confirmed)
    pub sold: u32,
    /// Available seats (total - reserved - sold)
    pub available: u32,
}

// ============================================================================
// Projection Query Trait
// ============================================================================

/// Trait for querying inventory projection data.
///
/// This trait defines the read operations needed by the Inventory aggregate
/// to load state from the projection when processing commands.
///
/// # Pattern: State Loading from Projections
///
/// According to the state-loading-patterns spec, aggregates load state on-demand
/// by querying projections. This trait is injected via the Environment to enable
/// the reducer to trigger state loading effects.
///
/// Note: Returns `BoxFuture` instead of async fn to be dyn-compatible (object-safe).
pub trait InventoryProjectionQuery: Send + Sync {
    /// Load inventory data for a specific event and section.
    ///
    /// Returns (counts, seat_assignments) where counts is (`total_capacity`, reserved, sold, available).
    /// The seat assignments provide the complete denormalized snapshot of individual seats.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn load_inventory(
        &self,
        event_id: &EventId,
        section: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<((u32, u32, u32, u32), Vec<SeatAssignment>)>, String>> + Send + '_>>;

    /// Query all sections for an event.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn get_all_sections(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<SectionAvailabilityData>, String>> + Send + '_>>;

    /// Query availability for a specific section.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn get_section_availability(
        &self,
        event_id: &EventId,
        section: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<SectionAvailabilityData>, String>> + Send + '_>>;

    /// Query total available seats across all sections for an event.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn get_total_available(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u32, String>> + Send + '_>>;
}

// ============================================================================
// Actions (Commands + Events)
// ============================================================================

/// Actions for the Inventory aggregate
///
/// Handles seat reservation, release, and confirmation with atomic operations
/// to prevent double-booking.
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum InventoryAction {
    // Commands
    /// Initialize inventory for an event section
    #[command]
    InitializeInventory {
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
        /// Total capacity
        capacity: Capacity,
        /// Optional specific seat numbers (None for general admission)
        seat_numbers: Option<Vec<SeatNumber>>,
    },

    /// Reserve seats for a reservation
    #[command]
    ReserveSeats {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
        /// Number of seats to reserve
        quantity: u32,
        /// Optional specific seat numbers
        specific_seats: Option<Vec<SeatNumber>>,
        /// When the reservation expires
        expires_at: DateTime<Utc>,
    },

    /// Confirm reservation (mark seats as sold)
    #[command]
    ConfirmReservation {
        /// Reservation to confirm
        reservation_id: ReservationId,
        /// Customer purchasing the seats
        customer_id: CustomerId,
    },

    /// Release reservation (return seats to available pool)
    #[command]
    ReleaseReservation {
        /// Reservation to release
        reservation_id: ReservationId,
    },

    /// Expire a reservation (timeout reached)
    #[command]
    ExpireReservation {
        /// Reservation to expire
        reservation_id: ReservationId,
    },

    /// Query all sections for an event
    #[command]
    GetAllSections {
        /// Event ID to query
        event_id: EventId,
    },

    /// Query availability for a specific section
    #[command]
    GetSectionAvailability {
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
    },

    /// Query total available seats across all sections for an event
    #[command]
    GetTotalAvailable {
        /// Event ID to query
        event_id: EventId,
    },

    // Events
    /// Inventory was initialized
    #[event]
    InventoryInitialized {
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
        /// Capacity
        capacity: Capacity,
        /// Created seat IDs
        seats: Vec<SeatId>,
        /// When initialized
        initialized_at: DateTime<Utc>,
    },

    /// Seats were reserved
    #[event]
    SeatsReserved {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
        /// Reserved seat IDs
        seats: Vec<SeatId>,
        /// Expiration time
        expires_at: DateTime<Utc>,
        /// When reserved
        reserved_at: DateTime<Utc>,
    },

    /// Seats were confirmed (sold)
    #[event]
    SeatsConfirmed {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Event ID (needed for projection rebuilding)
        event_id: EventId,
        /// Section name (needed for projection rebuilding)
        section: String,
        /// Customer ID
        customer_id: CustomerId,
        /// Confirmed seat IDs
        seats: Vec<SeatId>,
        /// When confirmed
        confirmed_at: DateTime<Utc>,
    },

    /// Seats were released back to available pool
    #[event]
    SeatsReleased {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Event ID (needed for projection rebuilding)
        event_id: EventId,
        /// Section name (needed for projection rebuilding)
        section: String,
        /// Released seat IDs
        seats: Vec<SeatId>,
        /// When released
        released_at: DateTime<Utc>,
    },

    /// Insufficient inventory (concurrency - someone else got the last seats)
    #[event]
    InsufficientInventory {
        /// Event ID
        event_id: EventId,
        /// Section
        section: String,
        /// Requested quantity
        requested: u32,
        /// Actually available
        available: u32,
    },

    /// Validation failed
    #[event]
    ValidationFailed {
        /// Error message
        error: String,
    },

    /// Inventory state loaded from projection
    #[event]
    InventoryStateLoaded {
        /// Event ID
        event_id: EventId,
        /// Section
        section: String,
        /// Loaded inventory data (total, available, reserved, sold)
        inventory_data: Option<(u32, u32, u32, u32)>,
        /// Loaded seat assignments from projection (complete snapshot)
        seat_assignments: Vec<SeatAssignment>,
    },

    /// All sections were queried (query result)
    #[event]
    AllSectionsQueried {
        /// Event ID that was queried
        event_id: EventId,
        /// Section availability data
        sections: Vec<SectionAvailabilityData>,
    },

    /// Section availability was queried (query result)
    #[event]
    SectionAvailabilityQueried {
        /// Event ID that was queried
        event_id: EventId,
        /// Section that was queried
        section: String,
        /// Availability data (None if section not found)
        data: Option<SectionAvailabilityData>,
    },

    /// Total available seats were queried (query result)
    #[event]
    TotalAvailableQueried {
        /// Event ID that was queried
        event_id: EventId,
        /// Total available seats across all sections
        total_available: u32,
    },
}

// ============================================================================
// Environment
// ============================================================================

/// Environment dependencies for the Inventory aggregate
#[derive(Clone)]
pub struct InventoryEnvironment {
    /// Clock for timestamps
    pub clock: Arc<dyn Clock>,
    /// Event store for persistence
    pub event_store: Arc<dyn EventStore>,
    /// Event bus for publishing
    pub event_bus: Arc<dyn EventBus>,
    /// Stream ID for this aggregate instance
    pub stream_id: StreamId,
    /// Projection query for loading state on-demand
    pub projection: Arc<dyn InventoryProjectionQuery>,
}

impl InventoryEnvironment {
    /// Creates a new `InventoryEnvironment`
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        event_store: Arc<dyn EventStore>,
        event_bus: Arc<dyn EventBus>,
        stream_id: StreamId,
        projection: Arc<dyn InventoryProjectionQuery>,
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

/// Reducer for the Inventory aggregate
///
/// CRITICAL: This reducer implements atomic seat reservation to prevent double-booking.
/// The key is checking `reserved + sold` against capacity, NOT just `sold`.
#[derive(Clone, Debug)]
pub struct InventoryReducer;

impl InventoryReducer {
    /// Creates a new `InventoryReducer`
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Creates effects for persisting and publishing an event
    fn create_effects(
        event: InventoryAction,
        env: &InventoryEnvironment,
    ) -> SmallVec<[Effect<InventoryAction>; 4]> {
        let ticketing_event = TicketingEvent::Inventory(event);
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
                on_error: |error| Some(InventoryAction::ValidationFailed {
                    error: error.to_string()
                })
            },
            publish_event! {
                bus: env.event_bus,
                topic: "inventory",
                event: serialized,
                on_success: || None,
                on_error: |error| Some(InventoryAction::ValidationFailed {
                    error: error.to_string()
                })
            }
        ]
    }

    /// Validates `InitializeInventory` command
    fn validate_initialize_inventory(
        state: &InventoryState,
        event_id: &EventId,
        section: &str,
        capacity: Capacity,
    ) -> Result<(), String> {
        // Check if inventory already exists for this event/section
        if state
            .get_inventory(event_id, section)
            .is_some()
        {
            return Err(format!(
                "Inventory for event {event_id}, section '{section}' already exists"
            ));
        }

        // Capacity must be > 0
        if capacity.value() == 0 {
            return Err("Capacity must be greater than zero".to_string());
        }

        Ok(())
    }

    /// Validates `ReserveSeats` command
    ///
    /// CRITICAL: This is where we prevent double-booking.
    fn validate_reserve_seats(
        state: &InventoryState,
        event_id: &EventId,
        section: &str,
        quantity: u32,
    ) -> Result<(), String> {
        // Quantity must be > 0 and <= 8 (max purchase)
        if quantity == 0 {
            return Err("Quantity must be greater than zero".to_string());
        }

        if quantity > 8 {
            return Err(format!(
                "Cannot reserve more than 8 seats at once (requested: {quantity})"
            ));
        }

        // Inventory must exist
        let Some(inventory) = state.get_inventory(event_id, section) else {
            return Err(format!(
                "Inventory for event {event_id}, section '{section}' not found"
            ));
        };

        // CRITICAL: Check actual availability (including reserved seats)
        let actually_available = inventory.available();

        if actually_available < quantity {
            return Err(format!(
                "Insufficient inventory: requested {quantity}, available {actually_available}"
            ));
        }

        Ok(())
    }

    /// Selects available seats for reservation
    ///
    /// For general admission, picks the first N available seats.
    fn select_available_seats(
        state: &InventoryState,
        event_id: &EventId,
        section: &str,
        quantity: u32,
    ) -> Vec<SeatId> {
        // IMPORTANT: Sort seats by ID to ensure deterministic selection
        // HashMap iteration order is non-deterministic, which would cause
        // different seats to be selected during event replay
        let mut available: Vec<SeatId> = state
            .seat_assignments
            .values()
            .filter(|seat| {
                seat.event_id == *event_id
                    && seat.section == *section
                    && seat.status == SeatStatus::Available
            })
            .map(|seat| seat.seat_id)
            .collect();

        // Sort to ensure consistent ordering
        available.sort();

        // Take only the requested quantity
        available.into_iter().take(quantity as usize).collect()
    }

    /// Finds seats by reservation ID
    fn find_seats_by_reservation(
        state: &InventoryState,
        reservation_id: &ReservationId,
    ) -> Vec<SeatId> {
        state
            .seat_assignments
            .values()
            .filter(|seat| seat.reserved_by == Some(*reservation_id))
            .map(|seat| seat.seat_id)
            .collect()
    }

    /// Finds `event_id` and section for a reservation
    /// Returns None if reservation not found
    fn find_reservation_location(
        state: &InventoryState,
        reservation_id: &ReservationId,
    ) -> Option<(EventId, String)> {
        state
            .seat_assignments
            .values()
            .find(|seat| seat.reserved_by == Some(*reservation_id))
            .map(|seat| (seat.event_id, seat.section.clone()))
    }

    /// Applies an event to state
    #[allow(clippy::too_many_lines)] // Complex state management required
    fn apply_event(state: &mut InventoryState, action: &InventoryAction) {
        match action {
            InventoryAction::InventoryInitialized {
                event_id,
                section,
                capacity,
                seats,
                ..
            } => {
                // Create inventory record
                let inventory = Inventory::new(*event_id, section.clone(), *capacity);
                state
                    .inventories
                    .insert((*event_id, section.clone()), inventory);

                // Create seat assignments
                for seat_id in seats {
                    let assignment = SeatAssignment::new(
                        *seat_id,
                        *event_id,
                        section.clone(),
                        None, // General admission (no specific seat numbers for now)
                    );
                    state.seat_assignments.insert(*seat_id, assignment);
                }

                state.last_error = None;
            }

            InventoryAction::SeatsReserved {
                reservation_id,
                event_id,
                section,
                seats,
                expires_at,
                ..
            } => {
                // Update inventory reserved count
                let key = (*event_id, section.clone());
                if let Some(inventory) = state.inventories.get_mut(&key) {
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        inventory.reserved += seats.len() as u32;
                    }
                }

                // Mark seats as reserved
                for seat_id in seats {
                    if let Some(seat) = state.seat_assignments.get_mut(seat_id) {
                        seat.status = SeatStatus::Reserved {
                            expires_at: *expires_at,
                        };
                        seat.reserved_by = Some(*reservation_id);
                    }
                }

                state.last_error = None;
            }

            InventoryAction::SeatsConfirmed {
                customer_id, seats, ..
            } => {
                // Find which inventory this belongs to
                if let Some(first_seat) = seats.first() {
                    if let Some(seat_assignment) = state.seat_assignments.get(first_seat) {
                        let key = (seat_assignment.event_id, seat_assignment.section.clone());
                        if let Some(inventory) = state.inventories.get_mut(&key) {
                            // Move from reserved to sold
                            #[allow(clippy::cast_possible_truncation)]
                            {
                                inventory.reserved = inventory.reserved.saturating_sub(seats.len() as u32);
                                inventory.sold += seats.len() as u32;
                            }
                        }
                    }
                }

                // Mark seats as sold
                for seat_id in seats {
                    if let Some(seat) = state.seat_assignments.get_mut(seat_id) {
                        seat.status = SeatStatus::Sold;
                        seat.sold_to = Some(*customer_id);
                        seat.reserved_by = None;
                    }
                }

                state.last_error = None;
            }

            InventoryAction::SeatsReleased { seats, .. } => {
                // Find which inventory this belongs to
                if let Some(first_seat) = seats.first() {
                    if let Some(seat_assignment) = state.seat_assignments.get(first_seat) {
                        let key = (seat_assignment.event_id, seat_assignment.section.clone());
                        if let Some(inventory) = state.inventories.get_mut(&key) {
                            // Return from reserved to available
                            #[allow(clippy::cast_possible_truncation)]
                            {
                                inventory.reserved = inventory.reserved.saturating_sub(seats.len() as u32);
                            }
                        }
                    }
                }

                // Mark seats as available
                for seat_id in seats {
                    if let Some(seat) = state.seat_assignments.get_mut(seat_id) {
                        seat.status = SeatStatus::Available;
                        seat.reserved_by = None;
                    }
                }

                state.last_error = None;
            }

            InventoryAction::ValidationFailed { error } => {
                state.last_error = Some(error.clone());
            }

            InventoryAction::InventoryStateLoaded {
                event_id,
                section,
                inventory_data,
                seat_assignments,
            } => {
                tracing::debug!(
                    "InventoryStateLoaded: event_id={}, section={}, data={:?}, seats={}",
                    event_id,
                    section,
                    inventory_data,
                    seat_assignments.len()
                );

                // Mark as loaded
                state.mark_loaded(*event_id, section.clone());

                // If data was found in projection, reconstruct the inventory
                // Note: projection returns (total_capacity, reserved, sold, available) + seat assignments
                if let Some((total, reserved, sold, _available)) = inventory_data {
                    tracing::debug!(
                        "Reconstructing inventory from projection snapshot: total={}, reserved={}, sold={}, seats={}",
                        total,
                        reserved,
                        sold,
                        seat_assignments.len()
                    );

                    let inventory = Inventory::new(*event_id, section.clone(), Capacity::new(*total));
                    let mut inventory = inventory;
                    // Note: 'available' is derived (total - reserved - sold), not stored
                    inventory.reserved = *reserved;
                    inventory.sold = *sold;

                    state.inventories.insert((*event_id, section.clone()), inventory);

                    // Load seat assignments from projection snapshot (no more placeholder generation!)
                    for assignment in seat_assignments {
                        state.seat_assignments.insert(assignment.seat_id, assignment.clone());
                    }

                    tracing::debug!(
                        "Inventory loaded from projection snapshot. State now has {} inventories and {} seat assignments",
                        state.inventories.len(),
                        state.seat_assignments.len()
                    );
                } else {
                    tracing::warn!(
                        "No inventory data found in projection for event_id={}, section={}",
                        event_id,
                        section
                    );
                }

                state.last_error = None;
            }

            // Commands and informational events don't modify state
            InventoryAction::InsufficientInventory { .. }
            | InventoryAction::InitializeInventory { .. }
            | InventoryAction::ReserveSeats { .. }
            | InventoryAction::ConfirmReservation { .. }
            | InventoryAction::ReleaseReservation { .. }
            | InventoryAction::ExpireReservation { .. }
            | InventoryAction::GetAllSections { .. }
            | InventoryAction::GetSectionAvailability { .. }
            | InventoryAction::GetTotalAvailable { .. }
            | InventoryAction::AllSectionsQueried { .. }
            | InventoryAction::SectionAvailabilityQueried { .. }
            | InventoryAction::TotalAvailableQueried { .. } => {
                // Commands don't modify state
                // Query actions and results are handled in reducer
                // InsufficientInventory is informational - no state change needed
                // Don't clear last_error - this represents a failure condition
            }
        }
    }
}

impl Default for InventoryReducer {
    fn default() -> Self {
        Self::new()
    }
}

impl Reducer for InventoryReducer {
    type State = InventoryState;
    type Action = InventoryAction;
    type Environment = InventoryEnvironment;

    #[allow(clippy::too_many_lines)] // Complex business logic required
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ========== Commands ==========
            InventoryAction::InitializeInventory {
                event_id,
                section,
                capacity,
                seat_numbers,
            } => {
                // Validate
                if let Err(error) =
                    Self::validate_initialize_inventory(state, &event_id, &section, capacity)
                {
                    Self::apply_event(state, &InventoryAction::ValidationFailed { error });
                    return SmallVec::new();
                }

                // Create seat IDs
                let seat_count = capacity.value();
                let seats: Vec<SeatId> = (0..seat_count).map(|_| SeatId::new()).collect();

                // In a real system, we'd use specific seat numbers if provided
                let _ = seat_numbers;

                // Create and apply event
                let event = InventoryAction::InventoryInitialized {
                    event_id,
                    section,
                    capacity,
                    seats,
                    initialized_at: env.clock.now(),
                };
                Self::apply_event(state, &event);

                // Serialize event
                let ticketing_event = TicketingEvent::Inventory(event);
                let serialized = match ticketing_event.serialize() {
                    Ok(s) => s,
                    Err(e) => {
                        Self::apply_event(state, &InventoryAction::ValidationFailed { error: e });
                        return SmallVec::new();
                    }
                };

                // Return effects for persistence and publishing
                smallvec![
                    append_events! {
                        store: env.event_store,
                        stream: env.stream_id.as_str(),
                        expected_version: None,
                        events: vec![serialized.clone()],
                        on_success: |_version| None,
                        on_error: |error| Some(InventoryAction::ValidationFailed {
                            error: error.to_string()
                        })
                    },
                    publish_event! {
                        bus: env.event_bus,
                        topic: "inventory",
                        event: serialized,
                        on_success: || None,
                        on_error: |error| Some(InventoryAction::ValidationFailed {
                            error: error.to_string()
                        })
                    }
                ]
            }

            InventoryAction::ReserveSeats {
                reservation_id,
                event_id,
                section,
                quantity,
                specific_seats,
                expires_at,
            } => {
                tracing::debug!(
                    "ReserveSeats: reservation_id={}, event_id={}, section={}, quantity={}, state.inventories.len()={}",
                    reservation_id,
                    event_id,
                    section,
                    quantity,
                    state.inventories.len()
                );

                // Check if state has been loaded from projection
                if !state.is_loaded(&event_id, &section) {
                    tracing::debug!(
                        "State not loaded for event_id={}, section={}. Triggering load from projection.",
                        event_id,
                        section
                    );

                    // Mark as loading to prevent duplicate load requests
                    state.mark_loading(event_id, section.clone());

                    // Create effect to load state from projection
                    let projection = env.projection.clone();
                    let event_id_copy = event_id;
                    let section_copy = section.clone();
                    let original_command = InventoryAction::ReserveSeats {
                        reservation_id,
                        event_id,
                        section: section.clone(),
                        quantity,
                        specific_seats,
                        expires_at,
                    };

                    // Use Sequential to: 1) load state, 2) retry original command
                    return smallvec![Effect::Sequential(vec![
                        Effect::Future(Box::pin(async move {
                            // Load inventory data from projection
                            let result = projection
                                .load_inventory(&event_id_copy, &section_copy)
                                .await
                                .ok()
                                .flatten();

                            // Destructure into counts and seat assignments
                            let (inventory_data, seat_assignments) = match result {
                                Some((counts, seats)) => (Some(counts), seats),
                                None => (None, Vec::new()),
                            };

                            // Return StateLoaded event with complete snapshot
                            Some(InventoryAction::InventoryStateLoaded {
                                event_id: event_id_copy,
                                section: section_copy,
                                inventory_data,
                                seat_assignments,
                            })
                        })),
                        // After state is loaded, retry the original command
                        Effect::Future(Box::pin(async move {
                            Some(original_command)
                        })),
                    ])];
                }

                tracing::debug!(
                    "State already loaded. Proceeding with validation. Has inventory: {}",
                    state.get_inventory(&event_id, &section).is_some()
                );

                // Validate
                if let Err(error) =
                    Self::validate_reserve_seats(state, &event_id, &section, quantity)
                {
                    tracing::warn!(
                        "Validation failed for ReserveSeats: {}",
                        error
                    );

                    // Emit ValidationFailed event
                    Self::apply_event(state, &InventoryAction::ValidationFailed {
                        error: error.clone(),
                    });

                    // Also emit InsufficientInventory for saga coordination
                    if error.contains("Insufficient inventory") {
                        if let Some(inventory) = state.get_inventory(&event_id, &section) {
                            let event = InventoryAction::InsufficientInventory {
                                event_id,
                                section,
                                requested: quantity,
                                available: inventory.available(),
                            };
                            Self::apply_event(state, &event);
                        }
                    }

                    return SmallVec::new();
                }

                tracing::debug!(
                    "Validation passed. Creating SeatsReserved event."
                );

                // Select seats
                let seats = if let Some(_specific) = specific_seats {
                    // In a real system, validate and use specific seats
                    // For now, just use general admission
                    Self::select_available_seats(state, &event_id, &section, quantity)
                } else {
                    Self::select_available_seats(state, &event_id, &section, quantity)
                };

                // Create and apply event
                let event = InventoryAction::SeatsReserved {
                    reservation_id,
                    event_id,
                    section: section.clone(),
                    seats,
                    expires_at,
                    reserved_at: env.clock.now(),
                };
                Self::apply_event(state, &event);

                // Serialize event
                let ticketing_event = TicketingEvent::Inventory(event);
                let serialized = match ticketing_event.serialize() {
                    Ok(s) => s,
                    Err(e) => {
                        Self::apply_event(state, &InventoryAction::ValidationFailed { error: e });
                        return SmallVec::new();
                    }
                };

                // Calculate timeout duration
                let now = env.clock.now();
                let timeout_duration = if expires_at > now {
                    let diff = expires_at - now;
                    #[allow(clippy::cast_sign_loss)]
                    std::time::Duration::from_secs(diff.num_seconds() as u64)
                } else {
                    std::time::Duration::from_secs(0)
                };

                // Return effects: persist, publish, and schedule expiration
                smallvec![
                    append_events! {
                        store: env.event_store,
                        stream: env.stream_id.as_str(),
                        expected_version: None,
                        events: vec![serialized.clone()],
                        on_success: |_version| None,
                        on_error: |error| Some(InventoryAction::ValidationFailed {
                            error: error.to_string()
                        })
                    },
                    publish_event! {
                        bus: env.event_bus,
                        topic: "inventory",
                        event: serialized,
                        on_success: || None,
                        on_error: |error| Some(InventoryAction::ValidationFailed {
                            error: error.to_string()
                        })
                    },
                    delay! {
                        duration: timeout_duration,
                        action: InventoryAction::ExpireReservation { reservation_id }
                    }
                ]
            }

            InventoryAction::ConfirmReservation {
                reservation_id,
                customer_id,
            } => {
                // Find seats for this reservation
                let seats = Self::find_seats_by_reservation(state, &reservation_id);

                if seats.is_empty() {
                    Self::apply_event(
                        state,
                        &InventoryAction::ValidationFailed {
                            error: format!("No seats found for reservation {reservation_id}"),
                        },
                    );
                    return SmallVec::new();
                }

                // Find event_id and section for this reservation
                let Some((event_id, section)) = Self::find_reservation_location(state, &reservation_id) else {
                    Self::apply_event(
                        state,
                        &InventoryAction::ValidationFailed {
                            error: format!("Could not find location for reservation {reservation_id}"),
                        },
                    );
                    return SmallVec::new();
                };

                // Create and apply event
                let event = InventoryAction::SeatsConfirmed {
                    reservation_id,
                    event_id,
                    section,
                    customer_id,
                    seats,
                    confirmed_at: env.clock.now(),
                };
                Self::apply_event(state, &event);

                Self::create_effects(event, env)
            }

            InventoryAction::ReleaseReservation { reservation_id } => {
                // Find seats for this reservation
                let seats = Self::find_seats_by_reservation(state, &reservation_id);

                if seats.is_empty() {
                    // Silently ignore - reservation might have already been released
                    return SmallVec::new();
                }

                // Find event_id and section for this reservation
                let Some((event_id, section)) = Self::find_reservation_location(state, &reservation_id) else {
                    // Silently ignore - reservation might have already been released
                    return SmallVec::new();
                };

                // Create and apply event
                let event = InventoryAction::SeatsReleased {
                    reservation_id,
                    event_id,
                    section,
                    seats,
                    released_at: env.clock.now(),
                };
                Self::apply_event(state, &event);

                Self::create_effects(event, env)
            }

            InventoryAction::ExpireReservation { reservation_id } => {
                // Same as release for now
                // In production, might have different analytics/metrics
                let seats = Self::find_seats_by_reservation(state, &reservation_id);

                if seats.is_empty() {
                    return SmallVec::new();
                }

                // Find event_id and section for this reservation
                let Some((event_id, section)) = Self::find_reservation_location(state, &reservation_id) else {
                    return SmallVec::new();
                };

                let event = InventoryAction::SeatsReleased {
                    reservation_id,
                    event_id,
                    section,
                    seats,
                    released_at: env.clock.now(),
                };
                Self::apply_event(state, &event);

                Self::create_effects(event, env)
            }

            // ========== Query Actions ==========
            InventoryAction::GetAllSections { event_id } => {
                let projection = env.projection.clone();
                let event_id_clone = event_id;
                smallvec![Effect::Future(Box::pin(async move {
                    match projection.get_all_sections(&event_id_clone).await {
                        Ok(sections) => Some(InventoryAction::AllSectionsQueried {
                            event_id: event_id_clone,
                            sections,
                        }),
                        Err(e) => Some(InventoryAction::ValidationFailed {
                            error: format!("Failed to query sections: {e}"),
                        }),
                    }
                }))]
            }

            InventoryAction::GetSectionAvailability { event_id, section } => {
                let projection = env.projection.clone();
                let event_id_clone = event_id;
                let section_clone = section.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    match projection.get_section_availability(&event_id_clone, &section_clone).await {
                        Ok(data) => Some(InventoryAction::SectionAvailabilityQueried {
                            event_id: event_id_clone,
                            section: section_clone,
                            data,
                        }),
                        Err(e) => Some(InventoryAction::ValidationFailed {
                            error: format!("Failed to query section availability: {e}"),
                        }),
                    }
                }))]
            }

            InventoryAction::GetTotalAvailable { event_id } => {
                let projection = env.projection.clone();
                let event_id_clone = event_id;
                smallvec![Effect::Future(Box::pin(async move {
                    match projection.get_total_available(&event_id_clone).await {
                        Ok(total_available) => Some(InventoryAction::TotalAvailableQueried {
                            event_id: event_id_clone,
                            total_available,
                        }),
                        Err(e) => Some(InventoryAction::ValidationFailed {
                            error: format!("Failed to query total available: {e}"),
                        }),
                    }
                }))]
            }

            // ========== Events (from event store replay) ==========
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
    use composable_rust_testing::{mocks::{InMemoryEventBus, InMemoryEventStore}, ReducerTest};

    // Mock projection query for tests
    #[derive(Clone)]
    struct MockInventoryQuery;

    impl InventoryProjectionQuery for MockInventoryQuery {
        fn load_inventory(
            &self,
            _event_id: &EventId,
            _section: &str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<((u32, u32, u32, u32), Vec<SeatAssignment>)>, String>> + Send + '_>> {
            // Return None for tests - state will be built from events
            Box::pin(async move { Ok(None) })
        }

        fn get_all_sections(
            &self,
            _event_id: &EventId,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<SectionAvailabilityData>, String>> + Send + '_>> {
            // Return empty list for tests
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn get_section_availability(
            &self,
            _event_id: &EventId,
            _section: &str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<SectionAvailabilityData>, String>> + Send + '_>> {
            // Return None for tests
            Box::pin(async move { Ok(None) })
        }

        fn get_total_available(
            &self,
            _event_id: &EventId,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u32, String>> + Send + '_>> {
            // Return 0 for tests
            Box::pin(async move { Ok(0) })
        }
    }

    fn create_test_env() -> InventoryEnvironment {
        InventoryEnvironment::new(
            Arc::new(SystemClock),
            Arc::new(InMemoryEventStore::new()),
            Arc::new(InMemoryEventBus::new()),
            StreamId::new("inventory-test"),
            Arc::new(MockInventoryQuery),
        )
    }

    #[test]
    fn test_initialize_inventory() {
        let event_id = EventId::new();

        ReducerTest::new(InventoryReducer::new())
            .with_env(create_test_env())
            .given_state(InventoryState::new())
            .when_action(InventoryAction::InitializeInventory {
                event_id,
                section: "General".to_string(),
                capacity: Capacity::new(100),
                seat_numbers: None,
            })
            .then_state(move |state| {
                assert_eq!(state.count_inventories(), 1);
                let inventory = state.get_inventory(&event_id, "General").unwrap();
                assert_eq!(inventory.total_capacity.value(), 100);
                assert_eq!(inventory.available(), 100);
                assert_eq!(inventory.reserved, 0);
                assert_eq!(inventory.sold, 0);
                assert_eq!(state.count_seats(), 100);
            })
            .then_effects(|effects| {
                // Should return 2 effects: AppendEvents + PublishEvent
                assert_eq!(effects.len(), 2);
            })
            .run();
    }

    #[test]
    fn test_reserve_seats_success() {
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();

        ReducerTest::new(InventoryReducer::new())
            .with_env(create_test_env())
            .given_state({
                // Initialize inventory first
                let mut state = InventoryState::new();
                let reducer = InventoryReducer::new();
                let env = create_test_env();
                reducer.reduce(
                    &mut state,
                    InventoryAction::InitializeInventory {
                        event_id,
                        section: "General".to_string(),
                        capacity: Capacity::new(100),
                        seat_numbers: None,
                    },
                    &env,
                );
                // Mark state as loaded to avoid load-then-process flow
                state.mark_loaded(event_id, "General".to_string());
                state
            })
            .when_action(InventoryAction::ReserveSeats {
                reservation_id,
                event_id,
                section: "General".to_string(),
                quantity: 2,
                specific_seats: None,
                expires_at: Utc::now() + chrono::Duration::minutes(5),
            })
            .then_state(move |state| {
                let inventory = state.get_inventory(&event_id, "General").unwrap();
                assert_eq!(inventory.reserved, 2);
                assert_eq!(inventory.sold, 0);
                assert_eq!(inventory.available(), 98);
            })
            .then_effects(|effects| {
                // Should return 3 effects: AppendEvents + PublishEvent + Delay (for expiration)
                assert_eq!(effects.len(), 3);
            })
            .run();
    }

    #[test]
    fn test_reserve_seats_insufficient_inventory() {
        let event_id = EventId::new();

        ReducerTest::new(InventoryReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = InventoryState::new();
                let reducer = InventoryReducer::new();
                let env = create_test_env();
                // Initialize with only 5 seats
                reducer.reduce(
                    &mut state,
                    InventoryAction::InitializeInventory {
                        event_id,
                        section: "VIP".to_string(),
                        capacity: Capacity::new(5),
                        seat_numbers: None,
                    },
                    &env,
                );
                // Mark state as loaded to avoid load-then-process flow
                state.mark_loaded(event_id, "VIP".to_string());
                state
            })
            .when_action(InventoryAction::ReserveSeats {
                reservation_id: ReservationId::new(),
                event_id,
                section: "VIP".to_string(),
                quantity: 10, // More than available
                specific_seats: None,
                expires_at: Utc::now() + chrono::Duration::minutes(5),
            })
            .then_state(move |state| {
                // No seats should be reserved
                let inventory = state.get_inventory(&event_id, "VIP").unwrap();
                assert_eq!(inventory.reserved, 0);
                assert!(state.last_error.is_some());
            })
            .then_effects(|effects| {
                // Validation failure - no effects
                assert_eq!(effects.len(), 0);
            })
            .run();
    }

    #[test]
    fn test_confirm_reservation() {
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

        ReducerTest::new(InventoryReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = InventoryState::new();
                let reducer = InventoryReducer::new();
                let env = create_test_env();

                // Initialize and reserve
                reducer.reduce(
                    &mut state,
                    InventoryAction::InitializeInventory {
                        event_id,
                        section: "General".to_string(),
                        capacity: Capacity::new(100),
                        seat_numbers: None,
                    },
                    &env,
                );
                // Mark state as loaded
                state.mark_loaded(event_id, "General".to_string());
                reducer.reduce(
                    &mut state,
                    InventoryAction::ReserveSeats {
                        reservation_id,
                        event_id,
                        section: "General".to_string(),
                        quantity: 2,
                        specific_seats: None,
                        expires_at: Utc::now() + chrono::Duration::minutes(5),
                    },
                    &env,
                );
                state
            })
            .when_action(InventoryAction::ConfirmReservation {
                reservation_id,
                customer_id,
            })
            .then_state(move |state| {
                let inventory = state.get_inventory(&event_id, "General").unwrap();
                assert_eq!(inventory.reserved, 0); // Moved from reserved to sold
                assert_eq!(inventory.sold, 2);
                assert_eq!(inventory.available(), 98);
            })
            .then_effects(|effects| {
                // Should return 2 effects: AppendEvents + PublishEvent
                assert_eq!(effects.len(), 2);
            })
            .run();
    }

    #[test]
    fn test_release_reservation() {
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();

        ReducerTest::new(InventoryReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = InventoryState::new();
                let reducer = InventoryReducer::new();
                let env = create_test_env();

                // Initialize and reserve
                reducer.reduce(
                    &mut state,
                    InventoryAction::InitializeInventory {
                        event_id,
                        section: "General".to_string(),
                        capacity: Capacity::new(100),
                        seat_numbers: None,
                    },
                    &env,
                );
                // Mark state as loaded
                state.mark_loaded(event_id, "General".to_string());
                reducer.reduce(
                    &mut state,
                    InventoryAction::ReserveSeats {
                        reservation_id,
                        event_id,
                        section: "General".to_string(),
                        quantity: 2,
                        specific_seats: None,
                        expires_at: Utc::now() + chrono::Duration::minutes(5),
                    },
                    &env,
                );
                state
            })
            .when_action(InventoryAction::ReleaseReservation { reservation_id })
            .then_state(move |state| {
                let inventory = state.get_inventory(&event_id, "General").unwrap();
                assert_eq!(inventory.reserved, 0); // Back to available
                assert_eq!(inventory.sold, 0);
                assert_eq!(inventory.available(), 100);
            })
            .then_effects(|effects| {
                // Should return 2 effects: AppendEvents + PublishEvent
                assert_eq!(effects.len(), 2);
            })
            .run();
    }

    #[test]
    fn test_last_seat_race_condition() {
        // This test simulates the critical "last seat" scenario
        let event_id = EventId::new();
        let reservation1 = ReservationId::new();
        let reservation2 = ReservationId::new();

        let mut state = InventoryState::new();
        let reducer = InventoryReducer::new();
        let env = create_test_env();

        // Initialize with only 1 seat
        reducer.reduce(
            &mut state,
            InventoryAction::InitializeInventory {
                event_id,
                section: "VIP".to_string(),
                capacity: Capacity::new(1),
                seat_numbers: None,
            },
            &env,
        );

        // Mark state as loaded
        state.mark_loaded(event_id, "VIP".to_string());

        // First reservation gets the seat
        reducer.reduce(
            &mut state,
            InventoryAction::ReserveSeats {
                reservation_id: reservation1,
                event_id,
                section: "VIP".to_string(),
                quantity: 1,
                specific_seats: None,
                expires_at: Utc::now() + chrono::Duration::minutes(5),
            },
            &env,
        );

        let inventory = state.get_inventory(&event_id, "VIP").unwrap();
        assert_eq!(inventory.reserved, 1);
        assert_eq!(inventory.available(), 0);

        // Second reservation should fail (no seats available)
        reducer.reduce(
            &mut state,
            InventoryAction::ReserveSeats {
                reservation_id: reservation2,
                event_id,
                section: "VIP".to_string(),
                quantity: 1,
                specific_seats: None,
                expires_at: Utc::now() + chrono::Duration::minutes(5),
            },
            &env,
        );

        // Verify: still only 1 reserved, not 2 (no double-booking)
        let inventory = state.get_inventory(&event_id, "VIP").unwrap();
        assert_eq!(inventory.reserved, 1); // CRITICAL: Not 2!
        assert_eq!(inventory.sold, 0);
        assert!(state.last_error.is_some());
    }
}
