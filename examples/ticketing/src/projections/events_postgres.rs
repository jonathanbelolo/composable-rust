//! PostgreSQL-backed events projection using JSONB.
//!
//! Stores full `Event` domain objects as JSONB for simplicity.
//! Much simpler than denormalized schema - just serialize/deserialize.

use crate::aggregates::event::EventProjectionQuery;
use crate::aggregates::EventAction;
use crate::projections::TicketingEvent;
use crate::types::{Event, EventId, EventStatus};
use composable_rust_core::projection::{Projection, ProjectionError};
use sqlx::PgPool;
use std::sync::Arc;

/// PostgreSQL-backed events projection.
///
/// Stores events as JSONB in the `events_projection` table.
pub struct PostgresEventsProjection {
    pool: Arc<PgPool>,
}

impl PostgresEventsProjection {
    /// Creates a new `PostgresEventsProjection`.
    #[must_use]
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    /// Get an event by ID.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get(&self, event_id: &uuid::Uuid) -> Result<Option<Event>, sqlx::Error> {
        let result: Option<(sqlx::types::JsonValue,)> = sqlx::query_as(
            "SELECT data FROM events_projection WHERE id = $1"
        )
        .bind(event_id)
        .fetch_optional(&*self.pool)
        .await?;

        match result {
            Some((json,)) => {
                let event: Event = serde_json::from_value(json)
                    .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
                Ok(Some(event))
            }
            None => Ok(None),
        }
    }

    /// List all events with optional status filter.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn list(
        &self,
        status_filter: Option<&str>,
    ) -> Result<Vec<Event>, sqlx::Error> {
        let rows: Vec<(sqlx::types::JsonValue,)> = if let Some(status) = status_filter {
            sqlx::query_as(
                "SELECT data FROM events_projection
                 WHERE data->>'status' = $1
                 ORDER BY created_at DESC"
            )
            .bind(status)
            .fetch_all(&*self.pool)
            .await?
        } else {
            sqlx::query_as(
                "SELECT data FROM events_projection
                 ORDER BY created_at DESC"
            )
            .fetch_all(&*self.pool)
            .await?
        };

        let events: Result<Vec<Event>, serde_json::Error> = rows
            .into_iter()
            .map(|(json,)| serde_json::from_value(json))
            .collect();

        events.map_err(|e| sqlx::Error::Decode(Box::new(e)))
    }
}

impl Projection for PostgresEventsProjection {
    type Event = TicketingEvent;

    fn name(&self) -> &str {
        "events"
    }

    #[tracing::instrument(skip(self, event), fields(projection = "events"))]
    async fn apply_event(&self, event: &Self::Event) -> composable_rust_core::projection::Result<()> {
        if let TicketingEvent::Event(event_action) = event {
            match event_action {
                EventAction::EventCreated {
                    id,
                    name,
                    owner_id,
                    venue,
                    date,
                    pricing_tiers,
                    created_at,
                } => {
                    let domain_event = Event::new(
                        *id,
                        name.clone(),
                        *owner_id,
                        venue.clone(),
                        *date,
                        pricing_tiers.clone(),
                        *created_at,
                    );

                    let json = serde_json::to_value(&domain_event)
                        .map_err(|e| ProjectionError::Serialization(e.to_string()))?;

                    sqlx::query(
                        "INSERT INTO events_projection (id, owner_id, data, created_at, updated_at)
                         VALUES ($1, $2, $3, $4, $5)
                         ON CONFLICT (id) DO UPDATE SET
                            owner_id = EXCLUDED.owner_id,
                            data = EXCLUDED.data,
                            updated_at = EXCLUDED.updated_at"
                    )
                    .bind(id.as_uuid())
                    .bind(&owner_id.0)
                    .bind(&json)
                    .bind(created_at)
                    .bind(chrono::Utc::now())
                    .execute(&*self.pool)
                    .await
                    .map_err(|e| ProjectionError::Storage(e.to_string()))?;
                }
                EventAction::EventPublished { event_id, .. }
                | EventAction::SalesOpened { event_id, .. }
                | EventAction::SalesClosed { event_id, .. }
                | EventAction::EventCancelled { event_id, .. } => {
                    // Update the status field in the JSONB
                    let new_status = match event_action {
                        EventAction::EventPublished { .. } => "Published",
                        EventAction::SalesOpened { .. } => "SalesOpen",
                        EventAction::SalesClosed { .. } => "SalesClosed",
                        EventAction::EventCancelled { .. } => "Cancelled",
                        _ => return Ok(()),
                    };

                    sqlx::query(
                        "UPDATE events_projection
                         SET data = jsonb_set(data, '{status}', $2::jsonb, false),
                             updated_at = $3
                         WHERE id = $1"
                    )
                    .bind(event_id.as_uuid())
                    .bind(format!("\"{}\"", new_status))
                    .bind(chrono::Utc::now())
                    .execute(&*self.pool)
                    .await
                    .map_err(|e| ProjectionError::Storage(e.to_string()))?;
                }
                EventAction::EventUpdated { event_id, name, .. } => {
                    // Update the name field in the JSONB if provided
                    if let Some(new_name) = name {
                        sqlx::query(
                            "UPDATE events_projection
                             SET data = jsonb_set(data, '{name}', $2::jsonb, false),
                                 updated_at = $3
                             WHERE id = $1"
                        )
                        .bind(event_id.as_uuid())
                        .bind(serde_json::to_string(new_name)
                            .map_err(|e| ProjectionError::Serialization(e.to_string()))?)
                        .bind(chrono::Utc::now())
                        .execute(&*self.pool)
                        .await
                        .map_err(|e| ProjectionError::Storage(e.to_string()))?;
                    }
                }
                _ => {
                    // Ignore commands and validation failures
                }
            }
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl EventProjectionQuery for PostgresEventsProjection {
    async fn load_event(&self, event_id: &EventId) -> Result<Option<Event>, String> {
        self.get(event_id.as_uuid())
            .await
            .map_err(|e| format!("Failed to load event: {e}"))
    }

    async fn load_events(&self, status_filter: Option<EventStatus>) -> Result<Vec<Event>, String> {
        // Convert EventStatus to string representation (e.g., "Draft", "Published")
        let status_str = status_filter.as_ref().map(|s| format!("{s:?}"));
        self.list(status_str.as_deref())
            .await
            .map_err(|e| format!("Failed to load events: {e}"))
    }
}
