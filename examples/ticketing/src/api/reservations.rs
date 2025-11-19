//! Reservation management API endpoints.
//!
//! Provides endpoints for the ticket reservation workflow (saga-coordinated):
//! - POST /api/reservations - Initiate a new reservation (requires auth)
//! - GET /api/reservations/:id - Get reservation status
//! - POST /api/reservations/:id/cancel - Cancel reservation (requires auth + ownership)
//!
//! # Reservation Flow
//!
//! 1. **Initiate**: POST with `event_id`, section, quantity
//! 2. **Reserve Seats**: Saga coordinates with Inventory aggregate
//! 3. **Payment Pending**: 5-minute window for payment
//! 4. **Complete**: Payment succeeds, tickets issued
//! 5. **Compensate**: On timeout/failure, seats released
//!
//! # State Machine
//!
//! ```text
//! Initiated → SeatsAllocated → PaymentPending → Completed
//!     ↓            ↓                  ↓
//!  Failed      Failed            Expired/Cancelled
//!                                    ↓
//!                              Compensated
//! ```

#![allow(clippy::missing_errors_doc)] // Example code - errors are standard AppError

use crate::auth::middleware::{RequireOwnership, SessionUser};
use crate::server::state::AppState;
use crate::types::{CustomerId, EventId, ReservationId, ReservationStatus};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use composable_rust_core::event::EventMetadata;
use composable_rust_web::error::AppError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Request/Response Types
// ============================================================================

/// Request to create a new reservation.
#[derive(Debug, Deserialize)]
pub struct CreateReservationRequest {
    /// Event ID to reserve tickets for
    pub event_id: Uuid,
    /// Section to reserve from (e.g., "VIP", "General")
    pub section: String,
    /// Number of tickets to reserve
    pub quantity: u32,
    /// Optional specific seat numbers (if None, any available seats)
    pub specific_seats: Option<Vec<String>>,
}

/// Response after creating a reservation.
#[derive(Debug, Serialize)]
pub struct CreateReservationResponse {
    /// Created reservation ID
    pub reservation_id: Uuid,
    /// Reservation status
    pub status: ReservationStatus,
    /// Expiration time (5 minutes from creation)
    pub expires_at: DateTime<Utc>,
    /// Message for the user
    pub message: String,
}

/// Reservation details response.
#[derive(Debug, Serialize)]
pub struct ReservationResponse {
    /// Reservation ID
    pub id: Uuid,
    /// Event ID
    pub event_id: Uuid,
    /// Customer ID
    pub customer_id: Uuid,
    /// Section
    pub section: String,
    /// Quantity of tickets
    pub quantity: u32,
    /// Current status
    pub status: ReservationStatus,
    /// Total amount (if calculated)
    pub total_amount: Option<f64>,
    /// Expiration time
    pub expires_at: Option<DateTime<Utc>>,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Completed timestamp (if applicable)
    pub completed_at: Option<DateTime<Utc>>,
}

/// Request to cancel a reservation.
#[derive(Debug, Deserialize)]
pub struct CancelReservationRequest {
    /// Optional cancellation reason
    pub reason: Option<String>,
}

/// Response after cancelling a reservation.
#[derive(Debug, Serialize)]
pub struct CancelReservationResponse {
    /// Reservation ID
    pub reservation_id: Uuid,
    /// Status after cancellation
    pub status: ReservationStatus,
    /// Message for the user
    pub message: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// Create a new reservation.
///
/// Requires authentication. Initiates the reservation saga which:
/// 1. Reserves seats in the Inventory aggregate
/// 2. Starts a 5-minute payment timer
/// 3. Returns reservation details
///
/// # Example
///
/// ```bash
/// curl -X POST http://localhost:8080/api/reservations \
///   -H "Authorization: Bearer <session_token>" \
///   -H "Content-Type: application/json" \
///   -d '{
///     "event_id": "550e8400-e29b-41d4-a716-446655440000",
///     "section": "VIP",
///     "quantity": 2,
///     "specific_seats": null
///   }'
/// ```
///
/// Response:
/// ```json
/// {
///   "reservation_id": "660e8400-e29b-41d4-a716-446655440001",
///   "status": "Pending",
///   "expires_at": "2024-01-01T12:05:00Z",
///   "message": "Reservation created. Complete payment within 5 minutes."
/// }
/// ```
pub async fn create_reservation(
    session: SessionUser,
    State(state): State<AppState>,
    Json(request): Json<CreateReservationRequest>,
) -> Result<(StatusCode, Json<CreateReservationResponse>), AppError> {
    use crate::aggregates::ReservationAction;

    // Validate request
    if request.quantity == 0 {
        return Err(AppError::bad_request("Quantity must be greater than 0"));
    }

    if request.quantity > 10 {
        return Err(AppError::bad_request(
            "Cannot reserve more than 10 tickets at once",
        ));
    }

    // Generate reservation ID
    let reservation_id = ReservationId::new();
    let event_id = EventId::from_uuid(request.event_id);
    let customer_id = CustomerId::from_uuid(session.user_id.0);

    // Generate correlation ID for request tracking
    let correlation_id = crate::projections::CorrelationId::new();

    // Convert specific_seats from Vec<String> to Vec<SeatNumber>
    // Note: For now, we skip specific seat conversion since SeatNumber is private
    // In a real system, you'd have a public API for creating SeatNumbers
    let specific_seats = None; // TODO: Convert request.specific_seats properly

    // Create InitiateReservation command (correlation_id injected at Store level)
    let command = ReservationAction::InitiateReservation {
        reservation_id,
        event_id,
        customer_id,
        section: request.section.clone(),
        quantity: request.quantity,
        specific_seats,
        correlation_id: None, // Will be injected by send_with_metadata
    };

    // Prepare metadata with correlation_id for projection tracking
    let metadata = EventMetadata::with_correlation_id(correlation_id.to_string());

    // Create fresh Reservation store for this request (per-request pattern)
    // The store starts with empty state and loads only what it needs from event store
    let reservation_store = state.create_reservation_store();

    // Send command with metadata to Reservation Store
    // The Store will:
    // 1. Call the reducer
    // 2. Post-process effects to inject correlation_id metadata
    // 3. Execute effects (persist with metadata, publish, send to child stores)
    // 4. Handle the saga coordination
    let _ = reservation_store.send_with_metadata(command, Some(metadata)).await;
    // Store dropped here - memory freed

    // Calculate expiration (5 minutes from now)
    let expires_at = Utc::now() + chrono::Duration::minutes(5);

    Ok((
        StatusCode::CREATED,
        Json(CreateReservationResponse {
            reservation_id: *reservation_id.as_uuid(),
            status: ReservationStatus::Initiated,
            expires_at,
            message: "Reservation created. Complete payment within 5 minutes.".to_string(),
        }),
    ))
}

/// Get reservation details by ID.
///
/// Public endpoint - anyone can check reservation status.
/// Useful for payment processing flows.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/api/reservations/660e8400-e29b-41d4-a716-446655440001
/// ```
///
/// Response:
/// ```json
/// {
///   "id": "660e8400-e29b-41d4-a716-446655440001",
///   "event_id": "550e8400-e29b-41d4-a716-446655440000",
///   "customer_id": "770e8400-e29b-41d4-a716-446655440002",
///   "section": "VIP",
///   "quantity": 2,
///   "status": "PaymentPending",
///   "total_amount": 200.00,
///   "expires_at": "2024-01-01T12:05:00Z",
///   "created_at": "2024-01-01T12:00:00Z",
///   "completed_at": null
/// }
/// ```
pub async fn get_reservation(
    Path(reservation_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<ReservationResponse>, AppError> {
    use crate::types::ReservationId;
    use crate::aggregates::ReservationAction;

    // ✅ CORRECT: Query goes through Store/Reducer for testability
    let reservation_id_typed = ReservationId::from_uuid(reservation_id);

    // Create fresh Reservation store for this request (per-request pattern)
    let reservation_store = state.create_reservation_store();

    // Send GetReservation query action to Store
    let action = ReservationAction::GetReservation {
        reservation_id: reservation_id_typed,
    };

    // Store executes reducer, which returns Effect::Future that queries projection
    // The Effect completes and produces ReservationQueried event with the result
    // Wait for the response event with a timeout
    let result_action = reservation_store
        .send_and_wait_for(
            action,
            |a| {
                matches!(
                    a,
                    ReservationAction::ReservationQueried { .. }
                        | ReservationAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
        .map_err(|_| AppError::timeout("Reservation query timed out"))?;

    // Extract reservation from ReservationQueried event
    let reservation = match result_action {
        ReservationAction::ReservationQueried { reservation: Some(r), .. } => r,
        ReservationAction::ReservationQueried { reservation: None, .. } => {
            return Err(AppError::not_found("Reservation", reservation_id));
        }
        ReservationAction::ValidationFailed { error } => {
            return Err(AppError::internal(format!("Query failed: {error}")));
        }
        _ => {
            return Err(AppError::internal("Unexpected response from query"));
        }
    };

    // Convert domain Reservation to API ReservationResponse
    let response = ReservationResponse {
        id: reservation_id,
        event_id: *reservation.event_id.as_uuid(),
        customer_id: *reservation.customer_id.as_uuid(),
        section: String::from("General Admission"), // TODO: Extract from domain model when available
        quantity: reservation.seats.len() as u32,
        status: reservation.status.clone(),
        total_amount: Some(reservation.total_amount.dollars() as f64),
        expires_at: Some(reservation.expires_at.inner()),
        created_at: reservation.created_at,
        completed_at: None, // TODO: Extract from JSONB if needed
    };

    Ok(Json(response))
}

/// Cancel a reservation.
///
/// Requires authentication and ownership (customer who created it).
/// Triggers compensation saga to release reserved seats.
///
/// # Example
///
/// ```bash
/// curl -X POST http://localhost:8080/api/reservations/660e8400-e29b-41d4-a716-446655440001/cancel \
///   -H "Authorization: Bearer <session_token>" \
///   -H "Content-Type: application/json" \
///   -d '{
///     "reason": "Changed my mind"
///   }'
/// ```
///
/// Response:
/// ```json
/// {
///   "reservation_id": "660e8400-e29b-41d4-a716-446655440001",
///   "status": "Cancelled",
///   "message": "Reservation cancelled and seats released"
/// }
/// ```
pub async fn cancel_reservation(
    ownership: RequireOwnership<ReservationId>,
    Path(reservation_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(_request): Json<CancelReservationRequest>,
) -> Result<Json<CancelReservationResponse>, AppError> {
    use crate::aggregates::ReservationAction;

    // ✅ CORRECT: Command goes through Store/Reducer for testability
    // Ownership verified by RequireOwnership extractor
    // ownership.user_id is the authenticated user who owns this reservation
    // ownership.resource is the ReservationId from the path
    let _ = ownership;

    let reservation_id_typed = ReservationId::from_uuid(reservation_id);

    // Create fresh Reservation store for this request (per-request pattern)
    let reservation_store = state.create_reservation_store();

    // Send CancelReservation command to Store
    let action = ReservationAction::CancelReservation {
        reservation_id: reservation_id_typed,
    };

    // Store executes reducer, which:
    // 1. Updates state (marks as Cancelled)
    // 2. Returns effects (persist event, publish to EventBus, release seats)
    let _ = reservation_store.send(action).await;
    // Store dropped here - memory freed

    Ok(Json(CancelReservationResponse {
        reservation_id,
        status: ReservationStatus::Cancelled,
        message: "Cancellation request submitted. Seats will be released shortly.".to_string(),
    }))
}

/// List user's reservations.
///
/// Requires authentication. Returns all reservations for the authenticated user.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/api/reservations \
///   -H "Authorization: Bearer <session_token>"
/// ```
///
/// Response:
/// ```json
/// {
///   "reservations": [
///     {
///       "id": "660e8400-e29b-41d4-a716-446655440001",
///       "event_id": "550e8400-e29b-41d4-a716-446655440000",
///       "status": "Completed",
///       "quantity": 2,
///       "created_at": "2024-01-01T12:00:00Z"
///     }
///   ],
///   "total": 1
/// }
/// ```
#[derive(Debug, Serialize)]
pub struct ListReservationsResponse {
    /// List of reservations
    pub reservations: Vec<ReservationSummary>,
    /// Total count
    pub total: usize,
}

/// Summary of a reservation for list view.
#[derive(Debug, Serialize)]
pub struct ReservationSummary {
    /// Reservation ID
    pub id: Uuid,
    /// Event ID
    pub event_id: Uuid,
    /// Section
    pub section: String,
    /// Quantity
    pub quantity: u32,
    /// Status
    pub status: ReservationStatus,
    /// Total amount (if available)
    pub total_amount: Option<f64>,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
}

/// List all reservations for the authenticated user.
///
/// Returns ALL reservations (pending, completed, cancelled, expired) from the
/// `PostgresReservationsProjection`. This provides complete visibility into
/// the user's reservation history across all states.
///
/// # Features
///
/// - Lists ALL reservation states (not just completed)
/// - Ordered by creation time (most recent first)
/// - Includes full reservation details (status, amounts, timestamps)
pub async fn list_user_reservations(
    session: SessionUser,
    State(state): State<AppState>,
) -> Result<Json<ListReservationsResponse>, AppError> {
    use crate::aggregates::ReservationAction;

    // ✅ CORRECT: Query goes through Store/Reducer for testability
    let customer_id = CustomerId::from_uuid(session.user_id.0);

    // Create fresh Reservation store for this request (per-request pattern)
    let reservation_store = state.create_reservation_store();

    // Send ListReservations query action to Store
    let action = ReservationAction::ListReservations { customer_id };

    // Store executes reducer, which returns Effect::Future that queries projection
    // The Effect completes and produces ReservationsListed event with the result
    // Wait for the response event with a timeout
    let result_action = reservation_store
        .send_and_wait_for(
            action,
            |a| {
                matches!(
                    a,
                    ReservationAction::ReservationsListed { .. }
                        | ReservationAction::ValidationFailed { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
        .map_err(|_| AppError::timeout("Reservation list query timed out"))?;

    // Extract reservations from ReservationsListed event
    let reservations_list = match result_action {
        ReservationAction::ReservationsListed { reservations, .. } => reservations,
        ReservationAction::ValidationFailed { error } => {
            return Err(AppError::internal(format!("Query failed: {error}")));
        }
        _ => {
            return Err(AppError::internal("Unexpected response from query"));
        }
    };

    // Convert domain Reservations to API reservation summaries
    let reservations: Vec<ReservationSummary> = reservations_list
        .iter()
        .map(|reservation| ReservationSummary {
            id: *reservation.id.as_uuid(),
            event_id: *reservation.event_id.as_uuid(),
            section: String::from("General Admission"), // TODO: Extract from domain model when available
            quantity: reservation.seats.len() as u32,
            status: reservation.status.clone(),
            total_amount: Some(reservation.total_amount.dollars() as f64),
            created_at: reservation.created_at,
        })
        .collect();

    let total = reservations.len();

    Ok(Json(ListReservationsResponse {
        reservations,
        total,
    }))
}
