//! HTTP handlers using the next-generation Handler pattern.
//!
//! This module demonstrates how to wire up HTTP endpoints with the new
//! separated architecture where:
//! - Business logic is pure (no infrastructure concerns)
//! - Handler orchestrates persistence, projection, and broadcasting
//! - HTTP handlers just translate requests/responses
//!
//! # Architecture
//!
//! ```text
//! HTTP Request
//!      │
//!      ▼
//! HTTP Handler (translate request → command)
//!      │
//!      ▼
//! Handler.handle(command) ─── orchestrates ───┐
//!      │                                      │
//!      │  ┌───────────────────────────────────┘
//!      │  │
//!      │  ▼
//!      │  1. Load state from EventStore
//!      │  2. Call BusinessLogic.process()
//!      │  3. Persist events to EventStore
//!      │  4. Project to read model (wait)
//!      │  5. Broadcast to EventBus
//!      │
//!      ▼
//! Result<HandleResult, HandlerError>
//!      │
//!      ▼
//! HTTP Handler (translate result → response)
//!      │
//!      ▼
//! HTTP Response
//! ```
//!
//! # Example
//!
//! ```bash
//! curl -X POST http://localhost:8080/api/v2/events \
//!   -H "Authorization: Bearer <token>" \
//!   -H "Content-Type: application/json" \
//!   -d '{"title": "Concert", "venue_name": "Arena", ...}'
//! ```

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use composable_rust_next::{Handler, HandlerError, NoOpCallExecutor};
use composable_rust_web::error::AppError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use super::{
    environment::{NoOpEventBus, NoOpProjector, TicketingEnvironment},
    EventBusinessLogic, EventCommand, EventError,
};
use crate::types::{
    Capacity, EventDate, EventId, Money, PricingTier, SeatType, TierType, Venue, VenueSection,
};
use composable_rust_auth::state::UserId;

// ═══════════════════════════════════════════════════════════════════════════
// Type Aliases
// ═══════════════════════════════════════════════════════════════════════════

/// Type alias for the Event aggregate handler with minimal environment.
pub type EventHandler = Handler<
    EventBusinessLogic,
    NoOpCallExecutor,
    TicketingEnvironment<NoOpProjector, NoOpEventBus>,
>;

/// Shared state containing the event handler.
#[derive(Clone)]
pub struct NextAppState {
    /// The event aggregate handler.
    pub event_handler: Arc<EventHandler>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Request/Response Types
// ═══════════════════════════════════════════════════════════════════════════

/// Request to create a new event.
#[derive(Debug, Deserialize)]
pub struct CreateEventRequest {
    /// Event title
    pub title: String,
    /// Event description (currently unused - for future extension)
    #[allow(dead_code)]
    pub description: Option<String>,
    /// Event start time
    pub start_time: DateTime<Utc>,
    /// Venue name
    pub venue_name: String,
    /// Total venue capacity
    pub capacity: u32,
    /// Ticket price in dollars
    pub price: f64,
    /// Optional owner ID (for demo purposes - would come from auth in production)
    pub owner_id: Option<Uuid>,
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
    /// Event start time
    pub start_time: DateTime<Utc>,
    /// Venue name
    pub venue_name: String,
    /// Event status
    pub status: String,
}

/// Response for publishing an event.
#[derive(Debug, Serialize)]
pub struct PublishEventResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Success message
    pub message: String,
}

// ═══════════════════════════════════════════════════════════════════════════
// Error Conversion
// ═══════════════════════════════════════════════════════════════════════════

/// Convert a handler error to an HTTP error response.
///
/// We use a function instead of `From` trait due to orphan rules
/// (neither `HandlerError` nor `AppError` is defined in this crate).
fn to_app_error(err: HandlerError<EventError>) -> AppError {
    match err {
        HandlerError::Business(EventError::AlreadyExists) => {
            AppError::conflict("Event already exists")
        }
        HandlerError::Business(EventError::NotFound) => AppError::not_found("Event", "unknown"),
        HandlerError::Business(EventError::InvalidStateTransition { from, to }) => {
            AppError::bad_request(format!("Invalid state transition from {from:?} to {to:?}"))
        }
        HandlerError::Business(EventError::ValidationFailed { message }) => {
            AppError::bad_request(message)
        }
        HandlerError::Load(e) => AppError::internal(format!("Failed to load state: {e}")),
        HandlerError::Persist(e) => AppError::internal(format!("Failed to persist: {e}")),
        HandlerError::Projection(e) => AppError::internal(format!("Projection failed: {e}")),
        HandlerError::Broadcast(e) => AppError::internal(format!("Broadcast failed: {e}")),
        HandlerError::Serialization(e) => {
            AppError::internal(format!("Serialization failed: {e}"))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HTTP Handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Create a new event.
///
/// Uses the new Handler pattern for clean separation of concerns:
/// - Business logic validation happens in `EventBusinessLogic`
/// - Persistence, projection, and broadcasting are handled by `Handler`
/// - This HTTP handler just translates request/response formats
///
/// # Request
///
/// ```json
/// {
///   "title": "Tech Conference 2024",
///   "start_time": "2024-06-01T09:00:00Z",
///   "venue_name": "Convention Center",
///   "capacity": 500,
///   "price": 50.00,
///   "owner_id": "550e8400-e29b-41d4-a716-446655440000"
/// }
/// ```
///
/// # Response
///
/// ```json
/// {
///   "event_id": "550e8400-e29b-41d4-a716-446655440000",
///   "message": "Event created successfully"
/// }
/// ```
pub async fn create_event(
    State(state): State<NextAppState>,
    Json(request): Json<CreateEventRequest>,
) -> Result<(StatusCode, Json<CreateEventResponse>), AppError> {
    // Generate new event ID
    let event_id = EventId::new();

    // Map API request to domain types
    let venue = create_venue(&request);
    let date = EventDate::new(request.start_time);
    let pricing_tiers = create_pricing_tiers(&request);

    // Use provided owner_id or generate a new one for demo purposes
    let owner_id = request.owner_id.map(UserId).unwrap_or_else(UserId::new);

    // Build command
    let command = EventCommand::Create {
        event_id,
        name: request.title,
        owner_id,
        venue,
        date,
        pricing_tiers,
    };

    // Handle command - business logic, persistence, projection, broadcast all done
    let _result = state
        .event_handler
        .handle(command)
        .await
        .map_err(to_app_error)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateEventResponse {
            event_id: *event_id.as_uuid(),
            message: "Event created successfully".to_string(),
        }),
    ))
}

/// Publish an event (transition from Draft to Published).
///
/// # Request
///
/// POST /api/v2/events/:id/publish
///
/// # Response
///
/// ```json
/// {
///   "event_id": "550e8400-e29b-41d4-a716-446655440000",
///   "message": "Event published successfully"
/// }
/// ```
pub async fn publish_event(
    Path(event_id): Path<Uuid>,
    State(state): State<NextAppState>,
) -> Result<Json<PublishEventResponse>, AppError> {
    let event_id = EventId::from_uuid(event_id);

    let command = EventCommand::Publish { event_id };

    let _result = state
        .event_handler
        .handle(command)
        .await
        .map_err(to_app_error)?;

    Ok(Json(PublishEventResponse {
        event_id: *event_id.as_uuid(),
        message: "Event published successfully".to_string(),
    }))
}

/// Cancel an event.
///
/// # Request
///
/// POST /api/v2/events/:id/cancel
///
/// ```json
/// {
///   "reason": "Venue unavailable"
/// }
/// ```
///
/// # Response
///
/// ```json
/// {
///   "event_id": "550e8400-e29b-41d4-a716-446655440000",
///   "message": "Event cancelled"
/// }
/// ```
#[derive(Debug, Deserialize)]
pub struct CancelEventRequest {
    /// Reason for cancellation
    pub reason: String,
}

/// Response after cancelling an event.
#[derive(Debug, Serialize)]
pub struct CancelEventResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Success message
    pub message: String,
}

/// Cancel an event.
pub async fn cancel_event(
    Path(event_id): Path<Uuid>,
    State(state): State<NextAppState>,
    Json(request): Json<CancelEventRequest>,
) -> Result<Json<CancelEventResponse>, AppError> {
    let event_id = EventId::from_uuid(event_id);

    let command = EventCommand::Cancel {
        event_id,
        reason: request.reason,
    };

    let _result = state
        .event_handler
        .handle(command)
        .await
        .map_err(to_app_error)?;

    Ok(Json(CancelEventResponse {
        event_id: *event_id.as_uuid(),
        message: "Event cancelled".to_string(),
    }))
}

// ═══════════════════════════════════════════════════════════════════════════
// Helper Functions
// ═══════════════════════════════════════════════════════════════════════════

/// Create a venue from the request.
fn create_venue(request: &CreateEventRequest) -> Venue {
    let section = VenueSection::new(
        "General Admission".to_string(),
        Capacity::new(request.capacity),
        SeatType::GeneralAdmission,
    );

    Venue::new(
        request.venue_name.clone(),
        Capacity::new(request.capacity),
        vec![section],
    )
}

/// Create pricing tiers from the request.
fn create_pricing_tiers(request: &CreateEventRequest) -> Vec<PricingTier> {
    vec![PricingTier::new(
        TierType::Regular,
        "General Admission".to_string(),
        Money::from_dollars(request.price as u64),
        Utc::now(),
        None,
    )]
}

// ═══════════════════════════════════════════════════════════════════════════
// Router Setup
// ═══════════════════════════════════════════════════════════════════════════

use axum::{routing::post, Router};

/// Create the v2 events API router.
///
/// This router uses the new Handler pattern for all endpoints.
///
/// # Routes
///
/// - POST /api/v2/events - Create a new event
/// - POST /api/v2/events/:id/publish - Publish an event
/// - POST /api/v2/events/:id/cancel - Cancel an event
pub fn events_v2_routes() -> Router<NextAppState> {
    Router::new()
        .route("/events", post(create_event))
        .route("/events/{id}/publish", post(publish_event))
        .route("/events/{id}/cancel", post(cancel_event))
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Integration tests requiring PostgreSQL are in the tests/ directory.
    // For unit tests of business logic, see event.rs tests.

    #[test]
    fn test_create_venue() {
        let request = CreateEventRequest {
            title: "Test Event".to_string(),
            description: None,
            start_time: Utc::now(),
            venue_name: "Test Venue".to_string(),
            capacity: 100,
            price: 25.0,
            owner_id: None,
        };

        let venue = create_venue(&request);
        assert_eq!(venue.name, "Test Venue");
        assert_eq!(venue.capacity.value(), 100);
        assert_eq!(venue.sections.len(), 1);
    }

    #[test]
    fn test_create_pricing_tiers() {
        let request = CreateEventRequest {
            title: "Test Event".to_string(),
            description: None,
            start_time: Utc::now(),
            venue_name: "Test Venue".to_string(),
            capacity: 100,
            price: 25.0,
            owner_id: None,
        };

        let tiers = create_pricing_tiers(&request);
        assert_eq!(tiers.len(), 1);
        assert_eq!(tiers[0].tier_type, TierType::Regular);
        assert_eq!(tiers[0].base_price.cents(), 2500); // $25.00 = 2500 cents
    }
}
