//! `PostgreSQL`-backed available seats projection.
//!
//! This projection maintains a denormalized view of seat availability in `PostgreSQL`,
//! enabling fast queries like "Show me all available VIP seats for Event X".
//!
//! # Architecture
//!
//! - **Storage**: `PostgreSQL` with custom queryable tables
//! - **Idempotency**: Tracks processed reservations in `processed_reservations` table
//! - **Checkpointing**: Uses framework's `PostgresProjectionCheckpoint`
//! - **CQRS**: Separate database from event store

use crate::aggregates::InventoryAction;
use crate::projections::TicketingEvent;
use crate::types::{EventId, ReservationId};
use composable_rust_core::projection::{Projection, ProjectionError, Result};
use sqlx::PgPool;
use std::sync::Arc;

/// Availability data for a section.
#[derive(Debug, Clone)]
pub struct SectionAvailability {
    /// Section identifier
    pub section: String,
    /// Total capacity
    pub total_capacity: i32,
    /// Currently reserved seats (pending payment)
    pub reserved: i32,
    /// Sold seats (payment confirmed)
    pub sold: i32,
    /// Available seats (total - reserved - sold)
    pub available: i32,
}

/// PostgreSQL-backed available seats projection.
///
/// Maintains real-time view of seat availability with proper idempotency
/// and crash recovery via checkpointing.
#[derive(Clone)]
pub struct PostgresAvailableSeatsProjection {
    pool: Arc<PgPool>,
}

impl PostgresAvailableSeatsProjection {
    /// Create a new PostgreSQL-backed projection.
    ///
    /// # Arguments
    ///
    /// - `pool`: Connection pool for projection database (separate from event store)
    #[must_use]
    pub const fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    /// Check if a reservation has already been processed (idempotency).
    async fn is_processed(&self, reservation_id: &ReservationId) -> Result<bool> {
        let result: (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM processed_reservations WHERE reservation_id = $1)"
        )
        .bind(reservation_id.as_uuid())
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to check idempotency: {e}")))?;

        Ok(result.0)
    }

    /// Mark a reservation as processed.
    async fn mark_processed(&self, reservation_id: &ReservationId) -> Result<()> {
        sqlx::query(
            "INSERT INTO processed_reservations (reservation_id, processed_at)
             VALUES ($1, NOW())
             ON CONFLICT (reservation_id) DO NOTHING"
        )
        .bind(reservation_id.as_uuid())
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to mark processed: {e}")))?;

        Ok(())
    }

    /// Query seat availability for a specific section.
    ///
    /// Returns (`total_capacity`, reserved, sold, available).
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_availability(
        &self,
        event_id: &EventId,
        section: &str,
    ) -> Result<Option<(u32, u32, u32, u32)>> {
        let result: Option<(i32, i32, i32, i32)> = sqlx::query_as(
            "SELECT total_capacity, reserved, sold, available
             FROM available_seats_projection
             WHERE event_id = $1 AND section = $2"
        )
        .bind(event_id.as_uuid())
        .bind(section)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query availability: {e}")))?;

        #[allow(clippy::cast_sign_loss)] // Counts are always non-negative in our domain
        Ok(result.map(|(total, reserved, sold, available)| {
            (total as u32, reserved as u32, sold as u32, available as u32)
        }))
    }

    /// Query seat availability for all sections of an event.
    ///
    /// Returns a vector of section availability data.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_all_sections(&self, event_id: &EventId) -> Result<Vec<SectionAvailability>> {
        let results: Vec<(String, i32, i32, i32, i32)> = sqlx::query_as(
            "SELECT section, total_capacity, reserved, sold, available
             FROM available_seats_projection
             WHERE event_id = $1
             ORDER BY section"
        )
        .bind(event_id.as_uuid())
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query all sections: {e}")))?;

        Ok(results
            .into_iter()
            .map(|(section, total_capacity, reserved, sold, available)| SectionAvailability {
                section,
                total_capacity,
                reserved,
                sold,
                available,
            })
            .collect())
    }

    /// Get total available seats across all sections for an event.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_total_available(&self, event_id: &EventId) -> Result<u32> {
        let result: Option<(i64,)> = sqlx::query_as(
            "SELECT COALESCE(SUM(available), 0)
             FROM available_seats_projection
             WHERE event_id = $1"
        )
        .bind(event_id.as_uuid())
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query total available: {e}")))?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(result.map_or(0, |(total,)| total as u32))
    }
}

impl Projection for PostgresAvailableSeatsProjection {
    type Event = TicketingEvent;

    fn name(&self) -> &'static str {
        "available_seats_projection"
    }

    #[allow(clippy::too_many_lines)] // Event handling is naturally long but simple
    async fn apply_event(&self, event: &Self::Event) -> Result<()> {
        match event {
            // Initialize inventory creates new availability record
            TicketingEvent::Inventory(InventoryAction::InventoryInitialized {
                event_id,
                section,
                capacity,
                ..
            }) => {
                let total = capacity.0;

                sqlx::query(
                    "INSERT INTO available_seats_projection
                     (event_id, section, total_capacity, reserved, sold, available, updated_at)
                     VALUES ($1, $2, $3, 0, 0, $3, NOW())
                     ON CONFLICT (event_id, section) DO NOTHING"
                )
                .bind(event_id.as_uuid())
                .bind(section)
                .bind(i32::try_from(total).map_err(|e| ProjectionError::EventProcessing(format!("Capacity overflow: {e}")))?)
                .execute(self.pool.as_ref())
                .await
                .map_err(|e| ProjectionError::Storage(format!("Failed to initialize inventory: {e}")))?;

                Ok(())
            }

            // Seats reserved: move from available to reserved
            TicketingEvent::Inventory(InventoryAction::SeatsReserved {
                reservation_id,
                event_id,
                section,
                seats,
                ..
            }) => {
                // Idempotency check
                if self.is_processed(reservation_id).await? {
                    return Ok(()); // Already processed, skip
                }
                #[allow(clippy::cast_possible_truncation)]
                let quantity = i32::try_from(seats.len())
                    .map_err(|e| ProjectionError::EventProcessing(format!("Seat count overflow: {e}")))?;

                // Update availability
                sqlx::query(
                    "UPDATE available_seats_projection
                     SET reserved = reserved + $3,
                         available = total_capacity - (reserved + $3) - sold,
                         updated_at = NOW()
                     WHERE event_id = $1 AND section = $2"
                )
                .bind(event_id.as_uuid())
                .bind(section)
                .bind(quantity)
                .execute(self.pool.as_ref())
                .await
                .map_err(|e| ProjectionError::Storage(format!("Failed to reserve seats: {e}")))?;

                // Mark as processed
                self.mark_processed(reservation_id).await?;

                Ok(())
            }

            // Seats confirmed: move from reserved to sold
            TicketingEvent::Inventory(InventoryAction::SeatsConfirmed {
                event_id,
                section,
                seats,
                ..
            }) => {
                let quantity = i32::try_from(seats.len())
                    .map_err(|e| ProjectionError::EventProcessing(format!("Seat count overflow: {e}")))?;

                sqlx::query(
                    "UPDATE available_seats_projection
                     SET reserved = GREATEST(0, reserved - $3),
                         sold = sold + $3,
                         available = total_capacity - GREATEST(0, reserved - $3) - (sold + $3),
                         updated_at = NOW()
                     WHERE event_id = $1 AND section = $2"
                )
                .bind(event_id.as_uuid())
                .bind(section)
                .bind(quantity)
                .execute(self.pool.as_ref())
                .await
                .map_err(|e| ProjectionError::Storage(format!("Failed to confirm seats: {e}")))?;

                Ok(())
            }

            // Seats released: move from reserved back to available
            TicketingEvent::Inventory(InventoryAction::SeatsReleased {
                event_id,
                section,
                seats,
                ..
            }) => {
                let quantity = i32::try_from(seats.len())
                    .map_err(|e| ProjectionError::EventProcessing(format!("Seat count overflow: {e}")))?;

                sqlx::query(
                    "UPDATE available_seats_projection
                     SET reserved = GREATEST(0, reserved - $3),
                         available = total_capacity - GREATEST(0, reserved - $3) - sold,
                         updated_at = NOW()
                     WHERE event_id = $1 AND section = $2"
                )
                .bind(event_id.as_uuid())
                .bind(section)
                .bind(quantity)
                .execute(self.pool.as_ref())
                .await
                .map_err(|e| ProjectionError::Storage(format!("Failed to release seats: {e}")))?;

                Ok(())
            }

            // Other events are not relevant to this projection
            _ => Ok(()),
        }
    }

    async fn rebuild(&self) -> Result<()> {
        // Truncate both tables to start fresh
        sqlx::query("TRUNCATE available_seats_projection, processed_reservations")
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to rebuild: {e}")))?;

        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // Test code can use unwrap/expect
mod tests {
    use super::*;

    #[test]
    fn test_projection_name() {
        // Placeholder - full integration tests require testcontainers
        let pool = Arc::new(
            sqlx::PgPool::connect_lazy("postgres://localhost/test")
                .expect("Failed to create test pool")
        );
        let projection = PostgresAvailableSeatsProjection::new(pool);
        assert_eq!(projection.name(), "available_seats_projection");
    }
}
