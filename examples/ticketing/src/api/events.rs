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
use crate::aggregates::event_inventory_saga::EventInventorySagaAction;
use crate::auth::middleware::SessionUser;
use crate::server::state::AppState;
use crate::types::{
    Capacity, EventDate, EventId, EventStatus, Money, PricingTier, ResponseChannel,
    SeatType, TierType, Venue, VenueSection,
};
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
/// This endpoint uses the Event-Inventory Saga to atomically:
/// 1. Create the event in the Event aggregate
/// 2. Initialize inventory for all venue sections
///
/// If any step fails, the saga handles compensation automatically.
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
///     "venue_name": "Convention Center",
///     "venue_address": "123 Main St, San Francisco, CA 94102",
///     "capacity": 500,
///     "price": 50.00
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

    // Create Event-Inventory Saga store for this request
    // The saga coordinates event creation + inventory initialization atomically
    let store = state.create_event_inventory_saga_store(event_id);

    // Build CreateEventWithInventory saga action
    let action = EventInventorySagaAction::CreateEventWithInventory {
        event_id,
        name: request.title,
        owner_id: session.user_id,
        venue,
        date,
        pricing_tiers,
    };

    // Send action to saga and wait for terminal event (via broadcast_on_success)
    match store
        .send_and_wait_for(
            action,
            |action| {
                matches!(
                    action,
                    EventInventorySagaAction::EventCreationCompleted { .. }
                        | EventInventorySagaAction::EventCreationFailed { .. }
                        | EventInventorySagaAction::InventoryInitializationFailed { .. }
                        | EventInventorySagaAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(30), // Longer timeout for saga (multiple steps)
        )
        .await
    {
        Ok(EventInventorySagaAction::EventCreationCompleted { .. }) => {
            Ok((
                StatusCode::CREATED,
                Json(CreateEventResponse {
                    event_id: *event_id.as_uuid(),
                    message: "Event created successfully with inventory initialized".to_string(),
                }),
            ))
        }
        Ok(EventInventorySagaAction::EventCreationFailed { error, .. }) => {
            Err(AppError::internal(format!("Event creation failed: {error}")))
        }
        Ok(EventInventorySagaAction::InventoryInitializationFailed { section, error, .. }) => {
            Err(AppError::internal(format!(
                "Inventory initialization failed for section '{section}': {error}"
            )))
        }
        Ok(EventInventorySagaAction::ValidationFailed { error }) => {
            Err(AppError::bad_request(error))
        }
        Ok(_) => Err(AppError::internal("Unexpected action received")),
        Err(e) => Err(AppError::internal(format!("Failed to create event: {e}"))),
    }
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
    // Query event from projection via store query action
    let event_id_typed = crate::types::EventId::from_uuid(event_id);
    let store = state.create_event_store(event_id_typed);

    let event = match store
        .send_and_wait_for(
            EventAction::GetEvent {
                event_id: event_id_typed,
            },
            |action| {
                matches!(
                    action,
                    EventAction::EventQueried { .. } | EventAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
    {
        Ok(EventAction::EventQueried { event, .. }) => {
            event.ok_or_else(|| AppError::not_found("Event", event_id))?
        }
        Ok(EventAction::ValidationFailed { error }) => {
            return Err(AppError::internal(format!("Query failed: {error}")))
        }
        Ok(_) => return Err(AppError::internal("Unexpected action received")),
        Err(e) => return Err(AppError::internal(format!("Failed to query event: {e}"))),
    };

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

    // Use nil UUID for query-only operations (no event store access)
    // This operation queries projections, not event streams
    let store = state.create_event_store(crate::types::EventId::from_uuid(uuid::Uuid::nil()));
    let all_events = match store
        .send_and_wait_for(
            EventAction::ListEvents {
                status_filter: query.status,
            },
            |action| {
                matches!(
                    action,
                    EventAction::EventsListed { .. } | EventAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
    {
        Ok(EventAction::EventsListed { events, .. }) => events,
        Ok(EventAction::ValidationFailed { error }) => {
            return Err(AppError::internal(format!("Query failed: {error}")))
        }
        Ok(_) => return Err(AppError::internal("Unexpected action received")),
        Err(e) => return Err(AppError::internal(format!("Failed to query events: {e}"))),
    };

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
    use crate::types::EventId;

    let event_id_typed = EventId::from_uuid(event_id);

    // Create event store once for all operations (per-instance stream)
    let store = state.create_event_store(event_id_typed);

    // Check if event exists and get it via query action
    let event = match store
        .send_and_wait_for(
            EventAction::GetEvent {
                event_id: event_id_typed,
            },
            |action| {
                matches!(
                    action,
                    EventAction::EventQueried { .. } | EventAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
    {
        Ok(EventAction::EventQueried { event, .. }) => {
            event.ok_or_else(|| AppError::not_found("Event", event_id))?
        }
        Ok(EventAction::ValidationFailed { error }) => {
            return Err(AppError::internal(format!("Query failed: {error}")))
        }
        Ok(_) => return Err(AppError::internal("Unexpected action received")),
        Err(e) => return Err(AppError::internal(format!("Failed to query event: {e}"))),
    };

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

    // Send UpdateEvent action and wait for projection confirmation
    let new_name = name.clone();
    let action = EventAction::UpdateEvent {
        event_id: event_id_typed,
        name,
    };

    // Wait for either success (EventProjectionConfirmed) or failure
    match store
        .send_and_wait_for_with_metadata(
            action,
            None, // No special metadata for this operation
            |action| {
                matches!(
                    action,
                    EventAction::EventProjectionConfirmed { .. }
                        | EventAction::EventProjectionFailed { .. }
                        | EventAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(10),
        )
        .await
    {
        Ok(EventAction::EventProjectionConfirmed { .. }) => {
            // Return response with the updated values
            let response = EventResponse {
                id: *event.id.as_uuid(),
                title: new_name.unwrap_or_else(|| event.name.clone()),
                description: String::from("Event description not yet available"), // TODO: Add description field to Event domain model
                start_time: event.date.inner(),
                end_time: event.date.inner(), // TODO: Add separate end_time to Event domain model
                venue_name: event.venue.name.clone(),
                status: event.status,
                created_at: event.created_at,
            };
            Ok(Json(response))
        }
        Ok(EventAction::EventProjectionFailed { reason, .. }) => {
            Err(AppError::internal(format!("Projection failed: {reason}")))
        }
        Ok(EventAction::ValidationFailed { error }) => Err(AppError::bad_request(error)),
        Ok(_) => Err(AppError::internal("Unexpected action received")),
        Err(e) => Err(AppError::internal(format!("Failed to update event: {e}"))),
    }
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
    use crate::types::EventId;

    let event_id_typed = EventId::from_uuid(event_id);

    // Create event store once for all operations (per-instance stream)
    let store = state.create_event_store(event_id_typed);

    // Check if event exists and get it via query action
    let event = match store
        .send_and_wait_for(
            EventAction::GetEvent {
                event_id: event_id_typed,
            },
            |action| {
                matches!(
                    action,
                    EventAction::EventQueried { .. } | EventAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
    {
        Ok(EventAction::EventQueried { event, .. }) => {
            event.ok_or_else(|| AppError::not_found("Event", event_id))?
        }
        Ok(EventAction::ValidationFailed { error }) => {
            return Err(AppError::internal(format!("Query failed: {error}")))
        }
        Ok(_) => return Err(AppError::internal("Unexpected action received")),
        Err(e) => return Err(AppError::internal(format!("Failed to query event: {e}"))),
    };

    // Verify ownership: only the event owner can delete it
    if event.owner_id != session.user_id {
        return Err(AppError::forbidden(
            "You do not have permission to delete this event. Only the event owner can delete it.",
        ));
    }

    // Send CancelEvent action to event aggregate
    let action = EventAction::CancelEvent {
        event_id: event_id_typed,
        reason: format!("Cancelled by user {}", session.user_id.0),
    };

    store
        .send(action)
        .await
        .map_err(|e| AppError::internal(format!("Failed to cancel event: {e}")))?;

    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// Pricing Management
// ============================================================================

/// Pricing tier for API representation.
#[derive(Debug, Serialize, Deserialize)]
pub struct PricingTierDto {
    /// Tier type (Regular, EarlyBird, LastMinute)
    pub tier_type: TierType,
    /// Section name
    pub section: String,
    /// Base price in cents
    pub price_cents: u64,
    /// When this tier becomes available
    pub available_from: DateTime<Utc>,
    /// When this tier expires (None = no expiration)
    pub available_until: Option<DateTime<Utc>>,
}

impl From<&PricingTier> for PricingTierDto {
    fn from(tier: &PricingTier) -> Self {
        Self {
            tier_type: tier.tier_type,
            section: tier.section.clone(),
            price_cents: tier.base_price.cents(),
            available_from: tier.available_from,
            available_until: tier.available_until,
        }
    }
}

impl PricingTierDto {
    fn to_domain(&self) -> PricingTier {
        PricingTier::new(
            self.tier_type,
            self.section.clone(),
            Money::from_cents(self.price_cents),
            self.available_from,
            self.available_until,
        )
    }
}

/// Response for getting event pricing.
#[derive(Debug, Serialize)]
pub struct GetPricingResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Pricing tiers for all sections
    pub pricing_tiers: Vec<PricingTierDto>,
}

/// Request to update event pricing.
#[derive(Debug, Deserialize)]
pub struct UpdatePricingRequest {
    /// Updated pricing tiers
    pub pricing_tiers: Vec<PricingTierDto>,
}

/// Venue section DTO for API requests/responses.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VenueSectionDto {
    /// Section name (e.g., "VIP", "Balcony")
    pub name: String,
    /// Section capacity
    pub capacity: u32,
    /// Type of seating ("numbered" or "general_admission")
    pub seat_type: String,
}

impl VenueSectionDto {
    /// Convert DTO to domain type
    fn to_domain(&self) -> Result<VenueSection, String> {
        let seat_type = match self.seat_type.as_str() {
            "general_admission" => SeatType::GeneralAdmission,
            "numbered" => {
                return Err(
                    "Numbered seating not yet supported in this endpoint".to_string()
                )
            }
            _ => {
                return Err(format!(
                    "Invalid seat_type '{}'. Must be 'general_admission' or 'numbered'",
                    self.seat_type
                ))
            }
        };

        Ok(VenueSection::new(
            self.name.clone(),
            Capacity::new(self.capacity),
            seat_type,
        ))
    }
}

/// Add venue sections request.
#[derive(Debug, Deserialize)]
pub struct AddVenueSectionsRequest {
    /// Sections to add
    pub sections: Vec<VenueSectionDto>,
}

/// Add venue sections response.
#[derive(Debug, Serialize)]
pub struct AddVenueSectionsResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Sections that were added
    pub sections: Vec<VenueSectionDto>,
}

/// Get event pricing tiers.
///
/// Public endpoint - no authentication required.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/api/events/550e8400-e29b-41d4-a716-446655440000/pricing
/// ```
pub async fn get_event_pricing(
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<GetPricingResponse>, AppError> {
    let event_id_typed = crate::types::EventId::from_uuid(event_id);

    // Create event store for this request
    let store = state.create_event_store(event_id_typed);

    // Query event via store action
    let event = match store
        .send_and_wait_for(
            EventAction::GetEvent {
                event_id: event_id_typed,
            },
            |action| {
                matches!(
                    action,
                    EventAction::EventQueried { .. } | EventAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
    {
        Ok(EventAction::EventQueried { event, .. }) => {
            event.ok_or_else(|| AppError::not_found("Event", event_id))?
        }
        Ok(EventAction::ValidationFailed { error }) => {
            return Err(AppError::internal(format!("Query failed: {error}")))
        }
        Ok(_) => return Err(AppError::internal("Unexpected action received")),
        Err(e) => return Err(AppError::internal(format!("Failed to query event: {e}"))),
    };

    // Convert pricing tiers to DTOs
    let pricing_tiers: Vec<PricingTierDto> = event
        .pricing_tiers
        .iter()
        .map(PricingTierDto::from)
        .collect();

    Ok(Json(GetPricingResponse {
        event_id,
        pricing_tiers,
    }))
}

/// Update event pricing tiers.
///
/// Requires authentication and event ownership.
///
/// # Example
///
/// ```bash
/// curl -X PATCH http://localhost:8080/api/events/550e8400-e29b-41d4-a716-446655440000/pricing \
///   -H "Authorization: Bearer <session_token>" \
///   -H "Content-Type: application/json" \
///   -d '{
///     "pricing_tiers": [
///       {
///         "tier_type": "EarlyBird",
///         "section": "General Admission",
///         "price_cents": 2500,
///         "available_from": "2024-01-01T00:00:00Z",
///         "available_until": "2024-02-01T00:00:00Z"
///       },
///       {
///         "tier_type": "Regular",
///         "section": "General Admission",
///         "price_cents": 3500,
///         "available_from": "2024-02-01T00:00:00Z",
///         "available_until": null
///       }
///     ]
///   }'
/// ```
pub async fn update_event_pricing(
    session: SessionUser,
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<UpdatePricingRequest>,
) -> Result<Json<GetPricingResponse>, AppError> {
    let event_id_typed = crate::types::EventId::from_uuid(event_id);

    // Create event store for this request
    let store = state.create_event_store(event_id_typed);

    // Check if event exists and get it via query action
    let event = match store
        .send_and_wait_for(
            EventAction::GetEvent {
                event_id: event_id_typed,
            },
            |action| {
                matches!(
                    action,
                    EventAction::EventQueried { .. } | EventAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
    {
        Ok(EventAction::EventQueried { event, .. }) => {
            event.ok_or_else(|| AppError::not_found("Event", event_id))?
        }
        Ok(EventAction::ValidationFailed { error }) => {
            return Err(AppError::internal(format!("Query failed: {error}")))
        }
        Ok(_) => return Err(AppError::internal("Unexpected action received")),
        Err(e) => return Err(AppError::internal(format!("Failed to query event: {e}"))),
    };

    // Verify ownership: only the event owner can update pricing
    if event.owner_id != session.user_id {
        return Err(AppError::forbidden(
            "You do not have permission to update pricing for this event. Only the event owner can update it.",
        ));
    }

    // Convert DTOs to domain types
    let pricing_tiers: Vec<PricingTier> = request
        .pricing_tiers
        .iter()
        .map(PricingTierDto::to_domain)
        .collect();

    // Validate that at least one tier is provided
    if pricing_tiers.is_empty() {
        return Err(AppError::bad_request(
            "At least one pricing tier must be provided",
        ));
    }

    // Send UpdatePricingTiers action and wait for all projections to complete
    let action = EventAction::UpdatePricingTiers {
        event_id: event_id_typed,
        pricing_tiers: pricing_tiers.clone(),
        respond_to: ResponseChannel::none(),
    };

    // Wait for either success (EventProjectionConfirmed) or validation failure
    // The async flow is: UpdatePricingTiers -> Effect::Future -> ExecuteUpdatePricingTiers -> EventProjectionConfirmed
    match store
        .send_and_wait_for_with_metadata(
            action,
            None, // No special metadata for this operation
            |action| {
                matches!(
                    action,
                    EventAction::EventProjectionConfirmed { .. }
                        | EventAction::EventProjectionFailed { .. }
                        | EventAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
    {
        Ok(EventAction::EventProjectionConfirmed { .. }) => {
            // Return the pricing tiers from the request (they were successfully applied)
            let pricing_tier_dtos: Vec<PricingTierDto> =
                pricing_tiers.iter().map(PricingTierDto::from).collect();

            Ok(Json(GetPricingResponse {
                event_id,
                pricing_tiers: pricing_tier_dtos,
            }))
        }
        Ok(EventAction::EventProjectionFailed { reason, .. }) => {
            Err(AppError::internal(format!(
                "Failed to update pricing: projection failed - {reason}"
            )))
        }
        Ok(EventAction::ValidationFailed { error }) => {
            Err(AppError::bad_request(format!(
                "Failed to update pricing: {error}"
            )))
        }
        Ok(_) => Err(AppError::internal("Unexpected action received")),
        Err(e) => Err(AppError::internal(format!(
            "Failed to update pricing: {e}"
        ))),
    }
}

/// Add venue sections to an event.
///
/// Requires authentication and event ownership.
///
/// # Example
///
/// ```bash
/// curl -X POST http://localhost:8080/api/events/550e8400-e29b-41d4-a716-446655440000/sections \
///   -H "Authorization: Bearer <session_token>" \
///   -H "Content-Type: application/json" \
///   -d '{
///     "sections": [
///       {
///         "name": "VIP",
///         "capacity": 100,
///         "seat_type": "general_admission"
///       },
///       {
///         "name": "Balcony",
///         "capacity": 200,
///         "seat_type": "general_admission"
///       }
///     ]
///   }'
/// ```
pub async fn add_venue_sections(
    session: SessionUser,
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<AddVenueSectionsRequest>,
) -> Result<Json<AddVenueSectionsResponse>, AppError> {
    let event_id_typed = crate::types::EventId::from_uuid(event_id);

    // Create event store for this request
    let store = state.create_event_store(event_id_typed);

    // Check if event exists and get it via query action
    let event = match store
        .send_and_wait_for(
            EventAction::GetEvent {
                event_id: event_id_typed,
            },
            |action| {
                matches!(
                    action,
                    EventAction::EventQueried { .. } | EventAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
    {
        Ok(EventAction::EventQueried { event, .. }) => {
            event.ok_or_else(|| AppError::not_found("Event", event_id))?
        }
        Ok(EventAction::ValidationFailed { error }) => {
            return Err(AppError::internal(format!("Query failed: {error}")))
        }
        Ok(_) => return Err(AppError::internal("Unexpected action received")),
        Err(e) => return Err(AppError::internal(format!("Failed to query event: {e}"))),
    };

    // Verify ownership: only the event owner can add sections
    if event.owner_id != session.user_id {
        return Err(AppError::forbidden(
            "You do not have permission to add sections to this event. Only the event owner can modify it.",
        ));
    }

    // Convert DTOs to domain types
    let sections: Result<Vec<VenueSection>, String> = request
        .sections
        .iter()
        .map(VenueSectionDto::to_domain)
        .collect();

    let sections = sections.map_err(|e| AppError::bad_request(e))?;

    // Validate that at least one section is provided
    if sections.is_empty() {
        return Err(AppError::bad_request(
            "At least one section must be provided",
        ));
    }

    // Send AddVenueSections action and wait for all projections to complete
    let action = EventAction::AddVenueSections {
        event_id: event_id_typed,
        sections: sections.clone(),
        respond_to: ResponseChannel::none(),
    };

    // Wait for either projection confirmation or validation failure
    match store
        .send_and_wait_for_with_metadata(
            action,
            None, // No special metadata for this operation
            |action| {
                matches!(
                    action,
                    EventAction::EventProjectionConfirmed { .. }
                        | EventAction::EventProjectionFailed { .. }
                        | EventAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(10),
        )
        .await
    {
        Ok(EventAction::EventProjectionConfirmed { .. }) => {
            // Return the requested sections (they were successfully added)
            let section_dtos: Vec<VenueSectionDto> = sections
                .iter()
                .map(|s| VenueSectionDto {
                    name: s.name.clone(),
                    capacity: s.capacity.value(),
                    seat_type: "general_admission".to_string(),
                })
                .collect();

            Ok(Json(AddVenueSectionsResponse {
                event_id,
                sections: section_dtos,
            }))
        }
        Ok(EventAction::EventProjectionFailed { reason, .. }) => {
            Err(AppError::projection_failed("Event", "AddVenueSections", reason))
        }
        Ok(EventAction::ValidationFailed { error }) => Err(AppError::bad_request(format!(
            "Failed to add sections: {error}"
        ))),
        Ok(_) => Err(AppError::internal("Unexpected action received")),
        Err(e) => Err(AppError::internal(format!(
            "Failed to add sections: {e}"
        ))),
    }
}
