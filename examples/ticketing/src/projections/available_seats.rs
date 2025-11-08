//! Available seats projection for fast seat availability queries.
//!
//! This projection maintains a denormalized view of seat availability,
//! enabling fast queries like "Show me all available VIP seats for Event X"
//! without recomputing from aggregate state.

use super::{Projection, TicketingEvent};
use crate::aggregates::InventoryAction;
use crate::types::{EventId, SeatId};
use std::collections::HashMap;

/// Seat availability information for a specific section.
#[derive(Clone, Debug)]
pub struct SeatAvailability {
    /// Event ID
    pub event_id: EventId,
    /// Section name (e.g., "VIP", "General")
    pub section: String,
    /// Total capacity
    pub total_capacity: u32,
    /// Currently reserved (pending payment)
    pub reserved: u32,
    /// Sold (payment completed)
    pub sold: u32,
    /// Available seats (computed)
    pub available: u32,
    /// Specific seat IDs that are available (if tracked)
    pub available_seats: Vec<SeatId>,
    /// Specific seat IDs that are reserved (if tracked)
    pub reserved_seats: Vec<SeatId>,
    /// Specific seat IDs that are sold (if tracked)
    pub sold_seats: Vec<SeatId>,
}

impl SeatAvailability {
    /// Creates a new `SeatAvailability`
    #[must_use]
    pub const fn new(event_id: EventId, section: String, total_capacity: u32) -> Self {
        Self {
            event_id,
            section,
            total_capacity,
            reserved: 0,
            sold: 0,
            available: total_capacity,
            available_seats: Vec::new(),
            reserved_seats: Vec::new(),
            sold_seats: Vec::new(),
        }
    }

    /// Update availability counts after a change
    fn recalculate_available(&mut self) {
        self.available = self.total_capacity.saturating_sub(self.reserved + self.sold);
    }
}

/// Projection for tracking seat availability in real-time.
///
/// This projection listens to inventory events and maintains a denormalized
/// view of which seats are available, reserved, or sold.
///
/// # Query Examples
///
/// ```rust,ignore
/// // Get all available seats for an event
/// let available = projection.get_available_seats(&event_id, "VIP");
///
/// // Check if specific seats are available
/// let has_availability = projection.has_availability(&event_id, "General", 4);
/// ```
#[derive(Default)]
pub struct AvailableSeatsProjection {
    /// Seat availability indexed by (event_id, section)
    availability: HashMap<(EventId, String), SeatAvailability>,
    /// Processed reservation IDs for idempotency (prevents double-processing)
    processed_reservations: std::collections::HashSet<crate::types::ReservationId>,
}

impl AvailableSeatsProjection {
    /// Creates a new `AvailableSeatsProjection`
    #[must_use]
    pub fn new() -> Self {
        Self {
            availability: HashMap::new(),
            processed_reservations: std::collections::HashSet::new(),
        }
    }

    /// Get seat availability for a specific section
    #[must_use]
    pub fn get_availability(&self, event_id: &EventId, section: &str) -> Option<&SeatAvailability> {
        self.availability.get(&(*event_id, section.to_string()))
    }

    /// Check if a section has enough available seats
    #[must_use]
    pub fn has_availability(&self, event_id: &EventId, section: &str, quantity: u32) -> bool {
        self.get_availability(event_id, section)
            .map_or(false, |avail| avail.available >= quantity)
    }

    /// Get all sections for an event
    #[must_use]
    pub fn get_event_sections(&self, event_id: &EventId) -> Vec<&SeatAvailability> {
        self.availability
            .values()
            .filter(|avail| avail.event_id == *event_id)
            .collect()
    }

    /// Get total available seats across all sections for an event
    #[must_use]
    pub fn get_total_available(&self, event_id: &EventId) -> u32 {
        self.get_event_sections(event_id)
            .iter()
            .map(|avail| avail.available)
            .sum()
    }
}

impl Projection for AvailableSeatsProjection {
    fn handle_event(&mut self, event: &TicketingEvent) -> Result<(), String> {
        match event {
            // Initialize inventory creates new availability record
            TicketingEvent::Inventory(InventoryAction::InventoryInitialized {
                event_id,
                section,
                capacity,
                ..
            }) => {
                let availability = SeatAvailability::new(
                    *event_id,
                    section.clone(),
                    capacity.0,
                );
                self.availability
                    .insert((*event_id, section.clone()), availability);
                Ok(())
            }

            // Seats reserved: move from available to reserved
            TicketingEvent::Inventory(InventoryAction::SeatsReserved {
                reservation_id,
                event_id,
                section,
                seats,
                ..
            }) => {
                // Idempotency check: Skip if already processed
                if !self.processed_reservations.insert(*reservation_id) {
                    return Ok(()); // Already processed
                }

                if let Some(avail) = self.availability.get_mut(&(*event_id, section.clone())) {
                    // Update counts
                    #[allow(clippy::cast_possible_truncation)]
                    let quantity = seats.len() as u32;
                    avail.reserved += quantity;
                    avail.recalculate_available();

                    // Track specific seats if provided
                    avail.reserved_seats.extend(seats.clone());
                }
                Ok(())
            }

            // Seats confirmed: move from reserved to sold
            TicketingEvent::Inventory(InventoryAction::SeatsConfirmed {
                event_id,
                section,
                seats,
                ..
            }) => {
                // Use event_id and section directly from the event
                if let Some(avail) = self.availability.get_mut(&(*event_id, section.clone())) {
                    #[allow(clippy::cast_possible_truncation)]
                    let quantity = seats.len() as u32;

                    // Move from reserved to sold
                    avail.reserved = avail.reserved.saturating_sub(quantity);
                    avail.sold += quantity;
                    avail.recalculate_available();

                    // Update seat lists
                    avail.reserved_seats.retain(|s| !seats.contains(s));
                    avail.sold_seats.extend(seats.clone());
                }
                Ok(())
            }

            // Seats released: move from reserved back to available
            TicketingEvent::Inventory(InventoryAction::SeatsReleased {
                event_id,
                section,
                seats,
                ..
            }) => {
                // Use event_id and section directly from the event
                if let Some(avail) = self.availability.get_mut(&(*event_id, section.clone())) {
                    #[allow(clippy::cast_possible_truncation)]
                    let quantity = seats.len() as u32;

                    // Move from reserved back to available
                    avail.reserved = avail.reserved.saturating_sub(quantity);
                    avail.recalculate_available();

                    // Update seat lists
                    avail.reserved_seats.retain(|s| !seats.contains(s));
                }
                Ok(())
            }

            // Other events are not relevant to this projection
            _ => Ok(()),
        }
    }

    fn name(&self) -> &'static str {
        "AvailableSeatsProjection"
    }

    fn reset(&mut self) {
        self.availability.clear();
        self.processed_reservations.clear();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::types::{Capacity, ReservationId};
    use chrono::Utc;

    #[test]
    fn test_initialize_inventory() {
        let mut projection = AvailableSeatsProjection::new();
        let event_id = EventId::new();

        let event = TicketingEvent::Inventory(InventoryAction::InventoryInitialized {
            event_id,
            section: "VIP".to_string(),
            capacity: Capacity::new(100),
            seats: vec![],
            initialized_at: Utc::now(),
        });

        projection.handle_event(&event).unwrap();

        let avail = projection.get_availability(&event_id, "VIP").unwrap();
        assert_eq!(avail.total_capacity, 100);
        assert_eq!(avail.available, 100);
        assert_eq!(avail.reserved, 0);
        assert_eq!(avail.sold, 0);
    }

    #[test]
    fn test_reserve_seats() {
        let mut projection = AvailableSeatsProjection::new();
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();

        // Initialize
        projection
            .handle_event(&TicketingEvent::Inventory(
                InventoryAction::InventoryInitialized {
                    event_id,
                    section: "General".to_string(),
                    capacity: Capacity::new(50),
                    seats: vec![],
                    initialized_at: Utc::now(),
                },
            ))
            .unwrap();

        // Reserve 10 seats
        let seats: Vec<SeatId> = (0..10).map(|_| SeatId::new()).collect();
        projection
            .handle_event(&TicketingEvent::Inventory(
                InventoryAction::SeatsReserved {
                    reservation_id,
                    event_id,
                    section: "General".to_string(),
                    seats: seats.clone(),
                    expires_at: Utc::now(),
                    reserved_at: Utc::now(),
                },
            ))
            .unwrap();

        let avail = projection.get_availability(&event_id, "General").unwrap();
        assert_eq!(avail.total_capacity, 50);
        assert_eq!(avail.reserved, 10);
        assert_eq!(avail.available, 40);
        assert_eq!(avail.reserved_seats.len(), 10);
    }

    #[test]
    fn test_confirm_seats() {
        let mut projection = AvailableSeatsProjection::new();
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();

        // Initialize
        projection
            .handle_event(&TicketingEvent::Inventory(
                InventoryAction::InventoryInitialized {
                    event_id,
                    section: "VIP".to_string(),
                    capacity: Capacity::new(20),
                    seats: vec![],
                    initialized_at: Utc::now(),
                },
            ))
            .unwrap();

        // Reserve 5 seats
        let seats: Vec<SeatId> = (0..5).map(|_| SeatId::new()).collect();
        projection
            .handle_event(&TicketingEvent::Inventory(
                InventoryAction::SeatsReserved {
                    reservation_id,
                    event_id,
                    section: "VIP".to_string(),
                    seats: seats.clone(),
                    expires_at: Utc::now(),
                    reserved_at: Utc::now(),
                },
            ))
            .unwrap();

        // Confirm those seats
        projection
            .handle_event(&TicketingEvent::Inventory(
                InventoryAction::SeatsConfirmed {
                    reservation_id,
                    event_id,
                    section: "VIP".to_string(),
                    customer_id: crate::types::CustomerId::new(),
                    seats: seats.clone(),
                    confirmed_at: Utc::now(),
                },
            ))
            .unwrap();

        let avail = projection.get_availability(&event_id, "VIP").unwrap();
        assert_eq!(avail.reserved, 0); // No longer reserved
        assert_eq!(avail.sold, 5); // Now sold
        assert_eq!(avail.available, 15); // 20 - 5 sold
        assert_eq!(avail.sold_seats.len(), 5);
        assert_eq!(avail.reserved_seats.len(), 0);
    }

    #[test]
    fn test_release_seats() {
        let mut projection = AvailableSeatsProjection::new();
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();

        // Initialize
        projection
            .handle_event(&TicketingEvent::Inventory(
                InventoryAction::InventoryInitialized {
                    event_id,
                    section: "General".to_string(),
                    capacity: Capacity::new(30),
                    seats: vec![],
                    initialized_at: Utc::now(),
                },
            ))
            .unwrap();

        // Reserve 8 seats
        let seats: Vec<SeatId> = (0..8).map(|_| SeatId::new()).collect();
        projection
            .handle_event(&TicketingEvent::Inventory(
                InventoryAction::SeatsReserved {
                    reservation_id,
                    event_id,
                    section: "General".to_string(),
                    seats: seats.clone(),
                    expires_at: Utc::now(),
                    reserved_at: Utc::now(),
                },
            ))
            .unwrap();

        // Release those seats (timeout or cancellation)
        projection
            .handle_event(&TicketingEvent::Inventory(
                InventoryAction::SeatsReleased {
                    reservation_id,
                    event_id,
                    section: "General".to_string(),
                    seats: seats.clone(),
                    released_at: Utc::now(),
                },
            ))
            .unwrap();

        let avail = projection.get_availability(&event_id, "General").unwrap();
        assert_eq!(avail.reserved, 0); // Released
        assert_eq!(avail.sold, 0); // Never sold
        assert_eq!(avail.available, 30); // Back to full capacity
        assert_eq!(avail.reserved_seats.len(), 0);
    }

    #[test]
    fn test_multiple_sections() {
        let mut projection = AvailableSeatsProjection::new();
        let event_id = EventId::new();

        // Initialize VIP section
        projection
            .handle_event(&TicketingEvent::Inventory(
                InventoryAction::InventoryInitialized {
                    event_id,
                    section: "VIP".to_string(),
                    capacity: Capacity::new(50),
                    seats: vec![],
                    initialized_at: Utc::now(),
                },
            ))
            .unwrap();

        // Initialize General section
        projection
            .handle_event(&TicketingEvent::Inventory(
                InventoryAction::InventoryInitialized {
                    event_id,
                    section: "General".to_string(),
                    capacity: Capacity::new(200),
                    seats: vec![],
                    initialized_at: Utc::now(),
                },
            ))
            .unwrap();

        let sections = projection.get_event_sections(&event_id);
        assert_eq!(sections.len(), 2);

        let total_available = projection.get_total_available(&event_id);
        assert_eq!(total_available, 250); // 50 + 200
    }

    #[test]
    fn test_has_availability() {
        let mut projection = AvailableSeatsProjection::new();
        let event_id = EventId::new();

        projection
            .handle_event(&TicketingEvent::Inventory(
                InventoryAction::InventoryInitialized {
                    event_id,
                    section: "VIP".to_string(),
                    capacity: Capacity::new(10),
                    seats: vec![],
                    initialized_at: Utc::now(),
                },
            ))
            .unwrap();

        assert!(projection.has_availability(&event_id, "VIP", 5));
        assert!(projection.has_availability(&event_id, "VIP", 10));
        assert!(!projection.has_availability(&event_id, "VIP", 11));
    }
}
