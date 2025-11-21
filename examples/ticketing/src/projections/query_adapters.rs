//! Query adapters for projection queries.
//!
//! This module provides adapter implementations that bridge the gap between
//! PostgreSQL projections and the query traits expected by aggregate environments.
//!
//! # Pattern: Dependency Injection via Traits
//!
//! Aggregates need to load state on-demand from projections, but they shouldn't
//! know about PostgreSQL, sqlx, or specific projection implementations.
//!
//! Solution:
//! 1. Aggregates define query traits (e.g., `InventoryProjectionQuery`)
//! 2. Adapters implement those traits by wrapping real projections
//! 3. Environments are injected with trait implementations (dependency injection)

use crate::aggregates::{
    analytics::AnalyticsProjectionQuery, inventory::InventoryProjectionQuery,
    payment::PaymentProjectionQuery, reservation::ReservationProjectionQuery,
};
use crate::projections::{
    CustomerHistoryProjection, CustomerProfile, PostgresAvailableSeatsProjection,
    PostgresCustomerHistoryProjection, PostgresPaymentsProjection,
    PostgresReservationsProjection, PostgresSalesAnalyticsProjection,
    SalesAnalyticsProjection,
};
use crate::projections::customer_history::CustomerProfile as InMemoryCustomerProfile;
use crate::projections::customer_history::CustomerPurchase as InMemoryCustomerPurchase;
use crate::projections::sales_analytics::SalesMetrics as InMemorySalesMetrics;
use crate::types::{CustomerId, EventId, Money, Payment, PaymentId, Reservation, ReservationId};
use std::sync::{Arc, RwLock};

// ============================================================================
// Inventory Projection Query Adapter
// ============================================================================

/// Adapter for querying inventory data from PostgreSQL projections.
#[derive(Clone)]
pub struct PostgresInventoryQuery {
    available_seats: Arc<PostgresAvailableSeatsProjection>,
}

impl PostgresInventoryQuery {
    /// Creates a new `PostgresInventoryQuery`.
    #[must_use]
    pub const fn new(available_seats: Arc<PostgresAvailableSeatsProjection>) -> Self {
        Self { available_seats }
    }
}

impl InventoryProjectionQuery for PostgresInventoryQuery {
    fn load_inventory(
        &self,
        event_id: &EventId,
        section: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<((u32, u32, u32, u32), Vec<crate::types::SeatAssignment>)>, String>> + Send + '_>> {
        let available_seats = self.available_seats.clone();
        let event_id = *event_id;
        let section = section.to_string();
        Box::pin(async move {
            // Load aggregate counts
            let counts = available_seats
                .get_availability(&event_id, &section)
                .await
                .map_err(|e| e.to_string())?;

            // If no counts found, return None (no data in projection)
            let Some(counts) = counts else {
                return Ok(None);
            };

            // Load individual seat assignments (complete snapshot)
            let seat_assignments = available_seats
                .load_seat_assignments(&event_id, &section)
                .await
                .map_err(|e| e.to_string())?;

            // Return complete snapshot: counts + seat assignments
            Ok(Some((counts, seat_assignments)))
        })
    }

    fn get_all_sections(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<crate::aggregates::inventory::SectionAvailabilityData>, String>> + Send + '_>> {
        let available_seats = self.available_seats.clone();
        let event_id = *event_id;
        Box::pin(async move {
            let sections = available_seats
                .get_all_sections(&event_id)
                .await
                .map_err(|e| e.to_string())?;

            #[allow(clippy::cast_sign_loss)] // Counts are always non-negative in our domain
            let data = sections
                .into_iter()
                .map(|s| crate::aggregates::inventory::SectionAvailabilityData {
                    section: s.section,
                    total_capacity: s.total_capacity as u32,
                    reserved: s.reserved as u32,
                    sold: s.sold as u32,
                    available: s.available as u32,
                })
                .collect();

            Ok(data)
        })
    }

    fn get_section_availability(
        &self,
        event_id: &EventId,
        section: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<crate::aggregates::inventory::SectionAvailabilityData>, String>> + Send + '_>> {
        let available_seats = self.available_seats.clone();
        let event_id = *event_id;
        let section_name = section.to_string();
        Box::pin(async move {
            let availability = available_seats
                .get_availability(&event_id, &section_name)
                .await
                .map_err(|e| e.to_string())?;

            let data = availability.map(|(total_capacity, reserved, sold, available)| {
                crate::aggregates::inventory::SectionAvailabilityData {
                    section: section_name,
                    total_capacity,
                    reserved,
                    sold,
                    available,
                }
            });

            Ok(data)
        })
    }

    fn get_total_available(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u32, String>> + Send + '_>> {
        let available_seats = self.available_seats.clone();
        let event_id = *event_id;
        Box::pin(async move {
            available_seats
                .get_total_available(&event_id)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

// ============================================================================
// Payment Projection Query Adapter
// ============================================================================

/// Adapter for querying payment data from PostgreSQL projections.
#[derive(Clone)]
pub struct PostgresPaymentQuery {
    payments: Arc<PostgresPaymentsProjection>,
}

impl PostgresPaymentQuery {
    /// Creates a new `PostgresPaymentQuery`.
    #[must_use]
    pub const fn new(payments: Arc<PostgresPaymentsProjection>) -> Self {
        Self { payments }
    }
}

#[async_trait::async_trait]
impl PaymentProjectionQuery for PostgresPaymentQuery {
    async fn load_payment(&self, payment_id: &PaymentId) -> Result<Option<Payment>, String> {
        self.payments
            .get_payment(payment_id)
            .await
            .map_err(|e| e.to_string())
    }

    async fn load_customer_payments(&self, customer_id: &CustomerId, limit: usize, offset: usize) -> Result<Vec<Payment>, String> {
        self.payments
            .load_customer_payments(customer_id, limit, offset)
            .await
    }
}

// ============================================================================
// Reservation Projection Query Adapter
// ============================================================================

/// Adapter for querying reservation data from PostgreSQL projections.
#[derive(Clone)]
pub struct PostgresReservationQuery {
    reservations: Arc<PostgresReservationsProjection>,
}

impl PostgresReservationQuery {
    /// Creates a new `PostgresReservationQuery`.
    #[must_use]
    pub const fn new(reservations: Arc<PostgresReservationsProjection>) -> Self {
        Self { reservations }
    }
}

impl ReservationProjectionQuery for PostgresReservationQuery {
    fn load_reservation(
        &self,
        reservation_id: &ReservationId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Reservation>, String>> + Send + '_>> {
        let reservations = self.reservations.clone();
        let reservation_id = *reservation_id;
        Box::pin(async move {
            reservations
                .get(&reservation_id)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn list_by_customer(
        &self,
        customer_id: &CustomerId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Reservation>, String>> + Send + '_>> {
        let reservations = self.reservations.clone();
        let customer_id = *customer_id;
        Box::pin(async move {
            reservations
                .list_by_customer(&customer_id)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

// ============================================================================
// Analytics Projection Query Adapter
// ============================================================================

/// Adapter for querying analytics data from in-memory projections.
///
/// This adapter wraps `RwLock`-protected in-memory projections for sales
/// and customer history analytics.
#[derive(Clone)]
pub struct InMemoryAnalyticsQuery {
    sales_projection: Arc<RwLock<SalesAnalyticsProjection>>,
    customer_projection: Arc<RwLock<CustomerHistoryProjection>>,
}

impl InMemoryAnalyticsQuery {
    /// Creates a new `InMemoryAnalyticsQuery`.
    #[must_use]
    pub const fn new(
        sales_projection: Arc<RwLock<SalesAnalyticsProjection>>,
        customer_projection: Arc<RwLock<CustomerHistoryProjection>>,
    ) -> Self {
        Self {
            sales_projection,
            customer_projection,
        }
    }
}

impl AnalyticsProjectionQuery for InMemoryAnalyticsQuery {
    fn get_event_sales(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<InMemorySalesMetrics>, String>> + Send + '_>> {
        let sales_projection = self.sales_projection.clone();
        let event_id = *event_id;
        Box::pin(async move {
            let projection = sales_projection
                .read()
                .map_err(|e| format!("Failed to acquire read lock: {e}"))?;

            let metrics = projection.get_metrics(&event_id).cloned();
            Ok(metrics)
        })
    }

    fn get_most_popular_section(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<(String, u32)>, String>> + Send + '_>> {
        let sales_projection = self.sales_projection.clone();
        let event_id = *event_id;
        Box::pin(async move {
            let projection = sales_projection
                .read()
                .map_err(|e| format!("Failed to acquire read lock: {e}"))?;

            let result = projection
                .get_most_popular_section(&event_id)
                .map(|(section, count)| (section.clone(), count));
            Ok(result)
        })
    }

    fn get_highest_revenue_section(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<(String, Money)>, String>> + Send + '_>> {
        let sales_projection = self.sales_projection.clone();
        let event_id = *event_id;
        Box::pin(async move {
            let projection = sales_projection
                .read()
                .map_err(|e| format!("Failed to acquire read lock: {e}"))?;

            let result = projection
                .get_highest_revenue_section(&event_id)
                .map(|(section, revenue)| (section.clone(), revenue));
            Ok(result)
        })
    }

    fn get_total_revenue_all_events(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Money, String>> + Send + '_>> {
        let sales_projection = self.sales_projection.clone();
        Box::pin(async move {
            let projection = sales_projection
                .read()
                .map_err(|e| format!("Failed to acquire read lock: {e}"))?;

            Ok(projection.get_total_revenue_all_events())
        })
    }

    fn get_total_tickets_sold(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u32, String>> + Send + '_>> {
        let sales_projection = self.sales_projection.clone();
        Box::pin(async move {
            let projection = sales_projection
                .read()
                .map_err(|e| format!("Failed to acquire read lock: {e}"))?;

            Ok(projection.get_total_tickets_sold())
        })
    }

    fn get_top_spenders(
        &self,
        limit: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<CustomerProfile>, String>> + Send + '_>> {
        let customer_projection = self.customer_projection.clone();
        Box::pin(async move {
            let projection = customer_projection
                .read()
                .map_err(|e| format!("Failed to acquire read lock: {e}"))?;

            let profiles = projection.get_top_spenders(limit);
            let owned_profiles: Vec<CustomerProfile> =
                profiles.into_iter().cloned().collect();
            Ok(owned_profiles)
        })
    }

    fn get_customer_count(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<usize, String>> + Send + '_>> {
        let customer_projection = self.customer_projection.clone();
        Box::pin(async move {
            let projection = customer_projection
                .read()
                .map_err(|e| format!("Failed to acquire read lock: {e}"))?;

            Ok(projection.get_customer_count())
        })
    }

    fn get_customer_profile(
        &self,
        customer_id: &CustomerId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<CustomerProfile>, String>> + Send + '_>> {
        let customer_projection = self.customer_projection.clone();
        let customer_id = *customer_id;
        Box::pin(async move {
            let projection = customer_projection
                .read()
                .map_err(|e| format!("Failed to acquire read lock: {e}"))?;

            let profile = projection.get_customer_profile(&customer_id).cloned();
            Ok(profile)
        })
    }
}

// ============================================================================
// Analytics Projection Query Adapter (PostgreSQL)
// ============================================================================

/// Adapter for querying analytics data from PostgreSQL projections.
///
/// This adapter bridges the `AnalyticsProjectionQuery` trait (used by the Analytics aggregate)
/// with the PostgreSQL-backed sales analytics and customer history projections.
///
/// Unlike the in-memory version, this adapter delegates to async PostgreSQL queries
/// and handles Result types from database operations.
#[derive(Clone)]
pub struct PostgresAnalyticsQuery {
    sales_projection: Arc<PostgresSalesAnalyticsProjection>,
    customer_projection: Arc<PostgresCustomerHistoryProjection>,
}

impl PostgresAnalyticsQuery {
    /// Creates a new `PostgresAnalyticsQuery`.
    #[must_use]
    pub const fn new(
        sales_projection: Arc<PostgresSalesAnalyticsProjection>,
        customer_projection: Arc<PostgresCustomerHistoryProjection>,
    ) -> Self {
        Self {
            sales_projection,
            customer_projection,
        }
    }
}

impl AnalyticsProjectionQuery for PostgresAnalyticsQuery {
    fn get_event_sales(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<InMemorySalesMetrics>, String>> + Send + '_>> {
        let sales_projection = self.sales_projection.clone();
        let event_id = *event_id;
        Box::pin(async move {
            let pg_metrics = sales_projection
                .get_metrics(&event_id)
                .await
                .map_err(|e| e.to_string())?;

            let Some(m) = pg_metrics else {
                return Ok(None);
            };

            // Load section-level data from PostgreSQL (separate query)
            let section_metrics = sales_projection
                .get_section_metrics(&event_id)
                .await
                .map_err(|e| e.to_string())?;

            // Build HashMaps from section metrics
            let mut revenue_by_section = std::collections::HashMap::new();
            let mut tickets_by_section = std::collections::HashMap::new();

            for section in section_metrics {
                revenue_by_section.insert(section.section.clone(), section.revenue);
                tickets_by_section.insert(section.section, section.tickets_sold);
            }

            // Convert PostgreSQL SalesMetrics to in-memory SalesMetrics with full data
            Ok(Some(InMemorySalesMetrics {
                event_id: m.event_id,
                total_revenue: m.total_revenue,
                tickets_sold: m.tickets_sold,
                completed_reservations: m.completed_reservations,
                cancelled_reservations: m.cancelled_reservations,
                revenue_by_section,     // Now loaded from PostgreSQL
                tickets_by_section,     // Now loaded from PostgreSQL
                average_ticket_price: m.average_ticket_price,
            }))
        })
    }

    fn get_most_popular_section(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<(String, u32)>, String>> + Send + '_>> {
        let sales_projection = self.sales_projection.clone();
        let event_id = *event_id;
        Box::pin(async move {
            sales_projection
                .get_most_popular_section(&event_id)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn get_highest_revenue_section(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<(String, Money)>, String>> + Send + '_>> {
        let sales_projection = self.sales_projection.clone();
        let event_id = *event_id;
        Box::pin(async move {
            sales_projection
                .get_highest_revenue_section(&event_id)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn get_total_revenue_all_events(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Money, String>> + Send + '_>> {
        let sales_projection = self.sales_projection.clone();
        Box::pin(async move {
            sales_projection
                .get_total_revenue_all_events()
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn get_total_tickets_sold(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u32, String>> + Send + '_>> {
        let sales_projection = self.sales_projection.clone();
        Box::pin(async move {
            sales_projection
                .get_total_tickets_sold()
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn get_top_spenders(
        &self,
        limit: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<CustomerProfile>, String>> + Send + '_>> {
        let customer_projection = self.customer_projection.clone();
        Box::pin(async move {
            // Note: PostgreSQL method expects i64 for limit
            #[allow(clippy::cast_possible_wrap)]
            let limit_i64 = limit as i64;
            let pg_profiles = customer_projection
                .get_top_spenders(limit_i64)
                .await
                .map_err(|e| e.to_string())?;

            // Convert PostgreSQL CustomerProfile to in-memory CustomerProfile
            // Load detailed purchase data for each customer from PostgreSQL
            let mut profiles = Vec::new();
            for pg in pg_profiles {
                // Load purchase history from PostgreSQL (separate query)
                let pg_purchases = customer_projection
                    .get_customer_purchases(&pg.customer_id)
                    .await
                    .map_err(|e| e.to_string())?;

                // Convert PostgreSQL purchases to in-memory purchases
                let purchases: Vec<InMemoryCustomerPurchase> = pg_purchases
                    .into_iter()
                    .map(|p| InMemoryCustomerPurchase {
                        reservation_id: p.reservation_id,
                        event_id: p.event_id,
                        section: p.section,
                        ticket_count: p.ticket_count,
                        amount_paid: p.amount_paid,
                        tickets: p.tickets,
                        completed_at: p.completed_at,
                    })
                    .collect();

                // Derive events attended from purchases (unique event IDs)
                let mut events_attended: Vec<_> = purchases
                    .iter()
                    .map(|p| p.event_id)
                    .collect();
                events_attended.sort_unstable();
                events_attended.dedup();

                profiles.push(InMemoryCustomerProfile {
                    customer_id: pg.customer_id,
                    purchases,               // Now loaded from PostgreSQL and converted
                    total_spent: pg.total_spent,
                    total_tickets: pg.total_tickets,
                    events_attended,         // Now derived from purchases
                    favorite_section: pg.favorite_section,
                });
            }

            Ok(profiles)
        })
    }

    fn get_customer_count(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<usize, String>> + Send + '_>> {
        let customer_projection = self.customer_projection.clone();
        Box::pin(async move {
            let count = customer_projection
                .get_customer_count()
                .await
                .map_err(|e| e.to_string())?;
            Ok(count as usize)
        })
    }

    fn get_customer_profile(
        &self,
        customer_id: &CustomerId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<CustomerProfile>, String>> + Send + '_>> {
        let customer_projection = self.customer_projection.clone();
        let customer_id = *customer_id;
        Box::pin(async move {
            let pg_profile = customer_projection
                .get_customer_profile(&customer_id)
                .await
                .map_err(|e| e.to_string())?;

            let Some(pg) = pg_profile else {
                return Ok(None);
            };

            // Load purchase history from PostgreSQL (separate query)
            let pg_purchases = customer_projection
                .get_customer_purchases(&customer_id)
                .await
                .map_err(|e| e.to_string())?;

            // Convert PostgreSQL purchases to in-memory purchases
            let purchases: Vec<InMemoryCustomerPurchase> = pg_purchases
                .into_iter()
                .map(|p| InMemoryCustomerPurchase {
                    reservation_id: p.reservation_id,
                    event_id: p.event_id,
                    section: p.section,
                    ticket_count: p.ticket_count,
                    amount_paid: p.amount_paid,
                    tickets: p.tickets,
                    completed_at: p.completed_at,
                })
                .collect();

            // Derive events attended from purchases (unique event IDs)
            let mut events_attended: Vec<_> = purchases
                .iter()
                .map(|p| p.event_id)
                .collect();
            events_attended.sort_unstable();
            events_attended.dedup();

            // Convert PostgreSQL CustomerProfile to in-memory CustomerProfile with full data
            Ok(Some(InMemoryCustomerProfile {
                customer_id: pg.customer_id,
                purchases,               // Now loaded from PostgreSQL and converted
                total_spent: pg.total_spent,
                total_tickets: pg.total_tickets,
                events_attended,         // Now derived from purchases
                favorite_section: pg.favorite_section,
            }))
        })
    }
}
