//! Projector implementation for event read models.
//!
//! This module provides [`EventProjector`] which updates the read model
//! when events are persisted, providing synchronous projection completion.

use composable_rust_next::{ProjectionError, Projector, SerializedEvent};
use sqlx::PgPool;
use tracing::instrument;

use super::EventEvent;

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
                    INSERT INTO events (
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
            }

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
                    "UPDATE events SET {} WHERE event_id = $1",
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
            }

            EventEvent::Published {
                event_id,
                published_at,
            } => {
                sqlx::query(
                    r"
                    UPDATE events
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
            }

            EventEvent::Cancelled {
                event_id,
                reason,
                cancelled_at,
            } => {
                sqlx::query(
                    r"
                    UPDATE events
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
            }
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
