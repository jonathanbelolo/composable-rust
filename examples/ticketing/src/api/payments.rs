//! Payment processing API endpoints.
//!
//! Provides endpoints for payment operations:
//! - POST /api/payments - Process payment for reservation (requires auth)
//! - GET /api/payments/:id - Get payment status
//! - POST /api/payments/:id/refund - Refund payment (requires auth + ownership)
//!
//! # Payment Flow
//!
//! 1. **Process Payment**: User submits payment for reservation
//! 2. **Gateway Integration**: Payment processed via external gateway (Stripe, `PayPal`, etc.)
//! 3. **Success**: Saga notified, tickets issued, reservation completed
//! 4. **Failure**: Saga compensates, seats released
//!
//! # Payment Methods
//!
//! - Credit Card (PCI-compliant tokenization required)
//! - `PayPal`
//! - Apple Pay

#![allow(clippy::missing_errors_doc)] // Example code - errors are standard AppError

use crate::auth::middleware::{RequireOwnership, SessionUser};
use crate::server::state::AppState;
use crate::types::{CustomerId, PaymentId, PaymentMethod, PaymentStatus, ReservationId};
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

/// Request to process a payment.
#[derive(Debug, Deserialize)]
pub struct ProcessPaymentRequest {
    /// Reservation ID to pay for
    pub reservation_id: Uuid,
    /// Payment method details
    pub payment_method: PaymentMethodRequest,
    /// Optional billing information
    pub billing_info: Option<BillingInfo>,
}

/// Payment method from client.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaymentMethodRequest {
    /// Credit card payment
    CreditCard {
        /// Payment token from gateway (not raw card number!)
        token: String,
        /// Last four digits for display
        last_four: String,
    },
    /// `PayPal` payment
    PayPal {
        /// `PayPal` email
        email: String,
    },
    /// Apple Pay
    ApplePay {
        /// Apple Pay token
        token: String,
    },
}

/// Billing information.
#[derive(Debug, Deserialize)]
pub struct BillingInfo {
    /// Full name
    pub name: String,
    /// Email address
    pub email: String,
    /// Billing address
    pub address: String,
    /// City
    pub city: String,
    /// State/Province
    pub state: String,
    /// Postal code
    pub postal_code: String,
    /// Country code (ISO 3166-1 alpha-2)
    pub country: String,
}

/// Response after processing payment.
#[derive(Debug, Serialize)]
pub struct ProcessPaymentResponse {
    /// Created payment ID
    pub payment_id: Uuid,
    /// Reservation ID
    pub reservation_id: Uuid,
    /// Payment status
    pub status: PaymentStatus,
    /// Amount charged
    pub amount: f64,
    /// Transaction ID from gateway (if successful)
    pub transaction_id: Option<String>,
    /// Message for the user
    pub message: String,
}

/// Payment details response.
#[derive(Debug, Serialize)]
pub struct PaymentResponse {
    /// Payment ID
    pub id: Uuid,
    /// Reservation ID
    pub reservation_id: Uuid,
    /// Customer ID
    pub customer_id: Uuid,
    /// Amount
    pub amount: f64,
    /// Payment method (sanitized - no sensitive data)
    pub payment_method: String,
    /// Status
    pub status: PaymentStatus,
    /// Transaction ID (if successful)
    pub transaction_id: Option<String>,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Processed timestamp (if completed)
    pub processed_at: Option<DateTime<Utc>>,
}

/// Request to refund a payment.
#[derive(Debug, Deserialize)]
pub struct RefundPaymentRequest {
    /// Amount to refund (if None, full refund)
    pub amount: Option<f64>,
    /// Refund reason
    pub reason: String,
}

/// Response after refunding payment.
#[derive(Debug, Serialize)]
pub struct RefundPaymentResponse {
    /// Payment ID
    pub payment_id: Uuid,
    /// Refund amount
    pub refund_amount: f64,
    /// Status after refund
    pub status: PaymentStatus,
    /// Message for the user
    pub message: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// Process a payment for a reservation.
///
/// Requires authentication. Must be called within reservation's 5-minute window.
///
/// # Security Notes
///
/// - NEVER send raw credit card numbers to the backend
/// - Use payment gateway tokenization (Stripe Elements, `PayPal` SDK, etc.)
/// - This endpoint receives tokens, not raw card data
/// - PCI compliance is handled by the payment gateway
///
/// # Example
///
/// ```bash
/// curl -X POST http://localhost:8080/api/payments \
///   -H "Authorization: Bearer <session_token>" \
///   -H "Content-Type: application/json" \
///   -d '{
///     "reservation_id": "660e8400-e29b-41d4-a716-446655440001",
///     "payment_method": {
///       "type": "credit_card",
///       "token": "tok_visa_4242424242424242",
///       "last_four": "4242"
///     },
///     "billing_info": {
///       "name": "John Doe",
///       "email": "john@example.com",
///       "address": "123 Main St",
///       "city": "San Francisco",
///       "state": "CA",
///       "postal_code": "94102",
///       "country": "US"
///     }
///   }'
/// ```
///
/// Response:
/// ```json
/// {
///   "payment_id": "770e8400-e29b-41d4-a716-446655440002",
///   "reservation_id": "660e8400-e29b-41d4-a716-446655440001",
///   "status": "Succeeded",
///   "amount": 200.00,
///   "transaction_id": "txn_1234567890",
///   "message": "Payment successful! Your tickets will be issued shortly."
/// }
/// ```
pub async fn process_payment(
    session: SessionUser,
    State(_state): State<AppState>,
    Json(request): Json<ProcessPaymentRequest>,
) -> Result<(StatusCode, Json<ProcessPaymentResponse>), AppError> {
    // Validate payment method
    let payment_method = match request.payment_method {
        PaymentMethodRequest::CreditCard { token, last_four } => {
            if token.is_empty() {
                return Err(AppError::bad_request("Payment token is required"));
            }
            if last_four.len() != 4 {
                return Err(AppError::bad_request("Last four digits must be exactly 4 digits"));
            }
            PaymentMethod::CreditCard { last_four }
        }
        PaymentMethodRequest::PayPal { email } => {
            if !email.contains('@') {
                return Err(AppError::bad_request("Invalid PayPal email"));
            }
            PaymentMethod::PayPal { email }
        }
        PaymentMethodRequest::ApplePay { token } => {
            if token.is_empty() {
                return Err(AppError::bad_request("Apple Pay token is required"));
            }
            PaymentMethod::ApplePay
        }
    };

    // Generate payment ID
    let payment_id = PaymentId::new();
    let reservation_id = ReservationId::from_uuid(request.reservation_id);
    let customer_id = CustomerId::from_uuid(session.user_id.0);

    // TODO: Verify reservation exists and belongs to user
    // CRITICAL SECURITY: Must verify reservation ownership before processing payment
    // Pseudocode:
    // let reservation = state.event_store.load_aggregate(&reservation_id).await?;
    // if reservation.customer_id != customer_id {
    //     return Err(AppError::forbidden("You don't own this reservation"));
    // }
    // if reservation.status != ReservationStatus::PaymentPending {
    //     return Err(AppError::bad_request("Reservation not ready for payment"));
    // }
    // let amount = reservation.total_amount;

    // TODO: Send ProcessPayment command to payment aggregate via event store
    // TODO: Integrate with real payment gateway (Stripe, PayPal, etc.)

    let _ = (customer_id, reservation_id, payment_method, request.billing_info);

    // Placeholder: simulate success
    Ok((
        StatusCode::CREATED,
        Json(ProcessPaymentResponse {
            payment_id: *payment_id.as_uuid(),
            reservation_id: request.reservation_id,
            status: PaymentStatus::Captured,
            amount: 200.0, // TODO: Get from reservation
            transaction_id: Some("txn_1234567890".to_string()),
            message: "Payment successful! Your tickets will be issued shortly.".to_string(),
        }),
    ))
}

/// Get payment details by ID.
///
/// Public endpoint - anyone can check payment status.
/// Useful for displaying payment confirmation.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/api/payments/770e8400-e29b-41d4-a716-446655440002
/// ```
///
/// Response:
/// ```json
/// {
///   "id": "770e8400-e29b-41d4-a716-446655440002",
///   "reservation_id": "660e8400-e29b-41d4-a716-446655440001",
///   "customer_id": "880e8400-e29b-41d4-a716-446655440003",
///   "amount": 200.00,
///   "payment_method": "Credit Card (****4242)",
///   "status": "Succeeded",
///   "transaction_id": "txn_1234567890",
///   "created_at": "2024-01-01T12:02:00Z",
///   "processed_at": "2024-01-01T12:02:05Z"
/// }
/// ```
pub async fn get_payment(
    Path(payment_id): Path<Uuid>,
    State(_state): State<AppState>,
) -> Result<Json<PaymentResponse>, AppError> {
    // TODO: Query payment state from event store or projection

    // Placeholder: return stub data for testing
    Ok(Json(PaymentResponse {
        id: payment_id,
        reservation_id: Uuid::new_v4(), // TODO: Get from actual payment record
        customer_id: Uuid::new_v4(),    // TODO: Get from actual payment record
        amount: 200.0,
        payment_method: "Credit Card (****4242)".to_string(),
        status: PaymentStatus::Captured,
        transaction_id: Some("txn_1234567890".to_string()),
        created_at: Utc::now(),
        processed_at: Some(Utc::now()),
    }))
}

/// Refund a payment.
///
/// Requires authentication and ownership (customer who made payment).
/// Admins can refund any payment.
///
/// # Refund Policy
///
/// - Full refunds available up to 24 hours before event
/// - Partial refunds available up to 7 days before event
/// - No refunds within 7 days of event (admin override only)
///
/// # Example
///
/// ```bash
/// curl -X POST http://localhost:8080/api/payments/770e8400-e29b-41d4-a716-446655440002/refund \
///   -H "Authorization: Bearer <session_token>" \
///   -H "Content-Type: application/json" \
///   -d '{
///     "amount": null,
///     "reason": "Event cancelled"
///   }'
/// ```
///
/// Response:
/// ```json
/// {
///   "payment_id": "770e8400-e29b-41d4-a716-446655440002",
///   "refund_amount": 200.00,
///   "status": "Refunded",
///   "message": "Refund processed successfully"
/// }
/// ```
pub async fn refund_payment(
    ownership: RequireOwnership<PaymentId>,
    Path(payment_id): Path<Uuid>,
    State(_state): State<AppState>,
    Json(request): Json<RefundPaymentRequest>,
) -> Result<Json<RefundPaymentResponse>, AppError> {
    // Ownership verified by RequireOwnership extractor
    // ownership.user_id is the authenticated user who owns this payment
    // ownership.resource is the PaymentId from the path
    //
    // Note: In production, also check if user is admin for override capability

    // Validate refund amount
    if let Some(amount) = request.amount {
        if amount <= 0.0 {
            return Err(AppError::bad_request("Refund amount must be positive"));
        }
    }

    if request.reason.is_empty() {
        return Err(AppError::bad_request("Refund reason is required"));
    }

    // TODO: Verify payment exists (should already be verified by ownership check)
    // TODO: Check refund policy eligibility (event date, refund window)
    // TODO: Send RefundPayment command to payment aggregate

    let _ = ownership;

    // Placeholder
    Err(AppError::not_found("Payment", payment_id))
}

/// List user's payments.
///
/// Requires authentication. Returns all payments for the authenticated user.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/api/payments \
///   -H "Authorization: Bearer <session_token>"
/// ```
///
/// Response:
/// ```json
/// {
///   "payments": [
///     {
///       "id": "770e8400-e29b-41d4-a716-446655440002",
///       "reservation_id": "660e8400-e29b-41d4-a716-446655440001",
///       "amount": 200.00,
///       "status": "Succeeded",
///       "created_at": "2024-01-01T12:02:00Z"
///     }
///   ],
///   "total": 1
/// }
/// ```
#[derive(Debug, Serialize)]
pub struct ListPaymentsResponse {
    /// List of payments
    pub payments: Vec<PaymentSummary>,
    /// Total count
    pub total: usize,
}

/// Summary of a payment for list view.
#[derive(Debug, Serialize)]
pub struct PaymentSummary {
    /// Payment ID
    pub id: Uuid,
    /// Reservation ID
    pub reservation_id: Uuid,
    /// Amount
    pub amount: f64,
    /// Payment method (sanitized)
    pub payment_method: String,
    /// Status
    pub status: PaymentStatus,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
}

/// List all payments for the authenticated user.
pub async fn list_user_payments(
    session: SessionUser,
    State(_state): State<AppState>,
) -> Result<Json<ListPaymentsResponse>, AppError> {
    // TODO: Query payments for session.user_id from projection
    let _ = session;

    // Placeholder
    Ok(Json(ListPaymentsResponse {
        payments: vec![],
        total: 0,
    }))
}
