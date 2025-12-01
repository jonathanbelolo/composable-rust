//! Reservation expiration worker.
//!
//! This module provides [`ReservationExpirationWorker`] which periodically
//! checks for expired reservations and releases their seats.
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────┐
//! │              ReservationExpirationWorker                   │
//! ├────────────────────────────────────────────────────────────┤
//! │                                                            │
//! │  1. On startup: release all expired reservations           │
//! │                                                            │
//! │  2. Loop every check_interval:                             │
//! │     ┌──────────────────────────────────────────────┐      │
//! │     │ SELECT * FROM reservations                    │      │
//! │     │ WHERE status = 'active'                       │      │
//! │     │   AND expires_at < NOW()                      │      │
//! │     └──────────────────────┬───────────────────────┘      │
//! │                            │                               │
//! │                            ▼                               │
//! │     For each expired reservation:                          │
//! │     ┌──────────────────────────────────────────────┐      │
//! │     │ InventoryHandler.handle(Release { ... })      │      │
//! │     └──────────────────────────────────────────────┘      │
//! │                                                            │
//! └────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Durability
//!
//! Because reservations are tracked in a durable database table:
//! - Server crashes don't lose reservation expiration info
//! - On restart, the worker catches up on any missed expirations
//! - No in-memory state required
//!
//! # Usage
//!
//! ```rust,ignore
//! use ticketing::next::{ReservationExpirationWorker, InventoryHandler};
//! use std::time::Duration;
//!
//! // Create the worker
//! let worker = ReservationExpirationWorker::new(
//!     projections_pool,
//!     inventory_handler,
//!     Duration::from_secs(30), // Check every 30 seconds
//! );
//!
//! // Spawn as a background task
//! tokio::spawn(worker.run());
//! ```

use chrono::{DateTime, Utc};
use composable_rust_next::{Handler, HandlerEnvironment, NoOpCallExecutor, NoOpQueryFetcher};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use super::{InventoryBusinessLogic, InventoryCommand, ReleaseReason};
use crate::types::ReservationId;

/// Record of an expired reservation from the database.
#[derive(Debug, sqlx::FromRow)]
struct ExpiredReservation {
    reservation_id: Uuid,
    event_id: Uuid,
    section: String,
    #[allow(dead_code)] // Part of query result, may be useful for metrics
    seat_count: i32,
    expires_at: DateTime<Utc>,
}

/// Worker that periodically releases expired reservations.
///
/// This worker ensures that reservations that aren't confirmed within
/// their expiration window have their seats returned to the available pool.
///
/// # Crash Recovery
///
/// On startup, the worker immediately checks for and releases any
/// reservations that expired while the server was down. This ensures
/// no seats are permanently stuck in a reserved state.
pub struct ReservationExpirationWorker<Env>
where
    Env: HandlerEnvironment,
{
    /// Database pool for querying reservations
    pool: PgPool,
    /// Handler to issue Release commands
    inventory_handler: Arc<Handler<InventoryBusinessLogic, NoOpCallExecutor, NoOpQueryFetcher, Env>>,
    /// How often to check for expired reservations
    check_interval: Duration,
}

impl<Env> ReservationExpirationWorker<Env>
where
    Env: HandlerEnvironment + Send + Sync + 'static,
{
    /// Create a new expiration worker.
    ///
    /// # Arguments
    ///
    /// * `pool` - Database pool for the projections database (where reservations table lives)
    /// * `inventory_handler` - Handler for issuing Release commands
    /// * `check_interval` - How often to check for expired reservations
    #[must_use]
    pub fn new(
        pool: PgPool,
        inventory_handler: Arc<Handler<InventoryBusinessLogic, NoOpCallExecutor, NoOpQueryFetcher, Env>>,
        check_interval: Duration,
    ) -> Self {
        Self {
            pool,
            inventory_handler,
            check_interval,
        }
    }

    /// Create a worker with the default 30-second check interval.
    #[must_use]
    pub fn with_default_interval(
        pool: PgPool,
        inventory_handler: Arc<Handler<InventoryBusinessLogic, NoOpCallExecutor, NoOpQueryFetcher, Env>>,
    ) -> Self {
        Self::new(pool, inventory_handler, Duration::from_secs(30))
    }

    /// Run the expiration worker.
    ///
    /// This method runs indefinitely, checking for expired reservations
    /// at the configured interval. It should be spawned as a background task.
    ///
    /// # Startup Recovery
    ///
    /// On startup, immediately checks for and releases any reservations
    /// that expired while the server was down.
    #[instrument(skip(self), name = "expiration_worker")]
    pub async fn run(self) {
        info!(
            check_interval_secs = self.check_interval.as_secs(),
            "Starting reservation expiration worker"
        );

        // Immediate check on startup for crash recovery
        info!("Performing startup recovery - checking for expired reservations");
        let recovered = self.release_expired_reservations().await;
        if recovered > 0 {
            info!(count = recovered, "Released reservations that expired during downtime");
        }

        // Main loop
        let mut interval = tokio::time::interval(self.check_interval);
        loop {
            interval.tick().await;

            let released = self.release_expired_reservations().await;
            if released > 0 {
                debug!(count = released, "Released expired reservations");
            }
        }
    }

    /// Check for and release all expired reservations.
    ///
    /// Returns the number of reservations released.
    #[instrument(skip(self))]
    async fn release_expired_reservations(&self) -> usize {
        // Query for expired active reservations
        let expired = match self.query_expired_reservations().await {
            Ok(reservations) => reservations,
            Err(e) => {
                error!(error = %e, "Failed to query expired reservations");
                return 0;
            }
        };

        if expired.is_empty() {
            return 0;
        }

        debug!(count = expired.len(), "Found expired reservations");

        let mut released_count = 0;
        for reservation in expired {
            match self.release_reservation(&reservation).await {
                Ok(()) => {
                    released_count += 1;
                    debug!(
                        reservation_id = %reservation.reservation_id,
                        event_id = %reservation.event_id,
                        section = %reservation.section,
                        expired_at = %reservation.expires_at,
                        "Released expired reservation"
                    );
                }
                Err(e) => {
                    // Log but continue - don't let one failure stop others
                    warn!(
                        reservation_id = %reservation.reservation_id,
                        error = %e,
                        "Failed to release expired reservation"
                    );
                }
            }
        }

        released_count
    }

    /// Query the database for active reservations that have expired.
    async fn query_expired_reservations(&self) -> Result<Vec<ExpiredReservation>, sqlx::Error> {
        sqlx::query_as::<_, ExpiredReservation>(
            r"
            SELECT reservation_id, event_id, section, seat_count, expires_at
            FROM reservations
            WHERE status = 'active'
              AND expires_at < NOW()
            ORDER BY expires_at ASC
            LIMIT 100
            ",
        )
        .fetch_all(&self.pool)
        .await
    }

    /// Release a single expired reservation by issuing a Release command.
    async fn release_reservation(&self, reservation: &ExpiredReservation) -> Result<(), String> {
        let command = InventoryCommand::Release {
            reservation_id: ReservationId::from_uuid(reservation.reservation_id),
            reason: ReleaseReason::Expired,
            fetched: None,
        };

        self.inventory_handler
            .handle(command)
            .await
            .map_err(|e| format!("Handler error: {e}"))?;

        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    // Note: Full integration tests require a real database.
    // These tests verify the basic structure compiles.

    #[test]
    fn expired_reservation_struct_has_expected_fields() {
        // This test ensures the struct matches our SQL query
        let _reservation = ExpiredReservation {
            reservation_id: Uuid::new_v4(),
            event_id: Uuid::new_v4(),
            section: "VIP".to_string(),
            seat_count: 5,
            expires_at: Utc::now(),
        };
    }
}
