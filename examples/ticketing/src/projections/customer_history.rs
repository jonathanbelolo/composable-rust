//! Customer history projection for purchase tracking and personalization.
//!
//! This projection maintains each customer's ticket purchase history,
//! enabling queries like "Show all tickets purchased by Customer X" or
//! "Has this customer attended this venue before?"

use super::{Projection, TicketingEvent};
use crate::aggregates::ReservationAction;
use crate::types::{CustomerId, EventId, Money, ReservationId, TicketId};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// A customer's completed purchase record.
#[derive(Clone, Debug)]
pub struct CustomerPurchase {
    /// Reservation ID
    pub reservation_id: ReservationId,
    /// Event ID
    pub event_id: EventId,
    /// Section purchased (e.g., "VIP", "General")
    pub section: String,
    /// Number of tickets
    pub ticket_count: u32,
    /// Total amount paid
    pub amount_paid: Money,
    /// Ticket IDs issued
    pub tickets: Vec<TicketId>,
    /// When the purchase was completed
    pub completed_at: DateTime<Utc>,
}

impl CustomerPurchase {
    /// Creates a new `CustomerPurchase`
    #[must_use]
    pub const fn new(
        reservation_id: ReservationId,
        event_id: EventId,
        section: String,
        ticket_count: u32,
        amount_paid: Money,
        tickets: Vec<TicketId>,
        completed_at: DateTime<Utc>,
    ) -> Self {
        Self {
            reservation_id,
            event_id,
            section,
            ticket_count,
            amount_paid,
            tickets,
            completed_at,
        }
    }
}

/// Customer profile summary.
#[derive(Clone, Debug)]
pub struct CustomerProfile {
    /// Customer ID
    pub customer_id: CustomerId,
    /// All completed purchases
    pub purchases: Vec<CustomerPurchase>,
    /// Total amount spent
    pub total_spent: Money,
    /// Total tickets purchased
    pub total_tickets: u32,
    /// Events attended (unique event IDs)
    pub events_attended: Vec<EventId>,
    /// Favorite section (most frequently purchased)
    pub favorite_section: Option<String>,
}

impl Default for CustomerProfile {
    fn default() -> Self {
        Self::new(CustomerId::new())
    }
}

impl CustomerProfile {
    /// Creates a new empty `CustomerProfile`
    #[must_use]
    pub const fn new(customer_id: CustomerId) -> Self {
        Self {
            customer_id,
            purchases: Vec::new(),
            total_spent: Money::from_cents(0),
            total_tickets: 0,
            events_attended: Vec::new(),
            favorite_section: None,
        }
    }

    /// Add a purchase to the customer's history
    fn add_purchase(&mut self, purchase: CustomerPurchase) {
        // Use checked arithmetic to prevent overflow
        self.total_spent = self
            .total_spent
            .checked_add(purchase.amount_paid)
            .unwrap_or(self.total_spent); // On overflow, keep current value (defensive)
        self.total_tickets += purchase.ticket_count;

        if !self.events_attended.contains(&purchase.event_id) {
            self.events_attended.push(purchase.event_id);
        }

        self.purchases.push(purchase);
        self.recalculate_favorite_section();
    }

    /// Recalculate the customer's favorite section based on purchase history
    fn recalculate_favorite_section(&mut self) {
        let mut section_counts: HashMap<String, u32> = HashMap::new();

        for purchase in &self.purchases {
            *section_counts.entry(purchase.section.clone()).or_insert(0) += 1;
        }

        self.favorite_section = section_counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(section, _)| section);
    }
}

/// Projection for tracking customer purchase history.
///
/// This projection listens to reservation events and builds customer profiles
/// with purchase history.
///
/// # Query Examples
///
/// ```rust,ignore
/// // Get customer's purchase history
/// let profile = projection.get_customer_profile(&customer_id);
/// println!("Total spent: ${}", profile.total_spent.dollars());
///
/// // Check if customer has attended an event
/// let has_attended = projection.has_attended_event(&customer_id, &event_id);
/// ```
#[derive(Default)]
pub struct CustomerHistoryProjection {
    /// Customer profiles indexed by customer_id
    profiles: HashMap<CustomerId, CustomerProfile>,
    /// Pending reservations: reservation_id -> (customer_id, event_id, section, amount, tickets)
    pending_reservations: HashMap<
        ReservationId,
        (CustomerId, EventId, String, u32, Money),
    >,
}

impl CustomerHistoryProjection {
    /// Creates a new `CustomerHistoryProjection`
    #[must_use]
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
            pending_reservations: HashMap::new(),
        }
    }

    /// Get a customer's profile
    #[must_use]
    pub fn get_customer_profile(&self, customer_id: &CustomerId) -> Option<&CustomerProfile> {
        self.profiles.get(customer_id)
    }

    /// Check if a customer has attended a specific event
    #[must_use]
    pub fn has_attended_event(&self, customer_id: &CustomerId, event_id: &EventId) -> bool {
        self.profiles
            .get(customer_id)
            .map_or(false, |profile| profile.events_attended.contains(event_id))
    }

    /// Get all customers who attended a specific event
    #[must_use]
    pub fn get_event_attendees(&self, event_id: &EventId) -> Vec<CustomerId> {
        self.profiles
            .values()
            .filter(|profile| profile.events_attended.contains(event_id))
            .map(|profile| profile.customer_id)
            .collect()
    }

    /// Get customers sorted by total spending (top spenders)
    #[must_use]
    pub fn get_top_spenders(&self, limit: usize) -> Vec<&CustomerProfile> {
        let mut profiles: Vec<&CustomerProfile> = self.profiles.values().collect();
        profiles.sort_by_key(|p| std::cmp::Reverse(p.total_spent.cents()));
        profiles.into_iter().take(limit).collect()
    }

    /// Get total number of customers
    #[must_use]
    pub fn get_customer_count(&self) -> usize {
        self.profiles.len()
    }

    /// Get or create a customer profile
    fn get_or_create_profile(&mut self, customer_id: CustomerId) -> &mut CustomerProfile {
        self.profiles
            .entry(customer_id)
            .or_insert_with(|| CustomerProfile::new(customer_id))
    }
}

impl Projection for CustomerHistoryProjection {
    fn handle_event(&mut self, event: &TicketingEvent) -> Result<(), String> {
        match event {
            // Track reservation initiation
            TicketingEvent::Reservation(ReservationAction::ReservationInitiated {
                reservation_id,
                event_id,
                customer_id,
                section,
                quantity,
                ..
            }) => {
                // Store pending reservation
                let estimated_price = Money::from_dollars(50); // Default price
                self.pending_reservations.insert(
                    *reservation_id,
                    (
                        *customer_id,
                        *event_id,
                        section.clone(),
                        *quantity,
                        estimated_price.multiply(*quantity),
                    ),
                );
                Ok(())
            }

            // Update with actual pricing
            TicketingEvent::Reservation(ReservationAction::SeatsAllocated {
                reservation_id,
                seats,
                total_amount,
            }) => {
                if let Some((customer_id, event_id, section, _, _)) =
                    self.pending_reservations.get(reservation_id)
                {
                    #[allow(clippy::cast_possible_truncation)]
                    let ticket_count = seats.len() as u32;
                    self.pending_reservations.insert(
                        *reservation_id,
                        (*customer_id, *event_id, section.clone(), ticket_count, *total_amount),
                    );
                }
                Ok(())
            }

            // Reservation completed: add to customer history
            TicketingEvent::Reservation(ReservationAction::ReservationCompleted {
                reservation_id,
                tickets_issued,
                completed_at,
            }) => {
                if let Some((customer_id, event_id, section, ticket_count, amount)) =
                    self.pending_reservations.remove(reservation_id)
                {
                    let purchase = CustomerPurchase::new(
                        *reservation_id,
                        event_id,
                        section,
                        ticket_count,
                        amount,
                        tickets_issued.clone(),
                        *completed_at,
                    );

                    let profile = self.get_or_create_profile(customer_id);
                    profile.add_purchase(purchase);
                }
                Ok(())
            }

            // Reservation cancelled/expired: remove from pending
            TicketingEvent::Reservation(
                ReservationAction::ReservationCancelled { reservation_id, .. }
                | ReservationAction::ReservationExpired { reservation_id, .. }
                | ReservationAction::ReservationCompensated { reservation_id, .. },
            ) => {
                self.pending_reservations.remove(reservation_id);
                Ok(())
            }

            // Other events are not relevant to this projection
            _ => Ok(()),
        }
    }

    fn name(&self) -> &'static str {
        "CustomerHistoryProjection"
    }

    fn reset(&mut self) {
        self.profiles.clear();
        self.pending_reservations.clear();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::types::{ReservationId, SeatId};
    use chrono::{Duration, Utc};

    #[test]
    fn test_customer_purchase_recorded() {
        let mut projection = CustomerHistoryProjection::new();
        let customer_id = CustomerId::new();
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();

        // Initiate reservation
        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::ReservationInitiated {
                    reservation_id,
                    event_id,
                    customer_id,
                    section: "VIP".to_string(),
                    quantity: 2,
                    expires_at: Utc::now() + Duration::minutes(5),
                    initiated_at: Utc::now(),
                },
            ))
            .unwrap();

        // Allocate seats
        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::SeatsAllocated {
                    reservation_id,
                    seats: vec![SeatId::new(), SeatId::new()],
                    total_amount: Money::from_dollars(200),
                },
            ))
            .unwrap();

        // Complete reservation
        let tickets = vec![TicketId::new(), TicketId::new()];
        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::ReservationCompleted {
                    reservation_id,
                    tickets_issued: tickets.clone(),
                    completed_at: Utc::now(),
                },
            ))
            .unwrap();

        let profile = projection.get_customer_profile(&customer_id).unwrap();
        assert_eq!(profile.purchases.len(), 1);
        assert_eq!(profile.total_spent, Money::from_dollars(200));
        assert_eq!(profile.total_tickets, 2);
        assert_eq!(profile.events_attended.len(), 1);
        assert_eq!(profile.events_attended[0], event_id);
    }

    #[test]
    fn test_multiple_purchases() {
        let mut projection = CustomerHistoryProjection::new();
        let customer_id = CustomerId::new();

        // Make 3 purchases
        for i in 0..3 {
            let event_id = EventId::new();
            let reservation_id = ReservationId::new();

            projection
                .handle_event(&TicketingEvent::Reservation(
                    ReservationAction::ReservationInitiated {
                        reservation_id,
                        event_id,
                        customer_id,
                        section: "VIP".to_string(),
                        quantity: 2,
                        expires_at: Utc::now() + Duration::minutes(5),
                        initiated_at: Utc::now(),
                    },
                ))
                .unwrap();

            projection
                .handle_event(&TicketingEvent::Reservation(
                    ReservationAction::SeatsAllocated {
                        reservation_id,
                        seats: vec![SeatId::new(), SeatId::new()],
                        total_amount: Money::from_dollars(100 * (i + 1)),
                    },
                ))
                .unwrap();

            projection
                .handle_event(&TicketingEvent::Reservation(
                    ReservationAction::ReservationCompleted {
                        reservation_id,
                        tickets_issued: vec![TicketId::new(), TicketId::new()],
                        completed_at: Utc::now(),
                    },
                ))
                .unwrap();
        }

        let profile = projection.get_customer_profile(&customer_id).unwrap();
        assert_eq!(profile.purchases.len(), 3);
        assert_eq!(profile.total_spent, Money::from_dollars(600)); // 100 + 200 + 300
        assert_eq!(profile.total_tickets, 6);
        assert_eq!(profile.events_attended.len(), 3);
    }

    #[test]
    fn test_favorite_section() {
        let mut projection = CustomerHistoryProjection::new();
        let customer_id = CustomerId::new();

        // Purchase VIP twice
        for _ in 0..2 {
            let reservation_id = ReservationId::new();
            projection
                .handle_event(&TicketingEvent::Reservation(
                    ReservationAction::ReservationInitiated {
                        reservation_id,
                        event_id: EventId::new(),
                        customer_id,
                        section: "VIP".to_string(),
                        quantity: 1,
                        expires_at: Utc::now() + Duration::minutes(5),
                        initiated_at: Utc::now(),
                    },
                ))
                .unwrap();

            projection
                .handle_event(&TicketingEvent::Reservation(
                    ReservationAction::SeatsAllocated {
                        reservation_id,
                        seats: vec![SeatId::new()],
                        total_amount: Money::from_dollars(100),
                    },
                ))
                .unwrap();

            projection
                .handle_event(&TicketingEvent::Reservation(
                    ReservationAction::ReservationCompleted {
                        reservation_id,
                        tickets_issued: vec![TicketId::new()],
                        completed_at: Utc::now(),
                    },
                ))
                .unwrap();
        }

        // Purchase General once
        let reservation_id = ReservationId::new();
        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::ReservationInitiated {
                    reservation_id,
                    event_id: EventId::new(),
                    customer_id,
                    section: "General".to_string(),
                    quantity: 1,
                    expires_at: Utc::now() + Duration::minutes(5),
                    initiated_at: Utc::now(),
                },
            ))
            .unwrap();

        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::SeatsAllocated {
                    reservation_id,
                    seats: vec![SeatId::new()],
                    total_amount: Money::from_dollars(50),
                },
            ))
            .unwrap();

        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::ReservationCompleted {
                    reservation_id,
                    tickets_issued: vec![TicketId::new()],
                    completed_at: Utc::now(),
                },
            ))
            .unwrap();

        let profile = projection.get_customer_profile(&customer_id).unwrap();
        assert_eq!(profile.favorite_section, Some("VIP".to_string()));
    }

    #[test]
    fn test_has_attended_event() {
        let mut projection = CustomerHistoryProjection::new();
        let customer_id = CustomerId::new();
        let event_id = EventId::new();
        let other_event = EventId::new();
        let reservation_id = ReservationId::new();

        // Complete purchase for event_id
        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::ReservationInitiated {
                    reservation_id,
                    event_id,
                    customer_id,
                    section: "General".to_string(),
                    quantity: 1,
                    expires_at: Utc::now() + Duration::minutes(5),
                    initiated_at: Utc::now(),
                },
            ))
            .unwrap();

        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::SeatsAllocated {
                    reservation_id,
                    seats: vec![SeatId::new()],
                    total_amount: Money::from_dollars(50),
                },
            ))
            .unwrap();

        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::ReservationCompleted {
                    reservation_id,
                    tickets_issued: vec![TicketId::new()],
                    completed_at: Utc::now(),
                },
            ))
            .unwrap();

        assert!(projection.has_attended_event(&customer_id, &event_id));
        assert!(!projection.has_attended_event(&customer_id, &other_event));
    }

    #[test]
    fn test_cancelled_reservation_not_recorded() {
        let mut projection = CustomerHistoryProjection::new();
        let customer_id = CustomerId::new();
        let reservation_id = ReservationId::new();

        // Initiate reservation
        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::ReservationInitiated {
                    reservation_id,
                    event_id: EventId::new(),
                    customer_id,
                    section: "VIP".to_string(),
                    quantity: 2,
                    expires_at: Utc::now() + Duration::minutes(5),
                    initiated_at: Utc::now(),
                },
            ))
            .unwrap();

        // Cancel reservation
        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::ReservationCancelled {
                    reservation_id,
                    reason: "Customer changed mind".to_string(),
                    cancelled_at: Utc::now(),
                },
            ))
            .unwrap();

        // Customer should have no purchases
        assert!(projection.get_customer_profile(&customer_id).is_none());
    }
}
