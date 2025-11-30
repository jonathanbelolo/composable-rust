//! Projection queries and query fetchers for CQRS read models.
//!
//! This module provides:
//! - [`EventProjectionQueries`]: Database queries for reading event projections
//! - [`EventQueryFetcher`]: Pre-fetches projection data for query commands
//!
//! # Architecture: Handler Pre-Fetches Query Data
//!
//! ```text
//! Handler.handle(input)
//!      │
//!      ├─► EventQueryFetcher.fetch(input, projections)
//!      │         │
//!      │         └─► EventProjectionQueries.get_event() / .list_by_owner()
//!      │                    │
//!      │                    └─► PostgreSQL projections table
//!      │
//!      ├─► BusinessLogic.process(state, prepared_input) ──► Pure logic
//!      │
//!      └─► HandleResult::Query(response)
//! ```
//!
//! The Handler (not HTTP handler) does all I/O. This keeps:
//! - Business logic pure and testable
//! - All I/O in the Handler (single point of control)
//! - HTTP handlers thin (just request/response translation)

use std::future::Future;

use composable_rust_auth::state::UserId;
use composable_rust_next::{FetchResult, ProjectionQueries, QueryFetcher};
use sqlx::PgPool;

use super::event::{EventCommand, EventDto};
use crate::types::{EventDate, EventId, EventStatus, PricingTier, Venue};

// ═══════════════════════════════════════════════════════════════════════════
// PostgreSQL Implementation
// ═══════════════════════════════════════════════════════════════════════════

/// PostgreSQL-backed projection queries for events.
///
/// This implementation reads from the `events` table in the projections
/// database, which is updated by the [`EventProjector`](super::EventProjector).
///
/// # Example
///
/// ```ignore
/// let queries = EventProjectionQueries::new(pool);
///
/// // Get a single event
/// let event = queries.get_event(event_id).await?;
///
/// // List events by owner
/// let events = queries.list_by_owner(user_id).await?;
/// ```
#[derive(Clone)]
pub struct EventProjectionQueries {
    pool: PgPool,
}

impl EventProjectionQueries {
    /// Create a new projection queries instance.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get an event by ID.
    ///
    /// Returns `None` if the event doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_event(&self, event_id: EventId) -> Result<Option<EventDto>, sqlx::Error> {
        let row: Option<EventRow> = sqlx::query_as(
            r"
            SELECT
                event_id, name, owner_id, venue, event_date,
                pricing_tiers, status
            FROM events
            WHERE event_id = $1
            ",
        )
        .bind(event_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;

        row.map(TryInto::try_into).transpose()
    }

    /// List all events owned by a user.
    ///
    /// Results are ordered by event date descending (most recent first).
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn list_by_owner(&self, user_id: UserId) -> Result<Vec<EventDto>, sqlx::Error> {
        let rows: Vec<EventRow> = sqlx::query_as(
            r"
            SELECT
                event_id, name, owner_id, venue, event_date,
                pricing_tiers, status
            FROM events
            WHERE owner_id = $1
            ORDER BY event_date DESC
            ",
        )
        .bind(user_id.0)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    /// List all events with optional status filter and pagination.
    ///
    /// Results are ordered by event date descending (most recent first).
    ///
    /// # Arguments
    ///
    /// * `status_filter` - Optional status to filter by
    /// * `page` - Page number (0-indexed)
    /// * `page_size` - Number of results per page
    ///
    /// # Returns
    ///
    /// A tuple of (events, total_count) where total_count is the total number
    /// of matching events (before pagination).
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn list_all(
        &self,
        status_filter: Option<EventStatus>,
        page: usize,
        page_size: usize,
    ) -> Result<(Vec<EventDto>, usize), sqlx::Error> {
        let offset = page * page_size;

        // Get total count
        let total: i64 = if let Some(status) = status_filter {
            let status_str = status_to_string(status);
            sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM events WHERE status = $1")
                .bind(status_str)
                .fetch_one(&self.pool)
                .await?
        } else {
            sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM events")
                .fetch_one(&self.pool)
                .await?
        };

        // Get paginated results
        #[allow(clippy::cast_possible_wrap)]
        let rows: Vec<EventRow> = if let Some(status) = status_filter {
            let status_str = status_to_string(status);
            sqlx::query_as(
                r"
                SELECT
                    event_id, name, owner_id, venue, event_date,
                    pricing_tiers, status
                FROM events
                WHERE status = $1
                ORDER BY event_date DESC
                LIMIT $2 OFFSET $3
                ",
            )
            .bind(status_str)
            .bind(page_size as i64)
            .bind(offset as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as(
                r"
                SELECT
                    event_id, name, owner_id, venue, event_date,
                    pricing_tiers, status
                FROM events
                ORDER BY event_date DESC
                LIMIT $1 OFFSET $2
                ",
            )
            .bind(page_size as i64)
            .bind(offset as i64)
            .fetch_all(&self.pool)
            .await?
        };

        let events = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        Ok((events, total as usize))
    }
}

/// Convert `EventStatus` to database string representation.
const fn status_to_string(status: EventStatus) -> &'static str {
    match status {
        EventStatus::Draft => "draft",
        EventStatus::Published => "published",
        EventStatus::Cancelled => "cancelled",
        EventStatus::SalesOpen => "sales_open",
        EventStatus::SalesClosed => "sales_closed",
        EventStatus::Completed => "completed",
    }
}

impl ProjectionQueries for EventProjectionQueries {
    type Error = sqlx::Error;
}

// ═══════════════════════════════════════════════════════════════════════════
// Database Row Type
// ═══════════════════════════════════════════════════════════════════════════

/// Raw database row from the events projection table.
#[derive(Debug, sqlx::FromRow)]
struct EventRow {
    event_id: uuid::Uuid,
    name: String,
    owner_id: uuid::Uuid,
    venue: serde_json::Value,
    event_date: chrono::DateTime<chrono::Utc>,
    pricing_tiers: serde_json::Value,
    status: String,
}

impl TryFrom<EventRow> for EventDto {
    type Error = sqlx::Error;

    fn try_from(row: EventRow) -> Result<Self, Self::Error> {
        let venue: Venue = serde_json::from_value(row.venue).map_err(|e| {
            sqlx::Error::Decode(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to deserialize venue: {e}"),
            )))
        })?;

        let pricing_tiers: Vec<PricingTier> =
            serde_json::from_value(row.pricing_tiers).map_err(|e| {
                sqlx::Error::Decode(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("failed to deserialize pricing_tiers: {e}"),
                )))
            })?;

        let status = match row.status.as_str() {
            "draft" => EventStatus::Draft,
            "published" => EventStatus::Published,
            "cancelled" => EventStatus::Cancelled,
            _ => EventStatus::Draft,
        };

        Ok(Self {
            id: EventId::from_uuid(row.event_id),
            name: row.name,
            owner_id: UserId(row.owner_id),
            venue,
            date: EventDate::new(row.event_date),
            status,
            pricing_tiers,
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Query Fetcher Implementation
// ═══════════════════════════════════════════════════════════════════════════

/// Query fetcher for Event aggregate commands.
///
/// This implementation detects query commands (`GetEvent`, `ListMyEvents`) and
/// pre-fetches the necessary data from [`EventProjectionQueries`] before the
/// Handler calls `process()`.
///
/// For non-query commands (Create, Publish, Cancel), the input passes through
/// unchanged.
///
/// # Example
///
/// ```ignore
/// let projections = EventProjectionQueries::new(pool);
/// let fetcher = EventQueryFetcher;
///
/// let handler = Handler::new(
///     EventBusinessLogic,
///     NoOpCallExecutor,
///     fetcher,
///     env,
/// );
///
/// // GetEvent command will have `fetched` populated by the Handler
/// let result = handler.handle(EventCommand::GetEvent {
///     event_id,
///     requesting_user_id: user_id,
///     fetched: None,  // Handler will populate this
/// }).await?;
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct EventQueryFetcher;

impl QueryFetcher<EventCommand, EventProjectionQueries> for EventQueryFetcher {
    type Error = sqlx::Error;

    async fn fetch(
        &self,
        input: EventCommand,
        projections: &EventProjectionQueries,
    ) -> Result<FetchResult<EventCommand>, Self::Error> {
        match input {
            EventCommand::GetEvent {
                event_id,
                requesting_user_id,
                fetched: _,
            } => {
                // Fetch event from projections
                let fetched = projections.get_event(event_id).await?;
                Ok(FetchResult::new_entity(EventCommand::GetEvent {
                    event_id,
                    requesting_user_id,
                    fetched,
                }))
            }

            EventCommand::ListMyEvents { user_id, fetched: _ } => {
                // Fetch all events owned by this user
                let fetched = projections.list_by_owner(user_id).await?;
                Ok(FetchResult::new_entity(EventCommand::ListMyEvents { user_id, fetched }))
            }

            EventCommand::ListEvents {
                status_filter,
                page,
                page_size,
                fetched: _,
            } => {
                // Fetch paginated events with optional filter
                let fetched = projections
                    .list_all(status_filter, page, page_size)
                    .await?;
                Ok(FetchResult::new_entity(EventCommand::ListEvents {
                    status_filter,
                    page,
                    page_size,
                    fetched,
                }))
            }

            EventCommand::GetEventPricing {
                event_id,
                fetched: _,
            } => {
                // Fetch event (which includes pricing tiers)
                let fetched = projections.get_event(event_id).await?;
                Ok(FetchResult::new_entity(EventCommand::GetEventPricing { event_id, fetched }))
            }

            // ═══════════════════════════════════════════════════════════════
            // Write commands that need projection data for authorization
            // ═══════════════════════════════════════════════════════════════

            EventCommand::Update {
                event_id,
                requesting_user_id,
                name,
                venue,
                date,
                fetched: _,
            } => {
                let fetched = projections.get_event(event_id).await?;
                Ok(FetchResult::new_entity(EventCommand::Update {
                    event_id,
                    requesting_user_id,
                    name,
                    venue,
                    date,
                    fetched,
                }))
            }

            EventCommand::Publish { event_id, fetched: _ } => {
                let fetched = projections.get_event(event_id).await?;
                Ok(FetchResult::new_entity(EventCommand::Publish { event_id, fetched }))
            }

            EventCommand::Cancel {
                event_id,
                reason,
                fetched: _,
            } => {
                let fetched = projections.get_event(event_id).await?;
                Ok(FetchResult::new_entity(EventCommand::Cancel {
                    event_id,
                    reason,
                    fetched,
                }))
            }

            EventCommand::UpdatePricing {
                event_id,
                requesting_user_id,
                pricing_tiers,
                fetched: _,
            } => {
                let fetched = projections.get_event(event_id).await?;
                Ok(FetchResult::new_entity(EventCommand::UpdatePricing {
                    event_id,
                    requesting_user_id,
                    pricing_tiers,
                    fetched,
                }))
            }

            EventCommand::AddVenueSections {
                event_id,
                requesting_user_id,
                sections,
                fetched: _,
            } => {
                let fetched = projections.get_event(event_id).await?;
                Ok(FetchResult::new_entity(EventCommand::AddVenueSections {
                    event_id,
                    requesting_user_id,
                    sections,
                    fetched,
                }))
            }

            EventCommand::Delete {
                event_id,
                requesting_user_id,
                fetched: _,
            } => {
                let fetched = projections.get_event(event_id).await?;
                Ok(FetchResult::new_entity(EventCommand::Delete {
                    event_id,
                    requesting_user_id,
                    fetched,
                }))
            }

            // Create doesn't need fetched data - it's creating a new entity
            EventCommand::Create { .. } => Ok(FetchResult::new_entity(input)),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// In-Memory Implementation (for testing)
// ═══════════════════════════════════════════════════════════════════════════

/// In-memory implementation of event projection queries for testing.
///
/// This allows tests to pre-populate projection data without a database.
#[derive(Debug, Default)]
pub struct InMemoryEventProjectionQueries {
    events: std::sync::RwLock<std::collections::HashMap<EventId, EventDto>>,
}

impl InMemoryEventProjectionQueries {
    /// Create a new empty in-memory store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an event into the store.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn insert(&self, event: EventDto) {
        self.events
            .write()
            .expect("lock poisoned")
            .insert(event.id, event);
    }

    /// Get an event by ID.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    #[must_use]
    pub fn get_event(&self, event_id: EventId) -> Option<EventDto> {
        self.events
            .read()
            .expect("lock poisoned")
            .get(&event_id)
            .cloned()
    }

    /// List all events owned by a user.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    #[must_use]
    pub fn list_by_owner(&self, user_id: UserId) -> Vec<EventDto> {
        self.events
            .read()
            .expect("lock poisoned")
            .values()
            .filter(|e| e.owner_id == user_id)
            .cloned()
            .collect()
    }

    /// List all events with optional status filter and pagination.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    #[must_use]
    pub fn list_all(
        &self,
        status_filter: Option<EventStatus>,
        page: usize,
        page_size: usize,
    ) -> (Vec<EventDto>, usize) {
        let events = self.events.read().expect("lock poisoned");

        // Filter by status if provided
        let filtered: Vec<EventDto> = if let Some(status) = status_filter {
            events
                .values()
                .filter(|e| e.status == status)
                .cloned()
                .collect()
        } else {
            events.values().cloned().collect()
        };

        let total = filtered.len();

        // Paginate
        let start = page * page_size;
        let paginated: Vec<EventDto> = filtered
            .into_iter()
            .skip(start)
            .take(page_size)
            .collect();

        (paginated, total)
    }
}

impl ProjectionQueries for InMemoryEventProjectionQueries {
    type Error = std::convert::Infallible;
}

/// In-memory query fetcher for testing.
///
/// Uses [`InMemoryEventProjectionQueries`] to fetch data.
#[derive(Debug, Clone)]
pub struct InMemoryEventQueryFetcher {
    projections: std::sync::Arc<InMemoryEventProjectionQueries>,
}

impl InMemoryEventQueryFetcher {
    /// Create a new in-memory query fetcher.
    #[must_use]
    pub fn new(projections: std::sync::Arc<InMemoryEventProjectionQueries>) -> Self {
        Self { projections }
    }
}

impl<P: ProjectionQueries> QueryFetcher<EventCommand, P> for InMemoryEventQueryFetcher {
    type Error = std::convert::Infallible;

    fn fetch(
        &self,
        input: EventCommand,
        _projections: &P,
    ) -> impl Future<Output = Result<FetchResult<EventCommand>, Self::Error>> + Send {
        let projections = self.projections.clone();
        async move {
            match input {
                EventCommand::GetEvent {
                    event_id,
                    requesting_user_id,
                    fetched: _,
                } => {
                    let fetched = projections.get_event(event_id);
                    Ok(FetchResult::new_entity(EventCommand::GetEvent {
                        event_id,
                        requesting_user_id,
                        fetched,
                    }))
                }

                EventCommand::ListMyEvents { user_id, fetched: _ } => {
                    let fetched = projections.list_by_owner(user_id);
                    Ok(FetchResult::new_entity(EventCommand::ListMyEvents { user_id, fetched }))
                }

                EventCommand::ListEvents {
                    status_filter,
                    page,
                    page_size,
                    fetched: _,
                } => {
                    let fetched = projections.list_all(status_filter, page, page_size);
                    Ok(FetchResult::new_entity(EventCommand::ListEvents {
                        status_filter,
                        page,
                        page_size,
                        fetched,
                    }))
                }

                EventCommand::GetEventPricing {
                    event_id,
                    fetched: _,
                } => {
                    let fetched = projections.get_event(event_id);
                    Ok(FetchResult::new_entity(EventCommand::GetEventPricing { event_id, fetched }))
                }

                other => Ok(FetchResult::new_entity(other)),
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Inventory Projection Queries
// ═══════════════════════════════════════════════════════════════════════════

use super::inventory::{InventoryCommand, SectionAvailabilityDto};

/// PostgreSQL-backed projection queries for inventory/availability.
///
/// Reads from the `inventory` table populated by [`InventoryProjector`].
#[derive(Clone)]
pub struct InventoryProjectionQueries {
    pool: PgPool,
}

impl InventoryProjectionQueries {
    /// Create a new inventory projection queries instance.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get availability for a specific section.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_section_availability(
        &self,
        event_id: crate::types::EventId,
        section: &str,
    ) -> Result<Option<SectionAvailabilityDto>, sqlx::Error> {
        let row: Option<InventoryRow> = sqlx::query_as(
            r"
            SELECT event_id, section, total_capacity, available_seats, reserved_seats
            FROM inventory
            WHERE event_id = $1 AND section = $2
            ",
        )
        .bind(event_id.as_uuid())
        .bind(section)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    /// Get availability for all sections of an event.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_event_availability(
        &self,
        event_id: crate::types::EventId,
    ) -> Result<Vec<SectionAvailabilityDto>, sqlx::Error> {
        let rows: Vec<InventoryRow> = sqlx::query_as(
            r"
            SELECT event_id, section, total_capacity, available_seats, reserved_seats
            FROM inventory
            WHERE event_id = $1
            ORDER BY section
            ",
        )
        .bind(event_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Get total available seats across all sections for an event.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_total_available(
        &self,
        event_id: crate::types::EventId,
    ) -> Result<u32, sqlx::Error> {
        let total: Option<i64> = sqlx::query_scalar(
            r"
            SELECT COALESCE(SUM(available_seats), 0)
            FROM inventory
            WHERE event_id = $1
            ",
        )
        .bind(event_id.as_uuid())
        .fetch_one(&self.pool)
        .await?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(total.unwrap_or(0) as u32)
    }

    /// Get inventory DTO for write command validation.
    ///
    /// Returns inventory state including available seat IDs for reservation.
    /// Returns `None` if inventory hasn't been initialized for this section.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_inventory_dto(
        &self,
        event_id: crate::types::EventId,
        section: &str,
    ) -> Result<Option<super::inventory::InventoryDto>, sqlx::Error> {
        // First check if inventory exists
        let row: Option<InventoryRow> = sqlx::query_as(
            r"
            SELECT event_id, section, total_capacity, available_seats, reserved_seats
            FROM inventory
            WHERE event_id = $1 AND section = $2
            ",
        )
        .bind(event_id.as_uuid())
        .bind(section)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        // Fetch available seat IDs
        let seat_ids: Vec<uuid::Uuid> = sqlx::query_scalar(
            r"
            SELECT seat_id FROM seats
            WHERE event_id = $1 AND section = $2 AND status = 'available'
            ORDER BY seat_id
            ",
        )
        .bind(event_id.as_uuid())
        .bind(section)
        .fetch_all(&self.pool)
        .await?;

        #[allow(clippy::cast_sign_loss)]
        Ok(Some(super::inventory::InventoryDto {
            initialized: true,
            capacity: row.total_capacity as u32,
            available_count: row.available_seats as u32,
            available_seats: seat_ids
                .into_iter()
                .map(crate::types::SeatId::from_uuid)
                .collect(),
        }))
    }

    /// Get reservation DTO for Confirm/Release command validation.
    ///
    /// Returns seat IDs and expiration for the given reservation.
    /// Returns `None` if reservation doesn't exist or isn't active.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_reservation_dto(
        &self,
        reservation_id: crate::types::ReservationId,
    ) -> Result<Option<super::inventory::ReservationDto>, sqlx::Error> {
        // Get reservation expiration
        let reservation: Option<(chrono::DateTime<chrono::Utc>,)> = sqlx::query_as(
            r"
            SELECT expires_at FROM reservations
            WHERE reservation_id = $1 AND status = 'active'
            ",
        )
        .bind(reservation_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;

        let Some((expires_at,)) = reservation else {
            return Ok(None);
        };

        // Get seat IDs for this reservation
        let seat_ids: Vec<uuid::Uuid> = sqlx::query_scalar(
            r"
            SELECT seat_id FROM seats
            WHERE reservation_id = $1 AND status = 'reserved'
            ORDER BY seat_id
            ",
        )
        .bind(reservation_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(Some(super::inventory::ReservationDto {
            seats: seat_ids
                .into_iter()
                .map(crate::types::SeatId::from_uuid)
                .collect(),
            expires_at,
        }))
    }
}

impl ProjectionQueries for InventoryProjectionQueries {
    type Error = sqlx::Error;
}

/// Raw database row from the inventory projection table.
#[derive(Debug, sqlx::FromRow)]
struct InventoryRow {
    event_id: uuid::Uuid,
    section: String,
    total_capacity: i32,
    available_seats: i32,
    reserved_seats: i32,
}

impl From<InventoryRow> for SectionAvailabilityDto {
    fn from(row: InventoryRow) -> Self {
        #[allow(clippy::cast_sign_loss)]
        Self {
            event_id: crate::types::EventId::from_uuid(row.event_id),
            section: row.section,
            total_capacity: row.total_capacity as u32,
            available_seats: row.available_seats as u32,
            reserved_seats: row.reserved_seats as u32,
            sold_seats: (row.total_capacity - row.available_seats - row.reserved_seats) as u32,
        }
    }
}

/// Query fetcher for Inventory aggregate commands.
///
/// This fetcher handles BOTH read and write commands:
/// - Query commands: Populates `fetched` with read data
/// - Write commands: Populates `fetched` with validation data
///
/// The Handler calls `fetch()` before `BusinessLogic.process()`, ensuring
/// all commands have the data they need for validation.
#[derive(Debug, Clone, Copy, Default)]
pub struct InventoryQueryFetcher;

impl QueryFetcher<InventoryCommand, InventoryProjectionQueries> for InventoryQueryFetcher {
    type Error = sqlx::Error;

    async fn fetch(
        &self,
        input: InventoryCommand,
        projections: &InventoryProjectionQueries,
    ) -> Result<FetchResult<InventoryCommand>, Self::Error> {
        match input {
            // ═══════════════════════════════════════════════════════════════
            // Write Commands - need validation data
            // ═══════════════════════════════════════════════════════════════

            InventoryCommand::Initialize {
                event_id,
                section,
                capacity,
                fetched: _,
            } => {
                // Fetch existing inventory to check if already initialized
                let fetched = projections.get_inventory_dto(event_id, &section).await?;
                Ok(FetchResult::new_entity(InventoryCommand::Initialize {
                    event_id,
                    section,
                    capacity,
                    fetched,
                }))
            }

            InventoryCommand::Reserve {
                reservation_id,
                event_id,
                section,
                quantity,
                expires_at,
                fetched: _,
            } => {
                // Fetch inventory state for availability check
                let fetched = projections.get_inventory_dto(event_id, &section).await?;
                Ok(FetchResult::new_entity(InventoryCommand::Reserve {
                    reservation_id,
                    event_id,
                    section,
                    quantity,
                    expires_at,
                    fetched,
                }))
            }

            InventoryCommand::Confirm {
                reservation_id,
                customer_id,
                fetched: _,
            } => {
                // Fetch reservation data for validation
                let fetched = projections.get_reservation_dto(reservation_id).await?;
                Ok(FetchResult::new_entity(InventoryCommand::Confirm {
                    reservation_id,
                    customer_id,
                    fetched,
                }))
            }

            InventoryCommand::Release {
                reservation_id,
                reason,
                fetched: _,
            } => {
                // Fetch reservation data for validation
                let fetched = projections.get_reservation_dto(reservation_id).await?;
                Ok(FetchResult::new_entity(InventoryCommand::Release {
                    reservation_id,
                    reason,
                    fetched,
                }))
            }

            // ═══════════════════════════════════════════════════════════════
            // Query Commands - need read data
            // ═══════════════════════════════════════════════════════════════

            InventoryCommand::GetSectionAvailability {
                event_id,
                section,
                fetched: _,
            } => {
                let fetched = projections
                    .get_section_availability(event_id, &section)
                    .await?;
                Ok(FetchResult::new_entity(InventoryCommand::GetSectionAvailability {
                    event_id,
                    section,
                    fetched,
                }))
            }

            InventoryCommand::GetEventAvailability { event_id, fetched: _ } => {
                let fetched = projections.get_event_availability(event_id).await?;
                Ok(FetchResult::new_entity(InventoryCommand::GetEventAvailability { event_id, fetched }))
            }

            InventoryCommand::GetTotalAvailable { event_id, fetched: _ } => {
                let fetched = projections.get_total_available(event_id).await?;
                Ok(FetchResult::new_entity(InventoryCommand::GetTotalAvailable { event_id, fetched }))
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Payment Projection Queries
// ═══════════════════════════════════════════════════════════════════════════

use super::payment::{PaymentCommand, PaymentDto, PaymentDtoStatus};
use crate::types::{CustomerId, Money, PaymentId, ReservationId};

/// PostgreSQL-backed projection queries for payments.
///
/// Reads from the `payments` table populated by [`PaymentProjector`].
#[derive(Clone)]
pub struct PaymentProjectionQueries {
    pool: PgPool,
}

impl PaymentProjectionQueries {
    /// Create a new payment projection queries instance.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get a single payment by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_payment(
        &self,
        payment_id: PaymentId,
    ) -> Result<Option<PaymentDto>, sqlx::Error> {
        let row: Option<PaymentRow> = sqlx::query_as(
            r"
            SELECT
                payment_id, reservation_id, customer_id, amount_cents,
                payment_method, status, transaction_id, failure_reason,
                refund_amount_cents, refund_reason
            FROM payments
            WHERE payment_id = $1
            ",
        )
        .bind(payment_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    /// List all payments for a customer.
    ///
    /// Results are ordered by creation date descending.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn list_by_customer(
        &self,
        customer_id: CustomerId,
    ) -> Result<Vec<PaymentDto>, sqlx::Error> {
        let rows: Vec<PaymentRow> = sqlx::query_as(
            r"
            SELECT
                payment_id, reservation_id, customer_id, amount_cents,
                payment_method, status, transaction_id, failure_reason,
                refund_amount_cents, refund_reason
            FROM payments
            WHERE customer_id = $1
            ORDER BY created_at DESC
            ",
        )
        .bind(customer_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }
}

impl ProjectionQueries for PaymentProjectionQueries {
    type Error = sqlx::Error;
}

/// Raw database row from the payments projection table.
#[derive(Debug, sqlx::FromRow)]
struct PaymentRow {
    payment_id: uuid::Uuid,
    reservation_id: uuid::Uuid,
    customer_id: uuid::Uuid,
    amount_cents: i64,
    payment_method: String,
    status: String,
    transaction_id: Option<String>,
    failure_reason: Option<String>,
    refund_amount_cents: Option<i64>,
    refund_reason: Option<String>,
}

impl From<PaymentRow> for PaymentDto {
    fn from(row: PaymentRow) -> Self {
        let status = match row.status.as_str() {
            "processing" => PaymentDtoStatus::Processing,
            "succeeded" => PaymentDtoStatus::Succeeded,
            "failed" => PaymentDtoStatus::Failed,
            "refunded" => PaymentDtoStatus::Refunded,
            _ => PaymentDtoStatus::Processing,
        };

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let amount = Money::from_cents(row.amount_cents as u64);

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let refund_amount = row.refund_amount_cents.map(|c| Money::from_cents(c as u64));

        Self {
            id: PaymentId::from_uuid(row.payment_id),
            reservation_id: ReservationId::from_uuid(row.reservation_id),
            customer_id: CustomerId::from_uuid(row.customer_id),
            amount,
            payment_method: row.payment_method,
            status,
            transaction_id: row.transaction_id,
            failure_reason: row.failure_reason,
            refund_amount,
            refund_reason: row.refund_reason,
        }
    }
}

/// Query fetcher for Payment aggregate commands.
#[derive(Debug, Clone, Copy, Default)]
pub struct PaymentQueryFetcher;

impl QueryFetcher<PaymentCommand, PaymentProjectionQueries> for PaymentQueryFetcher {
    type Error = sqlx::Error;

    async fn fetch(
        &self,
        input: PaymentCommand,
        projections: &PaymentProjectionQueries,
    ) -> Result<FetchResult<PaymentCommand>, Self::Error> {
        match input {
            PaymentCommand::GetPayment { payment_id, fetched: _ } => {
                let fetched = projections.get_payment(payment_id).await?;
                Ok(FetchResult::new_entity(PaymentCommand::GetPayment { payment_id, fetched }))
            }

            PaymentCommand::ListCustomerPayments { customer_id, fetched: _ } => {
                let fetched = projections.list_by_customer(customer_id).await?;
                Ok(FetchResult::new_entity(PaymentCommand::ListCustomerPayments { customer_id, fetched }))
            }

            // Non-query commands pass through unchanged
            other => Ok(FetchResult::new_entity(other)),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Analytics Projection Queries
// ═══════════════════════════════════════════════════════════════════════════

use super::analytics::{
    AnalyticsCommand, CustomerProfileDto, CustomerSpendingDto, EventSalesDto, PopularSectionsDto,
    PurchaseRecordDto, SectionPopularityDto, SectionSalesDto, TopSpendersDto, TotalRevenueDto,
};

/// PostgreSQL-backed projection queries for analytics.
///
/// Reads from the `sales_analytics_projection` and `customer_profiles` tables.
#[derive(Clone)]
pub struct AnalyticsProjectionQueries {
    pool: PgPool,
}

impl AnalyticsProjectionQueries {
    /// Create a new analytics projection queries instance.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get sales metrics for an event.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_event_sales(
        &self,
        event_id: crate::types::EventId,
    ) -> Result<Option<EventSalesDto>, sqlx::Error> {
        // Query main metrics
        let row: Option<SalesMetricsRow> = sqlx::query_as(
            r"
            SELECT total_revenue, tickets_sold, completed_reservations,
                   cancelled_reservations, average_ticket_price
            FROM sales_analytics_projection
            WHERE event_id = $1
            ",
        )
        .bind(event_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;

        let Some(metrics) = row else {
            return Ok(None);
        };

        // Query section breakdown
        let section_rows: Vec<SectionSalesRow> = sqlx::query_as(
            r"
            SELECT section, revenue, tickets_sold
            FROM sales_analytics_sections
            WHERE event_id = $1
            ",
        )
        .bind(event_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        let sections: Vec<SectionSalesDto> = section_rows
            .into_iter()
            .map(|r| SectionSalesDto {
                section: r.section,
                #[allow(clippy::cast_sign_loss)]
                revenue: crate::types::Money::from_cents(r.revenue as u64),
                #[allow(clippy::cast_sign_loss)]
                tickets_sold: r.tickets_sold as u32,
            })
            .collect();

        #[allow(clippy::cast_sign_loss)]
        Ok(Some(EventSalesDto {
            event_id,
            total_revenue: crate::types::Money::from_cents(metrics.total_revenue as u64),
            tickets_sold: metrics.tickets_sold as u32,
            completed_reservations: metrics.completed_reservations as u32,
            cancelled_reservations: metrics.cancelled_reservations as u32,
            average_ticket_price: crate::types::Money::from_cents(metrics.average_ticket_price as u64),
            sections,
        }))
    }

    /// Get most popular sections for an event.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_popular_sections(
        &self,
        event_id: crate::types::EventId,
    ) -> Result<Option<PopularSectionsDto>, sqlx::Error> {
        // Get most popular by ticket count
        let most_popular: Option<(String, i32, i64)> = sqlx::query_as(
            r"
            SELECT section, tickets_sold, revenue
            FROM sales_analytics_sections
            WHERE event_id = $1
            ORDER BY tickets_sold DESC
            LIMIT 1
            ",
        )
        .bind(event_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;

        // Get highest revenue section
        let highest_revenue: Option<(String, i32, i64)> = sqlx::query_as(
            r"
            SELECT section, tickets_sold, revenue
            FROM sales_analytics_sections
            WHERE event_id = $1
            ORDER BY revenue DESC
            LIMIT 1
            ",
        )
        .bind(event_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;

        if most_popular.is_none() && highest_revenue.is_none() {
            return Ok(None);
        }

        #[allow(clippy::cast_sign_loss)]
        Ok(Some(PopularSectionsDto {
            event_id,
            most_popular: most_popular.map(|(section, tickets, revenue)| SectionPopularityDto {
                section,
                tickets_sold: tickets as u32,
                revenue: crate::types::Money::from_cents(revenue as u64),
            }),
            highest_revenue: highest_revenue.map(|(section, tickets, revenue)| {
                SectionPopularityDto {
                    section,
                    tickets_sold: tickets as u32,
                    revenue: crate::types::Money::from_cents(revenue as u64),
                }
            }),
        }))
    }

    /// Get total revenue across all events.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_total_revenue(&self) -> Result<TotalRevenueDto, sqlx::Error> {
        let row: (i64, i64, i64) = sqlx::query_as(
            r"
            SELECT
                COALESCE(SUM(total_revenue)::BIGINT, 0),
                COALESCE(SUM(tickets_sold)::BIGINT, 0),
                COUNT(*)::BIGINT
            FROM sales_analytics_projection
            ",
        )
        .fetch_one(&self.pool)
        .await?;

        #[allow(clippy::cast_sign_loss)]
        Ok(TotalRevenueDto {
            total_revenue: crate::types::Money::from_cents(row.0 as u64),
            total_tickets_sold: row.1 as u32,
            events_with_sales: row.2 as usize,
        })
    }

    /// Get top spending customers.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_top_spenders(&self, limit: usize) -> Result<TopSpendersDto, sqlx::Error> {
        let rows: Vec<CustomerSpendingRow> = sqlx::query_as(
            r"
            SELECT customer_id, total_spent, total_tickets, purchase_count, favorite_section
            FROM customer_profiles
            ORDER BY total_spent DESC
            LIMIT $1
            ",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let total_customers: i64 = sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM customer_profiles")
            .fetch_one(&self.pool)
            .await?;

        #[allow(clippy::cast_sign_loss)]
        let customers: Vec<CustomerSpendingDto> = rows
            .into_iter()
            .map(|r| CustomerSpendingDto {
                customer_id: CustomerId::from_uuid(r.customer_id),
                total_spent: crate::types::Money::from_cents(r.total_spent as u64),
                total_tickets: r.total_tickets as u32,
                events_attended: r.purchase_count as usize,
                favorite_section: r.favorite_section,
            })
            .collect();

        Ok(TopSpendersDto {
            customers,
            total_customers: total_customers as usize,
        })
    }

    /// Get customer profile.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_customer_profile(
        &self,
        customer_id: CustomerId,
    ) -> Result<Option<CustomerProfileDto>, sqlx::Error> {
        // Get profile summary
        let profile: Option<CustomerProfileRow> = sqlx::query_as(
            r"
            SELECT customer_id, total_spent, total_tickets, purchase_count, favorite_section
            FROM customer_profiles
            WHERE customer_id = $1
            ",
        )
        .bind(customer_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;

        let Some(profile) = profile else {
            return Ok(None);
        };

        // Get events attended
        let events: Vec<(uuid::Uuid,)> = sqlx::query_as(
            r"
            SELECT event_id
            FROM customer_event_attendance
            WHERE customer_id = $1
            ",
        )
        .bind(customer_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        let events_attended: Vec<crate::types::EventId> = events
            .into_iter()
            .map(|(id,)| crate::types::EventId::from_uuid(id))
            .collect();

        // Get recent purchases (last 10)
        let purchases: Vec<PurchaseRow> = sqlx::query_as(
            r"
            SELECT reservation_id, event_id, section, ticket_count, amount_paid, completed_at
            FROM customer_purchases
            WHERE customer_id = $1
            ORDER BY completed_at DESC
            LIMIT 10
            ",
        )
        .bind(customer_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        #[allow(clippy::cast_sign_loss)]
        let recent_purchases: Vec<PurchaseRecordDto> = purchases
            .into_iter()
            .map(|r| PurchaseRecordDto {
                reservation_id: r.reservation_id,
                event_id: crate::types::EventId::from_uuid(r.event_id),
                section: r.section,
                ticket_count: r.ticket_count as u32,
                amount_paid: crate::types::Money::from_cents(r.amount_paid as u64),
                completed_at: r.completed_at,
            })
            .collect();

        #[allow(clippy::cast_sign_loss)]
        Ok(Some(CustomerProfileDto {
            customer_id,
            total_spent: crate::types::Money::from_cents(profile.total_spent as u64),
            total_tickets: profile.total_tickets as u32,
            events_attended,
            favorite_section: profile.favorite_section,
            recent_purchases,
        }))
    }
}

impl ProjectionQueries for AnalyticsProjectionQueries {
    type Error = sqlx::Error;
}

/// Raw database row for sales metrics.
#[derive(Debug, sqlx::FromRow)]
struct SalesMetricsRow {
    total_revenue: i64,
    tickets_sold: i32,
    completed_reservations: i32,
    cancelled_reservations: i32,
    average_ticket_price: i64,
}

/// Raw database row for section sales.
#[derive(Debug, sqlx::FromRow)]
struct SectionSalesRow {
    section: String,
    revenue: i64,
    tickets_sold: i32,
}

/// Raw database row for customer spending.
#[derive(Debug, sqlx::FromRow)]
struct CustomerSpendingRow {
    customer_id: uuid::Uuid,
    total_spent: i64,
    total_tickets: i32,
    purchase_count: i32,
    favorite_section: Option<String>,
}

/// Raw database row for customer profile.
#[derive(Debug, sqlx::FromRow)]
struct CustomerProfileRow {
    #[allow(dead_code)]
    customer_id: uuid::Uuid,
    total_spent: i64,
    total_tickets: i32,
    #[allow(dead_code)]
    purchase_count: i32,
    favorite_section: Option<String>,
}

/// Raw database row for purchases.
#[derive(Debug, sqlx::FromRow)]
struct PurchaseRow {
    reservation_id: uuid::Uuid,
    event_id: uuid::Uuid,
    section: String,
    ticket_count: i32,
    amount_paid: i64,
    completed_at: chrono::DateTime<chrono::Utc>,
}

/// Query fetcher for Analytics aggregate commands.
#[derive(Debug, Clone, Copy, Default)]
pub struct AnalyticsQueryFetcher;

impl QueryFetcher<AnalyticsCommand, AnalyticsProjectionQueries> for AnalyticsQueryFetcher {
    type Error = sqlx::Error;

    async fn fetch(
        &self,
        input: AnalyticsCommand,
        projections: &AnalyticsProjectionQueries,
    ) -> Result<FetchResult<AnalyticsCommand>, Self::Error> {
        match input {
            AnalyticsCommand::GetEventSales { event_id, fetched: _ } => {
                let fetched = projections.get_event_sales(event_id).await?;
                Ok(FetchResult::new_entity(AnalyticsCommand::GetEventSales { event_id, fetched }))
            }

            AnalyticsCommand::GetPopularSections { event_id, fetched: _ } => {
                let fetched = projections.get_popular_sections(event_id).await?;
                Ok(FetchResult::new_entity(AnalyticsCommand::GetPopularSections { event_id, fetched }))
            }

            AnalyticsCommand::GetTotalRevenue { fetched: _ } => {
                let fetched = projections.get_total_revenue().await?;
                Ok(FetchResult::new_entity(AnalyticsCommand::GetTotalRevenue {
                    fetched: Some(fetched),
                }))
            }

            AnalyticsCommand::GetTopSpenders { limit, fetched: _ } => {
                let fetched = projections.get_top_spenders(limit).await?;
                Ok(FetchResult::new_entity(AnalyticsCommand::GetTopSpenders {
                    limit,
                    fetched: Some(fetched),
                }))
            }

            AnalyticsCommand::GetCustomerProfile {
                customer_id,
                requesting_user_id,
                fetched: _,
            } => {
                let fetched = projections.get_customer_profile(customer_id).await?;
                Ok(FetchResult::new_entity(AnalyticsCommand::GetCustomerProfile {
                    customer_id,
                    requesting_user_id,
                    fetched,
                }))
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Reservation Projection Queries
// ═══════════════════════════════════════════════════════════════════════════

use super::reservation::{
    ReservationDto, ReservationListDto, ReservationQueryCommand, ReservationSummaryDto,
};
use crate::types::ReservationStatus;

/// PostgreSQL-backed projection queries for reservations.
///
/// Reads from the `reservations_projection` table.
#[derive(Clone)]
pub struct ReservationProjectionQueries {
    pool: PgPool,
}

impl ReservationProjectionQueries {
    /// Create a new reservation projection queries instance.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get a single reservation by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_reservation(
        &self,
        reservation_id: ReservationId,
    ) -> Result<Option<ReservationDto>, sqlx::Error> {
        let row: Option<ReservationRow> = sqlx::query_as(
            r"
            SELECT
                id, event_id, customer_id, section, quantity,
                status, total_amount_cents, expires_at, created_at, completed_at
            FROM reservations_projection
            WHERE id = $1
            ",
        )
        .bind(reservation_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    /// List all reservations for a customer.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn list_by_customer(
        &self,
        customer_id: CustomerId,
    ) -> Result<ReservationListDto, sqlx::Error> {
        let rows: Vec<ReservationSummaryRow> = sqlx::query_as(
            r"
            SELECT
                id, event_id, section, quantity, status, total_amount_cents, created_at
            FROM reservations_projection
            WHERE customer_id = $1
            ORDER BY created_at DESC
            ",
        )
        .bind(customer_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        let total = rows.len();
        let reservations: Vec<ReservationSummaryDto> = rows.into_iter().map(Into::into).collect();

        Ok(ReservationListDto {
            reservations,
            total,
        })
    }
}

impl ProjectionQueries for ReservationProjectionQueries {
    type Error = sqlx::Error;
}

/// Raw database row for reservation details.
#[derive(Debug, sqlx::FromRow)]
struct ReservationRow {
    id: uuid::Uuid,
    event_id: uuid::Uuid,
    customer_id: uuid::Uuid,
    section: String,
    quantity: i32,
    status: String,
    total_amount_cents: i64,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<ReservationRow> for ReservationDto {
    fn from(row: ReservationRow) -> Self {
        let status = match row.status.as_str() {
            "initiated" => ReservationStatus::Initiated,
            "seats_reserved" => ReservationStatus::SeatsReserved,
            "payment_pending" => ReservationStatus::PaymentPending,
            "payment_completed" => ReservationStatus::PaymentCompleted,
            "completed" => ReservationStatus::Completed,
            "cancelled" => ReservationStatus::Cancelled,
            "expired" => ReservationStatus::Expired,
            _ => ReservationStatus::Initiated,
        };

        #[allow(clippy::cast_sign_loss)]
        Self {
            id: ReservationId::from_uuid(row.id),
            event_id: crate::types::EventId::from_uuid(row.event_id),
            customer_id: CustomerId::from_uuid(row.customer_id),
            section: row.section,
            quantity: row.quantity as u32,
            status,
            total_amount: crate::types::Money::from_cents(row.total_amount_cents as u64),
            expires_at: row.expires_at,
            created_at: row.created_at,
            completed_at: row.completed_at,
        }
    }
}

/// Raw database row for reservation summary.
#[derive(Debug, sqlx::FromRow)]
struct ReservationSummaryRow {
    id: uuid::Uuid,
    event_id: uuid::Uuid,
    section: String,
    quantity: i32,
    status: String,
    total_amount_cents: i64,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<ReservationSummaryRow> for ReservationSummaryDto {
    fn from(row: ReservationSummaryRow) -> Self {
        let status = match row.status.as_str() {
            "initiated" => ReservationStatus::Initiated,
            "seats_reserved" => ReservationStatus::SeatsReserved,
            "payment_pending" => ReservationStatus::PaymentPending,
            "payment_completed" => ReservationStatus::PaymentCompleted,
            "completed" => ReservationStatus::Completed,
            "cancelled" => ReservationStatus::Cancelled,
            "expired" => ReservationStatus::Expired,
            _ => ReservationStatus::Initiated,
        };

        #[allow(clippy::cast_sign_loss)]
        Self {
            id: ReservationId::from_uuid(row.id),
            event_id: crate::types::EventId::from_uuid(row.event_id),
            section: row.section,
            quantity: row.quantity as u32,
            status,
            total_amount: crate::types::Money::from_cents(row.total_amount_cents as u64),
            created_at: row.created_at,
        }
    }
}

/// Query fetcher for Reservation query commands.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReservationQueryFetcher;

impl QueryFetcher<ReservationQueryCommand, ReservationProjectionQueries> for ReservationQueryFetcher {
    type Error = sqlx::Error;

    async fn fetch(
        &self,
        input: ReservationQueryCommand,
        projections: &ReservationProjectionQueries,
    ) -> Result<FetchResult<ReservationQueryCommand>, Self::Error> {
        match input {
            ReservationQueryCommand::GetReservation {
                reservation_id,
                fetched: _,
            } => {
                let fetched = projections.get_reservation(reservation_id).await?;
                Ok(FetchResult::new_entity(ReservationQueryCommand::GetReservation {
                    reservation_id,
                    fetched,
                }))
            }

            ReservationQueryCommand::ListUserReservations {
                customer_id,
                requesting_user_id,
                fetched: _,
            } => {
                let fetched = projections.list_by_customer(customer_id).await?;
                Ok(FetchResult::new_entity(ReservationQueryCommand::ListUserReservations {
                    customer_id,
                    requesting_user_id,
                    fetched: Some(fetched),
                }))
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Event-Inventory Saga Query Fetcher
// ═══════════════════════════════════════════════════════════════════════════

use super::event_inventory_saga::{SagaInput, SagaState};
use super::projector::InMemorySagaProjection;

/// In-memory projection queries for the Event-Inventory saga.
///
/// This is a thin wrapper around [`InMemorySagaProjection`] that implements
/// [`ProjectionQueries`]. It enables the [`SagaQueryFetcher`] to look up
/// saga state during feedback loops.
///
/// # Thread Safety
///
/// Uses async locking via `tokio::sync::RwLock` for concurrent access.
#[derive(Clone)]
pub struct SagaProjectionQueries {
    state: InMemorySagaProjection,
}

impl SagaProjectionQueries {
    /// Create new saga projection queries with the given shared state.
    #[must_use]
    pub const fn new(state: InMemorySagaProjection) -> Self {
        Self { state }
    }

    /// Get the current state for a saga by event_id.
    ///
    /// Returns `None` if no saga exists for this event_id.
    pub async fn get_saga_state(&self, event_id: crate::types::EventId) -> Option<SagaState> {
        self.state.read().await.get(&event_id).cloned()
    }
}

impl ProjectionQueries for SagaProjectionQueries {
    type Error = std::convert::Infallible;
}

/// Query fetcher for the Event-Inventory saga.
///
/// This fetcher populates the `fetched` field in [`SagaInput::Feedback`]
/// with the current saga state from the in-memory projection.
///
/// # Flow
///
/// ```text
/// Handler.handle(input)
///     │
///     ├─► SagaQueryFetcher.fetch(Feedback { event_id, fetched: None })
///     │         │
///     │         └─► SagaProjectionQueries.get_saga_state(event_id)
///     │                  │
///     │                  └─► InMemorySagaProjection (HashMap)
///     │
///     └─► BusinessLogic.process(Feedback { event_id, fetched: Some(state) })
/// ```
#[derive(Clone)]
pub struct SagaQueryFetcher {
    state: InMemorySagaProjection,
}

impl SagaQueryFetcher {
    /// Create a new saga query fetcher with the given shared state.
    ///
    /// # Arguments
    ///
    /// * `state` - Shared state with the [`EventInventorySagaProjector`](super::projector::EventInventorySagaProjector)
    #[must_use]
    pub const fn new(state: InMemorySagaProjection) -> Self {
        Self { state }
    }
}

impl QueryFetcher<SagaInput, SagaProjectionQueries> for SagaQueryFetcher {
    type Error = std::convert::Infallible;

    async fn fetch(
        &self,
        input: SagaInput,
        _projections: &SagaProjectionQueries,
    ) -> Result<FetchResult<SagaInput>, Self::Error> {
        match input {
            // For initial commands, pass through - no state to fetch
            SagaInput::CreateEventWithInventory { .. } => {
                Ok(FetchResult::new_entity(input))
            }

            // For feedback, look up saga state by event_id
            SagaInput::Feedback {
                event_id,
                results,
                fetched: _,
            } => {
                // Read state from the shared in-memory projection
                let fetched = self.state.read().await.get(&event_id).cloned();

                tracing::debug!(
                    event_id = %event_id,
                    found = fetched.is_some(),
                    phase = ?fetched.as_ref().map(|s| &s.phase),
                    "Fetched saga state for feedback"
                );

                Ok(FetchResult::new_entity(SagaInput::Feedback {
                    event_id,
                    results,
                    fetched,
                }))
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Reservation Saga Query Fetcher
// ═══════════════════════════════════════════════════════════════════════════

use super::projector::InMemoryReservationSagaProjection;
use super::reservation_saga::{ReservationSagaInput, ReservationSagaState};

/// In-memory projection queries for the Reservation saga.
///
/// This is a thin wrapper around [`InMemoryReservationSagaProjection`] that implements
/// [`ProjectionQueries`]. It enables the [`ReservationSagaQueryFetcher`] to look up
/// saga state during feedback loops.
#[derive(Clone)]
pub struct ReservationSagaProjectionQueries {
    state: InMemoryReservationSagaProjection,
}

impl ReservationSagaProjectionQueries {
    /// Create new reservation saga projection queries with the given shared state.
    #[must_use]
    pub const fn new(state: InMemoryReservationSagaProjection) -> Self {
        Self { state }
    }

    /// Get the current state for a saga by reservation_id.
    ///
    /// Returns `None` if no saga exists for this reservation_id.
    pub async fn get_saga_state(&self, reservation_id: ReservationId) -> Option<ReservationSagaState> {
        self.state.read().await.get(&reservation_id).cloned()
    }
}

impl ProjectionQueries for ReservationSagaProjectionQueries {
    type Error = std::convert::Infallible;
}

/// Query fetcher for the Reservation saga.
///
/// This fetcher populates the `fetched` field in [`ReservationSagaInput::Feedback`]
/// with the current saga state from the in-memory projection.
///
/// # Flow
///
/// ```text
/// Handler.handle(input)
///     │
///     ├─► ReservationSagaQueryFetcher.fetch(Feedback { reservation_id, fetched: None })
///     │         │
///     │         └─► ReservationSagaProjectionQueries.get_saga_state(reservation_id)
///     │                  │
///     │                  └─► InMemoryReservationSagaProjection (HashMap)
///     │
///     └─► BusinessLogic.process(Feedback { reservation_id, fetched: Some(state) })
/// ```
#[derive(Clone)]
pub struct ReservationSagaQueryFetcher {
    state: InMemoryReservationSagaProjection,
}

impl ReservationSagaQueryFetcher {
    /// Create a new reservation saga query fetcher with the given shared state.
    ///
    /// # Arguments
    ///
    /// * `state` - Shared state with the [`ReservationSagaProjector`](super::projector::ReservationSagaProjector)
    #[must_use]
    pub const fn new(state: InMemoryReservationSagaProjection) -> Self {
        Self { state }
    }
}

impl QueryFetcher<ReservationSagaInput, ReservationSagaProjectionQueries> for ReservationSagaQueryFetcher {
    type Error = std::convert::Infallible;

    async fn fetch(
        &self,
        input: ReservationSagaInput,
        _projections: &ReservationSagaProjectionQueries,
    ) -> Result<FetchResult<ReservationSagaInput>, Self::Error> {
        match input {
            // For initial commands, pass through - no state to fetch
            ReservationSagaInput::InitiateReservation { .. } => {
                Ok(FetchResult::new_entity(input))
            }

            // Cancel/Expire commands already have reservation_id for lookup
            ReservationSagaInput::CancelReservation {
                reservation_id,
                fetched: _,
            } => {
                let fetched = self.state.read().await.get(&reservation_id).cloned();
                Ok(FetchResult::new_entity(ReservationSagaInput::CancelReservation {
                    reservation_id,
                    fetched,
                }))
            }

            ReservationSagaInput::ExpireReservation {
                reservation_id,
                fetched: _,
            } => {
                let fetched = self.state.read().await.get(&reservation_id).cloned();
                Ok(FetchResult::new_entity(ReservationSagaInput::ExpireReservation {
                    reservation_id,
                    fetched,
                }))
            }

            // For feedback, look up saga state by reservation_id
            ReservationSagaInput::Feedback {
                reservation_id,
                results,
                fetched: _,
            } => {
                // Read state from the shared in-memory projection
                let fetched = self.state.read().await.get(&reservation_id).cloned();

                tracing::debug!(
                    reservation_id = %reservation_id,
                    found = fetched.is_some(),
                    phase = ?fetched.as_ref().map(|s| &s.phase),
                    "Fetched reservation saga state for feedback"
                );

                Ok(FetchResult::new_entity(ReservationSagaInput::Feedback {
                    reservation_id,
                    results,
                    fetched,
                }))
            }
        }
    }
}
