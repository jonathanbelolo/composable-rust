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
use crate::projections::PostgresAvailableSeatsProjection;
use crate::types::{EventId, Payment, PaymentId, Reservation, ReservationId};
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
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<(u32, u32, u32, u32)>, String>> + Send + '_>> {
        let available_seats = self.available_seats.clone();
        let event_id = *event_id;
        let section = section.to_string();
        Box::pin(async move {
            available_seats
                .get_availability(&event_id, &section)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

// ============================================================================
// Payment Projection Query Adapter
// ============================================================================

/// Adapter for querying payment data from PostgreSQL projections.
///
/// NOTE: Currently we don't have a dedicated payment projection with
/// payment history. This would typically query from a `PostgresPaymentProjection`
/// that tracks payment states, transaction history, etc.
///
/// For now, this is a stub that returns None (payment not found).
#[derive(Clone)]
pub struct PostgresPaymentQuery {
    // Future: Add Arc<PostgresPaymentProjection> when implemented
}

impl PostgresPaymentQuery {
    /// Creates a new `PostgresPaymentQuery`.
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for PostgresPaymentQuery {
    fn default() -> Self {
        Self::new()
    }
}

impl PaymentProjectionQuery for PostgresPaymentQuery {
    fn load_payment(
        &self,
        _payment_id: &PaymentId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Payment>, String>> + Send + '_>> {
        // TODO: Implement when we have a payment projection
        // For now, return None (payment state will be reconstructed from events)
        Box::pin(async move { Ok(None) })
    }
}

// ============================================================================
// Reservation Projection Query Adapter
// ============================================================================

/// Adapter for querying reservation data from PostgreSQL projections.
///
/// NOTE: Currently we don't have a dedicated reservation projection with
/// reservation history. This would typically query from a `PostgresReservationProjection`
/// that tracks reservation states, expiration times, etc.
///
/// For now, this is a stub that returns None (reservation not found).
#[derive(Clone)]
pub struct PostgresReservationQuery {
    // Future: Add Arc<PostgresReservationProjection> when implemented
}

impl PostgresReservationQuery {
    /// Creates a new `PostgresReservationQuery`.
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for PostgresReservationQuery {
    fn default() -> Self {
        Self::new()
    }
}

impl ReservationProjectionQuery for PostgresReservationQuery {
    fn load_reservation(
        &self,
        _reservation_id: &ReservationId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Reservation>, String>> + Send + '_>> {
        // TODO: Implement when we have a reservation projection
        // For now, return None (reservation state will be reconstructed from events)
        Box::pin(async move { Ok(None) })
    }
}
