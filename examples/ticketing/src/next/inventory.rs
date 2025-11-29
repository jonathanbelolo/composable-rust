//! Inventory aggregate business logic.
//!
//! Pure domain logic for managing seat inventory. This aggregate is CRITICAL for
//! preventing double-booking in high-concurrency scenarios.
//!
//! # Commands
//!
//! - `InitializeInventory`: Set up inventory for an event section
//! - `ReserveSeats`: Reserve seats for a customer (with expiration)
//! - `ConfirmReservation`: Convert reservation to sold
//! - `ReleaseReservation`: Return reserved seats to available pool
//!
//! # Concurrency Strategy
//!
//! Optimistic concurrency control via event store versioning. When two concurrent
//! reservations try to get the last seats, one will fail with `VersionConflict`.

use chrono::{DateTime, Utc};
use composable_rust_next::{BusinessLogic, BusinessResult, Clock, StreamId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;

use crate::types::{Capacity, CustomerId, EventId, ReservationId, SeatId};

// ═══════════════════════════════════════════════════════════════════════════
// Commands
// ═══════════════════════════════════════════════════════════════════════════

/// Commands for the Inventory aggregate.
#[derive(Debug, Clone)]
pub enum InventoryCommand {
    /// Initialize inventory for an event section.
    Initialize {
        /// Event ID this inventory belongs to
        event_id: EventId,
        /// Section name (e.g., "VIP", "General Admission")
        section: String,
        /// Total capacity for this section
        capacity: Capacity,
    },

    /// Reserve seats for a pending purchase.
    Reserve {
        /// Unique reservation ID
        reservation_id: ReservationId,
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
        /// Number of seats to reserve
        quantity: u32,
        /// When this reservation expires (if not confirmed)
        expires_at: DateTime<Utc>,
    },

    /// Confirm a reservation (mark as sold).
    Confirm {
        /// Reservation to confirm
        reservation_id: ReservationId,
        /// Customer who purchased
        customer_id: CustomerId,
    },

    /// Release a reservation (return seats to available pool).
    Release {
        /// Reservation to release
        reservation_id: ReservationId,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// Events
// ═══════════════════════════════════════════════════════════════════════════

/// Domain events for the Inventory aggregate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InventoryEvent {
    /// Inventory initialized for a section.
    Initialized {
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
        /// Total capacity
        capacity: u32,
        /// Generated seat IDs
        seats: Vec<SeatId>,
        /// When initialized
        initialized_at: DateTime<Utc>,
    },

    /// Seats reserved for a customer.
    SeatsReserved {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
        /// Reserved seat IDs
        seats: Vec<SeatId>,
        /// When reservation expires
        expires_at: DateTime<Utc>,
        /// When reserved
        reserved_at: DateTime<Utc>,
    },

    /// Reservation confirmed (seats sold).
    SeatsConfirmed {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Customer ID
        customer_id: CustomerId,
        /// Confirmed seat IDs
        seats: Vec<SeatId>,
        /// When confirmed
        confirmed_at: DateTime<Utc>,
    },

    /// Reservation released (seats returned to pool).
    SeatsReleased {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Released seat IDs
        seats: Vec<SeatId>,
        /// When released
        released_at: DateTime<Utc>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// State
// ═══════════════════════════════════════════════════════════════════════════

/// Status of a seat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeatStatus {
    /// Available for reservation
    Available,
    /// Reserved (pending payment)
    Reserved {
        /// Reservation ID
        reservation_id: ReservationId,
        /// When reservation expires
        expires_at: DateTime<Utc>,
    },
    /// Sold (payment confirmed)
    Sold {
        /// Customer who purchased
        customer_id: CustomerId,
    },
}

/// Reservation tracking.
#[derive(Debug, Clone)]
pub struct Reservation {
    /// Reserved seat IDs
    pub seats: Vec<SeatId>,
    /// When reservation expires
    pub expires_at: DateTime<Utc>,
}

/// State for the Inventory aggregate.
#[derive(Debug, Clone, Default)]
pub struct InventoryState {
    /// Event ID this inventory belongs to
    pub event_id: Option<EventId>,
    /// Section name
    pub section: Option<String>,
    /// Total capacity
    pub capacity: u32,
    /// Individual seat statuses
    pub seats: HashMap<SeatId, SeatStatus>,
    /// Active reservations
    pub reservations: HashMap<ReservationId, Reservation>,
    /// Whether inventory has been initialized
    pub initialized: bool,
}

impl InventoryState {
    /// Count available seats.
    #[must_use]
    pub fn available_count(&self) -> u32 {
        self.seats
            .values()
            .filter(|s| matches!(s, SeatStatus::Available))
            .count() as u32
    }

    /// Count reserved seats.
    #[must_use]
    pub fn reserved_count(&self) -> u32 {
        self.seats
            .values()
            .filter(|s| matches!(s, SeatStatus::Reserved { .. }))
            .count() as u32
    }

    /// Count sold seats.
    #[must_use]
    pub fn sold_count(&self) -> u32 {
        self.seats
            .values()
            .filter(|s| matches!(s, SeatStatus::Sold { .. }))
            .count() as u32
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Errors
// ═══════════════════════════════════════════════════════════════════════════

/// Business errors for the Inventory aggregate.
#[derive(Debug, Clone, thiserror::Error)]
pub enum InventoryError {
    /// Inventory already initialized
    #[error("Inventory already initialized for this section")]
    AlreadyInitialized,

    /// Inventory not initialized
    #[error("Inventory not initialized")]
    NotInitialized,

    /// Not enough seats available
    #[error("Insufficient inventory: requested {requested}, available {available}")]
    InsufficientInventory {
        /// Requested quantity
        requested: u32,
        /// Actually available
        available: u32,
    },

    /// Reservation not found
    #[error("Reservation not found: {0}")]
    ReservationNotFound(ReservationId),

    /// Validation failed
    #[error("Validation failed: {0}")]
    ValidationFailed(String),
}

// ═══════════════════════════════════════════════════════════════════════════
// Business Logic
// ═══════════════════════════════════════════════════════════════════════════

/// Pure business logic for the Inventory aggregate.
#[derive(Debug, Clone)]
pub struct InventoryBusinessLogic;

impl BusinessLogic for InventoryBusinessLogic {
    type State = InventoryState;
    type Input = InventoryCommand;
    type Event = InventoryEvent;
    type Error = InventoryError;
    type Call = Infallible;
    type CallResult = Infallible;

    fn stream_id(input: &Self::Input) -> StreamId {
        match input {
            InventoryCommand::Initialize { event_id, section, .. } => {
                StreamId::new(format!("inventory-{}-{}", event_id.as_uuid(), section))
            }
            InventoryCommand::Reserve { event_id, section, .. } => {
                StreamId::new(format!("inventory-{}-{}", event_id.as_uuid(), section))
            }
            InventoryCommand::Confirm { reservation_id, .. } => {
                // For confirm/release, we need the event_id and section from the reservation
                // In practice, the caller knows these - we'll use a placeholder pattern
                // Real implementation would include event_id and section in the command
                StreamId::new(format!("inventory-reservation-{}", reservation_id.as_uuid()))
            }
            InventoryCommand::Release { reservation_id, .. } => {
                StreamId::new(format!("inventory-reservation-{}", reservation_id.as_uuid()))
            }
        }
    }

    fn process(
        &self,
        state: &Self::State,
        input: Self::Input,
        clock: &dyn Clock,
    ) -> Result<BusinessResult<Self::Event, Self::Call>, Self::Error> {
        let now = clock.now();

        match input {
            InventoryCommand::Initialize {
                event_id,
                section,
                capacity,
            } => {
                // Validation
                if state.initialized {
                    return Err(InventoryError::AlreadyInitialized);
                }
                if capacity.value() == 0 {
                    return Err(InventoryError::ValidationFailed(
                        "Capacity must be greater than 0".to_string(),
                    ));
                }

                // Generate seat IDs
                let seats: Vec<SeatId> = (0..capacity.value())
                    .map(|_| SeatId::new())
                    .collect();

                Ok(BusinessResult::Done(vec![InventoryEvent::Initialized {
                    event_id,
                    section,
                    capacity: capacity.value(),
                    seats,
                    initialized_at: now,
                }]))
            }

            InventoryCommand::Reserve {
                reservation_id,
                event_id,
                section,
                quantity,
                expires_at,
            } => {
                // Validation
                if !state.initialized {
                    return Err(InventoryError::NotInitialized);
                }

                let available = state.available_count();
                if available < quantity {
                    return Err(InventoryError::InsufficientInventory {
                        requested: quantity,
                        available,
                    });
                }

                // Find available seats to reserve
                let seats: Vec<SeatId> = state
                    .seats
                    .iter()
                    .filter(|(_, status)| matches!(status, SeatStatus::Available))
                    .take(quantity as usize)
                    .map(|(id, _)| *id)
                    .collect();

                Ok(BusinessResult::Done(vec![InventoryEvent::SeatsReserved {
                    reservation_id,
                    event_id,
                    section,
                    seats,
                    expires_at,
                    reserved_at: now,
                }]))
            }

            InventoryCommand::Confirm {
                reservation_id,
                customer_id,
            } => {
                let reservation = state
                    .reservations
                    .get(&reservation_id)
                    .ok_or(InventoryError::ReservationNotFound(reservation_id))?;

                Ok(BusinessResult::Done(vec![InventoryEvent::SeatsConfirmed {
                    reservation_id,
                    customer_id,
                    seats: reservation.seats.clone(),
                    confirmed_at: now,
                }]))
            }

            InventoryCommand::Release { reservation_id } => {
                let reservation = state
                    .reservations
                    .get(&reservation_id)
                    .ok_or(InventoryError::ReservationNotFound(reservation_id))?;

                Ok(BusinessResult::Done(vec![InventoryEvent::SeatsReleased {
                    reservation_id,
                    seats: reservation.seats.clone(),
                    released_at: now,
                }]))
            }
        }
    }

    fn apply(&self, state: &mut Self::State, event: &Self::Event) {
        match event {
            InventoryEvent::Initialized {
                event_id,
                section,
                capacity,
                seats,
                ..
            } => {
                state.event_id = Some(*event_id);
                state.section = Some(section.clone());
                state.capacity = *capacity;
                state.initialized = true;

                for seat_id in seats {
                    state.seats.insert(*seat_id, SeatStatus::Available);
                }
            }

            InventoryEvent::SeatsReserved {
                reservation_id,
                seats,
                expires_at,
                ..
            } => {
                for seat_id in seats {
                    state.seats.insert(
                        *seat_id,
                        SeatStatus::Reserved {
                            reservation_id: *reservation_id,
                            expires_at: *expires_at,
                        },
                    );
                }

                state.reservations.insert(
                    *reservation_id,
                    Reservation {
                        seats: seats.clone(),
                        expires_at: *expires_at,
                    },
                );
            }

            InventoryEvent::SeatsConfirmed {
                reservation_id,
                customer_id,
                seats,
                ..
            } => {
                for seat_id in seats {
                    state.seats.insert(
                        *seat_id,
                        SeatStatus::Sold {
                            customer_id: *customer_id,
                        },
                    );
                }

                state.reservations.remove(reservation_id);
            }

            InventoryEvent::SeatsReleased {
                reservation_id,
                seats,
                ..
            } => {
                for seat_id in seats {
                    state.seats.insert(*seat_id, SeatStatus::Available);
                }

                state.reservations.remove(reservation_id);
            }
        }
    }

    fn event_type_name(event: &Self::Event) -> &'static str {
        match event {
            InventoryEvent::Initialized { .. } => "InventoryInitialized",
            InventoryEvent::SeatsReserved { .. } => "SeatsReserved",
            InventoryEvent::SeatsConfirmed { .. } => "SeatsConfirmed",
            InventoryEvent::SeatsReleased { .. } => "SeatsReleased",
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)] // Test code can use unwrap/panic
mod tests {
    use super::*;
    use composable_rust_next::FixedClock;

    fn test_clock() -> FixedClock {
        FixedClock::new(Utc::now())
    }

    #[test]
    fn initialize_inventory_succeeds() {
        let logic = InventoryBusinessLogic;
        let state = InventoryState::default();
        let clock = test_clock();

        let command = InventoryCommand::Initialize {
            event_id: EventId::new(),
            section: "VIP".to_string(),
            capacity: Capacity::new(100),
        };

        let result = logic.process(&state, command, &clock).unwrap();

        match result {
            BusinessResult::Done(events) => {
                assert_eq!(events.len(), 1);
                match &events[0] {
                    InventoryEvent::Initialized { capacity, seats, .. } => {
                        assert_eq!(*capacity, 100);
                        assert_eq!(seats.len(), 100);
                    }
                    _ => panic!("Expected Initialized event"),
                }
            }
            _ => panic!("Expected Done result"),
        }
    }

    #[test]
    fn initialize_already_initialized_fails() {
        let logic = InventoryBusinessLogic;
        let mut state = InventoryState::default();
        state.initialized = true;
        let clock = test_clock();

        let command = InventoryCommand::Initialize {
            event_id: EventId::new(),
            section: "VIP".to_string(),
            capacity: Capacity::new(100),
        };

        let result = logic.process(&state, command, &clock);

        assert!(matches!(result, Err(InventoryError::AlreadyInitialized)));
    }

    #[test]
    fn reserve_seats_succeeds() {
        let logic = InventoryBusinessLogic;
        let clock = test_clock();

        // Initialize first
        let mut state = InventoryState::default();
        let init_cmd = InventoryCommand::Initialize {
            event_id: EventId::new(),
            section: "VIP".to_string(),
            capacity: Capacity::new(100),
        };
        let init_result = logic.process(&state, init_cmd, &clock).unwrap();
        if let BusinessResult::Done(events) = init_result {
            for event in &events {
                logic.apply(&mut state, event);
            }
        }

        // Now reserve
        let reserve_cmd = InventoryCommand::Reserve {
            reservation_id: ReservationId::new(),
            event_id: EventId::new(),
            section: "VIP".to_string(),
            quantity: 5,
            expires_at: Utc::now() + chrono::Duration::minutes(15),
        };

        let result = logic.process(&state, reserve_cmd, &clock).unwrap();

        match result {
            BusinessResult::Done(events) => {
                assert_eq!(events.len(), 1);
                match &events[0] {
                    InventoryEvent::SeatsReserved { seats, .. } => {
                        assert_eq!(seats.len(), 5);
                    }
                    _ => panic!("Expected SeatsReserved event"),
                }
            }
            _ => panic!("Expected Done result"),
        }
    }

    #[test]
    fn reserve_insufficient_inventory_fails() {
        let logic = InventoryBusinessLogic;
        let clock = test_clock();

        // Initialize with small capacity
        let mut state = InventoryState::default();
        let init_cmd = InventoryCommand::Initialize {
            event_id: EventId::new(),
            section: "VIP".to_string(),
            capacity: Capacity::new(5),
        };
        let init_result = logic.process(&state, init_cmd, &clock).unwrap();
        if let BusinessResult::Done(events) = init_result {
            for event in &events {
                logic.apply(&mut state, event);
            }
        }

        // Try to reserve more than available
        let reserve_cmd = InventoryCommand::Reserve {
            reservation_id: ReservationId::new(),
            event_id: EventId::new(),
            section: "VIP".to_string(),
            quantity: 10, // More than capacity of 5
            expires_at: Utc::now() + chrono::Duration::minutes(15),
        };

        let result = logic.process(&state, reserve_cmd, &clock);

        assert!(matches!(
            result,
            Err(InventoryError::InsufficientInventory {
                requested: 10,
                available: 5
            })
        ));
    }

    #[test]
    fn apply_initialized_updates_state() {
        let logic = InventoryBusinessLogic;
        let mut state = InventoryState::default();

        let event_id = EventId::new();
        let seats: Vec<SeatId> = (0..3).map(|_| SeatId::new()).collect();

        let event = InventoryEvent::Initialized {
            event_id,
            section: "GA".to_string(),
            capacity: 3,
            seats: seats.clone(),
            initialized_at: Utc::now(),
        };

        logic.apply(&mut state, &event);

        assert!(state.initialized);
        assert_eq!(state.event_id, Some(event_id));
        assert_eq!(state.section, Some("GA".to_string()));
        assert_eq!(state.capacity, 3);
        assert_eq!(state.seats.len(), 3);
        assert_eq!(state.available_count(), 3);
    }
}
