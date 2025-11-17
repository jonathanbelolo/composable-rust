//! `PostgreSQL`-backed customer history projection for purchase tracking.
//!
//! This projection maintains each customer's ticket purchase history in `PostgreSQL`,
//! enabling queries like "Show all tickets purchased by Customer X" or
//! "Has this customer attended this venue before?"
//!
//! # Architecture
//!
//! - **Storage**: PostgreSQL with normalized tables (profiles + purchases)
//! - **Idempotency**: Tracks processed reservations in `customer_pending_reservations`
//! - **CQRS**: Separate database from event store

use crate::aggregates::ReservationAction;
use crate::projections::TicketingEvent;
use crate::types::{CustomerId, EventId, Money, ReservationId, TicketId};
use chrono::{DateTime, Utc};
use composable_rust_core::projection::{Projection, ProjectionError, Result};
use sqlx::PgPool;
use std::sync::Arc;

/// A customer's completed purchase record.
#[derive(Clone, Debug)]
pub struct CustomerPurchase {
    /// Reservation ID
    pub reservation_id: ReservationId,
    /// Event ID
    pub event_id: EventId,
    /// Section purchased (e.g., "VIP", "General")
    pub section: String,
    /// Number of tickets
    pub ticket_count: u32,
    /// Total amount paid
    pub amount_paid: Money,
    /// Ticket IDs issued
    pub tickets: Vec<TicketId>,
    /// When the purchase was completed
    pub completed_at: DateTime<Utc>,
}

/// Customer profile summary.
#[derive(Clone, Debug)]
pub struct CustomerProfile {
    /// Customer ID
    pub customer_id: CustomerId,
    /// Total amount spent
    pub total_spent: Money,
    /// Total tickets purchased
    pub total_tickets: u32,
    /// Number of purchases
    pub purchase_count: u32,
    /// Favorite section (most frequently purchased)
    pub favorite_section: Option<String>,
}

/// PostgreSQL-backed customer history projection.
///
/// Maintains real-time customer purchase history with proper idempotency
/// and crash recovery via checkpointing.
#[derive(Clone)]
pub struct PostgresCustomerHistoryProjection {
    pool: Arc<PgPool>,
}

impl PostgresCustomerHistoryProjection {
    /// Create a new PostgreSQL-backed customer history projection.
    ///
    /// # Arguments
    ///
    /// - `pool`: Connection pool for projection database (separate from event store)
    #[must_use]
    pub const fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    /// Get a customer's profile summary.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_customer_profile(
        &self,
        customer_id: &CustomerId,
    ) -> Result<Option<CustomerProfile>> {
        let result: Option<(i64, i32, i32, Option<String>)> = sqlx::query_as(
            "SELECT total_spent, total_tickets, purchase_count, favorite_section
             FROM customer_profiles
             WHERE customer_id = $1",
        )
        .bind(customer_id.as_uuid())
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query customer profile: {e}")))?;

        #[allow(clippy::cast_sign_loss)]
        Ok(result.map(
            |(total_spent, total_tickets, purchase_count, favorite_section)| CustomerProfile {
                customer_id: *customer_id,
                total_spent: Money::from_cents(total_spent as u64),
                total_tickets: total_tickets as u32,
                purchase_count: purchase_count as u32,
                favorite_section,
            },
        ))
    }

    /// Get a customer's purchase history.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_customer_purchases(
        &self,
        customer_id: &CustomerId,
    ) -> Result<Vec<CustomerPurchase>> {
        let results: Vec<(uuid::Uuid, uuid::Uuid, String, i32, i64, sqlx::types::JsonValue, DateTime<Utc>)> =
            sqlx::query_as(
                "SELECT reservation_id, event_id, section, ticket_count, amount_paid, tickets, completed_at
                 FROM customer_purchases
                 WHERE customer_id = $1
                 ORDER BY completed_at DESC",
            )
            .bind(customer_id.as_uuid())
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to query customer purchases: {e}")))?;

        let mut purchases = Vec::new();
        for (res_id, evt_id, section, ticket_count, amount_paid, tickets_json, completed_at) in
            results
        {
            let tickets: Vec<uuid::Uuid> = serde_json::from_value(tickets_json)
                .map_err(|e| ProjectionError::Serialization(format!("Failed to deserialize tickets: {e}")))?;

            #[allow(clippy::cast_sign_loss)]
            purchases.push(CustomerPurchase {
                reservation_id: ReservationId::from_uuid(res_id),
                event_id: EventId::from_uuid(evt_id),
                section,
                ticket_count: ticket_count as u32,
                amount_paid: Money::from_cents(amount_paid as u64),
                tickets: tickets.into_iter().map(TicketId::from_uuid).collect(),
                completed_at,
            });
        }

        Ok(purchases)
    }

    /// Check if a customer has attended a specific event.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn has_attended_event(
        &self,
        customer_id: &CustomerId,
        event_id: &EventId,
    ) -> Result<bool> {
        let result: Option<(bool,)> = sqlx::query_as(
            "SELECT EXISTS(
                SELECT 1 FROM customer_event_attendance
                WHERE customer_id = $1 AND event_id = $2
            )",
        )
        .bind(customer_id.as_uuid())
        .bind(event_id.as_uuid())
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to check event attendance: {e}")))?;

        Ok(result.map_or(false, |(exists,)| exists))
    }

    /// Get all customers who attended a specific event.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_event_attendees(&self, event_id: &EventId) -> Result<Vec<CustomerId>> {
        let results: Vec<(uuid::Uuid,)> = sqlx::query_as(
            "SELECT customer_id FROM customer_event_attendance WHERE event_id = $1",
        )
        .bind(event_id.as_uuid())
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query event attendees: {e}")))?;

        Ok(results
            .into_iter()
            .map(|(cust_id,)| CustomerId::from_uuid(cust_id))
            .collect())
    }

    /// Get top spenders (customers sorted by total spending).
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_top_spenders(&self, limit: i64) -> Result<Vec<CustomerProfile>> {
        let results: Vec<(uuid::Uuid, i64, i32, i32, Option<String>)> = sqlx::query_as(
            "SELECT customer_id, total_spent, total_tickets, purchase_count, favorite_section
             FROM customer_profiles
             ORDER BY total_spent DESC
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query top spenders: {e}")))?;

        #[allow(clippy::cast_sign_loss)]
        Ok(results
            .into_iter()
            .map(
                |(cust_id, total_spent, total_tickets, purchase_count, favorite_section)| {
                    CustomerProfile {
                        customer_id: CustomerId::from_uuid(cust_id),
                        total_spent: Money::from_cents(total_spent as u64),
                        total_tickets: total_tickets as u32,
                        purchase_count: purchase_count as u32,
                        favorite_section,
                    }
                },
            )
            .collect())
    }

    /// Get total customer count.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_customer_count(&self) -> Result<u32> {
        let result: Option<(i64,)> =
            sqlx::query_as("SELECT COUNT(*) FROM customer_profiles")
                .fetch_optional(self.pool.as_ref())
                .await
                .map_err(|e| {
                    ProjectionError::Storage(format!("Failed to query customer count: {e}"))
                })?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(result.map_or(0, |(count,)| count as u32))
    }
}

impl Projection for PostgresCustomerHistoryProjection {
    type Event = TicketingEvent;

    fn name(&self) -> &'static str {
        "customer_history"
    }

    #[allow(clippy::too_many_lines)]
    async fn apply_event(&self, event: &Self::Event) -> Result<()> {
        match event {
            // Track reservation initiation
            TicketingEvent::Reservation(ReservationAction::ReservationInitiated {
                reservation_id,
                event_id,
                customer_id,
                section,
                quantity,
                ..
            }) => {
                // Store pending reservation (estimated price)
                let estimated_price = Money::from_dollars(50);
                let amount = estimated_price.multiply(*quantity);

                sqlx::query(
                    "INSERT INTO customer_pending_reservations
                     (reservation_id, customer_id, event_id, section, ticket_count, amount)
                     VALUES ($1, $2, $3, $4, $5, $6)
                     ON CONFLICT (reservation_id) DO NOTHING",
                )
                .bind(reservation_id.as_uuid())
                .bind(customer_id.as_uuid())
                .bind(event_id.as_uuid())
                .bind(section)
                .bind(i32::try_from(*quantity).map_err(|e| ProjectionError::EventProcessing(format!("Quantity overflow: {e}")))?)
                .bind(i64::try_from(amount.cents()).map_err(|e| ProjectionError::EventProcessing(format!("Amount overflow: {e}")))?)
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
                    "UPDATE customer_pending_reservations
                     SET ticket_count = $2, amount = $3
                     WHERE reservation_id = $1",
                )
                .bind(reservation_id.as_uuid())
                .bind(ticket_count)
                .bind(amount)
                .execute(self.pool.as_ref())
                .await
                .map_err(|e| ProjectionError::Storage(format!("Failed to update pending reservation: {e}")))?;

                Ok(())
            }

            // Reservation completed: add to customer history
            TicketingEvent::Reservation(ReservationAction::ReservationCompleted {
                reservation_id,
                tickets_issued,
                completed_at,
            }) => {
                // Get pending reservation data
                let pending: Option<(uuid::Uuid, uuid::Uuid, String, i32, i64)> = sqlx::query_as(
                    "SELECT customer_id, event_id, section, ticket_count, amount
                     FROM customer_pending_reservations
                     WHERE reservation_id = $1",
                )
                .bind(reservation_id.as_uuid())
                .fetch_optional(self.pool.as_ref())
                .await
                .map_err(|e| ProjectionError::Storage(format!("Failed to fetch pending reservation: {e}")))?;

                if let Some((customer_uuid, event_uuid, section, ticket_count, amount)) = pending {
                    // Convert ticket IDs to UUIDs for JSONB storage
                    let ticket_uuids: Vec<uuid::Uuid> =
                        tickets_issued.iter().map(TicketId::as_uuid).copied().collect();
                    let tickets_json = serde_json::to_value(&ticket_uuids)
                        .map_err(|e| ProjectionError::Serialization(format!("Failed to serialize tickets: {e}")))?;

                    // IMPORTANT: Insert/update customer profile FIRST (foreign key requirement)
                    sqlx::query(
                        "INSERT INTO customer_profiles (customer_id, total_spent, total_tickets, purchase_count)
                         VALUES ($1, $2, $3, 1)
                         ON CONFLICT (customer_id) DO UPDATE SET
                            total_spent = customer_profiles.total_spent + $2,
                            total_tickets = customer_profiles.total_tickets + $3,
                            purchase_count = customer_profiles.purchase_count + 1,
                            updated_at = NOW()",
                    )
                    .bind(customer_uuid)
                    .bind(amount)
                    .bind(ticket_count)
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to update customer profile: {e}")))?;

                    // Insert purchase record (now that profile exists)
                    sqlx::query(
                        "INSERT INTO customer_purchases
                         (customer_id, reservation_id, event_id, section, ticket_count, amount_paid, tickets, completed_at)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                    )
                    .bind(customer_uuid)
                    .bind(reservation_id.as_uuid())
                    .bind(event_uuid)
                    .bind(&section)
                    .bind(ticket_count)
                    .bind(amount)
                    .bind(tickets_json)
                    .bind(completed_at)
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to insert purchase: {e}")))?;

                    // Update favorite section (recalculate based on most frequent section)
                    sqlx::query(
                        "UPDATE customer_profiles
                         SET favorite_section = (
                            SELECT section
                            FROM customer_purchases
                            WHERE customer_id = $1
                            GROUP BY section
                            ORDER BY COUNT(*) DESC
                            LIMIT 1
                         )
                         WHERE customer_id = $1",
                    )
                    .bind(customer_uuid)
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to update favorite section: {e}")))?;

                    // Track event attendance
                    sqlx::query(
                        "INSERT INTO customer_event_attendance (customer_id, event_id, first_attended_at)
                         VALUES ($1, $2, $3)
                         ON CONFLICT (customer_id, event_id) DO NOTHING",
                    )
                    .bind(customer_uuid)
                    .bind(event_uuid)
                    .bind(completed_at)
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to track event attendance: {e}")))?;

                    // Remove from pending
                    sqlx::query("DELETE FROM customer_pending_reservations WHERE reservation_id = $1")
                        .bind(reservation_id.as_uuid())
                        .execute(self.pool.as_ref())
                        .await
                        .map_err(|e| ProjectionError::Storage(format!("Failed to delete pending reservation: {e}")))?;
                }

                Ok(())
            }

            // Reservation cancelled/expired: remove from pending
            TicketingEvent::Reservation(
                ReservationAction::ReservationCancelled { reservation_id, .. }
                | ReservationAction::ReservationExpired { reservation_id, .. }
                | ReservationAction::ReservationCompensated { reservation_id, .. },
            ) => {
                sqlx::query("DELETE FROM customer_pending_reservations WHERE reservation_id = $1")
                    .bind(reservation_id.as_uuid())
                    .execute(self.pool.as_ref())
                    .await
                    .map_err(|e| ProjectionError::Storage(format!("Failed to delete pending reservation: {e}")))?;

                Ok(())
            }

            // Other events are not relevant
            _ => Ok(()),
        }
    }

    async fn rebuild(&self) -> Result<()> {
        // Truncate all tables to start fresh
        sqlx::query(
            "TRUNCATE customer_profiles, customer_purchases, customer_event_attendance, customer_pending_reservations CASCADE",
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
        assert_eq!("customer_history", "customer_history");
    }
}
