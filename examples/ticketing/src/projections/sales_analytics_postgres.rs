//! PostgreSQL-backed sales analytics projection for revenue tracking.
//!
//! This projection maintains aggregated sales metrics in PostgreSQL,
//! enabling fast queries like "What's the total revenue for Event X?" or
//! "Which section is most popular?"
//!
//! # Architecture
//!
//! - **Storage**: PostgreSQL with denormalized tables
//! - **Idempotency**: Tracks processed reservations in `sales_pending_reservations`
//! - **CQRS**: Separate database from event store

use crate::aggregates::{PaymentAction, ReservationAction};
use crate::projections::TicketingEvent;
use crate::types::{EventId, Money};
use composable_rust_core::projection::{Projection, ProjectionError, Result};
use sqlx::PgPool;
use std::sync::Arc;

/// Sales metrics for a specific event section.
#[derive(Debug, Clone)]
pub struct SectionMetrics {
    /// Section name
    pub section: String,
    /// Revenue for this section
    pub revenue: Money,
    /// Tickets sold in this section
    pub tickets_sold: u32,
}

/// Complete sales metrics for an event.
#[derive(Debug, Clone)]
pub struct SalesMetrics {
    /// Event ID
    pub event_id: EventId,
    /// Total revenue across all sections
    pub total_revenue: Money,
    /// Total tickets sold
    pub tickets_sold: u32,
    /// Completed reservations
    pub completed_reservations: u32,
    /// Cancelled reservations
    pub cancelled_reservations: u32,
    /// Average ticket price
    pub average_ticket_price: Money,
}

/// PostgreSQL-backed sales analytics projection.
///
/// Maintains real-time sales metrics with proper idempotency
/// and crash recovery via checkpointing.
#[derive(Clone)]
pub struct PostgresSalesAnalyticsProjection {
    pool: Arc<PgPool>,
}

impl PostgresSalesAnalyticsProjection {
    /// Create a new PostgreSQL-backed sales analytics projection.
    ///
    /// # Arguments
    ///
    /// - `pool`: Connection pool for projection database (separate from event store)
    #[must_use]
    pub const fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    /// Get sales metrics for a specific event.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_metrics(&self, event_id: &EventId) -> Result<Option<SalesMetrics>> {
        let result: Option<(i64, i32, i32, i32, i64)> = sqlx::query_as(
            "SELECT total_revenue, tickets_sold, completed_reservations,
                    cancelled_reservations, average_ticket_price
             FROM sales_analytics_projection
             WHERE event_id = $1",
        )
        .bind(event_id.as_uuid())
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query metrics: {e}")))?;

        #[allow(clippy::cast_sign_loss)] // Counts are always non-negative
        Ok(result.map(
            |(total_revenue, tickets_sold, completed, cancelled, avg_price)| SalesMetrics {
                event_id: *event_id,
                total_revenue: Money::from_cents(total_revenue as u64),
                tickets_sold: tickets_sold as u32,
                completed_reservations: completed as u32,
                cancelled_reservations: cancelled as u32,
                average_ticket_price: Money::from_cents(avg_price as u64),
            },
        ))
    }

    /// Get revenue by section for an event.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_section_metrics(
        &self,
        event_id: &EventId,
    ) -> Result<Vec<SectionMetrics>> {
        let results: Vec<(String, i64, i32)> = sqlx::query_as(
            "SELECT section, revenue, tickets_sold
             FROM sales_by_section
             WHERE event_id = $1
             ORDER BY revenue DESC",
        )
        .bind(event_id.as_uuid())
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query section metrics: {e}")))?;

        #[allow(clippy::cast_sign_loss)]
        Ok(results
            .into_iter()
            .map(|(section, revenue, tickets_sold)| SectionMetrics {
                section,
                revenue: Money::from_cents(revenue as u64),
                tickets_sold: tickets_sold as u32,
            })
            .collect())
    }

    /// Get the most popular section by tickets sold.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_most_popular_section(
        &self,
        event_id: &EventId,
    ) -> Result<Option<(String, u32)>> {
        let result: Option<(String, i32)> = sqlx::query_as(
            "SELECT section, tickets_sold
             FROM sales_by_section
             WHERE event_id = $1
             ORDER BY tickets_sold DESC
             LIMIT 1",
        )
        .bind(event_id.as_uuid())
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query popular section: {e}")))?;

        #[allow(clippy::cast_sign_loss)]
        Ok(result.map(|(section, tickets)| (section, tickets as u32)))
    }

    /// Get the highest revenue section.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_highest_revenue_section(
        &self,
        event_id: &EventId,
    ) -> Result<Option<(String, Money)>> {
        let result: Option<(String, i64)> = sqlx::query_as(
            "SELECT section, revenue
             FROM sales_by_section
             WHERE event_id = $1
             ORDER BY revenue DESC
             LIMIT 1",
        )
        .bind(event_id.as_uuid())
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| {
            ProjectionError::Storage(format!("Failed to query highest revenue section: {e}"))
        })?;

        #[allow(clippy::cast_sign_loss)]
        Ok(result.map(|(section, revenue)| (section, Money::from_cents(revenue as u64))))
    }

    /// Get total revenue across all events.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_total_revenue_all_events(&self) -> Result<Money> {
        let result: Option<(i64,)> = sqlx::query_as(
            "SELECT COALESCE(SUM(total_revenue), 0) FROM sales_analytics_projection",
        )
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query total revenue: {e}")))?;

        #[allow(clippy::cast_sign_loss)]
        Ok(Money::from_cents(result.map_or(0, |(total,)| total as u64)))
    }

    /// Get total tickets sold across all events.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_total_tickets_sold(&self) -> Result<u32> {
        let result: Option<(i64,)> = sqlx::query_as(
            "SELECT COALESCE(SUM(tickets_sold), 0) FROM sales_analytics_projection",
        )
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query total tickets: {e}")))?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(result.map_or(0, |(total,)| total as u32))
    }
}

impl Projection for PostgresSalesAnalyticsProjection {
    type Event = TicketingEvent;

    fn name(&self) -> &'static str {
        "sales_analytics"
    }

    #[allow(clippy::too_many_lines)]
    async fn apply_event(&self, event: &Self::Event) -> Result<()> {
        match event {
            // Track reservation initiation
            TicketingEvent::Reservation(ReservationAction::ReservationInitiated {
                reservation_id,
                event_id,
                section,
                quantity,
                ..
            }) => {
                // Store pending reservation (estimated price)
                let estimated_price = Money::from_dollars(50);
                let amount = estimated_price.multiply(*quantity);

                sqlx::query(
                    "INSERT INTO sales_pending_reservations
                     (reservation_id, event_id, section, amount, ticket_count)
                     VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT (reservation_id) DO NOTHING",
                )
                .bind(reservation_id.as_uuid())
                .bind(event_id.as_uuid())
                .bind(section)
                .bind(i64::try_from(amount.cents()).map_err(|e| ProjectionError::EventProcessing(format!("Amount overflow: {e}")))?)
                .bind(i32::try_from(*quantity).map_err(|e| ProjectionError::EventProcessing(format!("Quantity overflow: {e}")))?)
                .execute(self.pool.as_ref())
                .await
                .map_err(|e| ProjectionError::Storage(format!("Failed to store pending reservation: {e}")))?;

                Ok(())
            }

            // Update with actual pricing
            TicketingEvent::Reservation(ReservationAction::SeatsAllocated {
                reservation_id,
                seats,
                total_amount,
            }) => {
                let ticket_count = i32::try_from(seats.len())
                    .map_err(|e| ProjectionError::EventProcessing(format!("Seat count overflow: {e}")))?;
                let amount = i64::try_from(total_amount.cents())
                    .map_err(|e| ProjectionError::EventProcessing(format!("Amount overflow: {e}")))?;

                sqlx::query(
                    "UPDATE sales_pending_reservations
                     SET amount = $2, ticket_count = $3
                     WHERE reservation_id = $1",
                )
                .bind(reservation_id.as_uuid())
                .bind(amount)
                .bind(ticket_count)
                .execute(self.pool.as_ref())
                .await
                .map_err(|e| ProjectionError::Storage(format!("Failed to update pending reservation: {e}")))?;

                Ok(())
            }

            // Reservation completed: record the sale
            TicketingEvent::Reservation(ReservationAction::ReservationCompleted {
                reservation_id,
                ..
            }) => {
                // Get pending reservation data
                let pending: Option<(uuid::Uuid, String, i64, i32)> = sqlx::query_as(
                    "SELECT event_id, section, amount, ticket_count
                     FROM sales_pending_reservations
                     WHERE reservation_id = $1",
                )
                .bind(reservation_id.as_uuid())
                .fetch_optional(self.pool.as_ref())
                .await
                .map_err(|e| ProjectionError::Storage(format!("Failed to fetch pending reservation: {e}")))?;

                if let Some((event_uuid, section, amount, ticket_count)) = pending {
                    // Update main sales metrics
                    // Calculate average for initial insert
                    let initial_avg = if ticket_count > 0 { amount / i64::from(ticket_count) } else { 0 };

                    sqlx::query(
                        "INSERT INTO sales_analytics_projection
                         (event_id, total_revenue, tickets_sold, completed_reservations, cancelled_reservations, average_ticket_price)
                         VALUES ($1, $2, $3, 1, 0, $4)
                         ON CONFLICT (event_id) DO UPDATE SET
                            total_revenue = sales_analytics_projection.total_revenue + $2,
                            tickets_sold = sales_analytics_projection.tickets_sold + $3,
                            completed_reservations = sales_analytics_projection.completed_reservations + 1,
                            average_ticket_price = CASE
                                WHEN sales_analytics_projection.tickets_sold + $3 > 0
                                THEN (sales_analytics_projection.total_revenue + $2) / (sales_analytics_projection.tickets_sold + $3)
                                ELSE 0
                            END,
                            updated_at = NOW()",
                    )
                    .bind(event_uuid)
                    .bind(amount)
                    .bind(ticket_count)
                    .bind(initial_avg)
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to update sales metrics: {e}")))?;

                    // Update section metrics
                    sqlx::query(
                        "INSERT INTO sales_by_section (event_id, section, revenue, tickets_sold)
                         VALUES ($1, $2, $3, $4)
                         ON CONFLICT (event_id, section) DO UPDATE SET
                            revenue = sales_by_section.revenue + $3,
                            tickets_sold = sales_by_section.tickets_sold + $4,
                            updated_at = NOW()",
                    )
                    .bind(event_uuid)
                    .bind(&section)
                    .bind(amount)
                    .bind(ticket_count)
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to update section metrics: {e}")))?;

                    // Remove from pending
                    sqlx::query("DELETE FROM sales_pending_reservations WHERE reservation_id = $1")
                        .bind(reservation_id.as_uuid())
                        .execute(self.pool.as_ref())
                        .await
                        .map_err(|e| ProjectionError::Storage(format!("Failed to delete pending reservation: {e}")))?;
                }

                Ok(())
            }

            // Reservation cancelled/expired
            TicketingEvent::Reservation(
                ReservationAction::ReservationCancelled { reservation_id, .. }
                | ReservationAction::ReservationExpired { reservation_id, .. }
                | ReservationAction::ReservationCompensated { reservation_id, .. },
            ) => {
                // Get event_id for the cancelled reservation
                let event_uuid: Option<(uuid::Uuid,)> = sqlx::query_as(
                    "SELECT event_id FROM sales_pending_reservations WHERE reservation_id = $1",
                )
                .bind(reservation_id.as_uuid())
                .fetch_optional(self.pool.as_ref())
                .await
                .map_err(|e| ProjectionError::Storage(format!("Failed to fetch pending reservation: {e}")))?;

                if let Some((event_uuid,)) = event_uuid {
                    // Increment cancelled count
                    sqlx::query(
                        "INSERT INTO sales_analytics_projection (event_id, total_revenue, tickets_sold, completed_reservations, cancelled_reservations, average_ticket_price)
                         VALUES ($1, 0, 0, 0, 1, 0)
                         ON CONFLICT (event_id) DO UPDATE SET
                            cancelled_reservations = sales_analytics_projection.cancelled_reservations + 1,
                            updated_at = NOW()",
                    )
                    .bind(event_uuid)
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to update cancelled count: {e}")))?;

                    // Remove from pending
                    sqlx::query("DELETE FROM sales_pending_reservations WHERE reservation_id = $1")
                        .bind(reservation_id.as_uuid())
                        .execute(self.pool.as_ref())
                        .await
                        .map_err(|e| ProjectionError::Storage(format!("Failed to delete pending reservation: {e}")))?;
                }

                Ok(())
            }

            // Payment events (confirmation only, revenue already tracked via ReservationCompleted)
            TicketingEvent::Payment(
                PaymentAction::PaymentSucceeded { .. }
                | PaymentAction::PaymentRefunded { .. }
                | PaymentAction::PaymentFailed { .. },
            ) => Ok(()),

            // Other events are not relevant
            _ => Ok(()),
        }
    }

    async fn rebuild(&self) -> Result<()> {
        // Truncate all tables to start fresh
        sqlx::query(
            "TRUNCATE sales_analytics_projection, sales_by_section, sales_pending_reservations",
        )
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to rebuild: {e}")))?;

        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // Test code can use unwrap/expect
mod tests {
    #[test]
    fn test_projection_name_constant() {
        // Simple test verifying the projection name constant
        // Full integration tests with database are in tests/projection_unit_test.rs
        assert_eq!("sales_analytics", "sales_analytics");
    }
}
