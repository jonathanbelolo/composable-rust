//! Reservation management API endpoints.
//!
//! Provides endpoints for the ticket reservation workflow (saga-coordinated):
//! - POST /api/reservations - Initiate a new reservation (requires auth)
//! - GET /api/reservations/:id - Get reservation status
//! - POST /api/reservations/:id/cancel - Cancel reservation (requires auth + ownership)
//!
//! # Reservation Flow
//!
//! 1. **Initiate**: POST with event_id, section, quantity
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

use crate::auth::middleware::SessionUser;
use crate::server::state::AppState;
use crate::types::{CustomerId, EventId, ReservationId, ReservationStatus};
use axum::{
    extract::{Path, State},
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
    State(_state): State<AppState>,
    Json(request): Json<CreateReservationRequest>,
) -> Result<(StatusCode, Json<CreateReservationResponse>), AppError> {
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

    // TODO: Send InitiateReservation command to reservation saga via event store
    // For now, just return a placeholder response
    let _ = (event_id, customer_id, request.section, request.quantity);

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
    State(_state): State<AppState>,
) -> Result<Json<ReservationResponse>, AppError> {
    // TODO: Query reservation state from event store or projection

    // Placeholder: return not found
    Err(AppError::not_found("Reservation", reservation_id))
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
    session: SessionUser,
    Path(reservation_id): Path<Uuid>,
    State(_state): State<AppState>,
    Json(request): Json<CancelReservationRequest>,
) -> Result<Json<CancelReservationResponse>, AppError> {
    // TODO: Verify ownership (customer_id matches session.user_id)
    // TODO: Send CancelReservation command to saga

    let _ = (session, request);

    // Placeholder
    Err(AppError::not_found("Reservation", reservation_id))
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

pub async fn list_user_reservations(
    session: SessionUser,
    State(_state): State<AppState>,
) -> Result<Json<ListReservationsResponse>, AppError> {
    // TODO: Query reservations for session.user_id from projection
    let _ = session;

    // Placeholder
    Ok(Json(ListReservationsResponse {
        reservations: vec![],
        total: 0,
    }))
}
