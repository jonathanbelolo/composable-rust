//! Event projection for CQRS read model.
//!
//! Maintains an in-memory queryable view of all events.
//! Updated via `EventBus` subscriptions.

use crate::aggregates::EventAction;
use crate::projections::TicketingEvent;
use crate::types::{Event, EventId, EventStatus};
use std::collections::HashMap;

/// In-memory Event projection for fast queries.
///
/// Provides:
/// - List all events with filtering by status
/// - Get event by ID
/// - Pagination support
#[derive(Clone, Debug, Default)]
pub struct EventProjection {
    /// All events indexed by ID
    events: HashMap<EventId, Event>,
}

impl EventProjection {
    /// Creates a new empty `EventProjection`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            events: HashMap::new(),
        }
    }

    /// Get an event by ID.
    #[must_use]
    pub fn get(&self, event_id: &EventId) -> Option<&Event> {
        self.events.get(event_id)
    }

    /// List all events with optional status filter.
    ///
    /// Returns events sorted by creation date (newest first).
    #[must_use]
    pub fn list(&self, status_filter: Option<EventStatus>) -> Vec<Event> {
        let mut events: Vec<Event> = self
            .events
            .values()
            .filter(|event| {
                status_filter
                    .as_ref()
                    .is_none_or(|status| &event.status == status)
            })
            .cloned()
            .collect();

        // Sort by creation date, newest first
        events.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        events
    }

    /// Get paginated events.
    ///
    /// # Arguments
    ///
    /// - `page`: Page number (0-indexed)
    /// - `page_size`: Number of events per page
    /// - `status_filter`: Optional status filter
    ///
    /// # Returns
    ///
    /// Tuple of (events, `total_count`)
    #[must_use]
    pub fn list_paginated(
        &self,
        page: usize,
        page_size: usize,
        status_filter: Option<EventStatus>,
    ) -> (Vec<Event>, usize) {
        let all_events = self.list(status_filter);
        let total = all_events.len();

        let start = page * page_size;
        let end = (start + page_size).min(total);

        let page_events = if start < total {
            all_events[start..end].to_vec()
        } else {
            Vec::new()
        };

        (page_events, total)
    }

    /// Count total events.
    #[must_use]
    pub fn count(&self) -> usize {
        self.events.len()
    }

    /// Handle a ticketing event to update the projection.
    ///
    /// # Errors
    ///
    /// Returns error if event handling fails.
    pub fn handle_event(&mut self, event: &TicketingEvent) -> Result<(), String> {
        if let TicketingEvent::Event(event_action) = event {
            self.apply_event_action(event_action);
        }
        Ok(())
    }

    /// Apply an event action to update the projection state.
    fn apply_event_action(&mut self, action: &EventAction) {
        match action {
            // Commands - transform to corresponding events for projection
            EventAction::CreateEvent {
                id,
                name,
                venue,
                date,
                pricing_tiers,
            } => {
                // Treat command as if event already happened
                // Use current time since projection doesn't have access to exact event timestamp
                let event = Event::new(
                    *id,
                    name.clone(),
                    venue.clone(),
                    *date,
                    pricing_tiers.clone(),
                    chrono::Utc::now(),
                );
                self.events.insert(*id, event);
            }
            EventAction::PublishEvent { event_id } => {
                if let Some(event) = self.events.get_mut(event_id) {
                    event.status = EventStatus::Published;
                }
            }
            EventAction::OpenSales { event_id } => {
                if let Some(event) = self.events.get_mut(event_id) {
                    event.status = EventStatus::SalesOpen;
                }
            }
            EventAction::CloseSales { event_id } => {
                if let Some(event) = self.events.get_mut(event_id) {
                    event.status = EventStatus::SalesClosed;
                }
            }
            EventAction::CancelEvent { event_id, .. } => {
                if let Some(event) = self.events.get_mut(event_id) {
                    event.status = EventStatus::Cancelled;
                }
            }

            // Events - apply directly (same logic as commands above)
            EventAction::EventCreated {
                id,
                name,
                venue,
                date,
                pricing_tiers,
                created_at,
            } => {
                let event = Event::new(
                    *id,
                    name.clone(),
                    venue.clone(),
                    *date,
                    pricing_tiers.clone(),
                    *created_at,
                );
                self.events.insert(*id, event);
            }
            EventAction::EventPublished { event_id, .. } => {
                if let Some(event) = self.events.get_mut(event_id) {
                    event.status = EventStatus::Published;
                }
            }
            EventAction::SalesOpened { event_id, .. } => {
                if let Some(event) = self.events.get_mut(event_id) {
                    event.status = EventStatus::SalesOpen;
                }
            }
            EventAction::SalesClosed { event_id, .. } => {
                if let Some(event) = self.events.get_mut(event_id) {
                    event.status = EventStatus::SalesClosed;
                }
            }
            EventAction::EventCancelled { event_id, .. } => {
                if let Some(event) = self.events.get_mut(event_id) {
                    event.status = EventStatus::Cancelled;
                }
            }

            // Validation failures don't affect projection
            EventAction::ValidationFailed { .. } => {}
        }
    }

    /// Reset the projection to empty state.
    pub fn reset(&mut self) {
        self.events.clear();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // Test code can use unwrap
mod tests {
    use super::*;
    use crate::types::{Capacity, EventDate, Money, PricingTier, TierType, Venue, VenueSection, SeatType};
    use chrono::Utc;

    fn create_test_venue() -> Venue {
        Venue::new(
            "Test Venue".to_string(),
            Capacity::new(1000),
            vec![VenueSection::new(
                "General".to_string(),
                Capacity::new(1000),
                SeatType::GeneralAdmission,
            )],
        )
    }

    fn create_test_pricing() -> Vec<PricingTier> {
        vec![PricingTier::new(
            TierType::Regular,
            "General".to_string(),
            Money::from_dollars(50),
            Utc::now(),
            None,
        )]
    }

    #[test]
    fn test_event_created() {
        let mut projection = EventProjection::new();
        let event_id = EventId::new();

        let action = EventAction::EventCreated {
            id: event_id,
            name: "Test Concert".to_string(),
            venue: create_test_venue(),
            date: EventDate::new(Utc::now()),
            pricing_tiers: create_test_pricing(),
            created_at: Utc::now(),
        };

        projection.apply_event_action(&action);

        assert_eq!(projection.count(), 1);
        let event = projection.get(&event_id).unwrap();
        assert_eq!(event.name, "Test Concert");
        assert_eq!(event.status, EventStatus::Draft);
    }

    #[test]
    fn test_event_lifecycle() {
        let mut projection = EventProjection::new();
        let event_id = EventId::new();

        // Create event
        projection.apply_event_action(&EventAction::EventCreated {
            id: event_id,
            name: "Concert".to_string(),
            venue: create_test_venue(),
            date: EventDate::new(Utc::now()),
            pricing_tiers: create_test_pricing(),
            created_at: Utc::now(),
        });

        assert_eq!(projection.get(&event_id).unwrap().status, EventStatus::Draft);

        // Publish event
        projection.apply_event_action(&EventAction::EventPublished {
            event_id,
            published_at: Utc::now(),
        });
        assert_eq!(projection.get(&event_id).unwrap().status, EventStatus::Published);

        // Open sales
        projection.apply_event_action(&EventAction::SalesOpened {
            event_id,
            opened_at: Utc::now(),
        });
        assert_eq!(projection.get(&event_id).unwrap().status, EventStatus::SalesOpen);

        // Close sales
        projection.apply_event_action(&EventAction::SalesClosed {
            event_id,
            closed_at: Utc::now(),
        });
        assert_eq!(projection.get(&event_id).unwrap().status, EventStatus::SalesClosed);
    }

    #[test]
    fn test_list_with_filter() {
        let mut projection = EventProjection::new();

        // Create draft event
        let draft_id = EventId::new();
        projection.apply_event_action(&EventAction::EventCreated {
            id: draft_id,
            name: "Draft Event".to_string(),
            venue: create_test_venue(),
            date: EventDate::new(Utc::now()),
            pricing_tiers: create_test_pricing(),
            created_at: Utc::now(),
        });

        // Create published event
        let published_id = EventId::new();
        projection.apply_event_action(&EventAction::EventCreated {
            id: published_id,
            name: "Published Event".to_string(),
            venue: create_test_venue(),
            date: EventDate::new(Utc::now()),
            pricing_tiers: create_test_pricing(),
            created_at: Utc::now(),
        });
        projection.apply_event_action(&EventAction::EventPublished {
            event_id: published_id,
            published_at: Utc::now(),
        });

        // List all events
        let all_events = projection.list(None);
        assert_eq!(all_events.len(), 2);

        // Filter by Draft
        let draft_events = projection.list(Some(EventStatus::Draft));
        assert_eq!(draft_events.len(), 1);
        assert_eq!(draft_events[0].id, draft_id);

        // Filter by Published
        let published_events = projection.list(Some(EventStatus::Published));
        assert_eq!(published_events.len(), 1);
        assert_eq!(published_events[0].id, published_id);
    }

    #[test]
    fn test_pagination() {
        let mut projection = EventProjection::new();

        // Create 5 events
        for i in 0..5 {
            let event_id = EventId::new();
            projection.apply_event_action(&EventAction::EventCreated {
                id: event_id,
                name: format!("Event {i}"),
                venue: create_test_venue(),
                date: EventDate::new(Utc::now()),
                pricing_tiers: create_test_pricing(),
                created_at: Utc::now(),
            });
        }

        // Get first page (2 items)
        let (page1, total) = projection.list_paginated(0, 2, None);
        assert_eq!(page1.len(), 2);
        assert_eq!(total, 5);

        // Get second page
        let (page2, _) = projection.list_paginated(1, 2, None);
        assert_eq!(page2.len(), 2);

        // Get third page
        let (page3, _) = projection.list_paginated(2, 2, None);
        assert_eq!(page3.len(), 1);

        // Get out of bounds page
        let (page4, _) = projection.list_paginated(10, 2, None);
        assert_eq!(page4.len(), 0);
    }
}
