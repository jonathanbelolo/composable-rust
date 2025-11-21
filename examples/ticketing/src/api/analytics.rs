//! Analytics and reporting API endpoints.
//!
//! Provides endpoints for business intelligence and reporting:
//! - GET /api/analytics/events/:id/sales - Sales metrics for event
//! - GET /api/analytics/events/:id/sections/popular - Most popular sections
//! - GET /api/analytics/revenue - Total revenue across all events
//! - GET /api/analytics/customers/top-spenders - Top spending customers
//! - GET /api/analytics/customers/:id/profile - Customer purchase history (requires auth + ownership)
//!
//! # Analytics Architecture
//!
//! Analytics queries run against **read-side projections** (CQRS read models):
//! - `SalesAnalyticsProjection` - Revenue, tickets sold, section popularity
//! - `CustomerHistoryProjection` - Purchase history, spending patterns
//!
//! These projections are **eventually consistent** (milliseconds behind write side),
//! but optimized for fast aggregation queries without impacting write performance.
//!
//! # Security Notes
//!
//! - Event-level analytics are public (useful for ticket buyers to see popularity)
//! - System-wide revenue requires admin access
//! - Individual customer profiles require authentication + ownership verification

use crate::auth::middleware::{RequireAdmin, RequireOwnership};
use crate::server::state::AppState;
use crate::types::CustomerId;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use composable_rust_web::error::AppError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Request/Response Types
// ============================================================================

/// Sales metrics for a specific event.
#[derive(Debug, Serialize)]
pub struct EventSalesResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Total revenue for this event
    pub total_revenue: i64, // cents
    /// Number of tickets sold
    pub tickets_sold: u32,
    /// Number of completed reservations
    pub completed_reservations: u32,
    /// Number of cancelled/expired reservations
    pub cancelled_reservations: u32,
    /// Average ticket price
    pub average_ticket_price: i64, // cents
    /// Revenue breakdown by section
    pub sections: Vec<SectionSalesMetrics>,
}

/// Sales metrics for a section.
#[derive(Debug, Serialize)]
pub struct SectionSalesMetrics {
    /// Section name (e.g., "VIP", "General")
    pub section: String,
    /// Revenue from this section
    pub revenue: i64, // cents
    /// Tickets sold in this section
    pub tickets_sold: u32,
}

/// Popular sections response.
#[derive(Debug, Serialize)]
pub struct PopularSectionsResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Most popular section by ticket count
    pub most_popular: Option<SectionPopularity>,
    /// Highest revenue section
    pub highest_revenue: Option<SectionPopularity>,
}

/// Section popularity metrics.
#[derive(Debug, Serialize)]
pub struct SectionPopularity {
    /// Section name
    pub section: String,
    /// Tickets sold
    pub tickets_sold: u32,
    /// Revenue generated
    pub revenue: i64, // cents
}

/// Total revenue response.
#[derive(Debug, Serialize)]
pub struct TotalRevenueResponse {
    /// Total revenue across all events
    pub total_revenue: i64, // cents
    /// Total tickets sold
    pub total_tickets_sold: u32,
    /// Number of events with sales
    pub events_with_sales: usize,
}

/// Query parameters for top spenders.
#[derive(Debug, Deserialize)]
pub struct TopSpendersQuery {
    /// Number of top spenders to return (default: 10, max: 100)
    #[serde(default = "default_limit")]
    pub limit: usize,
}

const fn default_limit() -> usize {
    10
}

/// Top spending customers response.
#[derive(Debug, Serialize)]
pub struct TopSpendersResponse {
    /// Top spending customers
    pub customers: Vec<CustomerSpendingSummary>,
    /// Total number of customers in system
    pub total_customers: usize,
}

/// Customer spending summary for leaderboard.
#[derive(Debug, Serialize)]
pub struct CustomerSpendingSummary {
    /// Customer ID
    pub customer_id: Uuid,
    /// Total amount spent
    pub total_spent: i64, // cents
    /// Total tickets purchased
    pub total_tickets: u32,
    /// Number of events attended
    pub events_attended: usize,
    /// Favorite section (most frequently purchased)
    pub favorite_section: Option<String>,
}

/// Customer profile response.
#[derive(Debug, Serialize)]
pub struct CustomerProfileResponse {
    /// Customer ID
    pub customer_id: Uuid,
    /// Total amount spent
    pub total_spent: i64, // cents
    /// Total tickets purchased
    pub total_tickets: u32,
    /// Events attended (unique event IDs)
    pub events_attended: Vec<Uuid>,
    /// Favorite section
    pub favorite_section: Option<String>,
    /// Recent purchases (last 10)
    pub recent_purchases: Vec<PurchaseRecord>,
}

/// Purchase record for customer history.
#[derive(Debug, Serialize)]
pub struct PurchaseRecord {
    /// Reservation ID
    pub reservation_id: Uuid,
    /// Event ID
    pub event_id: Uuid,
    /// Section
    pub section: String,
    /// Number of tickets
    pub ticket_count: u32,
    /// Amount paid
    pub amount_paid: i64, // cents
    /// When purchased
    pub completed_at: DateTime<Utc>,
}

// ============================================================================
// Handlers
// ============================================================================

/// Get sales metrics for a specific event.
///
/// Returns aggregated sales data including revenue, tickets sold, and section breakdowns.
/// This endpoint is **public** - no authentication required. Useful for potential buyers
/// to see event popularity.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/api/analytics/events/550e8400-e29b-41d4-a716-446655440000/sales
/// ```
///
/// Response:
/// ```json
/// {
///   "event_id": "550e8400-e29b-41d4-a716-446655440000",
///   "total_revenue": 150000,
///   "tickets_sold": 500,
///   "completed_reservations": 120,
///   "cancelled_reservations": 10,
///   "average_ticket_price": 30000,
///   "sections": [
///     {
///       "section": "VIP",
///       "revenue": 100000,
///       "tickets_sold": 100
///     },
///     {
///       "section": "General",
///       "revenue": 50000,
///       "tickets_sold": 400
///     }
///   ]
/// }
/// ```
///
/// # Errors
///
/// Returns `AppError::NotFound` if event has no sales data.
#[allow(clippy::cast_possible_wrap)] // Money amounts in cents won't exceed i64::MAX in practice
pub async fn get_event_sales(
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<EventSalesResponse>, AppError> {
    use crate::aggregates::analytics::AnalyticsAction;
    use crate::types::EventId;

    let event_id_typed = EventId::from_uuid(event_id);

    // Create analytics store for this request
    let store = state.create_analytics_store();

    // Query sales metrics via store/reducer
    let metrics = match store
        .send_and_wait_for(
            AnalyticsAction::GetEventSales {
                event_id: event_id_typed,
            },
            |action| {
                matches!(
                    action,
                    AnalyticsAction::EventSalesQueried { .. }
                        | AnalyticsAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
    {
        Ok(AnalyticsAction::EventSalesQueried { metrics, .. }) => {
            metrics.ok_or_else(|| AppError::not_found("Sales data", event_id))?
        }
        Ok(AnalyticsAction::ValidationFailed { error }) => {
            return Err(AppError::internal(format!("Query failed: {error}")))
        }
        Ok(_) => return Err(AppError::internal("Unexpected action received")),
        Err(e) => return Err(AppError::internal(format!("Failed to query sales: {e}"))),
    };

    // Convert revenue_by_section HashMap to Vec
    let sections: Vec<SectionSalesMetrics> = metrics
        .revenue_by_section
        .iter()
        .map(|(section, revenue)| SectionSalesMetrics {
            section: section.clone(),
            revenue: revenue.cents() as i64,
            tickets_sold: *metrics.tickets_by_section.get(section).unwrap_or(&0),
        })
        .collect();

    Ok(Json(EventSalesResponse {
        event_id,
        total_revenue: metrics.total_revenue.cents() as i64,
        tickets_sold: metrics.tickets_sold,
        completed_reservations: metrics.completed_reservations,
        cancelled_reservations: metrics.cancelled_reservations,
        average_ticket_price: metrics.average_ticket_price.cents() as i64,
        sections,
    }))
}

/// Get most popular sections for an event.
///
/// Returns both most popular by ticket count and highest revenue section.
/// Public endpoint - no authentication required.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/api/analytics/events/550e8400-e29b-41d4-a716-446655440000/sections/popular
/// ```
///
/// Response:
/// ```json
/// {
///   "event_id": "550e8400-e29b-41d4-a716-446655440000",
///   "most_popular": {
///     "section": "General",
///     "tickets_sold": 400,
///     "revenue": 50000
///   },
///   "highest_revenue": {
///     "section": "VIP",
///     "tickets_sold": 100,
///     "revenue": 100000
///   }
/// }
/// ```
///
/// # Errors
///
/// Returns `AppError::NotFound` if event has no sales data.
#[allow(clippy::cast_possible_wrap)] // Money amounts in cents won't exceed i64::MAX in practice
pub async fn get_popular_sections(
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<PopularSectionsResponse>, AppError> {
    use crate::aggregates::analytics::AnalyticsAction;
    use crate::types::EventId;

    let event_id_typed = EventId::from_uuid(event_id);

    // Create analytics store for this request
    let store = state.create_analytics_store();

    // Query popular sections via store/reducer
    let (most_popular_data, most_popular_revenue_data) = match store
        .send_and_wait_for(
            AnalyticsAction::GetPopularSections {
                event_id: event_id_typed,
            },
            |action| {
                matches!(
                    action,
                    AnalyticsAction::PopularSectionsQueried { .. }
                        | AnalyticsAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
    {
        Ok(AnalyticsAction::PopularSectionsQueried {
            most_popular,
            most_popular_revenue,
            ..
        }) => (most_popular, most_popular_revenue),
        Ok(AnalyticsAction::ValidationFailed { error }) => {
            return Err(AppError::internal(format!("Query failed: {error}")))
        }
        Ok(_) => return Err(AppError::internal("Unexpected action received")),
        Err(e) => return Err(AppError::internal(format!("Failed to query sections: {e}"))),
    };

    // Convert to response format
    let most_popular = most_popular_data.as_ref().map(|(section, tickets_sold)| {
        // Need to get revenue for this section - query again
        let revenue = most_popular_revenue_data
            .as_ref()
            .filter(|(s, _)| s == section)
            .map(|(_, r)| *r)
            .unwrap_or_else(|| crate::types::Money::from_cents(0));

        SectionPopularity {
            section: section.clone(),
            tickets_sold: *tickets_sold,
            revenue: revenue.cents() as i64,
        }
    });

    let highest_revenue = most_popular_revenue_data.as_ref().map(|(section, revenue)| {
        // Need to get ticket count for this section
        let tickets_sold = most_popular_data
            .as_ref()
            .filter(|(s, _)| s == section)
            .map(|(_, count)| *count)
            .unwrap_or(0);

        SectionPopularity {
            section: section.clone(),
            tickets_sold,
            revenue: revenue.cents() as i64,
        }
    });

    // Return 404 if no data at all
    if most_popular.is_none() && highest_revenue.is_none() {
        return Err(AppError::not_found("Sales data", event_id));
    }

    Ok(Json(PopularSectionsResponse {
        event_id,
        most_popular,
        highest_revenue,
    }))
}

/// Get total revenue across all events.
///
/// Requires **admin** authentication. System-wide financial metrics are sensitive.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/api/analytics/revenue \
///   -H "Authorization: Bearer <admin_session_token>"
/// ```
///
/// Response:
/// ```json
/// {
///   "total_revenue": 5000000,
///   "total_tickets_sold": 10000,
///   "events_with_sales": 50
/// }
/// ```
///
/// # Errors
///
/// Returns `AppError::Unauthorized` if not authenticated.
/// Returns `AppError::Forbidden` if not admin.
#[allow(clippy::cast_possible_wrap)] // Money amounts in cents won't exceed i64::MAX in practice
pub async fn get_total_revenue(
    _admin: RequireAdmin,
    State(state): State<AppState>,
) -> Result<Json<TotalRevenueResponse>, AppError> {
    use crate::aggregates::analytics::AnalyticsAction;

    // Create analytics store for this request
    let store = state.create_analytics_store();

    // Query total revenue via store/reducer
    let (total_revenue, total_tickets_sold) = match store
        .send_and_wait_for(
            AnalyticsAction::GetTotalRevenue,
            |action| {
                matches!(
                    action,
                    AnalyticsAction::TotalRevenueQueried { .. }
                        | AnalyticsAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
    {
        Ok(AnalyticsAction::TotalRevenueQueried {
            total_revenue,
            total_tickets_sold,
        }) => (total_revenue, total_tickets_sold),
        Ok(AnalyticsAction::ValidationFailed { error }) => {
            return Err(AppError::internal(format!("Query failed: {error}")))
        }
        Ok(_) => return Err(AppError::internal("Unexpected action received")),
        Err(e) => return Err(AppError::internal(format!("Failed to query revenue: {e}"))),
    };

    // TODO: Add method to count all events with sales
    let events_with_sales = 0;

    Ok(Json(TotalRevenueResponse {
        total_revenue: total_revenue.cents() as i64,
        total_tickets_sold,
        events_with_sales, // TODO: Implement proper counting
    }))
}

/// Get top spending customers.
///
/// Requires **admin** authentication. Customer financial data is sensitive.
///
/// Returns customers sorted by total spending (highest first).
/// Limit can be specified via query parameter (default 10, max 100).
///
/// # Example
///
/// ```bash
/// curl 'http://localhost:8080/api/analytics/customers/top-spenders?limit=5' \
///   -H "Authorization: Bearer <admin_session_token>"
/// ```
///
/// Response:
/// ```json
/// {
///   "customers": [
///     {
///       "customer_id": "880e8400-e29b-41d4-a716-446655440003",
///       "total_spent": 500000,
///       "total_tickets": 50,
///       "events_attended": 10,
///       "favorite_section": "VIP"
///     }
///   ],
///   "total_customers": 1000
/// }
/// ```
///
/// # Errors
///
/// Returns `AppError::Unauthorized` if not authenticated.
/// Returns `AppError::Forbidden` if not admin.
/// Returns `AppError::BadRequest` if limit exceeds 100.
#[allow(clippy::cast_possible_wrap)] // Money amounts in cents won't exceed i64::MAX in practice
pub async fn get_top_spenders(
    _admin: RequireAdmin,
    Query(params): Query<TopSpendersQuery>,
    State(state): State<AppState>,
) -> Result<Json<TopSpendersResponse>, AppError> {
    use crate::aggregates::analytics::AnalyticsAction;

    // Validate limit
    if params.limit > 100 {
        return Err(AppError::bad_request(
            "Limit cannot exceed 100. Use pagination for larger datasets.",
        ));
    }

    // Create analytics store for this request
    let store = state.create_analytics_store();

    // Query top spenders via store/reducer
    let (top_spenders, total_customers) = match store
        .send_and_wait_for(
            AnalyticsAction::GetTopSpenders {
                limit: params.limit,
            },
            |action| {
                matches!(
                    action,
                    AnalyticsAction::TopSpendersQueried { .. }
                        | AnalyticsAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
    {
        Ok(AnalyticsAction::TopSpendersQueried {
            customers,
            total_customers,
        }) => (customers, total_customers),
        Ok(AnalyticsAction::ValidationFailed { error }) => {
            return Err(AppError::internal(format!("Query failed: {error}")))
        }
        Ok(_) => return Err(AppError::internal("Unexpected action received")),
        Err(e) => return Err(AppError::internal(format!("Failed to query top spenders: {e}"))),
    };

    // Map to response type
    let customers: Vec<CustomerSpendingSummary> = top_spenders
        .iter()
        .map(|profile| CustomerSpendingSummary {
            customer_id: *profile.customer_id.as_uuid(),
            total_spent: profile.total_spent.cents() as i64,
            total_tickets: profile.total_tickets,
            events_attended: profile.events_attended.len(),
            favorite_section: profile.favorite_section.clone(),
        })
        .collect();

    Ok(Json(TopSpendersResponse {
        customers,
        total_customers,
    }))
}

/// Get customer profile and purchase history.
///
/// Requires authentication and **ownership** - customers can only see their own profile.
/// Admins can override to view any customer profile (TODO: implement admin check).
///
/// Returns last 10 purchases by default. For full history, use paginated endpoint (TODO).
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/api/analytics/customers/880e8400-e29b-41d4-a716-446655440003/profile \
///   -H "Authorization: Bearer <session_token>"
/// ```
///
/// Response:
/// ```json
/// {
///   "customer_id": "880e8400-e29b-41d4-a716-446655440003",
///   "total_spent": 150000,
///   "total_tickets": 20,
///   "events_attended": ["550e8400-...", "660e8400-..."],
///   "favorite_section": "VIP",
///   "recent_purchases": [
///     {
///       "reservation_id": "770e8400-...",
///       "event_id": "550e8400-...",
///       "section": "VIP",
///       "ticket_count": 2,
///       "amount_paid": 20000,
///       "completed_at": "2024-01-15T14:30:00Z"
///     }
///   ]
/// }
/// ```
///
/// # Errors
///
/// Returns `AppError::Unauthorized` if not authenticated.
/// Returns `AppError::Forbidden` if customer ID doesn't match authenticated user.
/// Returns `AppError::NotFound` if customer has no purchase history.
#[allow(clippy::cast_possible_wrap)] // Money amounts in cents won't exceed i64::MAX in practice
pub async fn get_customer_profile(
    ownership: RequireOwnership<CustomerId>,
    Path(customer_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<CustomerProfileResponse>, AppError> {
    use crate::aggregates::analytics::AnalyticsAction;

    // Ownership verified by RequireOwnership extractor
    // ownership.user_id is the authenticated user
    // ownership.resource is the CustomerId from the path
    let _ = ownership;

    // TODO: Also check if user is admin for override capability

    let customer_id_typed = CustomerId::from_uuid(customer_id);

    // Create analytics store for this request
    let store = state.create_analytics_store();

    // Query customer profile via store/reducer
    let profile = match store
        .send_and_wait_for(
            AnalyticsAction::GetCustomerProfile {
                customer_id: customer_id_typed,
            },
            |action| {
                matches!(
                    action,
                    AnalyticsAction::CustomerProfileQueried { .. }
                        | AnalyticsAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
    {
        Ok(AnalyticsAction::CustomerProfileQueried { profile, .. }) => {
            profile.ok_or_else(|| AppError::not_found("Customer profile", customer_id))?
        }
        Ok(AnalyticsAction::ValidationFailed { error }) => {
            return Err(AppError::internal(format!("Query failed: {error}")))
        }
        Ok(_) => return Err(AppError::internal("Unexpected action received")),
        Err(e) => {
            return Err(AppError::internal(format!(
                "Failed to query customer profile: {e}"
            )))
        }
    };

    // Sort purchases by completed_at descending and take last 10
    let mut sorted_purchases = profile.purchases.clone();
    sorted_purchases.sort_by(|a, b| b.completed_at.cmp(&a.completed_at));
    let recent_purchases: Vec<PurchaseRecord> = sorted_purchases
        .iter()
        .take(10)
        .map(|purchase| PurchaseRecord {
            reservation_id: *purchase.reservation_id.as_uuid(),
            event_id: *purchase.event_id.as_uuid(),
            section: purchase.section.clone(),
            ticket_count: purchase.ticket_count,
            amount_paid: purchase.amount_paid.cents() as i64,
            completed_at: purchase.completed_at,
        })
        .collect();

    Ok(Json(CustomerProfileResponse {
        customer_id,
        total_spent: profile.total_spent.cents() as i64,
        total_tickets: profile.total_tickets,
        events_attended: profile.events_attended.iter().map(|id| *id.as_uuid()).collect(),
        favorite_section: profile.favorite_section.clone(),
        recent_purchases,
    }))
}
