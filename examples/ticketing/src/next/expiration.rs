//! Reservation expiration worker.
//!
//! Periodically finds reservation sagas whose deadline has passed and drives them
//! to a terminal state **through the saga** (so seat release runs via the saga's
//! own compensation and `saga_state` reaches a terminal phase), rather than poking
//! the inventory aggregate directly.
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────┐
//! │              ReservationExpirationWorker                   │
//! ├────────────────────────────────────────────────────────────┤
//! │  1. On startup: expire all overdue non-terminal sagas      │
//! │     (this is the expire-only boot recovery)                │
//! │                                                            │
//! │  2. Loop every check_interval (until shutdown):            │
//! │     SELECT reservation_id FROM saga_state                  │
//! │     WHERE phase NOT IN ('Completed','Failed')              │
//! │       AND expires_at < now()                               │
//! │                            │                               │
//! │                            ▼                               │
//! │     saga_handler.handle(ExpireReservation { .. })          │
//! └────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Durability
//!
//! `saga_state` is written transactionally with the saga's events, so it is the
//! authoritative, restart-safe source of overdue sagas — the startup sweep catches
//! anything that expired while the process was down. Expiring an already-terminal
//! saga is a phase-guarded no-op, so re-firing is safe.

use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use super::{ReservationSagaHandler, ReservationSagaInput};
use crate::types::ReservationId;

/// Worker that drives overdue reservation sagas to expiry through the saga handler.
pub struct ReservationExpirationWorker {
    /// Database pool for querying `saga_state`.
    pool: PgPool,
    /// The reservation saga handler used to issue `ExpireReservation`.
    saga_handler: Arc<ReservationSagaHandler>,
    /// How often to check for overdue sagas.
    check_interval: Duration,
}

impl ReservationExpirationWorker {
    /// Create a new expiration worker.
    #[must_use]
    pub fn new(
        pool: PgPool,
        saga_handler: Arc<ReservationSagaHandler>,
        check_interval: Duration,
    ) -> Self {
        Self {
            pool,
            saga_handler,
            check_interval,
        }
    }

    /// Create a worker with the default 30-second check interval.
    #[must_use]
    pub fn with_default_interval(pool: PgPool, saga_handler: Arc<ReservationSagaHandler>) -> Self {
        Self::new(pool, saga_handler, Duration::from_secs(30))
    }

    /// Run the expiration worker until `shutdown_rx` fires.
    ///
    /// On startup it immediately expires any overdue non-terminal sagas (expire-only
    /// boot recovery), then polls at `check_interval`. Spawn this as a background task.
    #[instrument(skip(self, shutdown_rx), name = "expiration_worker")]
    pub async fn run(self, mut shutdown_rx: broadcast::Receiver<()>) {
        info!(
            check_interval_secs = self.check_interval.as_secs(),
            "Starting reservation expiration worker"
        );

        // Startup sweep == expire-only boot recovery.
        let recovered = self.expire_due_sagas().await;
        if recovered > 0 {
            info!(count = recovered, "Expired overdue sagas on startup");
        }

        let mut interval = tokio::time::interval(self.check_interval);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let expired = self.expire_due_sagas().await;
                    if expired > 0 {
                        debug!(count = expired, "Expired overdue sagas");
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Reservation expiration worker shutting down");
                    break;
                }
            }
        }
    }

    /// Expire all overdue non-terminal sagas. Returns the number expired.
    #[instrument(skip(self))]
    async fn expire_due_sagas(&self) -> usize {
        let due = match self.query_due_sagas().await {
            Ok(ids) => ids,
            Err(e) => {
                error!(error = %e, "Failed to query overdue sagas");
                return 0;
            },
        };

        if due.is_empty() {
            return 0;
        }
        debug!(count = due.len(), "Found overdue sagas");

        let mut expired_count = 0;
        for reservation_id in due {
            match self.expire_one(reservation_id).await {
                Ok(()) => {
                    expired_count += 1;
                    debug!(reservation_id = %reservation_id, "Expired overdue saga");
                },
                Err(e) => {
                    // Log and continue — one failure must not stop the rest.
                    warn!(reservation_id = %reservation_id, error = %e, "Failed to expire saga");
                },
            }
        }
        expired_count
    }

    /// Query `saga_state` for non-terminal sagas whose deadline has passed.
    async fn query_due_sagas(&self) -> Result<Vec<Uuid>, sqlx::Error> {
        sqlx::query_scalar::<_, Uuid>(
            r"
            SELECT reservation_id
            FROM saga_state
            WHERE phase NOT IN ('Completed', 'Failed')
              AND expires_at < now()
            ORDER BY expires_at ASC
            LIMIT 100
            ",
        )
        .fetch_all(&self.pool)
        .await
    }

    /// Drive a single saga to expiry through the saga handler.
    async fn expire_one(&self, reservation_id: Uuid) -> Result<(), String> {
        self.saga_handler
            .handle(ReservationSagaInput::ExpireReservation {
                reservation_id: ReservationId::from_uuid(reservation_id),
                fetched: None,
            })
            .await
            .map_err(|e| format!("Handler error: {e}"))?;
        Ok(())
    }
}
