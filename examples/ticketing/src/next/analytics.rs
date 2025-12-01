//! Analytics aggregate for the next-generation architecture.
//!
//! This module provides the [`AnalyticsBusinessLogic`] which implements
//! [`BusinessLogic`] for querying sales analytics and customer data.
//!
//! # Commands
//!
//! - [`AnalyticsCommand::GetEventSales`] - Get sales metrics for an event
//! - [`AnalyticsCommand::GetPopularSections`] - Get most popular sections
//! - [`AnalyticsCommand::GetTotalRevenue`] - Get total revenue (admin only)
//! - [`AnalyticsCommand::GetTopSpenders`] - Get top spending customers (admin only)
//! - [`AnalyticsCommand::GetCustomerProfile`] - Get customer purchase profile
//!
//! # Architecture
//!
//! Analytics queries are read-only operations that return data from projections.
//! The BusinessLogic validates the request and returns the pre-fetched data
//! via the `Respond` result variant.

use chrono::{DateTime, Utc};
use composable_rust_next::{BusinessLogic, BusinessResult, Clock, StreamId};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use uuid::Uuid;

use crate::types::{CustomerId, EventId, Money};

// ═══════════════════════════════════════════════════════════════════════════
// DTOs (Data Transfer Objects)
// ═══════════════════════════════════════════════════════════════════════════

/// Sales metrics for a specific event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSalesDto {
    /// Event ID
    pub event_id: EventId,
    /// Total revenue for this event
    pub total_revenue: Money,
    /// Number of tickets sold
    pub tickets_sold: u32,
    /// Number of completed reservations
    pub completed_reservations: u32,
    /// Number of cancelled/expired reservations
    pub cancelled_reservations: u32,
    /// Average ticket price
    pub average_ticket_price: Money,
    /// Revenue breakdown by section
    pub sections: Vec<SectionSalesDto>,
}

/// Sales metrics for a section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionSalesDto {
    /// Section name (e.g., "VIP", "General")
    pub section: String,
    /// Revenue from this section
    pub revenue: Money,
    /// Tickets sold in this section
    pub tickets_sold: u32,
}

/// Popular sections response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PopularSectionsDto {
    /// Event ID
    pub event_id: EventId,
    /// Most popular section by ticket count
    pub most_popular: Option<SectionPopularityDto>,
    /// Highest revenue section
    pub highest_revenue: Option<SectionPopularityDto>,
}

/// Section popularity metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionPopularityDto {
    /// Section name
    pub section: String,
    /// Tickets sold
    pub tickets_sold: u32,
    /// Revenue generated
    pub revenue: Money,
}

/// Total revenue response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotalRevenueDto {
    /// Total revenue across all events
    pub total_revenue: Money,
    /// Total tickets sold
    pub total_tickets_sold: u32,
    /// Number of events with sales
    pub events_with_sales: usize,
}

/// Top spending customers response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopSpendersDto {
    /// Top spending customers
    pub customers: Vec<CustomerSpendingDto>,
    /// Total number of customers in system
    pub total_customers: usize,
}

/// Customer spending summary for leaderboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerSpendingDto {
    /// Customer ID
    pub customer_id: CustomerId,
    /// Total amount spent
    pub total_spent: Money,
    /// Total tickets purchased
    pub total_tickets: u32,
    /// Number of events attended
    pub events_attended: usize,
    /// Favorite section (most frequently purchased)
    pub favorite_section: Option<String>,
}

/// Customer profile response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerProfileDto {
    /// Customer ID
    pub customer_id: CustomerId,
    /// Total amount spent
    pub total_spent: Money,
    /// Total tickets purchased
    pub total_tickets: u32,
    /// Events attended (unique event IDs)
    pub events_attended: Vec<EventId>,
    /// Favorite section
    pub favorite_section: Option<String>,
    /// Recent purchases (last 10)
    pub recent_purchases: Vec<PurchaseRecordDto>,
}

/// Purchase record for customer history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurchaseRecordDto {
    /// Reservation ID
    pub reservation_id: Uuid,
    /// Event ID
    pub event_id: EventId,
    /// Section
    pub section: String,
    /// Number of tickets
    pub ticket_count: u32,
    /// Amount paid
    pub amount_paid: Money,
    /// When purchased
    pub completed_at: DateTime<Utc>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Commands (Input)
// ═══════════════════════════════════════════════════════════════════════════

/// Analytics commands.
#[derive(Debug, Clone)]
pub enum AnalyticsCommand {
    /// Get sales metrics for an event.
    ///
    /// Public endpoint - no authentication required.
    GetEventSales {
        /// Event ID to get sales for
        event_id: EventId,
        /// Pre-fetched sales data (populated by QueryFetcher)
        fetched: Option<EventSalesDto>,
    },

    /// Get most popular sections for an event.
    ///
    /// Public endpoint - no authentication required.
    GetPopularSections {
        /// Event ID
        event_id: EventId,
        /// Pre-fetched popularity data (populated by QueryFetcher)
        fetched: Option<PopularSectionsDto>,
    },

    /// Get total revenue across all events.
    ///
    /// Admin-only endpoint.
    GetTotalRevenue {
        /// Pre-fetched revenue data (populated by QueryFetcher)
        fetched: Option<TotalRevenueDto>,
    },

    /// Get top spending customers.
    ///
    /// Admin-only endpoint.
    GetTopSpenders {
        /// Number of top spenders to return (max 100)
        limit: usize,
        /// Pre-fetched spenders data (populated by QueryFetcher)
        fetched: Option<TopSpendersDto>,
    },

    /// Get customer profile and purchase history.
    ///
    /// Requires authentication and ownership verification.
    GetCustomerProfile {
        /// Customer ID to get profile for
        customer_id: CustomerId,
        /// Requesting user ID (for ownership verification)
        requesting_user_id: CustomerId,
        /// Pre-fetched profile data (populated by QueryFetcher)
        fetched: Option<CustomerProfileDto>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// Events (Output)
// ═══════════════════════════════════════════════════════════════════════════

/// Analytics events.
///
/// Analytics is read-only, so events are minimal - just for audit logging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AnalyticsEvent {
    /// Event sales were queried.
    EventSalesQueried {
        /// Event ID
        event_id: EventId,
        /// When queried
        queried_at: DateTime<Utc>,
    },

    /// Popular sections were queried.
    PopularSectionsQueried {
        /// Event ID
        event_id: EventId,
        /// When queried
        queried_at: DateTime<Utc>,
    },

    /// Total revenue was queried.
    TotalRevenueQueried {
        /// When queried
        queried_at: DateTime<Utc>,
    },

    /// Top spenders were queried.
    TopSpendersQueried {
        /// Limit requested
        limit: usize,
        /// When queried
        queried_at: DateTime<Utc>,
    },

    /// Customer profile was queried.
    CustomerProfileQueried {
        /// Customer ID
        customer_id: CustomerId,
        /// Who requested it
        requesting_user_id: CustomerId,
        /// When queried
        queried_at: DateTime<Utc>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// Response Types
// ═══════════════════════════════════════════════════════════════════════════

/// Analytics query responses.
#[derive(Debug, Clone)]
pub enum AnalyticsResponse {
    /// Event sales response.
    EventSales(EventSalesDto),
    /// Popular sections response.
    PopularSections(PopularSectionsDto),
    /// Total revenue response.
    TotalRevenue(TotalRevenueDto),
    /// Top spenders response.
    TopSpenders(TopSpendersDto),
    /// Customer profile response.
    CustomerProfile(CustomerProfileDto),
}

// ═══════════════════════════════════════════════════════════════════════════
// Errors
// ═══════════════════════════════════════════════════════════════════════════

/// Analytics errors.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AnalyticsError {
    /// Event not found or has no sales data.
    #[error("Event not found or has no sales data: {event_id}")]
    EventNotFound {
        /// The event ID that was not found.
        event_id: EventId,
    },

    /// Customer not found or has no profile.
    #[error("Customer not found or has no profile: {customer_id}")]
    CustomerNotFound {
        /// The customer ID that was not found.
        customer_id: CustomerId,
    },

    /// Access denied - user doesn't own this resource.
    #[error("Access denied: user {requesting_user_id} cannot access customer {customer_id}")]
    AccessDenied {
        /// The customer ID being accessed.
        customer_id: CustomerId,
        /// The user ID making the request.
        requesting_user_id: CustomerId,
    },

    /// Invalid limit parameter.
    #[error("Invalid limit: {limit}. Must be between 1 and 100.")]
    InvalidLimit {
        /// The invalid limit value.
        limit: usize,
    },

    /// Data not fetched - QueryFetcher didn't populate the data.
    #[error("Data not fetched for query")]
    DataNotFetched,
}

// ═══════════════════════════════════════════════════════════════════════════
// State
// ═══════════════════════════════════════════════════════════════════════════

/// Analytics state.
///
/// Analytics is stateless (read-only queries), so state is minimal.
#[derive(Debug, Clone, Default)]
pub struct AnalyticsState {
    /// Placeholder to satisfy the trait
    _placeholder: (),
}

impl AnalyticsState {
    /// Create a new analytics state.
    #[must_use]
    pub const fn new() -> Self {
        Self { _placeholder: () }
    }

    /// Apply an event to update state (no-op for analytics).
    pub fn apply(&mut self, _event: &AnalyticsEvent) {
        // Analytics is read-only, no state to update
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Business Logic
// ═══════════════════════════════════════════════════════════════════════════

/// Analytics business logic.
///
/// Handles validation and returns pre-fetched data from projections.
#[derive(Debug, Clone, Copy, Default)]
pub struct AnalyticsBusinessLogic;

impl BusinessLogic for AnalyticsBusinessLogic {
    type State = AnalyticsState;
    type Input = AnalyticsCommand;
    type Event = AnalyticsEvent;
    type Error = AnalyticsError;
    type Call = Infallible; // No external calls
    type CallResult = Infallible; // No call results
    type Response = AnalyticsResponse;

    fn stream_id(input: &Self::Input) -> StreamId {
        // Analytics queries don't persist events to a meaningful stream
        // Use a placeholder stream for audit logging
        match input {
            AnalyticsCommand::GetEventSales { event_id, .. } => {
                StreamId::new(format!("analytics-event-{}", event_id.as_uuid()))
            }
            AnalyticsCommand::GetPopularSections { event_id, .. } => {
                StreamId::new(format!("analytics-event-{}", event_id.as_uuid()))
            }
            AnalyticsCommand::GetTotalRevenue { .. } => StreamId::new("analytics-global"),
            AnalyticsCommand::GetTopSpenders { .. } => StreamId::new("analytics-global"),
            AnalyticsCommand::GetCustomerProfile { customer_id, .. } => {
                StreamId::new(format!("analytics-customer-{}", customer_id.as_uuid()))
            }
        }
    }

    fn event_type_name(event: &Self::Event) -> &'static str {
        match event {
            AnalyticsEvent::EventSalesQueried { .. } => "AnalyticsEventSalesQueried",
            AnalyticsEvent::PopularSectionsQueried { .. } => "AnalyticsPopularSectionsQueried",
            AnalyticsEvent::TotalRevenueQueried { .. } => "AnalyticsTotalRevenueQueried",
            AnalyticsEvent::TopSpendersQueried { .. } => "AnalyticsTopSpendersQueried",
            AnalyticsEvent::CustomerProfileQueried { .. } => "AnalyticsCustomerProfileQueried",
        }
    }

    fn process(
        &self,
        input: Self::Input,
        _clock: &dyn Clock,
    ) -> Result<BusinessResult<Self::Event, Self::Call, Self::Response>, Self::Error> {
        match input {
            AnalyticsCommand::GetEventSales { event_id, fetched } => {
                let data = fetched.ok_or(AnalyticsError::DataNotFetched)?;

                // Validate we have data
                if data.tickets_sold == 0 && data.total_revenue.cents() == 0 {
                    return Err(AnalyticsError::EventNotFound { event_id });
                }

                Ok(BusinessResult::Respond(AnalyticsResponse::EventSales(data)))
            }

            AnalyticsCommand::GetPopularSections { event_id, fetched } => {
                let data = fetched.ok_or(AnalyticsError::DataNotFetched)?;

                // Validate we have data
                if data.most_popular.is_none() && data.highest_revenue.is_none() {
                    return Err(AnalyticsError::EventNotFound { event_id });
                }

                Ok(BusinessResult::Respond(AnalyticsResponse::PopularSections(
                    data,
                )))
            }

            AnalyticsCommand::GetTotalRevenue { fetched } => {
                let data = fetched.ok_or(AnalyticsError::DataNotFetched)?;

                Ok(BusinessResult::Respond(AnalyticsResponse::TotalRevenue(
                    data,
                )))
            }

            AnalyticsCommand::GetTopSpenders { limit, fetched } => {
                // Validate limit
                if limit == 0 || limit > 100 {
                    return Err(AnalyticsError::InvalidLimit { limit });
                }

                let data = fetched.ok_or(AnalyticsError::DataNotFetched)?;

                Ok(BusinessResult::Respond(AnalyticsResponse::TopSpenders(data)))
            }

            AnalyticsCommand::GetCustomerProfile {
                customer_id,
                requesting_user_id,
                fetched,
            } => {
                // Verify ownership (customer can only see their own profile)
                // TODO: Add admin override check
                if customer_id != requesting_user_id {
                    return Err(AnalyticsError::AccessDenied {
                        customer_id,
                        requesting_user_id,
                    });
                }

                let data = fetched.ok_or(AnalyticsError::DataNotFetched)?;

                // Validate customer exists
                if data.total_tickets == 0 && data.total_spent.cents() == 0 {
                    return Err(AnalyticsError::CustomerNotFound { customer_id });
                }

                Ok(BusinessResult::Respond(AnalyticsResponse::CustomerProfile(
                    data,
                )))
            }
        }
    }

    fn apply(&self, _state: &mut Self::State, _event: &Self::Event) {
        // Analytics is read-only, no state changes
    }

    fn feedback_input(results: Vec<Self::CallResult>) -> Self::Input {
        // Analytics has no calls - Infallible means this is never called
        match results.first() {
            Some(infallible) => match *infallible {},
            None => unreachable!("Analytics has no external calls"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use composable_rust_next::FixedClock;

    fn test_clock() -> FixedClock {
        FixedClock::new(Utc::now())
    }

    #[test]
    fn get_event_sales_succeeds_with_data() {
        let logic = AnalyticsBusinessLogic;
        let clock = test_clock();

        let event_id = EventId::new();
        let sales_data = EventSalesDto {
            event_id,
            total_revenue: Money::from_cents(10000),
            tickets_sold: 50,
            completed_reservations: 45,
            cancelled_reservations: 5,
            average_ticket_price: Money::from_cents(200),
            sections: vec![SectionSalesDto {
                section: "VIP".to_string(),
                revenue: Money::from_cents(5000),
                tickets_sold: 25,
            }],
        };

        let input = AnalyticsCommand::GetEventSales {
            event_id,
            fetched: Some(sales_data.clone()),
        };

        let result = logic.process(input, &clock);
        assert!(result.is_ok());

        match result.unwrap() {
            BusinessResult::Respond(AnalyticsResponse::EventSales(data)) => {
                assert_eq!(data.tickets_sold, 50);
            }
            _ => panic!("Expected Respond with EventSales"),
        }
    }

    #[test]
    fn get_event_sales_fails_without_data() {
        let logic = AnalyticsBusinessLogic;
        let clock = test_clock();

        let input = AnalyticsCommand::GetEventSales {
            event_id: EventId::new(),
            fetched: None,
        };

        let result = logic.process(input, &clock);
        assert!(matches!(result, Err(AnalyticsError::DataNotFetched)));
    }

    #[test]
    fn get_customer_profile_fails_for_wrong_user() {
        let logic = AnalyticsBusinessLogic;
        let clock = test_clock();

        let customer_id = CustomerId::new();
        let other_user = CustomerId::new();

        let input = AnalyticsCommand::GetCustomerProfile {
            customer_id,
            requesting_user_id: other_user,
            fetched: Some(CustomerProfileDto {
                customer_id,
                total_spent: Money::from_cents(1000),
                total_tickets: 5,
                events_attended: vec![],
                favorite_section: None,
                recent_purchases: vec![],
            }),
        };

        let result = logic.process(input, &clock);
        assert!(matches!(result, Err(AnalyticsError::AccessDenied { .. })));
    }

    #[test]
    fn get_top_spenders_validates_limit() {
        let logic = AnalyticsBusinessLogic;
        let clock = test_clock();

        // Test limit = 0
        let input = AnalyticsCommand::GetTopSpenders {
            limit: 0,
            fetched: Some(TopSpendersDto {
                customers: vec![],
                total_customers: 0,
            }),
        };
        let result = logic.process(input, &clock);
        assert!(matches!(result, Err(AnalyticsError::InvalidLimit { .. })));

        // Test limit > 100
        let input = AnalyticsCommand::GetTopSpenders {
            limit: 101,
            fetched: Some(TopSpendersDto {
                customers: vec![],
                total_customers: 0,
            }),
        };
        let result = logic.process(input, &clock);
        assert!(matches!(result, Err(AnalyticsError::InvalidLimit { .. })));
    }
}
