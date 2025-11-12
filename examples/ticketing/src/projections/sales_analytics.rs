//! Sales analytics projection for revenue tracking and reporting.
//!
//! This projection maintains aggregated sales metrics, enabling fast queries
//! like "What's the total revenue for Event X?" or "Which section is most popular?"

use super::{Projection, TicketingEvent};
use crate::aggregates::{PaymentAction, ReservationAction};
use crate::types::{EventId, Money};
use std::collections::HashMap;

/// Sales metrics for a specific event.
#[derive(Clone, Debug)]
pub struct SalesMetrics {
    /// Event ID
    pub event_id: EventId,
    /// Total revenue (sum of all completed payments)
    pub total_revenue: Money,
    /// Number of tickets sold
    pub tickets_sold: u32,
    /// Number of completed reservations
    pub completed_reservations: u32,
    /// Number of cancelled/expired reservations
    pub cancelled_reservations: u32,
    /// Revenue by section (e.g., "VIP" -> $5000)
    pub revenue_by_section: HashMap<String, Money>,
    /// Tickets sold by section
    pub tickets_by_section: HashMap<String, u32>,
    /// Average ticket price
    pub average_ticket_price: Money,
}

impl Default for SalesMetrics {
    fn default() -> Self {
        Self::new(EventId::new())
    }
}

impl SalesMetrics {
    /// Creates a new `SalesMetrics` for an event
    #[must_use]
    pub fn new(event_id: EventId) -> Self {
        Self {
            event_id,
            total_revenue: Money::from_cents(0),
            tickets_sold: 0,
            completed_reservations: 0,
            cancelled_reservations: 0,
            revenue_by_section: HashMap::new(),
            tickets_by_section: HashMap::new(),
            average_ticket_price: Money::from_cents(0),
        }
    }

    /// Recalculate derived metrics (average price)
    fn recalculate_derived(&mut self) {
        if self.tickets_sold > 0 {
            self.average_ticket_price = Money::from_cents(
                self.total_revenue.cents() / u64::from(self.tickets_sold),
            );
        }
    }
}

/// Projection for tracking sales analytics and revenue.
///
/// This projection listens to payment and reservation events to build
/// aggregated sales metrics.
///
/// # Query Examples
///
/// ```rust,ignore
/// // Get total revenue for an event
/// let metrics = projection.get_metrics(&event_id);
/// println!("Total revenue: ${}", metrics.total_revenue.dollars());
///
/// // Get most popular section
/// let popular = projection.get_most_popular_section(&event_id);
/// ```
#[derive(Default)]
pub struct SalesAnalyticsProjection {
    /// Sales metrics indexed by `event_id`
    metrics: HashMap<EventId, SalesMetrics>,
    /// Reservation amount tracking (for when reservation completes)
    /// Maps `reservation_id` -> (`event_id`, section, amount, `ticket_count`)
    pending_reservations: HashMap<
        crate::types::ReservationId,
        (EventId, String, Money, u32),
    >,
}

impl SalesAnalyticsProjection {
    /// Creates a new `SalesAnalyticsProjection`
    #[must_use]
    pub fn new() -> Self {
        Self {
            metrics: HashMap::new(),
            pending_reservations: HashMap::new(),
        }
    }

    /// Get sales metrics for a specific event
    #[must_use]
    pub fn get_metrics(&self, event_id: &EventId) -> Option<&SalesMetrics> {
        self.metrics.get(event_id)
    }

    /// Get the most popular section by tickets sold for an event
    #[must_use]
    pub fn get_most_popular_section(&self, event_id: &EventId) -> Option<(&String, u32)> {
        self.metrics
            .get(event_id)?
            .tickets_by_section
            .iter()
            .max_by_key(|&(_, &count)| count)
            .map(|(section, &count)| (section, count))
    }

    /// Get the highest revenue section for an event
    #[must_use]
    pub fn get_highest_revenue_section(&self, event_id: &EventId) -> Option<(&String, Money)> {
        self.metrics
            .get(event_id)?
            .revenue_by_section
            .iter()
            .max_by_key(|&(_, &revenue)| revenue.cents())
            .map(|(section, &revenue)| (section, revenue))
    }

    /// Get total revenue across all events
    #[must_use]
    pub fn get_total_revenue_all_events(&self) -> Money {
        Money::from_cents(
            self.metrics
                .values()
                .map(|m| m.total_revenue.cents())
                .sum(),
        )
    }

    /// Get total tickets sold across all events
    #[must_use]
    pub fn get_total_tickets_sold(&self) -> u32 {
        self.metrics.values().map(|m| m.tickets_sold).sum()
    }

    /// Get or create metrics for an event
    fn get_or_create_metrics(&mut self, event_id: EventId) -> &mut SalesMetrics {
        self.metrics
            .entry(event_id)
            .or_insert_with(|| SalesMetrics::new(event_id))
    }
}

impl Projection for SalesAnalyticsProjection {
    fn handle_event(&mut self, event: &TicketingEvent) -> Result<(), String> {
        match event {
            // Track reservation initiation (will be completed or cancelled later)
            TicketingEvent::Reservation(ReservationAction::ReservationInitiated {
                reservation_id,
                event_id,
                section,
                quantity,
                ..
            }) => {
                // Store pending reservation info including section for analytics
                // We'll use a default price here; real implementation would track actual pricing
                let estimated_price = Money::from_dollars(50);
                self.pending_reservations.insert(
                    *reservation_id,
                    (*event_id, section.clone(), estimated_price.multiply(*quantity), *quantity),
                );
                Ok(())
            }

            // Track seats allocation (update quantity if different)
            TicketingEvent::Reservation(ReservationAction::SeatsAllocated {
                reservation_id,
                seats,
                total_amount,
            }) => {
                // Update pending reservation with actual seat count and amount (preserve section)
                if let Some((event_id, section, _, _)) = self.pending_reservations.get(reservation_id) {
                    #[allow(clippy::cast_possible_truncation)]
                    let quantity = seats.len() as u32;
                    self.pending_reservations
                        .insert(*reservation_id, (*event_id, section.clone(), *total_amount, quantity));
                }
                Ok(())
            }

            // Reservation completed: record the sale
            TicketingEvent::Reservation(ReservationAction::ReservationCompleted {
                reservation_id,
                ..
            }) => {
                if let Some((event_id, section, amount, ticket_count)) =
                    self.pending_reservations.remove(reservation_id)
                {
                    let metrics = self.get_or_create_metrics(event_id);
                    // Use checked arithmetic to prevent overflow
                    metrics.total_revenue = metrics
                        .total_revenue
                        .checked_add(amount)
                        .unwrap_or(metrics.total_revenue); // On overflow, keep current value (defensive)
                    metrics.tickets_sold += ticket_count;
                    metrics.completed_reservations += 1;

                    // Track section-based metrics
                    *metrics.revenue_by_section.entry(section.clone()).or_insert_with(|| Money::from_cents(0)) =
                        metrics.revenue_by_section.get(&section)
                            .copied()
                            .unwrap_or_else(|| Money::from_cents(0))
                            .checked_add(amount)
                            .unwrap_or_else(|| Money::from_cents(0));
                    *metrics.tickets_by_section.entry(section).or_insert(0) += ticket_count;

                    metrics.recalculate_derived();
                }
                Ok(())
            }

            // Reservation cancelled/expired: remove from pending
            TicketingEvent::Reservation(
                ReservationAction::ReservationCancelled { reservation_id, .. }
                | ReservationAction::ReservationExpired { reservation_id, .. }
                | ReservationAction::ReservationCompensated { reservation_id, .. },
            ) => {
                if let Some((event_id, _section, _amount, _ticket_count)) = self.pending_reservations.remove(reservation_id) {
                    let metrics = self.get_or_create_metrics(event_id);
                    metrics.cancelled_reservations += 1;
                }
                Ok(())
            }

            // Payment succeeded: confirmation of revenue
            TicketingEvent::Payment(PaymentAction::PaymentSucceeded { .. }) => {
                // Revenue is already tracked via ReservationCompleted
                // This event confirms payment processing
                Ok(())
            }

            // Payment refunded: subtract from revenue
            TicketingEvent::Payment(PaymentAction::PaymentRefunded {
                payment_id: _,
                amount,
                ..
            }) => {
                // In a real system, we'd track which event this refund belongs to
                // For now, we just acknowledge the refund event
                // A production system would need payment_id -> event_id mapping
                let _ = amount;
                Ok(())
            }

            // Other events are not relevant to this projection
            _ => Ok(()),
        }
    }

    fn name(&self) -> &'static str {
        "SalesAnalyticsProjection"
    }

    fn reset(&mut self) {
        self.metrics.clear();
        self.pending_reservations.clear();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::types::{CustomerId, ReservationId, SeatId, TicketId};
    use chrono::{Duration, Utc};

    #[test]
    fn test_reservation_completed() {
        let mut projection = SalesAnalyticsProjection::new();
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

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

        // Allocate seats with actual pricing
        let seats = vec![SeatId::new(), SeatId::new()];
        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::SeatsAllocated {
                    reservation_id,
                    seats,
                    total_amount: Money::from_dollars(200), // $100 per ticket
                },
            ))
            .unwrap();

        // Complete reservation
        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::ReservationCompleted {
                    reservation_id,
                    tickets_issued: vec![TicketId::new(), TicketId::new()],
                    completed_at: Utc::now(),
                },
            ))
            .unwrap();

        let metrics = projection.get_metrics(&event_id).unwrap();
        assert_eq!(metrics.total_revenue, Money::from_dollars(200));
        assert_eq!(metrics.tickets_sold, 2);
        assert_eq!(metrics.completed_reservations, 1);
        assert_eq!(metrics.average_ticket_price, Money::from_dollars(100));
    }

    #[test]
    fn test_reservation_cancelled() {
        let mut projection = SalesAnalyticsProjection::new();
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();

        // Initiate reservation
        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::ReservationInitiated {
                    reservation_id,
                    event_id,
                    customer_id: CustomerId::new(),
                    section: "General".to_string(),
                    quantity: 3,
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

        let metrics = projection.get_metrics(&event_id).unwrap();
        assert_eq!(metrics.total_revenue, Money::from_cents(0));
        assert_eq!(metrics.tickets_sold, 0);
        assert_eq!(metrics.cancelled_reservations, 1);
    }

    #[test]
    fn test_multiple_completions() {
        let mut projection = SalesAnalyticsProjection::new();
        let event_id = EventId::new();

        // Complete 3 reservations
        for i in 0..3 {
            let reservation_id = ReservationId::new();

            projection
                .handle_event(&TicketingEvent::Reservation(
                    ReservationAction::ReservationInitiated {
                        reservation_id,
                        event_id,
                        customer_id: CustomerId::new(),
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

        let metrics = projection.get_metrics(&event_id).unwrap();
        assert_eq!(metrics.total_revenue, Money::from_dollars(600)); // 100 + 200 + 300
        assert_eq!(metrics.tickets_sold, 6); // 2 + 2 + 2
        assert_eq!(metrics.completed_reservations, 3);
        assert_eq!(metrics.average_ticket_price, Money::from_dollars(100)); // 600 / 6
    }

    #[test]
    fn test_total_revenue_all_events() {
        let mut projection = SalesAnalyticsProjection::new();
        let event1 = EventId::new();
        let event2 = EventId::new();

        // Complete reservation for event 1
        let res1 = ReservationId::new();
        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::ReservationInitiated {
                    reservation_id: res1,
                    event_id: event1,
                    customer_id: CustomerId::new(),
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
                    reservation_id: res1,
                    seats: vec![SeatId::new(), SeatId::new()],
                    total_amount: Money::from_dollars(200),
                },
            ))
            .unwrap();

        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::ReservationCompleted {
                    reservation_id: res1,
                    tickets_issued: vec![TicketId::new(), TicketId::new()],
                    completed_at: Utc::now(),
                },
            ))
            .unwrap();

        // Complete reservation for event 2
        let res2 = ReservationId::new();
        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::ReservationInitiated {
                    reservation_id: res2,
                    event_id: event2,
                    customer_id: CustomerId::new(),
                    section: "General".to_string(),
                    quantity: 3,
                    expires_at: Utc::now() + Duration::minutes(5),
                    initiated_at: Utc::now(),
                },
            ))
            .unwrap();

        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::SeatsAllocated {
                    reservation_id: res2,
                    seats: vec![SeatId::new(), SeatId::new(), SeatId::new()],
                    total_amount: Money::from_dollars(150),
                },
            ))
            .unwrap();

        projection
            .handle_event(&TicketingEvent::Reservation(
                ReservationAction::ReservationCompleted {
                    reservation_id: res2,
                    tickets_issued: vec![TicketId::new(), TicketId::new(), TicketId::new()],
                    completed_at: Utc::now(),
                },
            ))
            .unwrap();

        assert_eq!(
            projection.get_total_revenue_all_events(),
            Money::from_dollars(350)
        );
        assert_eq!(projection.get_total_tickets_sold(), 5);
    }
}
