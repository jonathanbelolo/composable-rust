//! Analytics aggregate for sales and customer metrics queries.
//!
//! This aggregate provides read-only query operations against analytics projections.
//! It follows the same store/reducer pattern as command aggregates for architectural
//! consistency, even though it only handles queries.
//!
//! # Query Actions
//!
//! - `GetEventSales` - Get sales metrics for a specific event
//! - `GetPopularSections` - Get most popular sections for an event
//! - `GetTotalRevenue` - Get total revenue across all events
//! - `GetTopSpenders` - Get top spending customers
//! - `GetCustomerProfile` - Get customer profile and purchase history
//!
//! # Architecture
//!
//! Analytics is query-only (no commands), so it has:
//! - Empty state (queries are stateless)
//! - Query actions that trigger `Effect::Future`
//! - Result events containing queried data
//! - Dependency injection via `AnalyticsProjectionQuery` trait

use crate::projections::{CustomerProfile, SalesMetrics};
use crate::types::{CustomerId, EventId, Money};
use composable_rust_core::{effect::Effect, reducer::Reducer, smallvec, SmallVec};
use std::sync::Arc;

// ============================================================================
// State (Empty - Analytics is stateless)
// ============================================================================

/// Analytics aggregate state.
///
/// Analytics queries are stateless - they read from projections without
/// maintaining aggregate state.
#[derive(Debug, Clone, Default)]
pub struct AnalyticsState;

impl AnalyticsState {
    /// Creates a new `AnalyticsState`.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

// ============================================================================
// Actions (Query Commands + Result Events)
// ============================================================================

/// Actions for analytics queries.
///
/// All actions are queries (no commands). Each query action has a corresponding
/// result event.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)] // Result events contain query data
pub enum AnalyticsAction {
    // ========================================================================
    // Query Commands
    // ========================================================================

    /// Get sales metrics for a specific event.
    GetEventSales {
        /// Event ID to query
        event_id: EventId,
    },

    /// Get most popular sections for an event.
    GetPopularSections {
        /// Event ID to query
        event_id: EventId,
    },

    /// Get total revenue across all events.
    GetTotalRevenue,

    /// Get top spending customers.
    GetTopSpenders {
        /// Maximum number of customers to return
        limit: usize,
    },

    /// Get customer profile and purchase history.
    GetCustomerProfile {
        /// Customer ID to query
        customer_id: CustomerId,
    },

    // ========================================================================
    // Query Result Events
    // ========================================================================

    /// Event sales metrics query result.
    EventSalesQueried {
        /// Event ID
        event_id: EventId,
        /// Sales metrics (None if event not found)
        metrics: Option<SalesMetrics>,
    },

    /// Popular sections query result.
    PopularSectionsQueried {
        /// Event ID
        event_id: EventId,
        /// Most popular section by ticket count (section, count)
        most_popular: Option<(String, u32)>,
        /// Most popular section by revenue (section, revenue)
        most_popular_revenue: Option<(String, Money)>,
    },

    /// Total revenue query result.
    TotalRevenueQueried {
        /// Total revenue across all events
        total_revenue: Money,
        /// Total tickets sold across all events
        total_tickets_sold: u32,
    },

    /// Top spenders query result.
    TopSpendersQueried {
        /// Top spending customers
        customers: Vec<CustomerProfile>,
        /// Total number of customers in system
        total_customers: usize,
    },

    /// Customer profile query result.
    CustomerProfileQueried {
        /// Customer ID
        customer_id: CustomerId,
        /// Customer profile (None if customer not found)
        profile: Option<CustomerProfile>,
    },

    // ========================================================================
    // Error Result
    // ========================================================================

    /// Query validation failed.
    ValidationFailed {
        /// Error message
        error: String,
    },
}

// ============================================================================
// Projection Query Trait (Dependency Injection)
// ============================================================================

/// Trait for querying analytics projections.
///
/// This trait abstracts away the underlying projection implementations,
/// enabling dependency injection and testability.
pub trait AnalyticsProjectionQuery: Send + Sync {
    /// Get sales metrics for an event.
    fn get_event_sales(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<SalesMetrics>, String>> + Send + '_>>;

    /// Get most popular section by ticket count for an event.
    fn get_most_popular_section(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<(String, u32)>, String>> + Send + '_>>;

    /// Get highest revenue section for an event.
    fn get_highest_revenue_section(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<(String, Money)>, String>> + Send + '_>>;

    /// Get total revenue across all events.
    fn get_total_revenue_all_events(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Money, String>> + Send + '_>>;

    /// Get total tickets sold across all events.
    fn get_total_tickets_sold(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u32, String>> + Send + '_>>;

    /// Get top spending customers.
    fn get_top_spenders(
        &self,
        limit: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<CustomerProfile>, String>> + Send + '_>>;

    /// Get customer count.
    fn get_customer_count(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<usize, String>> + Send + '_>>;

    /// Get customer profile.
    fn get_customer_profile(
        &self,
        customer_id: &CustomerId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<CustomerProfile>, String>> + Send + '_>>;
}

// ============================================================================
// Environment (Dependency Injection)
// ============================================================================

/// Environment for analytics queries.
///
/// Contains projection query implementations for loading analytics data.
#[derive(Clone)]
pub struct AnalyticsEnvironment {
    /// Analytics projection query implementation
    pub analytics_query: Arc<dyn AnalyticsProjectionQuery>,
}

impl AnalyticsEnvironment {
    /// Creates a new `AnalyticsEnvironment`.
    #[must_use]
    pub fn new(analytics_query: Arc<dyn AnalyticsProjectionQuery>) -> Self {
        Self { analytics_query }
    }
}

// ============================================================================
// Reducer
// ============================================================================

/// Reducer for analytics queries.
///
/// Handles query actions by loading data from projections via `Effect::Future`.
#[derive(Debug, Default, Clone)]
pub struct AnalyticsReducer;

impl AnalyticsReducer {
    /// Creates a new `AnalyticsReducer`.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Reducer for AnalyticsReducer {
    type State = AnalyticsState;
    type Action = AnalyticsAction;
    type Environment = AnalyticsEnvironment;

    fn reduce(
        &self,
        _state: &mut Self::State, // State unused (queries are stateless)
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ================================================================
            // Query Commands → Effect::Future
            // ================================================================

            AnalyticsAction::GetEventSales { event_id } => {
                let query = env.analytics_query.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    match query.get_event_sales(&event_id).await {
                        Ok(metrics) => Some(AnalyticsAction::EventSalesQueried {
                            event_id,
                            metrics,
                        }),
                        Err(error) => Some(AnalyticsAction::ValidationFailed { error }),
                    }
                }))]
            }

            AnalyticsAction::GetPopularSections { event_id } => {
                let query = env.analytics_query.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    // Query both most popular by count and by revenue
                    let most_popular = query.get_most_popular_section(&event_id).await.ok().flatten();
                    let most_popular_revenue = query.get_highest_revenue_section(&event_id).await.ok().flatten();

                    Some(AnalyticsAction::PopularSectionsQueried {
                        event_id,
                        most_popular,
                        most_popular_revenue,
                    })
                }))]
            }

            AnalyticsAction::GetTotalRevenue => {
                let query = env.analytics_query.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    match (
                        query.get_total_revenue_all_events().await,
                        query.get_total_tickets_sold().await,
                    ) {
                        (Ok(total_revenue), Ok(total_tickets_sold)) => {
                            Some(AnalyticsAction::TotalRevenueQueried {
                                total_revenue,
                                total_tickets_sold,
                            })
                        }
                        (Err(error), _) | (_, Err(error)) => {
                            Some(AnalyticsAction::ValidationFailed { error })
                        }
                    }
                }))]
            }

            AnalyticsAction::GetTopSpenders { limit } => {
                let query = env.analytics_query.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    match (
                        query.get_top_spenders(limit).await,
                        query.get_customer_count().await,
                    ) {
                        (Ok(customers), Ok(total_customers)) => {
                            Some(AnalyticsAction::TopSpendersQueried {
                                customers,
                                total_customers,
                            })
                        }
                        (Err(error), _) | (_, Err(error)) => {
                            Some(AnalyticsAction::ValidationFailed { error })
                        }
                    }
                }))]
            }

            AnalyticsAction::GetCustomerProfile { customer_id } => {
                let query = env.analytics_query.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    match query.get_customer_profile(&customer_id).await {
                        Ok(profile) => Some(AnalyticsAction::CustomerProfileQueried {
                            customer_id,
                            profile,
                        }),
                        Err(error) => Some(AnalyticsAction::ValidationFailed { error }),
                    }
                }))]
            }

            // ================================================================
            // Query Results → No-op (handled by API layer)
            // ================================================================

            AnalyticsAction::EventSalesQueried { .. }
            | AnalyticsAction::PopularSectionsQueried { .. }
            | AnalyticsAction::TotalRevenueQueried { .. }
            | AnalyticsAction::TopSpendersQueried { .. }
            | AnalyticsAction::CustomerProfileQueried { .. }
            | AnalyticsAction::ValidationFailed { .. } => {
                // Result events don't trigger effects - they're consumed by API handlers
                smallvec![Effect::None]
            }
        }
    }
}

// ============================================================================
// Response Data Types (for API extraction)
// ============================================================================

/// Section popularity data extracted from query results.
#[derive(Debug, Clone)]
pub struct SectionPopularity {
    /// Section name
    pub section: String,
    /// Tickets sold
    pub tickets_sold: u32,
    /// Revenue generated
    pub revenue: Money,
}

impl SectionPopularity {
    /// Creates a new `SectionPopularity`.
    #[must_use]
    pub const fn new(section: String, tickets_sold: u32, revenue: Money) -> Self {
        Self {
            section,
            tickets_sold,
            revenue,
        }
    }
}
