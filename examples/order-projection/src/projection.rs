//! Customer Order History Projection
//!
//! This projection builds a denormalized read model from order events,
//! demonstrating the query side of CQRS.

use chrono::{DateTime, Utc};
use composable_rust_core::projection::{Projection, ProjectionError, Result};
use order_processing::OrderAction;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// A denormalized view of an order optimized for querying.
///
/// This struct represents how we want to query orders, which may differ
/// from how we store events. CQRS allows us to optimize reads separately
/// from writes.
#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct OrderSummary {
    /// Order identifier
    pub id: String,
    /// Customer who placed the order
    pub customer_id: String,
    /// Number of items in the order (i32 for PostgreSQL compatibility)
    pub item_count: i32,
    /// Total order value in cents
    pub total_cents: i64,
    /// Current order status
    pub status: String,
    /// When the order was placed
    pub placed_at: DateTime<Utc>,
    /// When the order was last updated
    pub updated_at: DateTime<Utc>,
    /// Tracking number (if shipped)
    pub tracking: Option<String>,
    /// Cancellation reason (if cancelled)
    pub cancellation_reason: Option<String>,
}

/// Projection that builds customer order history from order events.
///
/// This demonstrates the read model side of CQRS:
/// - Events are the source of truth (write model)
/// - Projection is an optimized view for queries (read model)
/// - Can use a separate database for true CQRS separation
pub struct CustomerOrderHistoryProjection {
    /// PostgreSQL connection pool for queries
    pool: PgPool,
}

impl CustomerOrderHistoryProjection {
    /// Create a new projection with the given PostgreSQL pool.
    ///
    /// # Arguments
    ///
    /// - `pool`: PostgreSQL connection pool (can be separate from event store)
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Separate database for projections (true CQRS)
    /// let projection_pool = PgPool::connect("postgres://localhost/projections").await?;
    /// let projection = CustomerOrderHistoryProjection::new(projection_pool);
    /// ```
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Query: Get all orders for a customer
    ///
    /// This is the query API - separate from the projection update logic.
    /// Shows how CQRS separates reads from writes.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError::Storage`] if query fails.
    pub async fn get_customer_orders(&self, customer_id: &str) -> Result<Vec<OrderSummary>> {
        sqlx::query_as::<_, OrderSummary>(
            "SELECT id, customer_id, item_count, total_cents, status,
                    placed_at, updated_at, tracking, cancellation_reason
             FROM order_projections
             WHERE customer_id = $1
             ORDER BY placed_at DESC
             LIMIT 100",
        )
        .bind(customer_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query customer orders: {e}")))
    }

    /// Query: Get a specific order by ID
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError::Storage`] if query fails.
    pub async fn get_order(&self, order_id: &str) -> Result<Option<OrderSummary>> {
        sqlx::query_as::<_, OrderSummary>(
            "SELECT id, customer_id, item_count, total_cents, status,
                    placed_at, updated_at, tracking, cancellation_reason
             FROM order_projections
             WHERE id = $1",
        )
        .bind(order_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query order: {e}")))
    }

    /// Query: Get recent orders across all customers
    ///
    /// Useful for admin dashboards, reports, etc.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError::Storage`] if query fails.
    pub async fn get_recent_orders(&self, limit: i64) -> Result<Vec<OrderSummary>> {
        sqlx::query_as::<_, OrderSummary>(
            "SELECT id, customer_id, item_count, total_cents, status,
                    placed_at, updated_at, tracking, cancellation_reason
             FROM order_projections
             ORDER BY placed_at DESC
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query recent orders: {e}")))
    }

    /// Query: Count orders by status
    ///
    /// Useful for metrics and dashboards.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError::Storage`] if query fails.
    pub async fn count_by_status(&self, status: &str) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM order_projections WHERE status = $1")
            .bind(status)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to count orders: {e}")))?;
        Ok(count)
    }
}

impl Projection for CustomerOrderHistoryProjection {
    type Event = OrderAction;

    fn name(&self) -> &str {
        "customer_order_history"
    }

    async fn apply_event(&self, event: &Self::Event) -> Result<()> {
        match event {
            // When an order is placed, insert a new record
            OrderAction::OrderPlaced {
                order_id,
                customer_id,
                items,
                total,
                timestamp,
            } => {
                tracing::info!(
                    order_id = %order_id,
                    customer_id = %customer_id,
                    item_count = items.len(),
                    total = %total,
                    "Applying OrderPlaced event to projection"
                );

                sqlx::query(
                    "INSERT INTO order_projections
                     (id, customer_id, item_count, total_cents, status,
                      placed_at, updated_at, tracking, cancellation_reason)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                     ON CONFLICT (id) DO UPDATE
                     SET customer_id = EXCLUDED.customer_id,
                         item_count = EXCLUDED.item_count,
                         total_cents = EXCLUDED.total_cents,
                         status = EXCLUDED.status,
                         placed_at = EXCLUDED.placed_at,
                         updated_at = EXCLUDED.updated_at",
                )
                .bind(order_id.as_str())
                .bind(customer_id.as_str())
                .bind(items.len() as i32)
                .bind(total.cents())
                .bind("placed")
                .bind(timestamp)
                .bind(Utc::now())
                .bind(None::<String>) // tracking
                .bind(None::<String>) // cancellation_reason
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Storage(format!("Failed to insert order: {e}")))?;

                Ok(())
            }

            // When an order is cancelled, update the status and reason
            OrderAction::OrderCancelled {
                order_id,
                reason,
                timestamp: _,
            } => {
                tracing::info!(
                    order_id = %order_id,
                    reason = %reason,
                    "Applying OrderCancelled event to projection"
                );

                sqlx::query(
                    "UPDATE order_projections
                     SET status = 'cancelled',
                         cancellation_reason = $2,
                         updated_at = now()
                     WHERE id = $1",
                )
                .bind(order_id.as_str())
                .bind(reason)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Storage(format!("Failed to update order: {e}")))?;

                Ok(())
            }

            // When an order is shipped, update the status and tracking
            OrderAction::OrderShipped {
                order_id,
                tracking,
                timestamp: _,
            } => {
                tracing::info!(
                    order_id = %order_id,
                    tracking = %tracking,
                    "Applying OrderShipped event to projection"
                );

                sqlx::query(
                    "UPDATE order_projections
                     SET status = 'shipped',
                         tracking = $2,
                         updated_at = now()
                     WHERE id = $1",
                )
                .bind(order_id.as_str())
                .bind(tracking)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Storage(format!("Failed to update order: {e}")))?;

                Ok(())
            }

            // Ignore commands and internal events
            OrderAction::PlaceOrder { .. }
            | OrderAction::CancelOrder { .. }
            | OrderAction::ShipOrder { .. }
            | OrderAction::ValidationFailed { .. }
            | OrderAction::EventPersisted { .. } => {
                // Commands and internal events are not persisted to the projection
                Ok(())
            }
        }
    }

    async fn rebuild(&self) -> Result<()> {
        tracing::info!("Rebuilding customer_order_history projection");

        // Clear all projection data (for rebuild)
        sqlx::query("TRUNCATE order_projections")
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to truncate table: {e}")))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Full integration tests with PostgreSQL are in the main.rs example
    // These are just basic type tests

    #[test]
    fn test_order_summary_creation() {
        let summary = OrderSummary {
            id: "order-1".to_string(),
            customer_id: "cust-1".to_string(),
            item_count: 3,
            total_cents: 9999,
            status: "placed".to_string(),
            placed_at: Utc::now(),
            updated_at: Utc::now(),
            tracking: None,
            cancellation_reason: None,
        };

        assert_eq!(summary.id, "order-1");
        assert_eq!(summary.item_count, 3);
        assert_eq!(summary.status, "placed");
    }
}
