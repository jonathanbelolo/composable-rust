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
    inventory::InventoryProjectionQuery, payment::PaymentProjectionQuery,
    reservation::ReservationProjectionQuery,
};
use crate::projections::{PostgresAvailableSeatsProjection, PostgresPaymentsProjection, PostgresReservationsProjection};
use crate::types::{CustomerId, EventId, Payment, PaymentId, Reservation, ReservationId};
use std::sync::Arc;

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
