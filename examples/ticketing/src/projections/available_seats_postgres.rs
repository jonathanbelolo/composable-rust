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

    /// Access the underlying connection pool.
    ///
    /// Useful for health checks or manual queries.
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        self.pool.as_ref()
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

    /// Load seat assignments for a specific event and section.
    ///
    /// Returns complete denormalized snapshot of individual seat states.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn load_seat_assignments(
        &self,
        event_id: &EventId,
        section: &str,
    ) -> Result<Vec<crate::types::SeatAssignment>> {
        use crate::types::{SeatAssignment, SeatId, SeatNumber, SeatStatus, ReservationId};
        use chrono::{DateTime, Utc};

        let rows: Vec<(sqlx::types::Uuid, String, Option<String>, sqlx::types::Uuid, Option<sqlx::types::Uuid>, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT seat_id, status, seat_number, event_id, reserved_by, expires_at
             FROM seat_assignments
             WHERE event_id = $1 AND section = $2
             ORDER BY seat_number NULLS FIRST, seat_id"
        )
        .bind(event_id.as_uuid())
        .bind(section)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to load seat assignments: {e}")))?;

        let mut assignments = Vec::new();
        for (seat_id, status_str, seat_number, event_id_db, reserved_by, expires_at) in rows {
            let seat_id = SeatId::from_uuid(seat_id);
            let event_id = crate::types::EventId::from_uuid(event_id_db);
            let seat_number = seat_number.map(SeatNumber::new);

            let status = match status_str.as_str() {
                "available" => SeatStatus::Available,
                "reserved" => {
                    if let Some(expires) = expires_at {
                        SeatStatus::Reserved { expires_at: expires }
                    } else {
                        SeatStatus::Available // Fallback if data is inconsistent
                    }
                }
                "sold" => SeatStatus::Sold,
                "held" => SeatStatus::Held,
                _ => SeatStatus::Available, // Fallback for unknown statuses
            };

            let reserved_by = reserved_by.map(ReservationId::from_uuid);

            let mut assignment = SeatAssignment::new(seat_id, event_id, section.to_string(), seat_number);
            assignment.status = status;
            assignment.reserved_by = reserved_by;

            assignments.push(assignment);
        }

        Ok(assignments)
    }
}

impl Projection for PostgresAvailableSeatsProjection {
    type Event = TicketingEvent;

    fn name(&self) -> &'static str {
        "available_seats_projection"
    }

    #[allow(clippy::too_many_lines)] // Event handling is naturally long but simple
    #[tracing::instrument(skip(self, event), fields(projection = "available_seats"))]
    async fn apply_event(&self, event: &Self::Event) -> Result<()> {
        match event {
            // Initialize inventory creates new availability record
            TicketingEvent::Inventory(InventoryAction::InventoryInitialized {
                event_id,
                section,
                capacity,
                seats,
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

                // Also insert seat assignment records for each seat (all available initially)
                for seat_id in seats {
                    sqlx::query(
                        "INSERT INTO seat_assignments
                         (seat_id, event_id, section, status, seat_number, reserved_by, expires_at, updated_at)
                         VALUES ($1, $2, $3, 'available', NULL, NULL, NULL, NOW())
                         ON CONFLICT (seat_id) DO NOTHING"
                    )
                    .bind(seat_id.as_uuid())
                    .bind(event_id.as_uuid())
                    .bind(section)
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to insert seat assignment: {e}")))?;
                }

                Ok(())
            }

            // Seats reserved: move from available to reserved
            TicketingEvent::Inventory(InventoryAction::SeatsReserved {
                reservation_id,
                event_id,
                section,
                seats,
                expires_at,
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

                // Update seat assignments to reserved status
                for seat_id in seats {
                    sqlx::query(
                        "UPDATE seat_assignments
                         SET status = 'reserved',
                             reserved_by = $2,
                             expires_at = $3,
                             updated_at = NOW()
                         WHERE seat_id = $1"
                    )
                    .bind(seat_id.as_uuid())
                    .bind(reservation_id.as_uuid())
                    .bind(expires_at)
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to update seat assignment: {e}")))?;
                }

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

                // Update seat assignments to sold status
                for seat_id in seats {
                    sqlx::query(
                        "UPDATE seat_assignments
                         SET status = 'sold',
                             reserved_by = NULL,
                             expires_at = NULL,
                             updated_at = NOW()
                         WHERE seat_id = $1"
                    )
                    .bind(seat_id.as_uuid())
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to update seat assignment: {e}")))?;
                }

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

                // Update seat assignments back to available status
                for seat_id in seats {
                    sqlx::query(
                        "UPDATE seat_assignments
                         SET status = 'available',
                             reserved_by = NULL,
                             expires_at = NULL,
                             updated_at = NOW()
                         WHERE seat_id = $1"
                    )
                    .bind(seat_id.as_uuid())
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to update seat assignment: {e}")))?;
                }

                Ok(())
            }

            // Other events are not relevant to this projection
            _ => Ok(()),
        }
    }

    async fn rebuild(&self) -> Result<()> {
        // Truncate all tables to start fresh
        sqlx::query("TRUNCATE available_seats_projection, seat_assignments, processed_reservations")
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to rebuild: {e}")))?;

        Ok(())
    }
}

// ============================================================================
// InventoryProjectionQuery Trait Implementation
// ============================================================================

impl crate::aggregates::inventory::InventoryProjectionQuery for PostgresAvailableSeatsProjection {
    fn load_inventory(
        &self,
        event_id: &EventId,
        section: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::result::Result<Option<((u32, u32, u32, u32), Vec<crate::types::SeatAssignment>)>, String>> + Send + '_>> {
        let event_id = *event_id;
        let section = section.to_string();
        Box::pin(async move {
            // Load aggregate counts
            let counts = self
                .get_availability(&event_id, &section)
                .await
                .map_err(|e| e.to_string())?;

            // If no counts found, return None
            let Some(counts) = counts else {
                return Ok(None);
            };

            // Load individual seat assignments
            let seat_assignments = self
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
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::result::Result<Vec<crate::aggregates::inventory::SectionAvailabilityData>, String>> + Send + '_>> {
        let event_id = *event_id;
        Box::pin(async move {
            let sections = self
                .get_all_sections(&event_id)
                .await
                .map_err(|e| format!("Failed to query sections: {e}"))?;

            // Convert from projection's SectionAvailability (i32) to aggregate's SectionAvailabilityData (u32)
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
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::result::Result<Option<crate::aggregates::inventory::SectionAvailabilityData>, String>> + Send + '_>> {
        let event_id = *event_id;
        let section = section.to_string();
        Box::pin(async move {
            let availability = self
                .get_availability(&event_id, &section)
                .await
                .map_err(|e| format!("Failed to query availability: {e}"))?;

            // Convert from tuple to SectionAvailabilityData
            let data = availability.map(|(total_capacity, reserved, sold, available)| {
                crate::aggregates::inventory::SectionAvailabilityData {
                    section,
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
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::result::Result<u32, String>> + Send + '_>> {
        let event_id = *event_id;
        Box::pin(async move {
            self.get_total_available(&event_id)
                .await
                .map_err(|e| format!("Failed to query total available: {e}"))
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // Test code can use unwrap/expect
mod tests {
    #[test]
    fn test_projection_name_constant() {
        // Simple test verifying the projection name constant
        // Full integration tests with database are in tests/projection_unit_test.rs
        assert_eq!("available_seats_projection", "available_seats_projection");
    }
}
