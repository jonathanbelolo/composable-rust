//! Event management API endpoints.
//!
//! Provides CRUD operations for events:
//! - POST /api/events - Create a new event (requires auth)
//! - GET /api/events/:id - Get event details
//! - GET /api/events - List events with pagination
//! - PUT /api/events/:id - Update event (requires ownership)
//! - DELETE /api/events/:id - Delete event (requires ownership)

#![allow(clippy::missing_errors_doc)] // Example code - errors are standard AppError

use crate::aggregates::event::EventAction;
use crate::auth::middleware::SessionUser;
use crate::server::state::AppState;
use crate::types::{Capacity, EventDate, EventId, EventStatus, Money, PricingTier, SeatType, TierType, Venue, VenueSection};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use composable_rust_web::error::AppError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Request/Response Types
// ============================================================================

/// Request to create a new event.
#[derive(Debug, Deserialize)]
pub struct CreateEventRequest {
    /// Event title
    pub title: String,
    /// Event description
    pub description: String,
    /// Event start time
    pub start_time: DateTime<Utc>,
    /// Event end time
    pub end_time: DateTime<Utc>,
    /// Venue name
    pub venue_name: String,
    /// Venue address
    pub venue_address: String,
    /// Total venue capacity
    pub capacity: u32,
    /// Ticket price in dollars
    pub price: f64,
}

impl CreateEventRequest {
    /// Maps API request to domain types with sensible defaults
    ///
    /// Creates a single "General Admission" section for the venue and a single "Regular" pricing tier.
    /// For production, this should be extended to support multiple sections and pricing tiers.
    fn to_domain_types(&self) -> (Venue, EventDate, Vec<PricingTier>) {
        // Create a single venue section with all capacity
        let section = VenueSection::new(
            "General Admission".to_string(),
            Capacity::new(self.capacity),
            SeatType::GeneralAdmission,
        );

        let venue = Venue::new(
            self.venue_name.clone(),
            Capacity::new(self.capacity),
            vec![section],
        );

        let event_date = EventDate::new(self.start_time);

        // Create a single "Regular" pricing tier
        let pricing_tier = PricingTier::new(
            TierType::Regular,
            "General Admission".to_string(),
            Money::from_dollars(self.price as u64),
            Utc::now(),
            None, // No expiration
        );

        (venue, event_date, vec![pricing_tier])
    }
}

/// Response after creating an event.
#[derive(Debug, Serialize)]
pub struct CreateEventResponse {
    /// Created event ID
    pub event_id: Uuid,
    /// Success message
    pub message: String,
}

/// Event details response.
#[derive(Debug, Serialize)]
pub struct EventResponse {
    /// Event ID
    pub id: Uuid,
    /// Event title
    pub title: String,
    /// Event description
    pub description: String,
    /// Event start time
    pub start_time: DateTime<Utc>,
    /// Event end time
    pub end_time: DateTime<Utc>,
    /// Venue name
    pub venue_name: String,
    /// Event status
    pub status: EventStatus,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
}

/// Query parameters for listing events.
#[derive(Debug, Deserialize)]
pub struct ListEventsQuery {
    /// Page number (0-indexed)
    #[serde(default)]
    pub page: usize,
    /// Page size (default: 20, max: 100)
    #[serde(default = "default_page_size")]
    pub page_size: usize,
    /// Filter by status
    pub status: Option<EventStatus>,
}

const fn default_page_size() -> usize {
    20
}

/// Response for listing events.
#[derive(Debug, Serialize)]
pub struct ListEventsResponse {
    /// List of events
    pub events: Vec<EventResponse>,
    /// Total count of events
    pub total: usize,
    /// Current page
    pub page: usize,
    /// Page size
    pub page_size: usize,
}

/// Request to update an event.
#[derive(Debug, Deserialize)]
pub struct UpdateEventRequest {
    /// Updated title
    pub title: Option<String>,
    /// Updated description
    pub description: Option<String>,
    /// Updated start time
    pub start_time: Option<DateTime<Utc>>,
    /// Updated end time
    pub end_time: Option<DateTime<Utc>>,
}

// ============================================================================
// Handlers
// ============================================================================

/// Create a new event.
///
/// Requires authentication. The authenticated user becomes the event organizer.
///
/// # Example
///
/// ```bash
/// curl -X POST http://localhost:8080/api/events \
///   -H "Authorization: Bearer <session_token>" \
///   -H "Content-Type: application/json" \
///   -d '{
///     "title": "Tech Conference 2024",
///     "description": "Annual technology conference",
///     "start_time": "2024-06-01T09:00:00Z",
///     "end_time": "2024-06-01T17:00:00Z",
///     "venue": {
///       "name": "Convention Center",
///       "address": "123 Main St",
///       "city": "San Francisco",
///       "state": "CA",
///       "zip_code": "94102",
///       "country": "USA"
///     }
///   }'
/// ```
pub async fn create_event(
    session: SessionUser,
    State(state): State<AppState>,
    Json(request): Json<CreateEventRequest>,
) -> Result<(StatusCode, Json<CreateEventResponse>), AppError> {
    // Generate new event ID
    let event_id = EventId::new();

    // Map API request to domain types
    let (venue, date, pricing_tiers) = request.to_domain_types();

    // Create Event store for this request
    let store = state.create_event_store();

    // Build CreateEvent action
    let action = EventAction::CreateEvent {
        id: event_id,
        name: request.title,
        owner_id: session.user_id,
        venue,
        date,
        pricing_tiers,
    };

    // Send action to store (Store executes effects automatically)
    store
        .send(action)
        .await
        .map_err(|e| AppError::internal(format!("Failed to create event: {e}")))?;

    Ok((
        StatusCode::CREATED,
        Json(CreateEventResponse {
            event_id: *event_id.as_uuid(),
            message: "Event created successfully".to_string(),
        }),
    ))
}

/// Get event details by ID.
///
/// Public endpoint - no authentication required.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/api/events/550e8400-e29b-41d4-a716-446655440000
/// ```
pub async fn get_event(
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<EventResponse>, AppError> {
    // Query event from projection
    let event = state
        .events_projection
        .get(&event_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to query event: {e}")))?
        .ok_or_else(|| AppError::not_found("Event", event_id))?;

    // Convert domain Event to API EventResponse
    // Note: Current domain model has limited fields. Using available data:
    // - name -> title
    // - date -> both start_time and end_time (TODO: extend domain model)
    // - venue.name -> venue_name
    // - description is not in domain model yet (TODO: add to Event type)
    let response = EventResponse {
        id: *event.id.as_uuid(),
        title: event.name,
        description: String::from("Event description not yet available"), // TODO: Add description field to Event domain model
        start_time: event.date.inner(),
        end_time: event.date.inner(), // TODO: Add separate end_time to Event domain model
        venue_name: event.venue.name,
        status: event.status,
        created_at: event.created_at,
    };

    Ok(Json(response))
}

/// List events with pagination.
///
/// Public endpoint - no authentication required.
///
/// # Example
///
/// ```bash
/// # Get first page
/// curl http://localhost:8080/api/events?page=0&page_size=20
///
/// # Filter by status
/// curl http://localhost:8080/api/events?status=Published
/// ```
pub async fn list_events(
    Query(query): Query<ListEventsQuery>,
    State(state): State<AppState>,
) -> Result<Json<ListEventsResponse>, AppError> {
    // Validate page size
    let page_size = query.page_size.min(100);

    // Query events from projection with optional status filter
    let status_str = query.status.as_ref().map(|s| format!("{s:?}"));
    let all_events = state
        .events_projection
        .list(status_str.as_deref())
        .await
        .map_err(|e| AppError::internal(format!("Failed to query events: {e}")))?;

    // Calculate pagination
    let total = all_events.len();
    let start = query.page * page_size;
    let end = start.saturating_add(page_size).min(total);

    // Paginate results
    let paginated_events: Vec<EventResponse> = all_events[start..end]
        .iter()
        .map(|event| EventResponse {
            id: *event.id.as_uuid(),
            title: event.name.clone(),
            description: String::from("Event description not yet available"), // TODO: Add description field to Event domain model
            start_time: event.date.inner(),
            end_time: event.date.inner(), // TODO: Add separate end_time to Event domain model
            venue_name: event.venue.name.clone(),
            status: event.status,
            created_at: event.created_at,
        })
        .collect();

    Ok(Json(ListEventsResponse {
        events: paginated_events,
        total,
        page: query.page,
        page_size,
    }))
}

/// Update an event.
///
/// Requires authentication and event ownership.
///
/// # Example
///
/// ```bash
/// curl -X PUT http://localhost:8080/api/events/550e8400-e29b-41d4-a716-446655440000 \
///   -H "Authorization: Bearer <session_token>" \
///   -H "Content-Type: application/json" \
///   -d '{
///     "title": "Updated Event Title",
///     "description": "Updated description"
///   }'
/// ```
pub async fn update_event(
    session: SessionUser,
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<UpdateEventRequest>,
) -> Result<Json<EventResponse>, AppError> {
    // Check if event exists and get it
    let event = state
        .events_projection
        .get(&event_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to query event: {e}")))?
        .ok_or_else(|| AppError::not_found("Event", event_id))?;

    // Verify ownership: only the event owner can update it
    if event.owner_id != session.user_id {
        return Err(AppError::forbidden(
            "You do not have permission to update this event. Only the event owner can update it.",
        ));
    }

    // Map API request fields to domain UpdateEvent command
    // Note: Currently only `title` -> `name` is supported in the domain model
    // TODO: Add support for description, start_time, end_time to Event domain model
    let name = request.title;

    // Validate that at least one field is being updated
    if name.is_none() {
        return Err(AppError::bad_request(
            "At least one field must be provided to update the event",
        ));
    }

    // Create event store and send UpdateEvent action
    use crate::aggregates::event::EventAction;
    use crate::types::EventId;

    let event_store = state.create_event_store();
    let action = EventAction::UpdateEvent {
        event_id: EventId::from_uuid(event_id),
        name,
    };

    event_store
        .send(action)
        .await
        .map_err(|e| AppError::internal(format!("Failed to update event: {e}")))?;

    // Query the updated event from projection
    let updated_event = state
        .events_projection
        .get(&event_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to query updated event: {e}")))?
        .ok_or_else(|| AppError::not_found("Event", event_id))?;

    // Convert to EventResponse
    let response = EventResponse {
        id: *updated_event.id.as_uuid(),
        title: updated_event.name,
        description: String::from("Event description not yet available"), // TODO: Add description field to Event domain model
        start_time: updated_event.date.inner(),
        end_time: updated_event.date.inner(), // TODO: Add separate end_time to Event domain model
        venue_name: updated_event.venue.name,
        status: updated_event.status,
        created_at: updated_event.created_at,
    };

    Ok(Json(response))
}

/// Delete an event.
///
/// Requires authentication and event ownership.
///
/// # Example
///
/// ```bash
/// curl -X DELETE http://localhost:8080/api/events/550e8400-e29b-41d4-a716-446655440000 \
///   -H "Authorization: Bearer <session_token>"
/// ```
pub async fn delete_event(
    session: SessionUser,
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<StatusCode, AppError> {
    // Check if event exists and get it
    let event = state
        .events_projection
        .get(&event_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to query event: {e}")))?
        .ok_or_else(|| AppError::not_found("Event", event_id))?;

    // Verify ownership: only the event owner can delete it
    if event.owner_id != session.user_id {
        return Err(AppError::forbidden(
            "You do not have permission to delete this event. Only the event owner can delete it.",
        ));
    }

    // Send CancelEvent action to event aggregate
    use crate::aggregates::event::EventAction;
    use crate::types::EventId;

    let event_store = state.create_event_store();
    let action = EventAction::CancelEvent {
        event_id: EventId::from_uuid(event_id),
        reason: format!("Cancelled by user {}", session.user_id.0),
    };

    event_store
        .send(action)
        .await
        .map_err(|e| AppError::internal(format!("Failed to cancel event: {e}")))?;

    Ok(StatusCode::NO_CONTENT)
}
