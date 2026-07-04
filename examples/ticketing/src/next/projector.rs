//! Projector implementations for read models.
//!
//! This module provides projectors that update read models when events
//! are persisted, providing synchronous projection completion.
//!
//! # Projectors
//!
//! - [`EventProjector`]: Updates the events read model
//! - [`InventoryProjector`]: Updates the inventory/seats availability read model
//! - [`PaymentProjector`]: Updates the payment history read model
//! - `EventInventorySagaProjector`: In-memory projection for saga state

use composable_rust_next::{
    AtomicError, BusinessLogic, DynAtomicPersist, ProjectionError, Projector, SerializedEvent,
    StreamId, Version,
};
use composable_rust_postgres_next::{PgTransactionalProjector, PostgresEventStore};
use sqlx::PgPool;
use tracing::instrument;

use crate::types::{EventId, ReservationId};

use super::event_inventory_saga::{EVENT_INVENTORY_SAGA_STATE_VERSION, SagaEvent, SagaState};
use super::reservation_saga::{
    RESERVATION_SAGA_STATE_VERSION, ReservationSagaEvent, ReservationSagaState,
};
use super::{
    EventEvent, EventInventorySagaLogic, InventoryEvent, PaymentEvent, ReservationSagaLogic,
};

/// Projector for the Event aggregate read model.
///
/// Updates the `events` table in the projections database when
/// domain events are persisted.
///
/// # Synchronous Completion
///
/// When [`Projector::project`] returns `Ok(())`, the caller knows
/// the read model is fully updated. This enables strong consistency
/// between writes and reads.
#[derive(Clone)]
pub struct EventProjector {
    pool: PgPool,
}

impl EventProjector {
    /// Create a new event projector.
    ///
    /// # Arguments
    ///
    /// * `pool` - Connection pool for the projections database
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Project a single event to the read model.
    #[instrument(skip(self, event), fields(event_type = %event.event_type))]
    #[allow(clippy::too_many_lines)] // Match arms for each event type
    async fn project_event(&self, event: &SerializedEvent) -> Result<(), ProjectionError> {
        // Deserialize the event
        let domain_event: EventEvent = bincode::deserialize(&event.payload).map_err(|e| {
            ProjectionError::Deserialization(format!("failed to deserialize event: {e}"))
        })?;

        match domain_event {
            EventEvent::Created {
                event_id,
                name,
                owner_id,
                venue,
                date,
                pricing_tiers,
                created_at,
            } => {
                // Serialize venue and pricing_tiers as JSON for storage
                let venue_json = serde_json::to_value(&venue).map_err(|e| {
                    ProjectionError::Custom(format!("failed to serialize venue: {e}"))
                })?;

                let pricing_json = serde_json::to_value(&pricing_tiers).map_err(|e| {
                    ProjectionError::Custom(format!("failed to serialize pricing_tiers: {e}"))
                })?;

                sqlx::query(
                    r"
                    INSERT INTO events_projection (
                        event_id, name, owner_id, venue, event_date,
                        pricing_tiers, status, created_at, updated_at
                    )
                    VALUES ($1, $2, $3, $4, $5, $6, 'draft', $7, $7)
                    ON CONFLICT (event_id) DO NOTHING
                    ",
                )
                .bind(event_id.as_uuid())
                .bind(&name)
                .bind(owner_id.0)
                .bind(&venue_json)
                .bind(date.inner())
                .bind(&pricing_json)
                .bind(created_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                tracing::debug!(event_id = %event_id, "Projected EventCreated");
            },

            EventEvent::Updated {
                event_id,
                name,
                venue,
                date,
                updated_at,
            } => {
                // Build dynamic update query
                let mut updates = vec!["updated_at = $2".to_string()];
                let mut param_idx = 3;

                if name.is_some() {
                    updates.push(format!("name = ${param_idx}"));
                    param_idx += 1;
                }
                if venue.is_some() {
                    updates.push(format!("venue = ${param_idx}"));
                    param_idx += 1;
                }
                if date.is_some() {
                    updates.push(format!("event_date = ${param_idx}"));
                }

                let query = format!(
                    "UPDATE events_projection SET {} WHERE event_id = $1",
                    updates.join(", ")
                );

                // Build and execute query with dynamic bindings
                let mut q = sqlx::query(&query)
                    .bind(event_id.as_uuid())
                    .bind(updated_at);

                if let Some(ref n) = name {
                    q = q.bind(n);
                }
                if let Some(ref v) = venue {
                    let venue_json = serde_json::to_value(v).map_err(|e| {
                        ProjectionError::Custom(format!("failed to serialize venue: {e}"))
                    })?;
                    q = q.bind(venue_json);
                }
                if let Some(d) = date {
                    q = q.bind(d.inner());
                }

                q.execute(&self.pool)
                    .await
                    .map_err(|e| ProjectionError::Database(e.to_string()))?;

                tracing::debug!(event_id = %event_id, "Projected EventUpdated");
            },

            EventEvent::Published {
                event_id,
                published_at,
            } => {
                sqlx::query(
                    r"
                    UPDATE events_projection
                    SET status = 'published', updated_at = $2
                    WHERE event_id = $1
                    ",
                )
                .bind(event_id.as_uuid())
                .bind(published_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                tracing::debug!(event_id = %event_id, "Projected EventPublished");
            },

            EventEvent::Cancelled {
                event_id,
                reason,
                cancelled_at,
            } => {
                sqlx::query(
                    r"
                    UPDATE events_projection
                    SET status = 'cancelled', cancellation_reason = $2, updated_at = $3
                    WHERE event_id = $1
                    ",
                )
                .bind(event_id.as_uuid())
                .bind(&reason)
                .bind(cancelled_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                tracing::debug!(event_id = %event_id, "Projected EventCancelled");
            },

            EventEvent::PricingUpdated {
                event_id,
                pricing_tiers,
                updated_at,
            } => {
                let pricing_json = serde_json::to_value(&pricing_tiers).map_err(|e| {
                    ProjectionError::Custom(format!("failed to serialize pricing_tiers: {e}"))
                })?;

                sqlx::query(
                    r"
                    UPDATE events_projection
                    SET pricing_tiers = $2, updated_at = $3
                    WHERE event_id = $1
                    ",
                )
                .bind(event_id.as_uuid())
                .bind(&pricing_json)
                .bind(updated_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                tracing::debug!(event_id = %event_id, "Projected EventPricingUpdated");
            },

            EventEvent::VenueSectionsAdded {
                event_id,
                sections,
                added_at,
            } => {
                // Fetch existing venue to update it
                let existing_venue: Option<serde_json::Value> =
                    sqlx::query_scalar("SELECT venue FROM events_projection WHERE event_id = $1")
                        .bind(event_id.as_uuid())
                        .fetch_optional(&self.pool)
                        .await
                        .map_err(|e| ProjectionError::Database(e.to_string()))?;

                if let Some(mut venue_json) = existing_venue {
                    // Add sections to existing venue
                    if let Some(venue_obj) = venue_json.as_object_mut() {
                        if let Some(existing_sections) = venue_obj.get_mut("sections") {
                            if let Some(sections_arr) = existing_sections.as_array_mut() {
                                let new_sections: Vec<serde_json::Value> = sections
                                    .iter()
                                    .filter_map(|s| serde_json::to_value(s).ok())
                                    .collect();
                                sections_arr.extend(new_sections);
                            }
                        }

                        // Update total capacity
                        if let Some(capacity_obj) = venue_obj.get_mut("capacity") {
                            if let Some(current_cap) = capacity_obj
                                .get("value")
                                .and_then(serde_json::Value::as_u64)
                            {
                                let additional: u64 =
                                    sections.iter().map(|s| u64::from(s.capacity.value())).sum();
                                capacity_obj["value"] =
                                    serde_json::Value::Number((current_cap + additional).into());
                            }
                        }
                    }

                    sqlx::query(
                        r"
                        UPDATE events_projection
                        SET venue = $2, updated_at = $3
                        WHERE event_id = $1
                        ",
                    )
                    .bind(event_id.as_uuid())
                    .bind(&venue_json)
                    .bind(added_at)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| ProjectionError::Database(e.to_string()))?;

                    tracing::debug!(
                        event_id = %event_id,
                        sections_added = sections.len(),
                        "Projected EventVenueSectionsAdded"
                    );
                }
            },
        }

        Ok(())
    }
}

impl Projector for EventProjector {
    #[instrument(skip(self, events), fields(event_count = events.len()))]
    fn project(
        &self,
        events: &[SerializedEvent],
    ) -> impl std::future::Future<Output = Result<(), ProjectionError>> + Send {
        // Clone data needed for the async block
        let events_owned: Vec<SerializedEvent> = events.to_vec();
        let this = self.clone();

        async move {
            for event in &events_owned {
                this.project_event(event).await?;
            }

            tracing::debug!(count = events_owned.len(), "Projected all events");
            Ok(())
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Inventory Projector
// ═══════════════════════════════════════════════════════════════════════════

/// Projector for the Inventory aggregate read model.
///
/// Updates the `inventory` and `reservations` tables in the projections
/// database when inventory events are persisted.
///
/// # Table Schemas
///
/// The projector expects tables with structure like:
///
/// ```sql
/// -- Aggregate seat counts per section
/// CREATE TABLE inventory (
///     event_id UUID NOT NULL,
///     section TEXT NOT NULL,
///     total_capacity INTEGER NOT NULL,
///     available_seats INTEGER NOT NULL,
///     reserved_seats INTEGER NOT NULL,
///     created_at TIMESTAMPTZ NOT NULL,
///     updated_at TIMESTAMPTZ NOT NULL,
///     PRIMARY KEY (event_id, section)
/// );
///
/// -- Individual reservation tracking (for expiration)
/// CREATE TABLE reservations (
///     reservation_id UUID PRIMARY KEY,
///     event_id UUID NOT NULL,
///     section TEXT NOT NULL,
///     seat_count INTEGER NOT NULL,
///     expires_at TIMESTAMPTZ NOT NULL,
///     status TEXT NOT NULL,  -- 'active', 'confirmed', 'released'
///     created_at TIMESTAMPTZ NOT NULL,
///     updated_at TIMESTAMPTZ NOT NULL
/// );
///
/// -- Index for efficient expiration queries
/// CREATE INDEX idx_reservations_expiration
///     ON reservations (status, expires_at)
///     WHERE status = 'active';
/// ```
#[derive(Clone)]
pub struct InventoryProjector {
    pool: PgPool,
}

impl InventoryProjector {
    /// Create a new inventory projector.
    ///
    /// # Arguments
    ///
    /// * `pool` - Connection pool for the projections database
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Project a single inventory event to the read model.
    #[instrument(skip(self, event), fields(event_type = %event.event_type))]
    async fn project_event(&self, event: &SerializedEvent) -> Result<(), ProjectionError> {
        // Deserialize the event
        let domain_event: InventoryEvent = bincode::deserialize(&event.payload).map_err(|e| {
            ProjectionError::Deserialization(format!("failed to deserialize inventory event: {e}"))
        })?;

        match domain_event {
            InventoryEvent::Initialized {
                event_id,
                section,
                capacity,
                seats,
                initialized_at,
            } => {
                #[allow(clippy::cast_possible_wrap)]
                let capacity_i32 = capacity as i32;

                // Insert aggregate inventory record
                sqlx::query(
                    r"
                    INSERT INTO inventory (
                        event_id, section, total_capacity, available_seats,
                        reserved_seats, created_at, updated_at
                    )
                    VALUES ($1, $2, $3, $3, 0, $4, $4)
                    ON CONFLICT (event_id, section) DO NOTHING
                    ",
                )
                .bind(event_id.as_uuid())
                .bind(&section)
                .bind(capacity_i32)
                .bind(initialized_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                // Bulk insert all seat records using UNNEST (single query instead of N)
                let seat_ids: Vec<uuid::Uuid> = seats.iter().map(|s| *s.as_uuid()).collect();
                sqlx::query(
                    r"
                    INSERT INTO seats (seat_id, event_id, section, status, created_at, updated_at)
                    SELECT unnest($1::uuid[]), $2, $3, 'available', $4, $4
                    ON CONFLICT (seat_id) DO NOTHING
                    ",
                )
                .bind(&seat_ids)
                .bind(event_id.as_uuid())
                .bind(&section)
                .bind(initialized_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                tracing::debug!(
                    event_id = %event_id,
                    section = %section,
                    seat_count = seats.len(),
                    "Projected InventoryInitialized with individual seats"
                );
            },

            InventoryEvent::SeatsReserved {
                reservation_id,
                event_id,
                section,
                seats,
                expires_at,
                reserved_at,
            } => {
                #[allow(clippy::cast_possible_wrap)]
                let seat_count = seats.len() as i32;

                // Update inventory counts
                sqlx::query(
                    r"
                    UPDATE inventory
                    SET available_seats = available_seats - $3,
                        reserved_seats = reserved_seats + $3,
                        updated_at = $4
                    WHERE event_id = $1 AND section = $2
                    ",
                )
                .bind(event_id.as_uuid())
                .bind(&section)
                .bind(seat_count)
                .bind(reserved_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                // Bulk update all seat records to reserved (single query instead of N)
                let seat_ids: Vec<uuid::Uuid> = seats.iter().map(|s| *s.as_uuid()).collect();
                sqlx::query(
                    r"
                    UPDATE seats
                    SET status = 'reserved',
                        reservation_id = $2,
                        expires_at = $3,
                        updated_at = $4
                    WHERE seat_id = ANY($1::uuid[])
                    ",
                )
                .bind(&seat_ids)
                .bind(reservation_id.as_uuid())
                .bind(expires_at)
                .bind(reserved_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                // Insert reservation record for expiration tracking
                sqlx::query(
                    r"
                    INSERT INTO reservations (
                        reservation_id, event_id, section, seat_count,
                        expires_at, status, created_at, updated_at
                    )
                    VALUES ($1, $2, $3, $4, $5, 'active', $6, $6)
                    ON CONFLICT (reservation_id) DO NOTHING
                    ",
                )
                .bind(reservation_id.as_uuid())
                .bind(event_id.as_uuid())
                .bind(&section)
                .bind(seat_count)
                .bind(expires_at)
                .bind(reserved_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                tracing::debug!(
                    event_id = %event_id,
                    section = %section,
                    reservation_id = %reservation_id,
                    seats = seat_count,
                    expires_at = %expires_at,
                    "Projected SeatsReserved"
                );
            },

            // SeatsConfirmed: update reservation status, reserved seats become sold
            InventoryEvent::SeatsConfirmed {
                reservation_id,
                seats,
                confirmed_at,
                ..
            } => {
                #[allow(clippy::cast_possible_wrap)]
                let seat_count = seats.len() as i32;

                // Bulk update all seat records to sold (single query instead of N)
                let seat_ids: Vec<uuid::Uuid> = seats.iter().map(|s| *s.as_uuid()).collect();
                sqlx::query(
                    r"
                    UPDATE seats
                    SET status = 'sold',
                        updated_at = $2
                    WHERE seat_id = ANY($1::uuid[])
                    ",
                )
                .bind(&seat_ids)
                .bind(confirmed_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                // Update reservation status to confirmed
                sqlx::query(
                    r"
                    UPDATE reservations
                    SET status = 'confirmed', updated_at = $2
                    WHERE reservation_id = $1
                    ",
                )
                .bind(reservation_id.as_uuid())
                .bind(confirmed_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                // Look up the reservation to get event_id and section for inventory update
                let reservation: Option<(uuid::Uuid, String)> = sqlx::query_as(
                    r"
                    SELECT event_id, section FROM reservations
                    WHERE reservation_id = $1
                    ",
                )
                .bind(reservation_id.as_uuid())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                if let Some((event_id, section)) = reservation {
                    // Update inventory counts - seats move from reserved to sold
                    // reserved_seats decreases, available_seats stays the same (already decremented when reserved)
                    sqlx::query(
                        r"
                        UPDATE inventory
                        SET reserved_seats = reserved_seats - $3,
                            updated_at = $4
                        WHERE event_id = $1 AND section = $2
                        ",
                    )
                    .bind(event_id)
                    .bind(&section)
                    .bind(seat_count)
                    .bind(confirmed_at)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| ProjectionError::Database(e.to_string()))?;

                    tracing::debug!(
                        reservation_id = %reservation_id,
                        event_id = %event_id,
                        section = %section,
                        seats = seat_count,
                        "Projected SeatsConfirmed - inventory updated"
                    );
                } else {
                    tracing::warn!(
                        reservation_id = %reservation_id,
                        seats = seats.len(),
                        "SeatsConfirmed for unknown reservation - inventory not updated"
                    );
                }
            },

            // SeatsReleased: update reservation status and return seats to available pool
            InventoryEvent::SeatsReleased {
                reservation_id,
                seats,
                reason,
                released_at,
            } => {
                #[allow(clippy::cast_possible_wrap)]
                let seat_count = seats.len() as i32;

                // Bulk update all seat records back to available (single query instead of N)
                let seat_ids: Vec<uuid::Uuid> = seats.iter().map(|s| *s.as_uuid()).collect();
                sqlx::query(
                    r"
                    UPDATE seats
                    SET status = 'available',
                        reservation_id = NULL,
                        expires_at = NULL,
                        updated_at = $2
                    WHERE seat_id = ANY($1::uuid[])
                    ",
                )
                .bind(&seat_ids)
                .bind(released_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                // Look up the reservation to get event_id and section
                let reservation: Option<(uuid::Uuid, String)> = sqlx::query_as(
                    r"
                    SELECT event_id, section FROM reservations
                    WHERE reservation_id = $1
                    ",
                )
                .bind(reservation_id.as_uuid())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                if let Some((event_id, section)) = reservation {
                    // Update inventory counts - return seats to available
                    sqlx::query(
                        r"
                        UPDATE inventory
                        SET available_seats = available_seats + $3,
                            reserved_seats = reserved_seats - $3,
                            updated_at = $4
                        WHERE event_id = $1 AND section = $2
                        ",
                    )
                    .bind(event_id)
                    .bind(&section)
                    .bind(seat_count)
                    .bind(released_at)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| ProjectionError::Database(e.to_string()))?;

                    tracing::debug!(
                        reservation_id = %reservation_id,
                        event_id = %event_id,
                        section = %section,
                        seats = seat_count,
                        reason = %reason,
                        "Projected SeatsReleased - inventory updated"
                    );
                } else {
                    tracing::warn!(
                        reservation_id = %reservation_id,
                        "SeatsReleased for unknown reservation - inventory not updated"
                    );
                }

                // Update reservation status to released with reason
                // Status format: 'released:{reason}' for analytics
                let status = format!("released:{reason}");
                sqlx::query(
                    r"
                    UPDATE reservations
                    SET status = $2, updated_at = $3
                    WHERE reservation_id = $1
                    ",
                )
                .bind(reservation_id.as_uuid())
                .bind(&status)
                .bind(released_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;
            },
        }

        Ok(())
    }
}

impl Projector for InventoryProjector {
    #[instrument(skip(self, events), fields(event_count = events.len()))]
    fn project(
        &self,
        events: &[SerializedEvent],
    ) -> impl std::future::Future<Output = Result<(), ProjectionError>> + Send {
        let events_owned: Vec<SerializedEvent> = events.to_vec();
        let this = self.clone();

        async move {
            for event in &events_owned {
                this.project_event(event).await?;
            }

            tracing::debug!(count = events_owned.len(), "Projected all inventory events");
            Ok(())
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Payment Projector
// ═══════════════════════════════════════════════════════════════════════════

/// Projector for the Payment aggregate read model.
///
/// Updates the `payments` table in the projections database when
/// payment events are persisted.
///
/// # Table Schema
///
/// The projector expects a table with structure like:
/// ```sql
/// CREATE TABLE payments (
///     payment_id UUID PRIMARY KEY,
///     reservation_id UUID NOT NULL,
///     customer_id UUID NOT NULL,
///     amount_cents BIGINT NOT NULL,
///     payment_method TEXT NOT NULL,
///     status TEXT NOT NULL,
///     transaction_id TEXT,
///     failure_reason TEXT,
///     refund_amount_cents BIGINT,
///     refund_reason TEXT,
///     created_at TIMESTAMPTZ NOT NULL,
///     updated_at TIMESTAMPTZ NOT NULL
/// );
/// ```
#[derive(Clone)]
pub struct PaymentProjector {
    pool: PgPool,
}

impl PaymentProjector {
    /// Create a new payment projector.
    ///
    /// # Arguments
    ///
    /// * `pool` - Connection pool for the projections database
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Project a single payment event to the read model.
    #[instrument(skip(self, event), fields(event_type = %event.event_type))]
    async fn project_event(&self, event: &SerializedEvent) -> Result<(), ProjectionError> {
        // Deserialize the event
        let domain_event: PaymentEvent = bincode::deserialize(&event.payload).map_err(|e| {
            ProjectionError::Deserialization(format!("failed to deserialize payment event: {e}"))
        })?;

        match domain_event {
            PaymentEvent::PaymentProcessed {
                payment_id,
                reservation_id,
                customer_id,
                amount,
                payment_method,
                processed_at,
            } => {
                let payment_method_str = format!("{payment_method:?}");

                #[allow(clippy::cast_possible_wrap)]
                let amount_cents = amount.cents() as i64;

                sqlx::query(
                    r"
                    INSERT INTO payments (
                        payment_id, reservation_id, customer_id, amount_cents,
                        payment_method, status, created_at, updated_at
                    )
                    VALUES ($1, $2, $3, $4, $5, 'processing', $6, $6)
                    ON CONFLICT (payment_id) DO NOTHING
                    ",
                )
                .bind(payment_id.as_uuid())
                .bind(reservation_id.as_uuid())
                .bind(customer_id.as_uuid())
                .bind(amount_cents)
                .bind(&payment_method_str)
                .bind(processed_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                tracing::debug!(payment_id = %payment_id, "Projected PaymentProcessed");
            },

            PaymentEvent::PaymentSucceeded {
                payment_id,
                transaction_id,
                succeeded_at,
            } => {
                sqlx::query(
                    r"
                    UPDATE payments
                    SET status = 'succeeded',
                        transaction_id = $2,
                        updated_at = $3
                    WHERE payment_id = $1
                    ",
                )
                .bind(payment_id.as_uuid())
                .bind(&transaction_id)
                .bind(succeeded_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                tracing::debug!(payment_id = %payment_id, "Projected PaymentSucceeded");
            },

            PaymentEvent::PaymentFailed {
                payment_id,
                reason,
                failed_at,
            } => {
                sqlx::query(
                    r"
                    UPDATE payments
                    SET status = 'failed',
                        failure_reason = $2,
                        updated_at = $3
                    WHERE payment_id = $1
                    ",
                )
                .bind(payment_id.as_uuid())
                .bind(&reason)
                .bind(failed_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                tracing::debug!(payment_id = %payment_id, "Projected PaymentFailed");
            },

            PaymentEvent::PaymentRefunded {
                payment_id,
                amount,
                reason,
                refunded_at,
            } => {
                #[allow(clippy::cast_possible_wrap)]
                let refund_cents = amount.cents() as i64;

                sqlx::query(
                    r"
                    UPDATE payments
                    SET status = 'refunded',
                        refund_amount_cents = $2,
                        refund_reason = $3,
                        updated_at = $4
                    WHERE payment_id = $1
                    ",
                )
                .bind(payment_id.as_uuid())
                .bind(refund_cents)
                .bind(&reason)
                .bind(refunded_at)
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Database(e.to_string()))?;

                tracing::debug!(payment_id = %payment_id, "Projected PaymentRefunded");
            },
        }

        Ok(())
    }
}

impl Projector for PaymentProjector {
    #[instrument(skip(self, events), fields(event_count = events.len()))]
    fn project(
        &self,
        events: &[SerializedEvent],
    ) -> impl std::future::Future<Output = Result<(), ProjectionError>> + Send {
        let events_owned: Vec<SerializedEvent> = events.to_vec();
        let this = self.clone();

        async move {
            for event in &events_owned {
                this.project_event(event).await?;
            }

            tracing::debug!(count = events_owned.len(), "Projected all payment events");
            Ok(())
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Event-Inventory Saga Durable State Projector
// ═══════════════════════════════════════════════════════════════════════════

/// Extract the correlating `event_id` from any [`SagaEvent`] (all variants carry it).
fn event_inventory_saga_id_of(event: &SagaEvent) -> EventId {
    match event {
        SagaEvent::Initiated { event_id, .. }
        | SagaEvent::EventCreated { event_id, .. }
        | SagaEvent::SectionInventoryInitialized { event_id, .. }
        | SagaEvent::Completed { event_id, .. }
        | SagaEvent::EventCreationFailed { event_id, .. }
        | SagaEvent::InventoryInitializationFailed { event_id, .. }
        | SagaEvent::CompensationStarted { event_id, .. }
        | SagaEvent::CompensationCompleted { event_id, .. }
        | SagaEvent::Failed { event_id, .. } => *event_id,
    }
}

/// Transactional projector maintaining the durable `saga_state_event_inventory` row.
///
/// Runs inside the saga's event-append transaction (via
/// [`PostgresEventStore::append_with_projection`]): it folds the just-appended events
/// into the full [`SagaState`] and upserts the authoritative row (`version` == stream
/// version) — committed atomically with the events, so it can never drift from the
/// event stream and is a trustworthy restart-safe resume source.
#[derive(Clone, Default)]
pub struct PgEventInventorySagaStateProjector {
    logic: EventInventorySagaLogic,
}

impl PgEventInventorySagaStateProjector {
    /// Create a new transactional saga-state projector.
    #[must_use]
    pub fn new() -> Self {
        Self {
            logic: EventInventorySagaLogic,
        }
    }
}

impl PgTransactionalProjector for PgEventInventorySagaStateProjector {
    async fn project_in_tx<'a>(
        &'a self,
        conn: &'a mut sqlx::PgConnection,
        final_version: Version,
        events: &'a [SerializedEvent],
    ) -> Result<(), ProjectionError> {
        // Decode the just-appended events (all share one saga stream).
        let mut decoded = Vec::with_capacity(events.len());
        for event in events {
            let saga_event: SagaEvent = bincode::deserialize(&event.payload).map_err(|e| {
                ProjectionError::Deserialization(format!("event-inventory saga event: {e}"))
            })?;
            decoded.push(saga_event);
        }
        let Some(first) = decoded.first() else {
            return Ok(());
        };
        let event_id = event_inventory_saga_id_of(first);

        // Load the prior authoritative state (locked) and fold the new events onto it.
        let prior: Option<(serde_json::Value, i16)> = sqlx::query_as(
            "SELECT state, state_version FROM saga_state_event_inventory \
             WHERE event_id = $1 FOR UPDATE",
        )
        .bind(event_id.as_uuid())
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| ProjectionError::Database(e.to_string()))?;

        let mut state: SagaState = match prior {
            Some((json, stored_version)) => {
                if stored_version != EVENT_INVENTORY_SAGA_STATE_VERSION {
                    return Err(ProjectionError::Custom(format!(
                        "saga_state_event_inventory.state_version {stored_version} != expected \
                         {EVENT_INVENTORY_SAGA_STATE_VERSION}; SagaState shape changed — run a \
                         state migration before deploying"
                    )));
                }
                serde_json::from_value(json).map_err(|e| {
                    ProjectionError::Deserialization(format!("saga_state_event_inventory: {e}"))
                })?
            },
            None => SagaState::default(),
        };
        for saga_event in &decoded {
            self.logic.apply(&mut state, saga_event);
        }

        // Upsert the authoritative row (monotonic: ignore stale replays).
        let state_json = serde_json::to_value(&state)
            .map_err(|e| ProjectionError::Custom(format!("serialize saga_state: {e}")))?;
        let phase = serde_json::to_value(&state.phase)
            .ok()
            .and_then(|v| v.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| format!("{:?}", state.phase));
        let version_i64 = i64::try_from(final_version.as_u64()).unwrap_or(i64::MAX);

        sqlx::query(
            r"
            INSERT INTO saga_state_event_inventory
                (event_id, version, phase, state, state_version, updated_at)
            VALUES ($1, $2, $3, $4, $5, now())
            ON CONFLICT (event_id) DO UPDATE
                SET version = EXCLUDED.version,
                    phase = EXCLUDED.phase,
                    state = EXCLUDED.state,
                    state_version = EXCLUDED.state_version,
                    updated_at = now()
            WHERE saga_state_event_inventory.version < EXCLUDED.version
            ",
        )
        .bind(event_id.as_uuid())
        .bind(version_i64)
        .bind(&phase)
        .bind(&state_json)
        .bind(EVENT_INVENTORY_SAGA_STATE_VERSION)
        .execute(&mut *conn)
        .await
        .map_err(|e| ProjectionError::Database(e.to_string()))?;

        Ok(())
    }
}

/// Adapts a [`PostgresEventStore`] + [`PgEventInventorySagaStateProjector`] into the
/// framework's [`DynAtomicPersist`] seam, so the saga's `Handler` appends events and
/// updates `saga_state_event_inventory` in a single transaction.
#[derive(Clone)]
pub struct PgEventInventorySagaAtomicPersist {
    event_store: PostgresEventStore,
    projector: PgEventInventorySagaStateProjector,
}

impl PgEventInventorySagaAtomicPersist {
    /// Create a new atomic-persist adapter over the given event store.
    #[must_use]
    pub fn new(event_store: PostgresEventStore) -> Self {
        Self {
            event_store,
            projector: PgEventInventorySagaStateProjector::new(),
        }
    }
}

impl DynAtomicPersist for PgEventInventorySagaAtomicPersist {
    fn append_and_project<'a>(
        &'a self,
        stream_id: &'a StreamId,
        expected_version: Option<Version>,
        events: Vec<SerializedEvent>,
    ) -> futures::future::BoxFuture<'a, Result<Version, AtomicError>> {
        Box::pin(async move {
            self.event_store
                .append_with_projection(stream_id, expected_version, events, &self.projector)
                .await
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Reservation Saga Transactional State Projector
// ═══════════════════════════════════════════════════════════════════════════

/// Extract the reservation id (the saga correlation key) from a saga event.
fn reservation_saga_id_of(event: &ReservationSagaEvent) -> ReservationId {
    match event {
        ReservationSagaEvent::ReservationInitiated { reservation_id, .. }
        | ReservationSagaEvent::SeatsAllocated { reservation_id, .. }
        | ReservationSagaEvent::PaymentRequested { reservation_id, .. }
        | ReservationSagaEvent::PaymentSucceeded { reservation_id, .. }
        | ReservationSagaEvent::PaymentFailed { reservation_id, .. }
        | ReservationSagaEvent::ReservationCompleted { reservation_id, .. }
        | ReservationSagaEvent::ReservationExpired { reservation_id, .. }
        | ReservationSagaEvent::ReservationCancelled { reservation_id, .. }
        | ReservationSagaEvent::ReservationCompensated { reservation_id, .. }
        | ReservationSagaEvent::InventoryReservationFailed { reservation_id, .. } => {
            *reservation_id
        },
    }
}

/// Update the customer-facing `reservations_projection` read model for one saga
/// event, running on the supplied transaction connection (so it commits atomically
/// with the event append).
async fn update_reservations_projection_tx(
    conn: &mut sqlx::PgConnection,
    event: &ReservationSagaEvent,
) -> Result<(), ProjectionError> {
    let db_err = |e: sqlx::Error| ProjectionError::Database(e.to_string());

    match event {
        ReservationSagaEvent::ReservationInitiated {
            reservation_id,
            event_id,
            customer_id,
            section,
            quantity,
            expires_at,
            initiated_at,
        } => {
            sqlx::query(
                r"
                INSERT INTO reservations_projection (
                    id, event_id, customer_id, section, quantity,
                    status, total_amount_cents, expires_at, created_at
                )
                VALUES ($1, $2, $3, $4, $5, 'initiated', 0, $6, $7)
                ON CONFLICT (id) DO NOTHING
                ",
            )
            .bind(reservation_id.as_uuid())
            .bind(event_id.as_uuid())
            .bind(customer_id.as_uuid())
            .bind(section)
            .bind(i32::try_from(*quantity).unwrap_or(i32::MAX))
            .bind(expires_at)
            .bind(initiated_at)
            .execute(&mut *conn)
            .await
            .map_err(db_err)?;
        },
        ReservationSagaEvent::SeatsAllocated {
            reservation_id,
            total_amount,
            ..
        } => {
            sqlx::query(
                r"
                UPDATE reservations_projection
                SET status = 'seats_reserved', total_amount_cents = $2
                WHERE id = $1
                ",
            )
            .bind(reservation_id.as_uuid())
            .bind(i64::try_from(total_amount.cents()).unwrap_or(i64::MAX))
            .execute(&mut *conn)
            .await
            .map_err(db_err)?;
        },
        ReservationSagaEvent::PaymentRequested { reservation_id, .. } => {
            set_reservation_status(conn, *reservation_id, "payment_pending").await?;
        },
        ReservationSagaEvent::PaymentSucceeded { reservation_id, .. } => {
            set_reservation_status(conn, *reservation_id, "payment_completed").await?;
        },
        ReservationSagaEvent::PaymentFailed { reservation_id, .. }
        | ReservationSagaEvent::ReservationCancelled { reservation_id, .. }
        | ReservationSagaEvent::ReservationCompensated { reservation_id, .. }
        | ReservationSagaEvent::InventoryReservationFailed { reservation_id, .. } => {
            set_reservation_status(conn, *reservation_id, "cancelled").await?;
        },
        ReservationSagaEvent::ReservationExpired { reservation_id, .. } => {
            set_reservation_status(conn, *reservation_id, "expired").await?;
        },
        ReservationSagaEvent::ReservationCompleted {
            reservation_id,
            completed_at,
            ..
        } => {
            sqlx::query(
                r"
                UPDATE reservations_projection
                SET status = 'completed', completed_at = $2
                WHERE id = $1
                ",
            )
            .bind(reservation_id.as_uuid())
            .bind(completed_at)
            .execute(&mut *conn)
            .await
            .map_err(db_err)?;
        },
    }

    Ok(())
}

/// Set the `reservations_projection.status` for a reservation on a tx connection.
async fn set_reservation_status(
    conn: &mut sqlx::PgConnection,
    reservation_id: ReservationId,
    status: &str,
) -> Result<(), ProjectionError> {
    sqlx::query("UPDATE reservations_projection SET status = $2 WHERE id = $1")
        .bind(reservation_id.as_uuid())
        .bind(status)
        .execute(&mut *conn)
        .await
        .map_err(|e| ProjectionError::Database(e.to_string()))?;
    Ok(())
}

/// Transactional projector for the Reservation saga's durable state.
///
/// Runs inside the saga's event-append transaction (via
/// [`PostgresEventStore::append_with_projection`]): it folds the just-appended
/// events into the full [`ReservationSagaState`], upserts the authoritative
/// `saga_state` row (`version` == stream version), and updates the customer-facing
/// `reservations_projection` — all committed atomically with the events. Because
/// `saga_state` is written in the same transaction, it can never drift from the
/// event stream and is a trustworthy resume source.
#[derive(Clone, Default)]
pub struct PgReservationSagaStateProjector {
    logic: ReservationSagaLogic,
}

impl PgReservationSagaStateProjector {
    /// Create a new transactional saga-state projector.
    #[must_use]
    pub fn new() -> Self {
        Self {
            logic: ReservationSagaLogic,
        }
    }
}

impl PgTransactionalProjector for PgReservationSagaStateProjector {
    async fn project_in_tx<'a>(
        &'a self,
        conn: &'a mut sqlx::PgConnection,
        final_version: Version,
        events: &'a [SerializedEvent],
    ) -> Result<(), ProjectionError> {
        // Decode the just-appended events (all share one saga stream).
        let mut decoded = Vec::with_capacity(events.len());
        for event in events {
            let saga_event: ReservationSagaEvent =
                bincode::deserialize(&event.payload).map_err(|e| {
                    ProjectionError::Deserialization(format!("reservation saga event: {e}"))
                })?;
            decoded.push(saga_event);
        }
        let Some(first) = decoded.first() else {
            return Ok(());
        };
        let reservation_id = reservation_saga_id_of(first);

        // Load the prior authoritative state (locked) and fold the new events onto it.
        let prior: Option<(serde_json::Value, i16)> = sqlx::query_as(
            "SELECT state, state_version FROM saga_state WHERE reservation_id = $1 FOR UPDATE",
        )
        .bind(reservation_id.as_uuid())
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| ProjectionError::Database(e.to_string()))?;

        let mut state: ReservationSagaState = match prior {
            Some((json, stored_version)) => {
                if stored_version != RESERVATION_SAGA_STATE_VERSION {
                    return Err(ProjectionError::Custom(format!(
                        "saga_state.state_version {stored_version} != expected \
                         {RESERVATION_SAGA_STATE_VERSION}; ReservationSagaState shape changed — \
                         run a state migration before deploying"
                    )));
                }
                serde_json::from_value(json)
                    .map_err(|e| ProjectionError::Deserialization(format!("saga_state: {e}")))?
            },
            None => ReservationSagaState::default(),
        };
        for saga_event in &decoded {
            self.logic.apply(&mut state, saga_event);
        }

        // Upsert the authoritative row (monotonic: ignore stale replays).
        let state_json = serde_json::to_value(&state)
            .map_err(|e| ProjectionError::Custom(format!("serialize saga_state: {e}")))?;
        let phase = serde_json::to_value(&state.phase)
            .ok()
            .and_then(|v| v.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| format!("{:?}", state.phase));
        let version_i64 = i64::try_from(final_version.as_u64()).unwrap_or(i64::MAX);

        sqlx::query(
            r"
            INSERT INTO saga_state
                (reservation_id, version, phase, expires_at, state, state_version, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, now())
            ON CONFLICT (reservation_id) DO UPDATE
                SET version = EXCLUDED.version,
                    phase = EXCLUDED.phase,
                    expires_at = EXCLUDED.expires_at,
                    state = EXCLUDED.state,
                    state_version = EXCLUDED.state_version,
                    updated_at = now()
            WHERE saga_state.version < EXCLUDED.version
            ",
        )
        .bind(reservation_id.as_uuid())
        .bind(version_i64)
        .bind(&phase)
        .bind(state.expires_at)
        .bind(&state_json)
        .bind(RESERVATION_SAGA_STATE_VERSION)
        .execute(&mut *conn)
        .await
        .map_err(|e| ProjectionError::Database(e.to_string()))?;

        // Update the customer-facing read model in the same transaction.
        for saga_event in &decoded {
            update_reservations_projection_tx(&mut *conn, saga_event).await?;
        }

        Ok(())
    }
}

/// Adapts a [`PostgresEventStore`] + [`PgReservationSagaStateProjector`] into the
/// framework's [`DynAtomicPersist`] seam, so the saga's `Handler` appends events
/// and updates `saga_state` in a single transaction.
#[derive(Clone)]
pub struct PgAtomicPersist {
    event_store: PostgresEventStore,
    projector: PgReservationSagaStateProjector,
}

impl PgAtomicPersist {
    /// Create a new atomic-persist adapter over the given event store.
    #[must_use]
    pub fn new(event_store: PostgresEventStore) -> Self {
        Self {
            event_store,
            projector: PgReservationSagaStateProjector::new(),
        }
    }
}

impl DynAtomicPersist for PgAtomicPersist {
    fn append_and_project<'a>(
        &'a self,
        stream_id: &'a StreamId,
        expected_version: Option<Version>,
        events: Vec<SerializedEvent>,
    ) -> futures::future::BoxFuture<'a, Result<Version, AtomicError>> {
        Box::pin(async move {
            self.event_store
                .append_with_projection(stream_id, expected_version, events, &self.projector)
                .await
        })
    }
}
