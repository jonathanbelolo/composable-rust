//! Inventory aggregate for the Event Ticketing System.
//!
//! Manages seat availability and reservations. This aggregate is CRITICAL for preventing
//! double-booking in high-concurrency scenarios (the "last seat" problem).
//!
//! **Concurrency Strategy**: Optimistic concurrency control - check available seats including
//! reserved count to prevent overselling during concurrent reservation attempts.

use crate::types::{
    Capacity, CustomerId, EventId, Inventory, InventoryState, ReservationId, SeatAssignment,
    SeatId, SeatNumber, SeatStatus,
};
use chrono::{DateTime, Utc};
use composable_rust_core::{
    effect::Effect, environment::Clock, reducer::Reducer, SmallVec,
};
use composable_rust_macros::Action;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
}

// ============================================================================
// Environment
// ============================================================================

/// Environment dependencies for the Inventory aggregate
#[derive(Clone)]
pub struct InventoryEnvironment {
    /// Clock for timestamps
    pub clock: Arc<dyn Clock>,
}

impl InventoryEnvironment {
    /// Creates a new `InventoryEnvironment`
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock }
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

    /// Finds event_id and section for a reservation
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

            // Commands and informational events don't modify state
            InventoryAction::InsufficientInventory { .. }
            | InventoryAction::InitializeInventory { .. }
            | InventoryAction::ReserveSeats { .. }
            | InventoryAction::ConfirmReservation { .. }
            | InventoryAction::ReleaseReservation { .. }
            | InventoryAction::ExpireReservation { .. } => {
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

                SmallVec::new()
            }

            InventoryAction::ReserveSeats {
                reservation_id,
                event_id,
                section,
                quantity,
                specific_seats,
                expires_at,
            } => {
                // Validate
                if let Err(error) =
                    Self::validate_reserve_seats(state, &event_id, &section, quantity)
                {
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
                    section,
                    seats,
                    expires_at,
                    reserved_at: env.clock.now(),
                };
                Self::apply_event(state, &event);

                SmallVec::new()
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

                SmallVec::new()
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

                SmallVec::new()
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

                SmallVec::new()
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
    use composable_rust_testing::{assertions, ReducerTest};

    fn create_test_env() -> InventoryEnvironment {
        InventoryEnvironment::new(Arc::new(SystemClock))
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
            .then_effects(assertions::assert_no_effects)
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
            .then_effects(assertions::assert_no_effects)
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
            .then_effects(assertions::assert_no_effects)
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
            .then_effects(assertions::assert_no_effects)
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
            .then_effects(assertions::assert_no_effects)
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
