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
    extract::{FromRequestParts, Path, State},
    http::{request::Parts, StatusCode},
    Json,
};
use chrono::{DateTime, Utc};
use composable_rust_next::{
    EventStoreError, Handler, HandlerError, NoOpCallExecutor, NoOpQueryFetcher,
};
use composable_rust_web::error::AppError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use super::{
    call_executor::EventInventorySagaCallExecutor,
    environment::{NoOpEventBus, NoOpProjector, ProductionEnvironment, TicketingEnvironment},
    event::{EventDto, EventResponse as DomainEventResponse},
    event_inventory_saga::{EventInventorySagaLogic, SagaError, SagaInput},
    projection_queries::{
        EventInventorySagaProjectionQueries, EventInventorySagaQueryFetcher, EventProjectionQueries,
        EventQueryFetcher,
    },
    projector::{EventProjector, InventoryProjector, PaymentProjector},
    reservation_saga::ReservationSagaError,
    EventBusinessLogic, EventCommand, EventError, InventoryBusinessLogic, PaymentBusinessLogic,
};
use crate::types::{
    Capacity, CustomerId, EventDate, EventId, Money, PricingTier, ReservationId, SeatType,
    TierType, Venue, VenueSection,
};
use composable_rust_auth::state::UserId;
use composable_rust_next::SystemClock;
use composable_rust_postgres_next::PostgresEventStore;

// ═══════════════════════════════════════════════════════════════════════════
// Type Aliases
// ═══════════════════════════════════════════════════════════════════════════

/// Type alias for the Event aggregate handler with projection support.
///
/// Uses `SystemClock`, `PostgresEventStore`, and `EventProjector` for
/// synchronous projection updates after writes.
pub type EventAggregateHandler = Handler<
    EventBusinessLogic,
    NoOpCallExecutor,
    NoOpQueryFetcher,
    ProductionEnvironment<EventProjector, NoOpEventBus>,
>;

/// Type alias for the Inventory aggregate handler with projection support.
pub type InventoryAggregateHandler = Handler<
    InventoryBusinessLogic,
    NoOpCallExecutor,
    NoOpQueryFetcher,
    ProductionEnvironment<InventoryProjector, NoOpEventBus>,
>;

/// Type alias for the Payment aggregate handler with projection support.
pub type PaymentAggregateHandler = Handler<
    PaymentBusinessLogic,
    NoOpCallExecutor,
    NoOpQueryFetcher,
    ProductionEnvironment<PaymentProjector, NoOpEventBus>,
>;

// ───────────────────────────────────────────────────────────────────────────
// Query-Enabled Handler Types
// ───────────────────────────────────────────────────────────────────────────

/// Environment type with projection queries enabled for CQRS reads.
///
/// This environment includes `EventProjectionQueries` which allows the
/// `EventQueryFetcher` to pre-fetch data for query commands.
pub type QueryEnabledEnvironment = TicketingEnvironment<
    SystemClock,
    PostgresEventStore,
    NoOpProjector,
    NoOpEventBus,
    EventProjectionQueries,
>;

/// Event handler with query support.
///
/// This handler uses `EventQueryFetcher` to pre-fetch projection data
/// before calling `process()` for query commands (GetEvent, ListMyEvents).
///
/// For write commands (Create, Publish, Cancel), the fetcher passes
/// the input through unchanged.
pub type QueryEnabledEventHandler = Handler<
    EventBusinessLogic,
    NoOpCallExecutor,
    EventQueryFetcher,
    QueryEnabledEnvironment,
>;

// ───────────────────────────────────────────────────────────────────────────
// Full Event Handler (Query + Projection)
// ───────────────────────────────────────────────────────────────────────────

/// Environment type with BOTH projector AND queries for full CQRS support.
///
/// This environment includes:
/// - `EventProjector` - Updates projection database on writes
/// - `EventProjectionQueries` - Allows query fetcher to pre-fetch data
///
/// Use this for write operations that need to validate against projections
/// (e.g., Publish needs to check event exists and is in Draft status).
pub type FullEventEnvironment = TicketingEnvironment<
    SystemClock,
    PostgresEventStore,
    EventProjector,
    NoOpEventBus,
    EventProjectionQueries,
>;

/// Full event handler with both query fetching and projection updates.
///
/// This handler:
/// - Uses `EventQueryFetcher` to pre-fetch projection data for validation
/// - Uses `EventProjector` to update projections after writes
///
/// Use this for write commands that need pre-fetched data (Publish, Cancel, Update).
pub type FullEventHandler = Handler<
    EventBusinessLogic,
    NoOpCallExecutor,
    EventQueryFetcher,
    FullEventEnvironment,
>;

/// Shared state containing handlers.
///
/// Note: The saga handlers are constructed on-demand from the aggregate handlers
/// because they require specific type parameters based on the environment.
#[derive(Clone)]
pub struct NextAppState {
    /// The event aggregate handler.
    pub event_handler: Arc<EventAggregateHandler>,
    /// The inventory aggregate handler.
    pub inventory_handler: Arc<InventoryAggregateHandler>,
    /// The payment aggregate handler.
    pub payment_handler: Arc<PaymentAggregateHandler>,
}

// ───────────────────────────────────────────────────────────────────────────
// Event Creation App State (Saga-based)
// ───────────────────────────────────────────────────────────────────────────

/// Type alias for the Event-Inventory saga environment.
///
/// Saga state is maintained transactionally in `saga_state_event_inventory` via the
/// environment's `atomic_persist` capability, so the environment's projector is a no-op.
type EventInventorySagaEnv = TicketingEnvironment<
    SystemClock,
    PostgresEventStore,
    NoOpProjector,
    NoOpEventBus,
    EventInventorySagaProjectionQueries,
>;

/// Type alias for the Event-Inventory saga handler (durable, restart-safe state).
pub type EventInventorySagaHandler = Handler<
    EventInventorySagaLogic,
    EventInventorySagaCallExecutor<PostgresEventStore>,
    EventInventorySagaQueryFetcher,
    EventInventorySagaEnv,
>;

/// State for event creation routes that use the Event-Inventory saga.
///
/// This state uses the saga to coordinate:
/// 1. Creating the Event aggregate
/// 2. Initializing Inventory for each venue section
#[derive(Clone)]
pub struct EventCreationAppState {
    /// The Event-Inventory saga handler.
    pub saga_handler: Arc<EventInventorySagaHandler>,
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

// ───────────────────────────────────────────────────────────────────────────
// Saga Request/Response Types
// ───────────────────────────────────────────────────────────────────────────

/// Request to create an event with inventory in one atomic saga.
///
/// This endpoint coordinates creating an Event aggregate and initializing
/// Inventory aggregates for each venue section.
#[derive(Debug, Deserialize)]
pub struct CreateEventWithInventoryRequest {
    /// Event title
    pub title: String,
    /// Event description
    #[allow(dead_code)]
    pub description: Option<String>,
    /// Event start time
    pub start_time: DateTime<Utc>,
    /// Venue configuration with sections
    pub venue: VenueRequest,
    /// Ticket price in dollars
    pub price: f64,
    /// Optional owner ID (for demo purposes)
    pub owner_id: Option<Uuid>,
}

/// Venue configuration for saga request.
#[derive(Debug, Deserialize)]
pub struct VenueRequest {
    /// Venue name
    pub name: String,
    /// Venue sections with capacities
    pub sections: Vec<SectionRequest>,
}

/// Section configuration for saga request.
#[derive(Debug, Deserialize)]
pub struct SectionRequest {
    /// Section name (e.g., "VIP", "General Admission")
    pub name: String,
    /// Section capacity
    pub capacity: u32,
}

/// Response after creating event with inventory.
#[derive(Debug, Serialize)]
pub struct CreateEventWithInventoryResponse {
    /// Created event ID
    pub event_id: Uuid,
    /// Sections initialized
    pub sections: Vec<String>,
    /// Success message
    pub message: String,
}

// ───────────────────────────────────────────────────────────────────────────
// Reservation Saga Request/Response Types
// ───────────────────────────────────────────────────────────────────────────

/// Request to create a ticket reservation.
///
/// This endpoint coordinates reserving seats and processing payment.
#[derive(Debug, Deserialize)]
pub struct CreateReservationRequest {
    /// Event ID to reserve seats for
    pub event_id: Uuid,
    /// Section to reserve seats in
    pub section: String,
    /// Number of seats to reserve
    pub quantity: u32,
    /// Customer ID (for demo purposes - would come from auth in production)
    pub customer_id: Option<Uuid>,
}

/// Response after creating a reservation.
#[derive(Debug, Serialize)]
pub struct CreateReservationResponse {
    /// Created reservation ID
    pub reservation_id: Uuid,
    /// Number of seats reserved
    pub seats_reserved: u32,
    /// Saga status
    pub status: String,
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
        HandlerError::Business(EventError::Forbidden) => {
            AppError::forbidden("Not authorized to access this resource")
        }
        HandlerError::Load(e) => AppError::internal(format!("Failed to load state: {e}")),
        HandlerError::Persist(e) => AppError::internal(format!("Failed to persist: {e}")),
        HandlerError::Projection(e) => AppError::internal(format!("Projection failed: {e}")),
        HandlerError::Broadcast(e) => AppError::internal(format!("Broadcast failed: {e}")),
        HandlerError::Serialization(e) => {
            AppError::internal(format!("Serialization failed: {e}"))
        }
        HandlerError::QueryFetch(e) => {
            AppError::internal(format!("Query fetch failed: {e}"))
        }
        HandlerError::SagaIterationsExceeded { max_iterations } => {
            AppError::internal(format!("Saga exceeded max iterations: {max_iterations}"))
        }
        err @ (HandlerError::DoneWithOutstandingCalls { .. }
        | HandlerError::SagaStuck
        | HandlerError::RespondInFeedbackCycle
        | HandlerError::TotalCallsExceeded { .. }) => {
            AppError::internal(format!("Durable saga error: {err}"))
        }
    }
}

/// Convert an event-inventory saga handler error to an HTTP error response.
#[allow(dead_code)] // Will be used when saga HTTP handlers are fully implemented
fn event_inventory_saga_to_app_error(err: HandlerError<SagaError>) -> AppError {
    match err {
        HandlerError::Business(SagaError::AlreadyInProgress) => {
            AppError::conflict("Saga already in progress")
        }
        HandlerError::Business(SagaError::InvalidStateTransition { from, to }) => {
            AppError::bad_request(format!("Invalid saga transition from {from:?} to {to:?}"))
        }
        HandlerError::Business(SagaError::ValidationFailed(message)) => {
            AppError::bad_request(message)
        }
        HandlerError::Load(e) => AppError::internal(format!("Failed to load saga state: {e}")),
        HandlerError::Persist(e) => AppError::internal(format!("Failed to persist saga: {e}")),
        HandlerError::Projection(e) => AppError::internal(format!("Saga projection failed: {e}")),
        HandlerError::Broadcast(e) => AppError::internal(format!("Saga broadcast failed: {e}")),
        HandlerError::Serialization(e) => {
            AppError::internal(format!("Saga serialization failed: {e}"))
        }
        HandlerError::QueryFetch(e) => {
            AppError::internal(format!("Saga query fetch failed: {e}"))
        }
        HandlerError::SagaIterationsExceeded { max_iterations } => {
            AppError::internal(format!("Saga exceeded max iterations: {max_iterations}"))
        }
        err @ (HandlerError::DoneWithOutstandingCalls { .. }
        | HandlerError::SagaStuck
        | HandlerError::RespondInFeedbackCycle
        | HandlerError::TotalCallsExceeded { .. }) => {
            AppError::internal(format!("Durable saga error: {err}"))
        }
    }
}

/// Convert a reservation saga handler error to an HTTP error response.
#[allow(dead_code)] // Will be used when saga HTTP handlers are fully implemented
fn reservation_saga_to_app_error(err: HandlerError<ReservationSagaError>) -> AppError {
    match err {
        HandlerError::Business(ReservationSagaError::AlreadyInProgress) => {
            AppError::conflict("Reservation saga already in progress")
        }
        HandlerError::Business(ReservationSagaError::InvalidStateTransition { from }) => {
            AppError::bad_request(format!("Invalid saga transition from {from:?}"))
        }
        HandlerError::Business(ReservationSagaError::MissingData(field)) => {
            AppError::internal(format!("Missing saga data: {field}"))
        }
        HandlerError::Business(ReservationSagaError::ValidationFailed(message)) => {
            AppError::bad_request(message)
        }
        HandlerError::Load(e) => AppError::internal(format!("Failed to load saga state: {e}")),
        HandlerError::Persist(e) => AppError::internal(format!("Failed to persist saga: {e}")),
        HandlerError::Projection(e) => AppError::internal(format!("Saga projection failed: {e}")),
        HandlerError::Broadcast(e) => AppError::internal(format!("Saga broadcast failed: {e}")),
        HandlerError::Serialization(e) => {
            AppError::internal(format!("Saga serialization failed: {e}"))
        }
        HandlerError::QueryFetch(e) => {
            AppError::internal(format!("Saga query fetch failed: {e}"))
        }
        HandlerError::SagaIterationsExceeded { max_iterations } => {
            AppError::internal(format!("Saga exceeded max iterations: {max_iterations}"))
        }
        err @ (HandlerError::DoneWithOutstandingCalls { .. }
        | HandlerError::SagaStuck
        | HandlerError::RespondInFeedbackCycle
        | HandlerError::TotalCallsExceeded { .. }) => {
            AppError::internal(format!("Durable saga error: {err}"))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HTTP Handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Create a new event with inventory initialization.
///
/// Uses the Event-Inventory saga to coordinate:
/// 1. Creating the Event aggregate
/// 2. Initializing Inventory for each venue section
///
/// This ensures that when an event is created, its inventory is also properly
/// initialized and ready for reservations.
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
    State(state): State<EventCreationAppState>,
    user: SessionUser,
    Json(request): Json<CreateEventRequest>,
) -> Result<(StatusCode, Json<CreateEventResponse>), AppError> {
    // Generate new event ID
    let event_id = EventId::new();

    // Map API request to domain types
    let venue = create_venue(&request);
    let date = EventDate::new(request.start_time);
    let pricing_tiers = create_pricing_tiers(&request);

    // Use authenticated user as owner (request.owner_id is deprecated, always use auth)
    let owner_id = user.0;

    // Build saga command - this will create the event AND initialize inventory
    let command = SagaInput::CreateEventWithInventory {
        event_id,
        name: request.title,
        owner_id,
        venue,
        date,
        pricing_tiers,
    };

    // Handle saga - creates event and initializes inventory for all sections
    let _result = state
        .saga_handler
        .handle(command)
        .await
        .map_err(event_inventory_saga_to_app_error)?;

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
    State(state): State<FullQueryAppState>,
) -> Result<Json<PublishEventResponse>, AppError> {
    let event_id = EventId::from_uuid(event_id);

    let command = EventCommand::Publish {
        event_id,
        fetched: None,
    };

    // Use full_event_handler which has both:
    // - EventQueryFetcher to pre-fetch event data for validation
    // - EventProjector to update projections after write
    let _result = state
        .full_event_handler
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
    State(state): State<FullQueryAppState>,
    Json(request): Json<CancelEventRequest>,
) -> Result<Json<CancelEventResponse>, AppError> {
    let event_id = EventId::from_uuid(event_id);

    let command = EventCommand::Cancel {
        event_id,
        reason: request.reason,
        fetched: None,
    };

    // Use full_event_handler which has both:
    // - EventQueryFetcher to pre-fetch event data for validation
    // - EventProjector to update projections after write
    let _result = state
        .full_event_handler
        .handle(command)
        .await
        .map_err(to_app_error)?;

    Ok(Json(CancelEventResponse {
        event_id: *event_id.as_uuid(),
        message: "Event cancelled".to_string(),
    }))
}

// ───────────────────────────────────────────────────────────────────────────
// Query Handlers
// ───────────────────────────────────────────────────────────────────────────

/// Venue response for API
#[derive(Debug, Serialize)]
pub struct VenueResponse {
    /// Venue name
    pub name: String,
    /// Venue address (placeholder for now)
    pub address: String,
    /// Venue sections
    pub sections: Vec<VenueSectionResponse>,
}

/// Venue section response for API
#[derive(Debug, Serialize)]
pub struct VenueSectionResponse {
    /// Section name
    pub name: String,
    /// Section capacity
    pub capacity: u32,
    /// Seat type: "numbered" or "general_admission"
    pub seat_type: String,
}

/// Pricing tier response for API
#[derive(Debug, Serialize)]
pub struct PricingTierResponse {
    /// Tier type: "vip", "standard", "early_bird", "group"
    pub tier_type: String,
    /// Display name for the tier
    pub name: String,
    /// Price in cents
    pub price_cents: u64,
    /// Quantity available (null for unlimited)
    pub quantity_available: Option<u32>,
}

/// Response for a single event query.
#[derive(Debug, Serialize)]
pub struct GetEventQueryResponse {
    /// Event ID
    pub id: Uuid,
    /// Event title/name
    pub title: String,
    /// Event description
    pub description: Option<String>,
    /// Owner ID
    pub owner_id: Uuid,
    /// Full venue details
    pub venue: VenueResponse,
    /// Event start time
    pub start_time: DateTime<Utc>,
    /// Event end time (null if not specified)
    pub end_time: Option<DateTime<Utc>>,
    /// Event status: "draft", "published", or "cancelled"
    pub status: String,
    /// Full pricing tiers
    pub pricing_tiers: Vec<PricingTierResponse>,
    /// When the event was created
    pub created_at: DateTime<Utc>,
    /// When the event was last updated
    pub updated_at: DateTime<Utc>,
}

impl From<EventDto> for GetEventQueryResponse {
    fn from(dto: EventDto) -> Self {
        let venue = VenueResponse {
            name: dto.venue.name.clone(),
            address: String::new(), // Placeholder - backend doesn't have address yet
            sections: dto
                .venue
                .sections
                .iter()
                .map(|s| VenueSectionResponse {
                    name: s.name.clone(),
                    capacity: s.capacity.value(),
                    seat_type: match s.seat_type {
                        SeatType::Numbered { .. } => "numbered".to_string(),
                        SeatType::GeneralAdmission => "general_admission".to_string(),
                    },
                })
                .collect(),
        };

        let pricing_tiers = dto
            .pricing_tiers
            .iter()
            .map(|p| PricingTierResponse {
                tier_type: match p.tier_type {
                    TierType::EarlyBird => "early_bird".to_string(),
                    TierType::Regular => "standard".to_string(),
                    TierType::LastMinute => "standard".to_string(), // Map to standard
                },
                name: p.section.clone(), // Use section as display name
                price_cents: p.base_price.cents(),
                quantity_available: None, // Would need inventory data
            })
            .collect();

        let now = Utc::now();
        let status = match dto.status {
            crate::types::EventStatus::Draft => "draft",
            crate::types::EventStatus::Published => "published",
            crate::types::EventStatus::Cancelled => "cancelled",
            crate::types::EventStatus::SalesOpen => "published",
            crate::types::EventStatus::SalesClosed => "published",
            crate::types::EventStatus::Completed => "cancelled",
        };

        Self {
            id: *dto.id.as_uuid(),
            title: dto.name,
            description: None, // Backend doesn't have description yet
            owner_id: dto.owner_id.0,
            venue,
            start_time: dto.date.inner(),
            end_time: None, // Backend doesn't have end_time yet
            status: status.to_string(),
            pricing_tiers,
            created_at: now, // Would need actual created_at from event
            updated_at: now, // Would need actual updated_at from event
        }
    }
}

/// Response for listing events.
#[derive(Debug, Serialize)]
pub struct ListEventsQueryResponse {
    /// List of events
    pub events: Vec<GetEventQueryResponse>,
    /// Total count
    pub total: usize,
}

/// State for query-enabled handlers.
///
/// This state includes handlers configured with `EventQueryFetcher` for
/// pre-fetching projection data during query operations.
#[derive(Clone)]
pub struct QueryAppState {
    /// Query-enabled event handler
    pub event_handler: Arc<QueryEnabledEventHandler>,
}

/// Get a single event by ID.
///
/// This query endpoint uses the CQRS read model (projections) to fetch
/// event data. The Handler pre-fetches the data before calling business
/// logic, which only does authorization.
///
/// # Request
///
/// GET /api/v2/events/:id
///
/// # Authorization
///
/// The requesting user must be the owner of the event.
///
/// # Response
///
/// ```json
/// {
///   "id": "550e8400-e29b-41d4-a716-446655440000",
///   "name": "Tech Conference 2024",
///   "owner_id": "...",
///   "venue_name": "Convention Center",
///   "date": "2024-06-01T09:00:00Z",
///   "status": "Published",
///   "pricing_tiers_count": 1
/// }
/// ```
pub async fn get_event(
    Path(event_id): Path<Uuid>,
    State(state): State<QueryAppState>,
    user: SessionUser,
) -> Result<Json<GetEventQueryResponse>, AppError> {
    let event_id = EventId::from_uuid(event_id);
    let user_id = user.0;

    // Build query command - Handler will pre-fetch the data
    let command = EventCommand::GetEvent {
        event_id,
        requesting_user_id: user_id,
        fetched: None, // Handler will populate this via EventQueryFetcher
    };

    // Handle query - business logic just does authorization and returns data
    let result = state
        .event_handler
        .handle(command)
        .await
        .map_err(to_app_error)?;

    // Extract query response
    match result.into_query_response() {
        Some(DomainEventResponse::Single(dto)) => Ok(Json(dto.into())),
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

/// List events owned by the authenticated user.
///
/// This query endpoint uses the CQRS read model (projections) to fetch
/// all events owned by the requesting user.
///
/// # Request
///
/// GET /api/v2/my-events
///
/// # Response
///
/// ```json
/// {
///   "events": [...],
///   "total": 5
/// }
/// ```
pub async fn list_my_events(
    State(state): State<QueryAppState>,
    user: SessionUser,
) -> Result<Json<ListEventsQueryResponse>, AppError> {
    let user_id = user.0;

    // Build query command - Handler will pre-fetch the data
    let command = EventCommand::ListMyEvents {
        user_id,
        fetched: Vec::new(), // Handler will populate this via EventQueryFetcher
    };

    // Handle query
    let result = state
        .event_handler
        .handle(command)
        .await
        .map_err(to_app_error)?;

    // Extract query response
    match result.into_query_response() {
        Some(DomainEventResponse::List(dtos)) => {
            let events: Vec<GetEventQueryResponse> = dtos.into_iter().map(Into::into).collect();
            let total = events.len();
            Ok(Json(ListEventsQueryResponse { events, total }))
        }
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

/// Query parameters for authentication (demo purposes).
///
/// In production, user identity would come from authentication middleware
/// (e.g., JWT token, session cookie) rather than query parameters.
#[derive(Debug, Deserialize)]
pub struct AuthQuery {
    /// User ID for authentication (demo only)
    pub user_id: Option<Uuid>,
}

/// Session user extractor that parses Bearer tokens.
///
/// Supports multiple token formats:
/// 1. Combined token: `Authorization: Bearer {session_id}:{user_id}` (preferred)
/// 2. Test token: `Authorization: Bearer test-user-{uuid}`
/// 3. Query parameter fallback: `?user_id={uuid}` (for backwards compatibility)
///
/// The combined token format allows the backend to use the persistent `user_id`
/// for lookups (e.g., reservations) while still having access to the `session_id`
/// for potential session validation.
///
/// # Example
///
/// ```ignore
/// async fn handler(user: SessionUser) -> impl IntoResponse {
///     format!("User: {}", user.0.0)
/// }
/// ```
#[derive(Debug, Clone)]
pub struct SessionUser(pub UserId);

impl<S> FromRequestParts<S> for SessionUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Try Bearer token first
        if let Some(auth_header) = parts.headers.get("authorization") {
            if let Ok(auth_str) = auth_header.to_str() {
                if let Some(token) = auth_str.strip_prefix("Bearer ") {
                    // Check for test-user-{uuid} pattern
                    if let Some(uuid_str) = token.strip_prefix("test-user-") {
                        if let Ok(uuid) = Uuid::parse_str(uuid_str) {
                            return Ok(Self(UserId(uuid)));
                        }
                    }
                    // Check for combined token format: "session_id:user_id"
                    // This is the format returned by magic link verification
                    if let Some((_session_part, user_part)) = token.split_once(':') {
                        if let Ok(user_uuid) = Uuid::parse_str(user_part) {
                            return Ok(Self(UserId(user_uuid)));
                        }
                    }
                    // Legacy: Try parsing the whole token as UUID
                    // (for backwards compatibility with old tokens)
                    if let Ok(uuid) = Uuid::parse_str(token) {
                        return Ok(Self(UserId(uuid)));
                    }
                }
            }
        }

        // Fallback to query parameter
        let query = parts.uri.query().unwrap_or("");
        for param in query.split('&') {
            if let Some(value) = param.strip_prefix("user_id=") {
                if let Ok(uuid) = Uuid::parse_str(value) {
                    return Ok(Self(UserId(uuid)));
                }
            }
        }

        Err(AppError::unauthorized("Authentication required: provide Bearer token or user_id query parameter"))
    }
}

/// Optional `Idempotency-Key` request header.
///
/// When present, callers get idempotent command initiation (see `create_reservation`).
/// Absent → `None` (non-idempotent, fresh id per request).
#[derive(Debug, Clone)]
pub struct IdempotencyKey(pub Option<String>);

impl<S> FromRequestParts<S> for IdempotencyKey
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let key = parts
            .headers
            .get("idempotency-key")
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
        Ok(Self(key))
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Additional Event Handlers
// ───────────────────────────────────────────────────────────────────────────

/// Request to update an event.
#[derive(Debug, Deserialize)]
pub struct UpdateEventRequest {
    /// New event name (optional)
    pub name: Option<String>,
    /// New venue name (optional)
    pub venue_name: Option<String>,
    /// New event date (optional)
    pub date: Option<DateTime<Utc>>,
}

/// Response after updating an event.
#[derive(Debug, Serialize)]
pub struct UpdateEventResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Success message
    pub message: String,
}

/// Update an event.
///
/// PUT /api/v2/events/:id
pub async fn update_event(
    Path(event_id): Path<Uuid>,
    State(state): State<FullQueryAppState>,
    user: SessionUser,
    Json(request): Json<UpdateEventRequest>,
) -> Result<Json<UpdateEventResponse>, AppError> {
    let event_id = EventId::from_uuid(event_id);
    let requesting_user_id = user.0;

    // Build optional venue update
    let venue = request.venue_name.map(|name| {
        Venue::new(name, Capacity::new(0), vec![]) // Capacity managed separately
    });

    let date = request.date.map(EventDate::new);

    let command = EventCommand::Update {
        event_id,
        requesting_user_id,
        name: request.name,
        venue,
        date,
        fetched: None,
    };

    let _result = state
        .query_event_handler
        .handle(command)
        .await
        .map_err(to_app_error)?;

    Ok(Json(UpdateEventResponse {
        event_id: *event_id.as_uuid(),
        message: "Event updated successfully".to_string(),
    }))
}

/// Delete (cancel) an event.
///
/// DELETE /api/v2/events/:id
pub async fn delete_event(
    Path(event_id): Path<Uuid>,
    State(state): State<FullQueryAppState>,
    user: SessionUser,
) -> Result<StatusCode, AppError> {
    let event_id = EventId::from_uuid(event_id);
    let requesting_user_id = user.0;

    let command = EventCommand::Delete {
        event_id,
        requesting_user_id,
        fetched: None,
    };

    let _result = state
        .query_event_handler
        .handle(command)
        .await
        .map_err(to_app_error)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Response for listing all events with pagination.
#[derive(Debug, Serialize)]
pub struct ListAllEventsResponse {
    /// List of events
    pub events: Vec<GetEventQueryResponse>,
    /// Total count (before pagination)
    pub total: usize,
    /// Current page
    pub page: usize,
    /// Page size
    pub page_size: usize,
}

/// Query params for listing events.
#[derive(Debug, Deserialize)]
pub struct ListEventsQuery {
    /// Optional status filter
    pub status: Option<String>,
    /// Page number (0-indexed)
    #[serde(default)]
    pub page: usize,
    /// Page size (default 20)
    #[serde(default = "default_page_size")]
    pub page_size: usize,
}

fn default_page_size() -> usize {
    20
}

/// List all events with optional filtering and pagination.
///
/// GET /api/v2/events
pub async fn list_events(
    State(state): State<QueryAppState>,
    axum::extract::Query(query): axum::extract::Query<ListEventsQuery>,
) -> Result<Json<ListAllEventsResponse>, AppError> {
    use crate::types::EventStatus;

    let status_filter = query.status.as_deref().and_then(|s| match s {
        "draft" => Some(EventStatus::Draft),
        "published" => Some(EventStatus::Published),
        "cancelled" => Some(EventStatus::Cancelled),
        _ => None,
    });

    let command = EventCommand::ListEvents {
        status_filter,
        page: query.page,
        page_size: query.page_size,
        fetched: (Vec::new(), 0),
    };

    let result = state
        .event_handler
        .handle(command)
        .await
        .map_err(to_app_error)?;

    match result.into_query_response() {
        Some(DomainEventResponse::Paginated {
            events,
            total,
            page,
            page_size,
        }) => {
            let events: Vec<GetEventQueryResponse> = events.into_iter().map(Into::into).collect();
            Ok(Json(ListAllEventsResponse {
                events,
                total,
                page,
                page_size,
            }))
        }
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

/// Response for pricing query.
#[derive(Debug, Serialize)]
pub struct GetPricingResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Pricing tiers
    pub pricing_tiers: Vec<PricingTierResponse>,
}

// PricingTierResponse is defined above with the other response types

/// Get pricing tiers for an event.
///
/// GET /api/v2/events/:id/pricing
pub async fn get_event_pricing(
    Path(event_id): Path<Uuid>,
    State(state): State<QueryAppState>,
) -> Result<Json<GetPricingResponse>, AppError> {
    let event_id = EventId::from_uuid(event_id);

    let command = EventCommand::GetEventPricing {
        event_id,
        fetched: None,
    };

    let result = state
        .event_handler
        .handle(command)
        .await
        .map_err(to_app_error)?;

    match result.into_query_response() {
        Some(DomainEventResponse::Pricing {
            event_id,
            pricing_tiers,
        }) => {
            let tiers: Vec<PricingTierResponse> = pricing_tiers
                .into_iter()
                .map(|t| PricingTierResponse {
                    tier_type: match t.tier_type {
                        TierType::EarlyBird => "early_bird".to_string(),
                        TierType::Regular => "standard".to_string(),
                        TierType::LastMinute => "standard".to_string(),
                    },
                    name: t.section.clone(),
                    price_cents: t.base_price.cents(),
                    quantity_available: None,
                })
                .collect();
            Ok(Json(GetPricingResponse {
                event_id: *event_id.as_uuid(),
                pricing_tiers: tiers,
            }))
        }
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

/// Request to update pricing.
#[derive(Debug, Deserialize)]
pub struct UpdatePricingRequest {
    /// New pricing tiers
    pub pricing_tiers: Vec<PricingTierRequest>,
}

/// Pricing tier in request.
#[derive(Debug, Deserialize)]
pub struct PricingTierRequest {
    /// Tier type (Regular, EarlyBird, LastMinute)
    pub tier_type: String,
    /// Section name
    pub section: String,
    /// Price in dollars
    pub price: f64,
}

/// Update pricing tiers for an event.
///
/// PATCH /api/v2/events/:id/pricing
pub async fn update_event_pricing(
    Path(event_id): Path<Uuid>,
    State(state): State<FullQueryAppState>,
    user: SessionUser,
    Json(request): Json<UpdatePricingRequest>,
) -> Result<Json<UpdateEventResponse>, AppError> {
    let event_id = EventId::from_uuid(event_id);
    let requesting_user_id = user.0;

    let pricing_tiers: Vec<PricingTier> = request
        .pricing_tiers
        .into_iter()
        .map(|t| {
            let tier_type = match t.tier_type.as_str() {
                "EarlyBird" => TierType::EarlyBird,
                "LastMinute" => TierType::LastMinute,
                _ => TierType::Regular,
            };
            #[allow(clippy::cast_possible_truncation)]
            PricingTier::new(
                tier_type,
                t.section,
                Money::from_dollars(t.price as u64),
                Utc::now(),
                None,
            )
        })
        .collect();

    let command = EventCommand::UpdatePricing {
        event_id,
        requesting_user_id,
        pricing_tiers,
        fetched: None,
    };

    let _result = state
        .query_event_handler
        .handle(command)
        .await
        .map_err(to_app_error)?;

    Ok(Json(UpdateEventResponse {
        event_id: *event_id.as_uuid(),
        message: "Pricing updated successfully".to_string(),
    }))
}

/// Request to add venue sections.
#[derive(Debug, Deserialize)]
pub struct AddSectionsRequest {
    /// Sections to add
    pub sections: Vec<SectionRequest>,
}

/// Add venue sections to an event.
///
/// POST /api/v2/events/:id/sections
pub async fn add_venue_sections(
    Path(event_id): Path<Uuid>,
    State(state): State<FullQueryAppState>,
    user: SessionUser,
    Json(request): Json<AddSectionsRequest>,
) -> Result<Json<UpdateEventResponse>, AppError> {
    let event_id = EventId::from_uuid(event_id);
    let requesting_user_id = user.0;

    let sections: Vec<VenueSection> = request
        .sections
        .into_iter()
        .map(|s| VenueSection::new(s.name, Capacity::new(s.capacity), SeatType::GeneralAdmission))
        .collect();

    let command = EventCommand::AddVenueSections {
        event_id,
        requesting_user_id,
        sections,
        fetched: None,
    };

    let _result = state
        .query_event_handler
        .handle(command)
        .await
        .map_err(to_app_error)?;

    Ok(Json(UpdateEventResponse {
        event_id: *event_id.as_uuid(),
        message: "Sections added successfully".to_string(),
    }))
}

// ───────────────────────────────────────────────────────────────────────────
// Availability (Inventory) Handlers
// ───────────────────────────────────────────────────────────────────────────

use super::{
    inventory::{InventoryCommand, InventoryError, InventoryResponse, SectionAvailabilityDto},
    payment::{
        PaymentCommand, PaymentDto, PaymentDtoStatus, PaymentError,
        PaymentResponse as DomainPaymentResponse,
    },
    projection_queries::{InventoryProjectionQueries, InventoryQueryFetcher, PaymentProjectionQueries, PaymentQueryFetcher},
};
use crate::types::{PaymentId, PaymentMethod};

// ───────────────────────────────────────────────────────────────────────────
// Availability Query State
// ───────────────────────────────────────────────────────────────────────────

/// Environment type with inventory projection queries enabled.
pub type InventoryQueryEnvironment = TicketingEnvironment<
    SystemClock,
    PostgresEventStore,
    NoOpProjector,
    NoOpEventBus,
    InventoryProjectionQueries,
>;

/// Inventory handler with query support.
pub type QueryEnabledInventoryHandler = Handler<
    InventoryBusinessLogic,
    NoOpCallExecutor,
    InventoryQueryFetcher,
    InventoryQueryEnvironment,
>;

/// Environment type with payment projection queries enabled.
/// Uses `PaymentProjector` to project payment events to the read model.
pub type PaymentQueryEnvironment = TicketingEnvironment<
    SystemClock,
    PostgresEventStore,
    PaymentProjector,
    NoOpEventBus,
    PaymentProjectionQueries,
>;

/// Payment handler with query support.
pub type QueryEnabledPaymentHandler = Handler<
    PaymentBusinessLogic,
    NoOpCallExecutor,
    PaymentQueryFetcher,
    PaymentQueryEnvironment,
>;

/// Extended state including all query-enabled handlers.
#[derive(Clone)]
pub struct FullQueryAppState {
    /// Event handler (for commands without pre-fetch needs)
    pub event_handler: Arc<EventAggregateHandler>,
    /// Query-enabled event handler (reads only)
    pub query_event_handler: Arc<QueryEnabledEventHandler>,
    /// Full event handler (writes that need pre-fetch + projection)
    pub full_event_handler: Arc<FullEventHandler>,
    /// Inventory handler (for commands)
    pub inventory_handler: Arc<InventoryAggregateHandler>,
    /// Query-enabled inventory handler
    pub query_inventory_handler: Arc<QueryEnabledInventoryHandler>,
    /// Payment handler (for commands)
    pub payment_handler: Arc<PaymentAggregateHandler>,
    /// Query-enabled payment handler
    pub query_payment_handler: Arc<QueryEnabledPaymentHandler>,
}

// ───────────────────────────────────────────────────────────────────────────
// Availability Response Types
// ───────────────────────────────────────────────────────────────────────────

/// Response for section availability.
#[derive(Debug, Serialize)]
pub struct SectionAvailabilityResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Section name
    pub section: String,
    /// Total capacity
    pub total_capacity: u32,
    /// Available seats
    pub available_seats: u32,
    /// Reserved seats (pending payment)
    pub reserved_seats: u32,
    /// Sold seats (confirmed)
    pub sold_seats: u32,
}

impl From<SectionAvailabilityDto> for SectionAvailabilityResponse {
    fn from(dto: SectionAvailabilityDto) -> Self {
        Self {
            event_id: *dto.event_id.as_uuid(),
            section: dto.section,
            total_capacity: dto.total_capacity,
            available_seats: dto.available_seats,
            reserved_seats: dto.reserved_seats,
            sold_seats: dto.sold_seats,
        }
    }
}

/// Response for event availability (all sections).
#[derive(Debug, Serialize)]
pub struct EventAvailabilityResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Sections with availability
    pub sections: Vec<SectionAvailabilityResponse>,
    /// Total available across all sections
    pub total_available: u32,
}

/// Response for total available seats.
#[derive(Debug, Serialize)]
pub struct TotalAvailableResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Total available seats
    pub total_available: u32,
}

// ───────────────────────────────────────────────────────────────────────────
// Availability Handlers
// ───────────────────────────────────────────────────────────────────────────

/// Convert inventory error to app error.
fn inventory_to_app_error(err: HandlerError<InventoryError>) -> AppError {
    match err {
        HandlerError::Business(InventoryError::AlreadyInitialized) => {
            AppError::conflict("Inventory already initialized")
        }
        HandlerError::Business(InventoryError::NotInitialized) => {
            AppError::not_found("Inventory", "section")
        }
        HandlerError::Business(InventoryError::InsufficientInventory { requested, available }) => {
            AppError::bad_request(format!(
                "Insufficient inventory: requested {requested}, available {available}"
            ))
        }
        HandlerError::Business(InventoryError::ReservationNotFound(id)) => {
            AppError::not_found("Reservation", id.to_string())
        }
        HandlerError::Business(InventoryError::ValidationFailed(msg)) => {
            AppError::bad_request(msg)
        }
        HandlerError::Load(e) => AppError::internal(format!("Failed to load: {e}")),
        HandlerError::Persist(e) => AppError::internal(format!("Failed to persist: {e}")),
        HandlerError::Projection(e) => AppError::internal(format!("Projection failed: {e}")),
        HandlerError::Broadcast(e) => AppError::internal(format!("Broadcast failed: {e}")),
        HandlerError::Serialization(e) => AppError::internal(format!("Serialization failed: {e}")),
        HandlerError::QueryFetch(e) => AppError::internal(format!("Query fetch failed: {e}")),
        HandlerError::SagaIterationsExceeded { max_iterations } => {
            AppError::internal(format!("Saga exceeded max iterations: {max_iterations}"))
        }
        err @ (HandlerError::DoneWithOutstandingCalls { .. }
        | HandlerError::SagaStuck
        | HandlerError::RespondInFeedbackCycle
        | HandlerError::TotalCallsExceeded { .. }) => {
            AppError::internal(format!("Durable saga error: {err}"))
        }
    }
}

/// Get availability for a specific section.
///
/// GET /api/v2/events/:event_id/sections/:section/availability
pub async fn get_section_availability(
    Path((event_id, section)): Path<(Uuid, String)>,
    State(state): State<FullQueryAppState>,
) -> Result<Json<SectionAvailabilityResponse>, AppError> {
    let event_id = EventId::from_uuid(event_id);

    let command = InventoryCommand::GetSectionAvailability {
        event_id,
        section,
        fetched: None,
    };

    let result = state
        .query_inventory_handler
        .handle(command)
        .await
        .map_err(inventory_to_app_error)?;

    match result.into_query_response() {
        Some(InventoryResponse::SectionAvailability(dto)) => Ok(Json(dto.into())),
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

/// Get availability for all sections of an event.
///
/// GET /api/v2/events/:event_id/availability
pub async fn get_event_availability(
    Path(event_id): Path<Uuid>,
    State(state): State<FullQueryAppState>,
) -> Result<Json<EventAvailabilityResponse>, AppError> {
    let event_id_typed = EventId::from_uuid(event_id);

    let command = InventoryCommand::GetEventAvailability {
        event_id: event_id_typed,
        fetched: Vec::new(),
    };

    let result = state
        .query_inventory_handler
        .handle(command)
        .await
        .map_err(inventory_to_app_error)?;

    match result.into_query_response() {
        Some(InventoryResponse::EventAvailability(dtos)) => {
            let total_available: u32 = dtos.iter().map(|d| d.available_seats).sum();
            let sections: Vec<SectionAvailabilityResponse> =
                dtos.into_iter().map(Into::into).collect();
            Ok(Json(EventAvailabilityResponse {
                event_id,
                sections,
                total_available,
            }))
        }
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

/// Get total available seats across all sections.
///
/// GET /api/v2/events/:event_id/total-available
pub async fn get_total_available(
    Path(event_id): Path<Uuid>,
    State(state): State<FullQueryAppState>,
) -> Result<Json<TotalAvailableResponse>, AppError> {
    let event_id_typed = EventId::from_uuid(event_id);

    let command = InventoryCommand::GetTotalAvailable {
        event_id: event_id_typed,
        fetched: 0,
    };

    let result = state
        .query_inventory_handler
        .handle(command)
        .await
        .map_err(inventory_to_app_error)?;

    match result.into_query_response() {
        Some(InventoryResponse::TotalAvailable(total)) => Ok(Json(TotalAvailableResponse {
            event_id,
            total_available: total,
        })),
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Payment Handlers
// ───────────────────────────────────────────────────────────────────────────

/// Convert payment error to app error.
fn payment_to_app_error(err: HandlerError<PaymentError>) -> AppError {
    match err {
        HandlerError::Business(PaymentError::AlreadyExists(id)) => {
            AppError::conflict(format!("Payment already exists: {id}"))
        }
        HandlerError::Business(PaymentError::NotFound(id)) => {
            AppError::not_found("Payment", id.to_string())
        }
        HandlerError::Business(PaymentError::PaymentNotFound) => {
            AppError::not_found("Payment", "unknown")
        }
        HandlerError::Business(PaymentError::InvalidAmount(msg)) => {
            AppError::bad_request(format!("Invalid amount: {msg}"))
        }
        HandlerError::Business(PaymentError::CannotRefund(msg)) => {
            AppError::bad_request(format!("Cannot refund: {msg}"))
        }
        HandlerError::Business(PaymentError::ValidationFailed(msg)) => {
            AppError::bad_request(msg)
        }
        HandlerError::Business(PaymentError::PaymentDeclined(reason)) => {
            AppError::bad_request(format!("Payment declined: {reason}"))
        }
        HandlerError::Load(e) => AppError::internal(format!("Failed to load: {e}")),
        HandlerError::Persist(e) => AppError::internal(format!("Failed to persist: {e}")),
        HandlerError::Projection(e) => AppError::internal(format!("Projection failed: {e}")),
        HandlerError::Broadcast(e) => AppError::internal(format!("Broadcast failed: {e}")),
        HandlerError::Serialization(e) => AppError::internal(format!("Serialization failed: {e}")),
        HandlerError::QueryFetch(e) => AppError::internal(format!("Query fetch failed: {e}")),
        HandlerError::SagaIterationsExceeded { max_iterations } => {
            AppError::internal(format!("Saga exceeded max iterations: {max_iterations}"))
        }
        err @ (HandlerError::DoneWithOutstandingCalls { .. }
        | HandlerError::SagaStuck
        | HandlerError::RespondInFeedbackCycle
        | HandlerError::TotalCallsExceeded { .. }) => {
            AppError::internal(format!("Durable saga error: {err}"))
        }
    }
}

/// Request to process a payment.
#[derive(Debug, Deserialize)]
pub struct ProcessPaymentRequest {
    /// Reservation ID
    pub reservation_id: Uuid,
    /// Customer ID
    pub customer_id: Uuid,
    /// Amount in cents
    pub amount_cents: u64,
    /// Payment method type
    pub payment_method: PaymentMethodRequest,
}

/// Payment method in request.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum PaymentMethodRequest {
    /// Credit card
    #[serde(rename = "credit_card")]
    CreditCard {
        /// Last four digits
        last_four: String,
    },
    /// PayPal
    #[serde(rename = "paypal")]
    PayPal {
        /// PayPal email
        email: String,
    },
    /// Apple Pay
    #[serde(rename = "apple_pay")]
    ApplePay,
}

impl From<PaymentMethodRequest> for PaymentMethod {
    fn from(req: PaymentMethodRequest) -> Self {
        match req {
            PaymentMethodRequest::CreditCard { last_four } => {
                PaymentMethod::CreditCard { last_four }
            }
            PaymentMethodRequest::PayPal { email } => PaymentMethod::PayPal { email },
            PaymentMethodRequest::ApplePay => PaymentMethod::ApplePay,
        }
    }
}

/// Response after processing a payment.
#[derive(Debug, Serialize)]
pub struct ProcessPaymentResponse {
    /// Payment ID
    pub payment_id: Uuid,
    /// Payment status
    pub status: String,
    /// Success message
    pub message: String,
}

/// Response for payment details.
#[derive(Debug, Serialize)]
pub struct PaymentDetailsResponse {
    /// Payment ID
    pub id: Uuid,
    /// Reservation ID
    pub reservation_id: Uuid,
    /// Customer ID
    pub customer_id: Uuid,
    /// Amount in cents
    pub amount_cents: u64,
    /// Payment method
    pub payment_method: String,
    /// Status
    pub status: String,
    /// Transaction ID (if succeeded)
    pub transaction_id: Option<String>,
    /// Failure reason (if failed)
    pub failure_reason: Option<String>,
}

impl From<PaymentDto> for PaymentDetailsResponse {
    fn from(dto: PaymentDto) -> Self {
        Self {
            id: *dto.id.as_uuid(),
            reservation_id: *dto.reservation_id.as_uuid(),
            customer_id: *dto.customer_id.as_uuid(),
            amount_cents: dto.amount.cents(),
            payment_method: dto.payment_method,
            status: match dto.status {
                PaymentDtoStatus::Processing => "processing".to_string(),
                PaymentDtoStatus::Succeeded => "succeeded".to_string(),
                PaymentDtoStatus::Failed => "failed".to_string(),
                PaymentDtoStatus::Refunded => "refunded".to_string(),
            },
            transaction_id: dto.transaction_id,
            failure_reason: dto.failure_reason,
        }
    }
}

/// Response for listing payments.
#[derive(Debug, Serialize)]
pub struct ListPaymentsResponse {
    /// Payments
    pub payments: Vec<PaymentDetailsResponse>,
    /// Total count
    pub total: usize,
}

/// Request to refund a payment.
#[derive(Debug, Deserialize)]
pub struct RefundPaymentRequest {
    /// Amount to refund in cents
    pub amount_cents: u64,
    /// Reason for refund
    pub reason: String,
}

/// Process a new payment.
///
/// POST /api/v2/payments
pub async fn process_payment(
    State(state): State<FullQueryAppState>,
    Json(request): Json<ProcessPaymentRequest>,
) -> Result<(StatusCode, Json<ProcessPaymentResponse>), AppError> {
    let payment_id = PaymentId::new();

    let command = PaymentCommand::ProcessPayment {
        payment_id,
        reservation_id: ReservationId::from_uuid(request.reservation_id),
        customer_id: CustomerId::from_uuid(request.customer_id),
        amount: Money::from_cents(request.amount_cents),
        payment_method: request.payment_method.into(),
        fetched: None,
    };

    let _result = state
        .payment_handler
        .handle(command)
        .await
        .map_err(payment_to_app_error)?;

    Ok((
        StatusCode::CREATED,
        Json(ProcessPaymentResponse {
            payment_id: *payment_id.as_uuid(),
            status: "succeeded".to_string(),
            message: "Payment processed successfully".to_string(),
        }),
    ))
}

/// Get payment details.
///
/// GET /api/v2/payments/:payment_id
pub async fn get_payment(
    Path(payment_id): Path<Uuid>,
    State(state): State<FullQueryAppState>,
) -> Result<Json<PaymentDetailsResponse>, AppError> {
    let payment_id = PaymentId::from_uuid(payment_id);

    let command = PaymentCommand::GetPayment {
        payment_id,
        fetched: None,
    };

    let result = state
        .query_payment_handler
        .handle(command)
        .await
        .map_err(payment_to_app_error)?;

    match result.into_query_response() {
        Some(DomainPaymentResponse::Single(dto)) => Ok(Json(dto.into())),
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

/// List payments for a customer.
///
/// GET /api/v2/customers/:customer_id/payments
pub async fn list_customer_payments(
    Path(customer_id): Path<Uuid>,
    State(state): State<FullQueryAppState>,
) -> Result<Json<ListPaymentsResponse>, AppError> {
    let customer_id = CustomerId::from_uuid(customer_id);

    let command = PaymentCommand::ListCustomerPayments {
        customer_id,
        fetched: Vec::new(),
    };

    let result = state
        .query_payment_handler
        .handle(command)
        .await
        .map_err(payment_to_app_error)?;

    match result.into_query_response() {
        Some(DomainPaymentResponse::List(dtos)) => {
            let total = dtos.len();
            let payments: Vec<PaymentDetailsResponse> =
                dtos.into_iter().map(Into::into).collect();
            Ok(Json(ListPaymentsResponse { payments, total }))
        }
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

/// Refund a payment.
///
/// POST /api/v2/payments/:payment_id/refund
pub async fn refund_payment(
    Path(payment_id): Path<Uuid>,
    State(state): State<FullQueryAppState>,
    Json(request): Json<RefundPaymentRequest>,
) -> Result<Json<ProcessPaymentResponse>, AppError> {
    let payment_id = PaymentId::from_uuid(payment_id);

    let command = PaymentCommand::RefundPayment {
        payment_id,
        amount: Money::from_cents(request.amount_cents),
        reason: request.reason,
        fetched: None,
    };

    let _result = state
        .payment_handler
        .handle(command)
        .await
        .map_err(payment_to_app_error)?;

    Ok(Json(ProcessPaymentResponse {
        payment_id: *payment_id.as_uuid(),
        status: "refunded".to_string(),
        message: "Payment refunded successfully".to_string(),
    }))
}

// ───────────────────────────────────────────────────────────────────────────
// Health/Metrics Handlers
// ───────────────────────────────────────────────────────────────────────────

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    /// Service status
    pub status: String,
    /// Service version
    pub version: String,
}

/// Health check endpoint.
///
/// GET /health
pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Readiness check endpoint.
///
/// GET /ready
pub async fn readiness_check() -> StatusCode {
    // In production, would check database connections, etc.
    StatusCode::OK
}

// ───────────────────────────────────────────────────────────────────────────
// Reservation Saga Handlers
// ───────────────────────────────────────────────────────────────────────────

use super::{
    ReservationSagaCallExecutor, ReservationSagaInput, ReservationSagaLogic, ReservationSagaPhase,
    ReservationSagaProjectionQueries, ReservationSagaQueryFetcher, ReservationSagaState,
};

/// Type alias for the Reservation Saga environment.
///
/// The saga's read model is updated transactionally via the environment's
/// `atomic_persist` capability, so the environment's projector is a no-op.
type ReservationSagaEnv = TicketingEnvironment<
    SystemClock,
    PostgresEventStore,
    NoOpProjector,
    NoOpEventBus,
    ReservationSagaProjectionQueries,
>;

/// Type alias for the inventory handler used in reservation saga.
///
/// Uses `InventoryQueryFetcher` to pre-populate `fetched` fields on write commands.
pub type ReservationInventoryHandler = Handler<
    InventoryBusinessLogic,
    NoOpCallExecutor,
    InventoryQueryFetcher,
    InventoryQueryEnvironment,
>;

/// Type alias for the payment handler used in reservation saga.
///
/// Uses `PaymentQueryFetcher` to pre-populate `fetched` fields on write commands.
pub type ReservationPaymentHandler = Handler<
    PaymentBusinessLogic,
    NoOpCallExecutor,
    PaymentQueryFetcher,
    PaymentQueryEnvironment,
>;

/// Type alias for the Reservation Saga Handler.
///
/// The saga handler requires:
/// - `ReservationSagaLogic` - pure business logic
/// - `ReservationSagaCallExecutor` - dispatches calls to Inventory/Payment handlers with real query fetchers
/// - `ReservationSagaQueryFetcher` - fetches saga state for feedback loops
/// - `ReservationSagaEnv` - environment with the in-memory saga projection
pub type ReservationSagaHandler = Handler<
    ReservationSagaLogic,
    ReservationSagaCallExecutor<PostgresEventStore>,
    ReservationSagaQueryFetcher,
    ReservationSagaEnv,
>;

/// App state for reservation saga endpoints.
#[derive(Clone)]
pub struct ReservationAppState {
    /// The saga handler
    pub saga_handler: Arc<ReservationSagaHandler>,
}

// ───────────────────────────────────────────────────────────────────────────
// Reservation Status Response (for querying saga state)
// ───────────────────────────────────────────────────────────────────────────

/// Response for reservation status query.
#[derive(Debug, Serialize)]
pub struct ReservationStatusResponse {
    /// Reservation ID
    pub reservation_id: Uuid,
    /// Current status
    pub status: String,
    /// Event ID
    pub event_id: Option<Uuid>,
    /// Section
    pub section: Option<String>,
    /// Quantity
    pub quantity: u32,
    /// Number of seats allocated
    pub seats_count: usize,
    /// Total amount (if calculated)
    pub total_amount_cents: Option<u64>,
    /// Expiration time
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Ticket IDs (if completed)
    pub tickets: Vec<Uuid>,
    /// Error message (if failed)
    pub error: Option<String>,
}

/// Convert `ReservationSagaPhase` to string.
fn saga_phase_to_string(phase: &ReservationSagaPhase) -> String {
    match phase {
        ReservationSagaPhase::Initial => "initial".to_string(),
        ReservationSagaPhase::ReservingSeats => "reserving_seats".to_string(),
        ReservationSagaPhase::AwaitingPayment => "awaiting_payment".to_string(),
        ReservationSagaPhase::ProcessingPayment => "processing_payment".to_string(),
        ReservationSagaPhase::Completed => "completed".to_string(),
        ReservationSagaPhase::Compensating => "compensating".to_string(),
        ReservationSagaPhase::Failed => "failed".to_string(),
    }
}

impl From<&ReservationSagaState> for ReservationStatusResponse {
    fn from(state: &ReservationSagaState) -> Self {
        Self {
            reservation_id: state
                .reservation_id
                .map(|id| *id.as_uuid())
                .unwrap_or_default(),
            status: saga_phase_to_string(&state.phase),
            event_id: state.event_id.map(|id| *id.as_uuid()),
            section: state.section.clone(),
            quantity: state.quantity,
            seats_count: state.seats.len(),
            total_amount_cents: state.total_amount.map(|m| m.cents()),
            expires_at: state.expires_at,
            tickets: state.tickets.iter().map(|t| *t.as_uuid()).collect(),
            error: state.last_error.clone(),
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Reservation Error Handling
// ───────────────────────────────────────────────────────────────────────────

/// Convert saga error to app error.
fn saga_to_app_error(err: HandlerError<ReservationSagaError>) -> AppError {
    match err {
        HandlerError::Business(ReservationSagaError::AlreadyInProgress) => {
            AppError::conflict("Reservation already in progress")
        }
        HandlerError::Business(ReservationSagaError::InvalidStateTransition { from }) => {
            AppError::bad_request(format!("Invalid state transition from {from:?}"))
        }
        HandlerError::Business(ReservationSagaError::ValidationFailed(msg)) => {
            AppError::bad_request(msg)
        }
        HandlerError::Business(ReservationSagaError::MissingData(field)) => {
            AppError::internal(format!("Missing required data: {field}"))
        }
        HandlerError::Load(e) => AppError::internal(format!("Failed to load: {e}")),
        HandlerError::Persist(e) => AppError::internal(format!("Failed to persist: {e}")),
        HandlerError::Projection(e) => AppError::internal(format!("Projection failed: {e}")),
        HandlerError::Broadcast(e) => AppError::internal(format!("Broadcast failed: {e}")),
        HandlerError::Serialization(e) => AppError::internal(format!("Serialization failed: {e}")),
        HandlerError::QueryFetch(e) => AppError::internal(format!("Query fetch failed: {e}")),
        HandlerError::SagaIterationsExceeded { max_iterations } => {
            AppError::internal(format!("Saga exceeded max iterations: {max_iterations}"))
        }
        err @ (HandlerError::DoneWithOutstandingCalls { .. }
        | HandlerError::SagaStuck
        | HandlerError::RespondInFeedbackCycle
        | HandlerError::TotalCallsExceeded { .. }) => {
            AppError::internal(format!("Durable saga error: {err}"))
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Reservation HTTP Handlers
// ───────────────────────────────────────────────────────────────────────────

/// Create a new reservation.
///
/// POST /api/v2/reservations
///
/// This initiates the reservation saga which will:
/// 1. Reserve seats in the inventory
/// 2. Wait for payment (5 minute timeout)
/// 3. On payment success: confirm seats and issue tickets
/// 4. On payment failure or timeout: release seats
pub async fn create_reservation(
    State(state): State<ReservationAppState>,
    user: SessionUser,
    idempotency_key: IdempotencyKey,
    Json(request): Json<CreateReservationRequest>,
) -> Result<(StatusCode, Json<CreateReservationResponse>), AppError> {
    // Get customer_id from auth or request (request is for backwards compatibility)
    let customer_id = request
        .customer_id
        .map(CustomerId::from_uuid)
        .unwrap_or_else(|| CustomerId::from_uuid(user.0.0));

    // With an `Idempotency-Key`, derive a deterministic reservation id so retries map
    // to the SAME saga stream; create-time optimistic concurrency then dedups them.
    // The key is scoped per-customer, so two customers reusing the same key never
    // collide onto one reservation. Without a key, mint a fresh id (non-idempotent,
    // prior behavior). Dedup hits are not separately audited — the deterministic id is
    // the only record.
    let reservation_id = match idempotency_key.0.as_deref() {
        Some(key) => ReservationId::from_uuid(Uuid::new_v5(
            &RESERVATION_IDEMPOTENCY_NAMESPACE,
            format!("reservations.create:{}:{key}", customer_id.as_uuid()).as_bytes(),
        )),
        None => ReservationId::new(),
    };

    let input = ReservationSagaInput::InitiateReservation {
        reservation_id,
        event_id: EventId::from_uuid(request.event_id),
        customer_id,
        section: request.section.clone(),
        quantity: request.quantity,
    };

    match state.saga_handler.handle(input).await {
        Ok(_) => Ok((
            StatusCode::CREATED,
            Json(CreateReservationResponse {
                reservation_id: *reservation_id.as_uuid(),
                seats_reserved: request.quantity,
                status: "initiated".to_string(),
                message: "Reservation initiated. Seats are being reserved.".to_string(),
            }),
        )),
        // Idempotent replay: the same key resolved to a stream that already exists.
        Err(HandlerError::Persist(EventStoreError::VersionConflict { .. }))
            if idempotency_key.0.is_some() =>
        {
            Ok((
                StatusCode::OK,
                Json(CreateReservationResponse {
                    reservation_id: *reservation_id.as_uuid(),
                    seats_reserved: request.quantity,
                    status: "already_initiated".to_string(),
                    message: "Reservation already initiated for this idempotency key."
                        .to_string(),
                }),
            ))
        }
        Err(e) => Err(saga_to_app_error(e)),
    }
}

/// Namespace for deterministic reservation ids derived from idempotency keys.
/// Fixed namespace for deriving deterministic reservation ids from an
/// `Idempotency-Key` (see [`create_reservation`]). A stable, well-formed UUID — unlike
/// building one from raw ASCII bytes, this keeps the `Uuid::new_v5` namespace input
/// conventional.
const RESERVATION_IDEMPOTENCY_NAMESPACE: Uuid =
    Uuid::from_u128(0x6f3c_9a12_5e7b_4d80_a1c2_b3d4_e5f6_0718);

/// Cancel a reservation.
///
/// POST /api/v2/reservations/:reservation_id/cancel
pub async fn cancel_reservation(
    Path(reservation_id): Path<Uuid>,
    State(state): State<ReservationAppState>,
    _user: SessionUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let input = ReservationSagaInput::CancelReservation {
        reservation_id: ReservationId::from_uuid(reservation_id),
        fetched: None, // Handler will populate from state
    };

    let _result = state
        .saga_handler
        .handle(input)
        .await
        .map_err(saga_to_app_error)?;

    Ok(Json(serde_json::json!({
        "reservation_id": reservation_id,
        "status": "cancelled",
        "message": "Reservation cancelled successfully"
    })))
}

/// Create reservation routes.
///
/// # Routes
///
/// - POST /api/v2/reservations - Create a new reservation (initiates saga)
/// - POST /api/v2/reservations/:id/cancel - Cancel a reservation
///
/// Note: The create_reservation endpoint requires authentication middleware
/// that extracts `UserId` from the request and adds it as an `Extension`.
pub fn reservation_routes() -> Router<ReservationAppState> {
    Router::new()
        .route("/reservations", post(create_reservation))
        .route(
            "/reservations/{reservation_id}/cancel",
            post(cancel_reservation),
        )
}

//
// For production, you would set this up in main.rs with:
// - Create PostgresEventStore
// - Create each aggregate handler with the same event store
// - Create CallExecutor with refs to aggregate handlers + event store
// - Create saga handler with CallExecutor
//
// Below are the HTTP handlers that would be used - they require additional
// state fields in NextAppState that are commented out for now.

// ───────────────────────────────────────────────────────────────────────────
// Analytics Handlers
// ───────────────────────────────────────────────────────────────────────────

use super::{
    analytics::{
        AnalyticsBusinessLogic, AnalyticsCommand, AnalyticsError,
        AnalyticsResponse as DomainAnalyticsResponse, CustomerProfileDto, CustomerSpendingDto,
        EventSalesDto, PopularSectionsDto, PurchaseRecordDto, SectionPopularityDto, SectionSalesDto,
        TopSpendersDto, TotalRevenueDto,
    },
    projection_queries::{
        AnalyticsProjectionQueries, AnalyticsQueryFetcher, ReservationProjectionQueries,
        ReservationQueryFetcher,
    },
    reservation::{
        ReservationDto, ReservationQueryCommand, ReservationQueryError, ReservationQueryLogic,
        ReservationQueryResponse, ReservationSummaryDto,
    },
};

// ───────────────────────────────────────────────────────────────────────────
// Analytics Query State
// ───────────────────────────────────────────────────────────────────────────

/// Environment type with analytics projection queries enabled.
pub type AnalyticsQueryEnvironment = TicketingEnvironment<
    SystemClock,
    PostgresEventStore,
    NoOpProjector,
    NoOpEventBus,
    AnalyticsProjectionQueries,
>;

/// Analytics handler with query support.
pub type QueryEnabledAnalyticsHandler = Handler<
    AnalyticsBusinessLogic,
    NoOpCallExecutor,
    AnalyticsQueryFetcher,
    AnalyticsQueryEnvironment,
>;

/// Environment type with reservation projection queries enabled.
pub type ReservationQueryEnvironment = TicketingEnvironment<
    SystemClock,
    PostgresEventStore,
    NoOpProjector,
    NoOpEventBus,
    ReservationProjectionQueries,
>;

/// Reservation query handler with query support.
pub type QueryEnabledReservationQueryHandler = Handler<
    ReservationQueryLogic,
    NoOpCallExecutor,
    ReservationQueryFetcher,
    ReservationQueryEnvironment,
>;

/// App state for analytics endpoints.
#[derive(Clone)]
pub struct AnalyticsAppState {
    /// Query-enabled analytics handler
    pub analytics_handler: Arc<QueryEnabledAnalyticsHandler>,
}

/// App state for reservation query endpoints.
#[derive(Clone)]
pub struct ReservationQueryAppState {
    /// Query-enabled reservation query handler
    pub reservation_handler: Arc<QueryEnabledReservationQueryHandler>,
}

// ───────────────────────────────────────────────────────────────────────────
// Analytics Response Types
// ───────────────────────────────────────────────────────────────────────────

/// Response for event sales analytics.
#[derive(Debug, Serialize)]
pub struct EventSalesResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Total revenue in cents
    pub total_revenue_cents: u64,
    /// Number of tickets sold
    pub tickets_sold: u32,
    /// Completed reservations
    pub completed_reservations: u32,
    /// Cancelled reservations
    pub cancelled_reservations: u32,
    /// Average ticket price in cents
    pub average_ticket_price_cents: u64,
    /// Revenue breakdown by section
    pub sections: Vec<SectionSalesResponse>,
}

/// Section sales in response.
#[derive(Debug, Serialize)]
pub struct SectionSalesResponse {
    /// Section name
    pub section: String,
    /// Revenue in cents
    pub revenue_cents: u64,
    /// Tickets sold
    pub tickets_sold: u32,
}

impl From<EventSalesDto> for EventSalesResponse {
    fn from(dto: EventSalesDto) -> Self {
        Self {
            event_id: *dto.event_id.as_uuid(),
            total_revenue_cents: dto.total_revenue.cents(),
            tickets_sold: dto.tickets_sold,
            completed_reservations: dto.completed_reservations,
            cancelled_reservations: dto.cancelled_reservations,
            average_ticket_price_cents: dto.average_ticket_price.cents(),
            sections: dto.sections.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<SectionSalesDto> for SectionSalesResponse {
    fn from(dto: SectionSalesDto) -> Self {
        Self {
            section: dto.section,
            revenue_cents: dto.revenue.cents(),
            tickets_sold: dto.tickets_sold,
        }
    }
}

/// Response for popular sections.
#[derive(Debug, Serialize)]
pub struct PopularSectionsResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Most popular section by ticket count
    pub most_popular: Option<SectionPopularityResponse>,
    /// Highest revenue section
    pub highest_revenue: Option<SectionPopularityResponse>,
}

/// Section popularity in response.
#[derive(Debug, Serialize)]
pub struct SectionPopularityResponse {
    /// Section name
    pub section: String,
    /// Tickets sold
    pub tickets_sold: u32,
    /// Revenue in cents
    pub revenue_cents: u64,
}

impl From<PopularSectionsDto> for PopularSectionsResponse {
    fn from(dto: PopularSectionsDto) -> Self {
        Self {
            event_id: *dto.event_id.as_uuid(),
            most_popular: dto.most_popular.map(Into::into),
            highest_revenue: dto.highest_revenue.map(Into::into),
        }
    }
}

impl From<SectionPopularityDto> for SectionPopularityResponse {
    fn from(dto: SectionPopularityDto) -> Self {
        Self {
            section: dto.section,
            tickets_sold: dto.tickets_sold,
            revenue_cents: dto.revenue.cents(),
        }
    }
}

/// Response for total revenue.
#[derive(Debug, Serialize)]
pub struct TotalRevenueResponse {
    /// Total revenue in cents
    pub total_revenue_cents: u64,
    /// Total tickets sold
    pub total_tickets_sold: u32,
    /// Number of events with sales
    pub events_with_sales: usize,
}

impl From<TotalRevenueDto> for TotalRevenueResponse {
    fn from(dto: TotalRevenueDto) -> Self {
        Self {
            total_revenue_cents: dto.total_revenue.cents(),
            total_tickets_sold: dto.total_tickets_sold,
            events_with_sales: dto.events_with_sales,
        }
    }
}

/// Response for top spenders.
#[derive(Debug, Serialize)]
pub struct TopSpendersResponse {
    /// Top spending customers
    pub customers: Vec<CustomerSpendingResponse>,
    /// Total number of customers
    pub total_customers: usize,
}

/// Customer spending in response.
#[derive(Debug, Serialize)]
pub struct CustomerSpendingResponse {
    /// Customer ID
    pub customer_id: Uuid,
    /// Total spent in cents
    pub total_spent_cents: u64,
    /// Total tickets purchased
    pub total_tickets: u32,
    /// Events attended
    pub events_attended: usize,
    /// Favorite section
    pub favorite_section: Option<String>,
}

impl From<TopSpendersDto> for TopSpendersResponse {
    fn from(dto: TopSpendersDto) -> Self {
        Self {
            customers: dto.customers.into_iter().map(Into::into).collect(),
            total_customers: dto.total_customers,
        }
    }
}

impl From<CustomerSpendingDto> for CustomerSpendingResponse {
    fn from(dto: CustomerSpendingDto) -> Self {
        Self {
            customer_id: *dto.customer_id.as_uuid(),
            total_spent_cents: dto.total_spent.cents(),
            total_tickets: dto.total_tickets,
            events_attended: dto.events_attended,
            favorite_section: dto.favorite_section,
        }
    }
}

/// Response for customer profile.
#[derive(Debug, Serialize)]
pub struct CustomerProfileResponse {
    /// Customer ID
    pub customer_id: Uuid,
    /// Total spent in cents
    pub total_spent_cents: u64,
    /// Total tickets
    pub total_tickets: u32,
    /// Events attended
    pub events_attended: Vec<Uuid>,
    /// Favorite section
    pub favorite_section: Option<String>,
    /// Recent purchases
    pub recent_purchases: Vec<PurchaseRecordResponse>,
}

/// Purchase record in response.
#[derive(Debug, Serialize)]
pub struct PurchaseRecordResponse {
    /// Reservation ID
    pub reservation_id: Uuid,
    /// Event ID
    pub event_id: Uuid,
    /// Section
    pub section: String,
    /// Ticket count
    pub ticket_count: u32,
    /// Amount paid in cents
    pub amount_paid_cents: u64,
    /// When completed
    pub completed_at: DateTime<Utc>,
}

impl From<CustomerProfileDto> for CustomerProfileResponse {
    fn from(dto: CustomerProfileDto) -> Self {
        Self {
            customer_id: *dto.customer_id.as_uuid(),
            total_spent_cents: dto.total_spent.cents(),
            total_tickets: dto.total_tickets,
            events_attended: dto.events_attended.iter().map(|id| *id.as_uuid()).collect(),
            favorite_section: dto.favorite_section,
            recent_purchases: dto.recent_purchases.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<PurchaseRecordDto> for PurchaseRecordResponse {
    fn from(dto: PurchaseRecordDto) -> Self {
        Self {
            reservation_id: dto.reservation_id,
            event_id: *dto.event_id.as_uuid(),
            section: dto.section,
            ticket_count: dto.ticket_count,
            amount_paid_cents: dto.amount_paid.cents(),
            completed_at: dto.completed_at,
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Analytics Error Conversion
// ───────────────────────────────────────────────────────────────────────────

/// Convert analytics error to app error.
fn analytics_to_app_error(err: HandlerError<AnalyticsError>) -> AppError {
    match err {
        HandlerError::Business(AnalyticsError::EventNotFound { event_id }) => {
            AppError::not_found("Event", event_id.as_uuid().to_string())
        }
        HandlerError::Business(AnalyticsError::CustomerNotFound { customer_id }) => {
            AppError::not_found("Customer", customer_id.as_uuid().to_string())
        }
        HandlerError::Business(AnalyticsError::AccessDenied { .. }) => {
            AppError::forbidden("Not authorized to access this resource")
        }
        HandlerError::Business(AnalyticsError::InvalidLimit { limit }) => {
            AppError::bad_request(format!("Invalid limit: {limit}. Must be between 1 and 100."))
        }
        HandlerError::Business(AnalyticsError::DataNotFetched) => {
            AppError::internal("Analytics data not fetched")
        }
        HandlerError::Load(e) => AppError::internal(format!("Failed to load: {e}")),
        HandlerError::Persist(e) => AppError::internal(format!("Failed to persist: {e}")),
        HandlerError::Projection(e) => AppError::internal(format!("Projection failed: {e}")),
        HandlerError::Broadcast(e) => AppError::internal(format!("Broadcast failed: {e}")),
        HandlerError::Serialization(e) => AppError::internal(format!("Serialization failed: {e}")),
        HandlerError::QueryFetch(e) => AppError::internal(format!("Query fetch failed: {e}")),
        HandlerError::SagaIterationsExceeded { max_iterations } => {
            AppError::internal(format!("Exceeded max iterations: {max_iterations}"))
        }
        err @ (HandlerError::DoneWithOutstandingCalls { .. }
        | HandlerError::SagaStuck
        | HandlerError::RespondInFeedbackCycle
        | HandlerError::TotalCallsExceeded { .. }) => {
            AppError::internal(format!("Durable saga error: {err}"))
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Analytics HTTP Handlers
// ───────────────────────────────────────────────────────────────────────────

/// Get sales metrics for an event.
///
/// GET /api/v2/analytics/events/:event_id/sales
pub async fn get_event_sales(
    Path(event_id): Path<Uuid>,
    State(state): State<AnalyticsAppState>,
) -> Result<Json<EventSalesResponse>, AppError> {
    let event_id = EventId::from_uuid(event_id);

    let command = AnalyticsCommand::GetEventSales {
        event_id,
        fetched: None,
    };

    let result = state
        .analytics_handler
        .handle(command)
        .await
        .map_err(analytics_to_app_error)?;

    match result.into_query_response() {
        Some(DomainAnalyticsResponse::EventSales(dto)) => Ok(Json(dto.into())),
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

/// Get popular sections for an event.
///
/// GET /api/v2/analytics/events/:event_id/sections/popular
pub async fn get_popular_sections(
    Path(event_id): Path<Uuid>,
    State(state): State<AnalyticsAppState>,
) -> Result<Json<PopularSectionsResponse>, AppError> {
    let event_id = EventId::from_uuid(event_id);

    let command = AnalyticsCommand::GetPopularSections {
        event_id,
        fetched: None,
    };

    let result = state
        .analytics_handler
        .handle(command)
        .await
        .map_err(analytics_to_app_error)?;

    match result.into_query_response() {
        Some(DomainAnalyticsResponse::PopularSections(dto)) => Ok(Json(dto.into())),
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

/// Get total revenue across all events.
///
/// GET /api/v2/analytics/revenue
pub async fn get_total_revenue(
    State(state): State<AnalyticsAppState>,
) -> Result<Json<TotalRevenueResponse>, AppError> {
    let command = AnalyticsCommand::GetTotalRevenue { fetched: None };

    let result = state
        .analytics_handler
        .handle(command)
        .await
        .map_err(analytics_to_app_error)?;

    match result.into_query_response() {
        Some(DomainAnalyticsResponse::TotalRevenue(dto)) => Ok(Json(dto.into())),
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

/// Query params for top spenders.
#[derive(Debug, Deserialize)]
pub struct TopSpendersQuery {
    /// Number of top spenders to return (default 10, max 100)
    #[serde(default = "default_top_spenders_limit")]
    pub limit: usize,
}

fn default_top_spenders_limit() -> usize {
    10
}

/// Get top spending customers.
///
/// GET /api/v2/analytics/customers/top-spenders
pub async fn get_top_spenders(
    State(state): State<AnalyticsAppState>,
    axum::extract::Query(query): axum::extract::Query<TopSpendersQuery>,
) -> Result<Json<TopSpendersResponse>, AppError> {
    let command = AnalyticsCommand::GetTopSpenders {
        limit: query.limit,
        fetched: None,
    };

    let result = state
        .analytics_handler
        .handle(command)
        .await
        .map_err(analytics_to_app_error)?;

    match result.into_query_response() {
        Some(DomainAnalyticsResponse::TopSpenders(dto)) => Ok(Json(dto.into())),
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

/// Get customer profile and purchase history.
///
/// GET /api/v2/analytics/customers/:customer_id/profile
pub async fn get_customer_profile(
    Path(customer_id): Path<Uuid>,
    State(state): State<AnalyticsAppState>,
    user: SessionUser,
) -> Result<Json<CustomerProfileResponse>, AppError> {
    let customer_id = CustomerId::from_uuid(customer_id);
    let requesting_user_id = CustomerId::from_uuid(user.0.0);

    let command = AnalyticsCommand::GetCustomerProfile {
        customer_id,
        requesting_user_id,
        fetched: None,
    };

    let result = state
        .analytics_handler
        .handle(command)
        .await
        .map_err(analytics_to_app_error)?;

    match result.into_query_response() {
        Some(DomainAnalyticsResponse::CustomerProfile(dto)) => Ok(Json(dto.into())),
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Reservation Query Response Types
// ───────────────────────────────────────────────────────────────────────────

/// Response for a single reservation.
#[derive(Debug, Serialize)]
pub struct ReservationResponse {
    /// Reservation ID
    pub id: Uuid,
    /// Event ID
    pub event_id: Uuid,
    /// Customer ID
    pub customer_id: Uuid,
    /// Section name
    pub section: String,
    /// Number of tickets
    pub quantity: u32,
    /// Current status
    pub status: String,
    /// Total amount in cents
    pub total_amount_cents: u64,
    /// Expiration time
    pub expires_at: Option<DateTime<Utc>>,
    /// When created
    pub created_at: DateTime<Utc>,
    /// When completed
    pub completed_at: Option<DateTime<Utc>>,
}

impl From<ReservationDto> for ReservationResponse {
    fn from(dto: ReservationDto) -> Self {
        Self {
            id: *dto.id.as_uuid(),
            event_id: *dto.event_id.as_uuid(),
            customer_id: *dto.customer_id.as_uuid(),
            section: dto.section,
            quantity: dto.quantity,
            status: format!("{:?}", dto.status),
            total_amount_cents: dto.total_amount.cents(),
            expires_at: dto.expires_at,
            created_at: dto.created_at,
            completed_at: dto.completed_at,
        }
    }
}

/// Response for reservation summary in list.
#[derive(Debug, Serialize)]
pub struct ReservationSummaryResponse {
    /// Reservation ID
    pub id: Uuid,
    /// Event ID
    pub event_id: Uuid,
    /// Section name
    pub section: String,
    /// Number of tickets
    pub quantity: u32,
    /// Current status
    pub status: String,
    /// Total amount in cents
    pub total_amount_cents: u64,
    /// When created
    pub created_at: DateTime<Utc>,
}

impl From<ReservationSummaryDto> for ReservationSummaryResponse {
    fn from(dto: ReservationSummaryDto) -> Self {
        Self {
            id: *dto.id.as_uuid(),
            event_id: *dto.event_id.as_uuid(),
            section: dto.section,
            quantity: dto.quantity,
            status: format!("{:?}", dto.status),
            total_amount_cents: dto.total_amount.cents(),
            created_at: dto.created_at,
        }
    }
}

/// Response for listing reservations.
#[derive(Debug, Serialize)]
pub struct ListReservationsResponse {
    /// Reservations
    pub reservations: Vec<ReservationSummaryResponse>,
    /// Total count
    pub total: usize,
}

// ───────────────────────────────────────────────────────────────────────────
// Reservation Query Error Conversion
// ───────────────────────────────────────────────────────────────────────────

/// Convert reservation query error to app error.
fn reservation_query_to_app_error(err: HandlerError<ReservationQueryError>) -> AppError {
    match err {
        HandlerError::Business(ReservationQueryError::ReservationNotFound { reservation_id }) => {
            AppError::not_found("Reservation", reservation_id.as_uuid().to_string())
        }
        HandlerError::Business(ReservationQueryError::AccessDenied { .. }) => {
            AppError::forbidden("Not authorized to access these reservations")
        }
        HandlerError::Business(ReservationQueryError::DataNotFetched) => {
            AppError::internal("Reservation data not fetched")
        }
        HandlerError::Load(e) => AppError::internal(format!("Failed to load: {e}")),
        HandlerError::Persist(e) => AppError::internal(format!("Failed to persist: {e}")),
        HandlerError::Projection(e) => AppError::internal(format!("Projection failed: {e}")),
        HandlerError::Broadcast(e) => AppError::internal(format!("Broadcast failed: {e}")),
        HandlerError::Serialization(e) => AppError::internal(format!("Serialization failed: {e}")),
        HandlerError::QueryFetch(e) => AppError::internal(format!("Query fetch failed: {e}")),
        HandlerError::SagaIterationsExceeded { max_iterations } => {
            AppError::internal(format!("Exceeded max iterations: {max_iterations}"))
        }
        err @ (HandlerError::DoneWithOutstandingCalls { .. }
        | HandlerError::SagaStuck
        | HandlerError::RespondInFeedbackCycle
        | HandlerError::TotalCallsExceeded { .. }) => {
            AppError::internal(format!("Durable saga error: {err}"))
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Reservation Query HTTP Handlers
// ───────────────────────────────────────────────────────────────────────────

/// Get a single reservation by ID.
///
/// GET /api/v2/reservations/:reservation_id
pub async fn get_reservation_by_id(
    Path(reservation_id): Path<Uuid>,
    State(state): State<ReservationQueryAppState>,
) -> Result<Json<ReservationResponse>, AppError> {
    let reservation_id = ReservationId::from_uuid(reservation_id);

    let command = ReservationQueryCommand::GetReservation {
        reservation_id,
        fetched: None,
    };

    let result = state
        .reservation_handler
        .handle(command)
        .await
        .map_err(reservation_query_to_app_error)?;

    match result.into_query_response() {
        Some(ReservationQueryResponse::Reservation(dto)) => Ok(Json(dto.into())),
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

/// List reservations for the authenticated user.
///
/// GET /api/v2/reservations
pub async fn list_user_reservations(
    State(state): State<ReservationQueryAppState>,
    user: SessionUser,
) -> Result<Json<ListReservationsResponse>, AppError> {
    let user_id = CustomerId::from_uuid(user.0.0);

    let command = ReservationQueryCommand::ListUserReservations {
        customer_id: user_id,
        requesting_user_id: user_id, // Same user
        fetched: None,
    };

    let result = state
        .reservation_handler
        .handle(command)
        .await
        .map_err(reservation_query_to_app_error)?;

    match result.into_query_response() {
        Some(ReservationQueryResponse::ReservationList(dto)) => {
            let reservations: Vec<ReservationSummaryResponse> =
                dto.reservations.into_iter().map(Into::into).collect();
            Ok(Json(ListReservationsResponse {
                reservations,
                total: dto.total,
            }))
        }
        _ => Err(AppError::internal("Unexpected response type")),
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Analytics Routes
// ───────────────────────────────────────────────────────────────────────────

/// Create analytics API routes.
///
/// # Routes
///
/// - GET /analytics/events/:event_id/sales - Get event sales metrics
/// - GET /analytics/events/:event_id/sections/popular - Get popular sections
/// - GET /analytics/revenue - Get total revenue
/// - GET /analytics/customers/top-spenders - Get top spending customers
/// - GET /analytics/customers/:customer_id/profile - Get customer profile
pub fn analytics_routes() -> Router<AnalyticsAppState> {
    Router::new()
        .route("/analytics/events/{event_id}/sales", get(get_event_sales))
        .route(
            "/analytics/events/{event_id}/sections/popular",
            get(get_popular_sections),
        )
        .route("/analytics/revenue", get(get_total_revenue))
        .route(
            "/analytics/customers/top-spenders",
            get(get_top_spenders),
        )
        .route(
            "/analytics/customers/{customer_id}/profile",
            get(get_customer_profile),
        )
}

/// Create reservation query routes.
///
/// # Routes
///
/// - GET /reservations - List user's reservations
/// - GET /reservations/:id - Get a single reservation
pub fn reservation_query_routes() -> Router<ReservationQueryAppState> {
    Router::new()
        .route("/reservations", get(list_user_reservations))
        .route("/reservations/{id}", get(get_reservation_by_id))
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

/// Create a venue from the saga request.
#[allow(dead_code)] // For future saga handlers
fn create_venue_from_saga_request(request: &CreateEventWithInventoryRequest) -> Venue {
    let sections: Vec<VenueSection> = request
        .venue
        .sections
        .iter()
        .map(|s| {
            VenueSection::new(
                s.name.clone(),
                Capacity::new(s.capacity),
                SeatType::GeneralAdmission,
            )
        })
        .collect();

    let total_capacity: u32 = request.venue.sections.iter().map(|s| s.capacity).sum();

    Venue::new(
        request.venue.name.clone(),
        Capacity::new(total_capacity),
        sections,
    )
}

/// Create pricing tiers for saga request.
#[allow(dead_code)] // For future saga handlers
fn create_pricing_tiers_for_saga(request: &CreateEventWithInventoryRequest) -> Vec<PricingTier> {
    vec![PricingTier::new(
        TierType::Regular,
        "Standard".to_string(),
        Money::from_dollars(request.price as u64),
        Utc::now(),
        None,
    )]
}

// ═══════════════════════════════════════════════════════════════════════════
// Router Setup
// ═══════════════════════════════════════════════════════════════════════════

use axum::{routing::{get, patch, post, put}, Router};

/// Create the event creation router using the Event-Inventory saga.
///
/// This router uses the saga to coordinate event creation with inventory initialization.
///
/// # Routes
///
/// - POST /api/v2/events - Create a new event with inventory
pub fn event_creation_routes() -> Router<EventCreationAppState> {
    Router::new().route("/events", post(create_event))
}

/// Create the v2 events API router for write operations (update, delete, etc).
///
/// This router uses the new Handler pattern for all endpoints.
/// Note: POST /events is handled by `event_creation_routes` using the saga.
///
/// # Routes
///
/// - PUT /api/v2/events/:id - Update an event
/// - DELETE /api/v2/events/:id - Delete an event
/// - POST /api/v2/events/:id/publish - Publish an event
/// - POST /api/v2/events/:id/cancel - Cancel an event
/// - PATCH /api/v2/events/:id/pricing - Update pricing
/// - POST /api/v2/events/:id/sections - Add sections
pub fn events_v2_routes() -> Router<FullQueryAppState> {
    Router::new()
        .route("/events/{id}", put(update_event).delete(delete_event))
        .route("/events/{id}/publish", post(publish_event))
        .route("/events/{id}/cancel", post(cancel_event))
        .route("/events/{id}/pricing", patch(update_event_pricing))
        .route("/events/{id}/sections", post(add_venue_sections))
}

/// Create the query-enabled events API router.
///
/// This router uses `QueryAppState` which includes handlers configured
/// with `EventQueryFetcher` for pre-fetching projection data.
///
/// # Routes
///
/// - GET /api/v2/events - List all events with pagination
/// - GET /api/v2/events/:id - Get a single event by ID
/// - GET /api/v2/events/:id/pricing - Get pricing tiers
/// - GET /api/v2/my-events - List events owned by authenticated user
///
/// # Authentication
///
/// All query endpoints require authentication. For demo purposes, the
/// `user_id` is passed as a query parameter. In production, this would
/// come from authentication middleware.
pub fn query_routes() -> Router<QueryAppState> {
    Router::new()
        .route("/events", get(list_events))
        .route("/events/{id}", get(get_event))
        .route("/events/{id}/pricing", get(get_event_pricing))
        .route("/my-events", get(list_my_events))
}

/// Create the availability (inventory) API router.
///
/// # Routes
///
/// - GET /api/v2/events/:event_id/availability - Get all sections availability
/// - GET /api/v2/events/:event_id/total-available - Get total available seats
/// - GET /api/v2/events/:event_id/sections/:section/availability - Get section availability
pub fn availability_routes() -> Router<FullQueryAppState> {
    Router::new()
        .route("/events/{event_id}/availability", get(get_event_availability))
        .route("/events/{event_id}/total-available", get(get_total_available))
        .route(
            "/events/{event_id}/sections/{section}/availability",
            get(get_section_availability),
        )
}

/// Create the payment API router.
///
/// # Routes
///
/// - POST /api/v2/payments - Process a payment
/// - GET /api/v2/payments/:payment_id - Get payment details
/// - POST /api/v2/payments/:payment_id/refund - Refund a payment
/// - GET /api/v2/customers/:customer_id/payments - List customer payments
pub fn payment_routes() -> Router<FullQueryAppState> {
    Router::new()
        .route("/payments", post(process_payment))
        .route("/payments/{payment_id}", get(get_payment))
        .route("/payments/{payment_id}/refund", post(refund_payment))
        .route("/customers/{customer_id}/payments", get(list_customer_payments))
}

/// Create health check routes.
///
/// # Routes
///
/// - GET /health - Health check
/// - GET /ready - Readiness check
pub fn health_routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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
