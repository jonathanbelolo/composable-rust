//! PostgreSQL-backed reservations projection using JSONB.
//!
//! Stores full `Reservation` domain objects as JSONB for simplicity.
//! Provides fast queries by reservation_id and customer_id.

use crate::aggregates::ReservationAction;
use crate::projections::TicketingEvent;
use crate::types::{CustomerId, Reservation, ReservationId};
use composable_rust_core::projection::{Projection, ProjectionError, Result};
use sqlx::PgPool;
use std::sync::Arc;

/// PostgreSQL-backed reservations projection.
///
/// Stores reservations as JSONB in the `reservations_projection` table.
/// Indexed by reservation_id and customer_id for fast lookups.
#[derive(Clone)]
pub struct PostgresReservationsProjection {
    pool: Arc<PgPool>,
}

impl PostgresReservationsProjection {
    /// Creates a new `PostgresReservationsProjection`.
    #[must_use]
    pub const fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    /// Get a reservation by ID.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails or JSON deserialization fails.
    pub async fn get(&self, reservation_id: &ReservationId) -> Result<Option<Reservation>> {
        let result: Option<(sqlx::types::JsonValue,)> = sqlx::query_as(
            "SELECT data FROM reservations_projection WHERE id = $1"
        )
        .bind(reservation_id.as_uuid())
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query reservation: {e}")))?;

        match result {
            Some((json,)) => {
                let reservation: Reservation = serde_json::from_value(json)
                    .map_err(|e| ProjectionError::Storage(format!("Failed to deserialize reservation: {e}")))?;
                Ok(Some(reservation))
            }
            None => Ok(None),
        }
    }

    /// List all reservations for a specific customer.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails or JSON deserialization fails.
    pub async fn list_by_customer(&self, customer_id: &CustomerId) -> Result<Vec<Reservation>> {
        let rows: Vec<(sqlx::types::JsonValue,)> = sqlx::query_as(
            "SELECT data FROM reservations_projection
             WHERE customer_id = $1
             ORDER BY created_at DESC"
        )
        .bind(customer_id.as_uuid())
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query reservations: {e}")))?;

        let reservations: std::result::Result<Vec<Reservation>, serde_json::Error> = rows
            .into_iter()
            .map(|(json,)| serde_json::from_value(json))
            .collect();

        reservations.map_err(|e| ProjectionError::Storage(format!("Failed to deserialize reservations: {e}")))
    }

    /// List all reservations (for admin purposes).
    ///
    /// # Errors
    ///
    /// Returns error if database query fails or JSON deserialization fails.
    pub async fn list_all(&self) -> Result<Vec<Reservation>> {
        let rows: Vec<(sqlx::types::JsonValue,)> = sqlx::query_as(
            "SELECT data FROM reservations_projection
             ORDER BY created_at DESC"
        )
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query reservations: {e}")))?;

        let reservations: std::result::Result<Vec<Reservation>, serde_json::Error> = rows
            .into_iter()
            .map(|(json,)| serde_json::from_value(json))
            .collect();

        reservations.map_err(|e| ProjectionError::Storage(format!("Failed to deserialize reservations: {e}")))
    }
}

impl Projection for PostgresReservationsProjection {
    type Event = TicketingEvent;

    fn name(&self) -> &'static str {
        "reservations"
    }

    #[tracing::instrument(skip(self, event), fields(projection = "reservations"))]
    async fn apply_event(&self, event: &Self::Event) -> Result<()> {
        if let TicketingEvent::Reservation(reservation_action) = event {
            match reservation_action {
                ReservationAction::ReservationInitiated {
                    reservation_id,
                    event_id,
                    customer_id,
                    section,
                    quantity,
                    ..
                } => {
                    // Create initial reservation record
                    let reservation = Reservation {
                        id: *reservation_id,
                        event_id: *event_id,
                        customer_id: *customer_id,
                        seats: vec![], // Seats added when SeatsReserved event arrives
                        total_amount: crate::types::Money::from_cents(0), // Set when payment calculated
                        status: crate::types::ReservationStatus::Initiated,
                        expires_at: crate::types::ReservationExpiry::new(
                            chrono::Utc::now() + chrono::Duration::minutes(5)
                        ),
                        created_at: chrono::Utc::now(),
                    };

                    let json = serde_json::to_value(&reservation)
                        .map_err(|e| ProjectionError::EventProcessing(format!("Failed to serialize reservation: {e}")))?;

                    sqlx::query(
                        "INSERT INTO reservations_projection (id, customer_id, data, created_at)
                         VALUES ($1, $2, $3, $4)
                         ON CONFLICT (id) DO NOTHING"
                    )
                    .bind(reservation_id.as_uuid())
                    .bind(customer_id.as_uuid())
                    .bind(&json)
                    .bind(reservation.created_at)
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to insert reservation: {e}")))?;

                    let _ = (section, quantity); // Used in domain model but not stored separately here
                    Ok(())
                }

                ReservationAction::SeatsAllocated {
                    reservation_id,
                    seats,
                    ..
                } => {
                    // Update reservation with allocated seats
                    sqlx::query(
                        "UPDATE reservations_projection
                         SET data = jsonb_set(
                             jsonb_set(data, '{seats}', $2::jsonb),
                             '{status}', '\"SeatsAllocated\"'::jsonb
                         )
                         WHERE id = $1"
                    )
                    .bind(reservation_id.as_uuid())
                    .bind(serde_json::to_value(seats)
                        .map_err(|e| ProjectionError::EventProcessing(format!("Failed to serialize seats: {e}")))?)
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to update reservation: {e}")))?;

                    Ok(())
                }

                ReservationAction::PaymentRequested {
                    reservation_id,
                    amount,
                    ..
                } => {
                    // Update reservation with payment amount
                    sqlx::query(
                        "UPDATE reservations_projection
                         SET data = jsonb_set(
                             jsonb_set(data, '{total_amount}', $2::jsonb),
                             '{status}', '\"PaymentPending\"'::jsonb
                         )
                         WHERE id = $1"
                    )
                    .bind(reservation_id.as_uuid())
                    .bind(serde_json::to_value(amount)
                        .map_err(|e| ProjectionError::EventProcessing(format!("Failed to serialize amount: {e}")))?)
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to update reservation: {e}")))?;

                    Ok(())
                }

                ReservationAction::ReservationCompleted {
                    reservation_id,
                    ..
                } => {
                    // Mark reservation as completed
                    sqlx::query(
                        "UPDATE reservations_projection
                         SET data = jsonb_set(
                             jsonb_set(data, '{status}', '\"Completed\"'::jsonb),
                             '{completed_at}', to_jsonb(NOW())
                         )
                         WHERE id = $1"
                    )
                    .bind(reservation_id.as_uuid())
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to update reservation: {e}")))?;

                    Ok(())
                }

                ReservationAction::ReservationCancelled {
                    reservation_id,
                    ..
                } | ReservationAction::ReservationExpired {
                    reservation_id,
                    ..
                } => {
                    // Mark reservation as cancelled/expired
                    let status = match reservation_action {
                        ReservationAction::ReservationCancelled { .. } => "Cancelled",
                        _ => "Expired",
                    };

                    sqlx::query(
                        "UPDATE reservations_projection
                         SET data = jsonb_set(data, '{status}', $2::jsonb)
                         WHERE id = $1"
                    )
                    .bind(reservation_id.as_uuid())
                    .bind(serde_json::to_value(status)
                        .map_err(|e| ProjectionError::EventProcessing(format!("Failed to serialize status: {e}")))?)
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to update reservation: {e}")))?;

                    Ok(())
                }

                // Other actions don't affect the reservation projection
                _ => Ok(()),
            }
        } else {
            Ok(())
        }
    }

    async fn rebuild(&self) -> Result<()> {
        sqlx::query("TRUNCATE reservations_projection")
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to rebuild: {e}")))?;

        Ok(())
    }
}
