//! Event management API endpoints.
//!
//! Provides CRUD operations for events:
//! - POST /api/events - Create a new event (requires auth)
//! - GET /api/events/:id - Get event details
//! - GET /api/events - List events with pagination
//! - PUT /api/events/:id - Update event (requires ownership)
//! - DELETE /api/events/:id - Delete event (requires ownership)

use crate::auth::middleware::SessionUser;
use crate::server::state::AppState;
use crate::types::{EventId, EventStatus};
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
    State(_state): State<AppState>,
    Json(_request): Json<CreateEventRequest>,
) -> Result<(StatusCode, Json<CreateEventResponse>), AppError> {
    // Generate new event ID
    let event_id = EventId::new();

    // TODO: Send CreateEvent action to event aggregate via event store
    // For now, just return success
    let _ = session; // Use session.user_id as organizer_id

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
    State(_state): State<AppState>,
) -> Result<Json<EventResponse>, AppError> {
    // TODO: Query event from projection or event store

    // Placeholder response
    Err(AppError::not_found("Event", event_id))
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
    State(_state): State<AppState>,
) -> Result<Json<ListEventsResponse>, AppError> {
    // Validate page size
    let page_size = query.page_size.min(100);

    // TODO: Query events from projection
    let _ = query.status;

    // Placeholder response
    Ok(Json(ListEventsResponse {
        events: vec![],
        total: 0,
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
    State(_state): State<AppState>,
    Json(_request): Json<UpdateEventRequest>,
) -> Result<Json<EventResponse>, AppError> {
    // TODO: Verify ownership via RequireOwnership extractor
    // TODO: Send UpdateEvent action to event aggregate
    let _ = session;

    Err(AppError::not_found("Event", event_id))
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
    State(_state): State<AppState>,
) -> Result<StatusCode, AppError> {
    // TODO: Verify ownership via RequireOwnership extractor
    // TODO: Send CancelEvent action to event aggregate
    let _ = session;

    Err(AppError::not_found("Event", event_id))
}
