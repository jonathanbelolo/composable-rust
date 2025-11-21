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

use crate::aggregates::PaymentAction;
use crate::auth::middleware::{RequireOwnership, SessionUser};
use crate::projections::{CorrelationId, TicketingEvent};
use crate::server::state::AppState;
use crate::types::{CustomerId, Money, PaymentId, PaymentMethod, PaymentStatus, ReservationId};
use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use composable_rust_core::event::EventMetadata;
use composable_rust_web::error::AppError;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;
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
    Extension(correlation_uuid): Extension<Uuid>,
    State(state): State<AppState>,
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

    // TODO (Phase 12.4): Verify reservation ownership via query layer
    // CRITICAL SECURITY: Must verify reservation ownership before processing payment
    // This requires query infrastructure from Phase 12.4:
    // - Query reservation from projection
    // - Verify customer_id matches session.user_id
    // - Verify status is PaymentPending
    // - Get actual amount from reservation
    let _ = customer_id; // Will be used for ownership check in Phase 12.4

    // TODO (Phase 12.5): Replace placeholder amount with actual amount from reservation
    // For now, using hardcoded amount since query layer not yet implemented
    let amount = Money::from_dollars(200); // Placeholder

    // TODO (Phase 12.5): Payment gateway integration (Stripe, PayPal, etc.)
    // For now, the reducer simulates payment processing
    let _ = request.billing_info; // Will be used for gateway integration in Phase 12.5

    // Extract correlation ID from middleware (injected by correlation_id_layer)
    // This enables tracking the request through the entire event lifecycle
    let correlation_id = CorrelationId::from_uuid(correlation_uuid);

    // Prepare metadata with correlation_id for projection tracking
    let metadata = EventMetadata::with_correlation_id(correlation_id.to_string());

    // Create Payment store for this request
    let store = state.create_payment_store();

    // Build ProcessPayment action
    let action = PaymentAction::ProcessPayment {
        payment_id,
        reservation_id,
        amount,
        payment_method,
    };

    // Subscribe to BOTH topics BEFORE sending command to avoid race conditions:
    // - payment topic: for PaymentConfirmed business domain event
    // - projection.completed: for ProjectionCompleted infrastructure event
    let mut payment_subscriber = state
        .event_bus
        .subscribe(&[&state.config.redpanda.payment_topic])
        .await
        .map_err(|e| AppError::internal(format!("Failed to subscribe to payment events: {e}")))?;

    let mut projection_subscriber = state
        .event_bus
        .subscribe(&["projection.completed"])
        .await
        .map_err(|e| {
            AppError::internal(format!("Failed to subscribe to projection events: {e}"))
        })?;

    // Send action with metadata to store (Store executes effects automatically)
    // The correlation_id in metadata will be propagated to events and projections
    store
        .send_with_metadata(action, Some(metadata))
        .await
        .map_err(|e| AppError::internal(format!("Failed to process payment: {e}")))?;

    // Event loop: forward ProjectionCompleted to store, wait for PaymentConfirmed
    // Flow: ProcessPayment → PaymentProcessed → Projection → ProjectionCompleted →
    //       (HTTP handler forwards to same store) → PaymentConfirmed
    let timeout = Duration::from_secs(10);
    let start = std::time::Instant::now();

    loop {
        let remaining = timeout
            .checked_sub(start.elapsed())
            .unwrap_or(Duration::ZERO);

        if remaining.is_zero() {
            return Err(AppError::internal(
                "Timeout waiting for payment confirmation".to_string(),
            ));
        }

        tokio::select! {
            // Listen for ProjectionCompleted from projection.completed topic
            result = tokio::time::timeout(remaining, projection_subscriber.next()) => {
                match result {
                    Ok(Some(Ok(event_data))) => {
                        // Deserialize ProjectionCompleted (JSON from projections)
                        if let Ok(completed) = serde_json::from_slice::<crate::projections::ProjectionCompleted>(&event_data.data) {
                            // Check if this is for our payment
                            if completed.correlation_id.to_string() == correlation_id.to_string()
                                && completed.projection_name == "payments_projection"
                            {
                                tracing::info!(
                                    correlation_id = %correlation_id,
                                    "Received ProjectionCompleted - forwarding to store"
                                );

                                // Forward ProjectionCompleted to the SAME store instance
                                let action = PaymentAction::ProjectionCompleted {
                                    correlation_id: correlation_id.to_string(),
                                    projection_name: completed.projection_name.clone(),
                                };

                                store.send(action).await.map_err(|e| {
                                    AppError::internal(format!("Failed to forward ProjectionCompleted: {e}"))
                                })?;
                            }
                        }
                    }
                    Ok(Some(Err(e))) => {
                        return Err(AppError::internal(format!("Projection event bus error: {e}")));
                    }
                    Ok(None) => {
                        return Err(AppError::internal(
                            "Projection stream ended unexpectedly".to_string(),
                        ));
                    }
                    Err(_) => {
                        return Err(AppError::internal(
                            "Timeout waiting for projection completion".to_string(),
                        ));
                    }
                }
            }

            // Listen for PaymentConfirmed from payment topic
            result = tokio::time::timeout(remaining, payment_subscriber.next()) => {
                match result {
                    Ok(Some(Ok(event_data))) => {
                        // Deserialize event from SerializedEvent.data
                        if let Ok(event) = bincode::deserialize::<TicketingEvent>(&event_data.data) {
                            // Check if this is our PaymentConfirmed event
                            if let TicketingEvent::Payment(PaymentAction::PaymentConfirmed {
                                payment_id: confirmed_id,
                            }) = event
                            {
                                if confirmed_id == payment_id {
                                    tracing::info!(
                                        payment_id = %payment_id.as_uuid(),
                                        "Payment confirmed - projection updated"
                                    );
                                    break;
                                }
                            }
                            // Check for PaymentProjectionFailed
                            else if let TicketingEvent::Payment(PaymentAction::PaymentProjectionFailed {
                                payment_id: failed_id,
                                reason,
                            }) = event
                            {
                                if failed_id == payment_id {
                                    return Err(AppError::internal(format!(
                                        "Payment projection failed: {reason}"
                                    )));
                                }
                            }
                        }
                    }
                    Ok(Some(Err(e))) => {
                        return Err(AppError::internal(format!("Payment event bus error: {e}")));
                    }
                    Ok(None) => {
                        return Err(AppError::internal(
                            "Payment stream ended unexpectedly".to_string(),
                        ));
                    }
                    Err(_) => {
                        return Err(AppError::internal(
                            "Timeout waiting for payment confirmation".to_string(),
                        ));
                    }
                }
            }
        }
    }

    // Return success response (projection either completed or will complete soon)
    Ok((
        StatusCode::CREATED,
        Json(ProcessPaymentResponse {
            payment_id: *payment_id.as_uuid(),
            reservation_id: request.reservation_id,
            status: PaymentStatus::Captured,
            amount: 200.0, // TODO: Get from reservation in Phase 12.4
            transaction_id: Some("simulated_txn".to_string()),
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
    State(state): State<AppState>,
) -> Result<Json<PaymentResponse>, AppError> {
    // Query payment through store/reducer pattern
    let payment_id_typed = PaymentId::from_uuid(payment_id);
    let store = state.create_payment_store();

    // Send GetPayment query action and wait for PaymentQueried result
    let result = store
        .send_and_wait_for(
            PaymentAction::GetPayment {
                payment_id: payment_id_typed,
            },
            |action| matches!(action, PaymentAction::PaymentQueried { .. }),
            Duration::from_secs(5),
        )
        .await
        .map_err(|e| AppError::internal(format!("Failed to query payment: {e}")))?;

    // Extract payment from result action
    let payment = match result {
        PaymentAction::PaymentQueried {
            payment_id: _,
            payment: Some(payment),
        } => payment,
        PaymentAction::PaymentQueried {
            payment_id: _,
            payment: None,
        } => {
            return Err(AppError::not_found("Payment", payment_id));
        }
        _ => {
            return Err(AppError::internal("Unexpected response from payment query"));
        }
    };

    // Convert payment method to display string
    let payment_method_display = match &payment.payment_method {
        PaymentMethod::CreditCard { last_four } => format!("Credit Card (****{last_four})"),
        PaymentMethod::PayPal { email } => format!("PayPal ({email})"),
        PaymentMethod::ApplePay => "Apple Pay".to_string(),
    };

    // Convert domain Payment to API PaymentResponse
    Ok(Json(PaymentResponse {
        id: *payment.id.as_uuid(),
        reservation_id: *payment.reservation_id.as_uuid(),
        customer_id: *payment.customer_id.as_uuid(),
        amount: payment.amount.dollars() as f64,
        payment_method: payment_method_display,
        status: payment.status,
        transaction_id: None, // TODO: Add transaction_id to Payment domain model
        created_at: payment.processed_at.unwrap_or_else(Utc::now),
        processed_at: payment.processed_at,
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
    State(state): State<AppState>,
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

    let payment_id_typed = PaymentId::from_uuid(payment_id);

    // Query payment through store/reducer pattern to get current amount and status
    let store = state.create_payment_store();

    let result = store
        .send_and_wait_for(
            PaymentAction::GetPayment {
                payment_id: payment_id_typed,
            },
            |action| matches!(action, PaymentAction::PaymentQueried { .. }),
            Duration::from_secs(5),
        )
        .await
        .map_err(|e| AppError::internal(format!("Failed to query payment: {e}")))?;

    // Extract payment from result action
    let payment = match result {
        PaymentAction::PaymentQueried {
            payment_id: _,
            payment: Some(payment),
        } => payment,
        PaymentAction::PaymentQueried {
            payment_id: _,
            payment: None,
        } => {
            return Err(AppError::not_found("Payment", payment_id));
        }
        _ => {
            return Err(AppError::internal("Unexpected response from payment query"));
        }
    };

    // Verify payment is captured (can be refunded)
    if !matches!(payment.status, PaymentStatus::Captured) {
        return Err(AppError::bad_request(
            "Payment cannot be refunded. Only captured payments can be refunded.",
        ));
    }

    // Determine refund amount (full or partial)
    let refund_amount = match request.amount {
        Some(amount) => {
            let requested = Money::from_dollars(amount as u64);
            if requested > payment.amount {
                return Err(AppError::bad_request(
                    "Refund amount cannot exceed payment amount",
                ));
            }
            requested
        }
        None => payment.amount, // Full refund
    };

    // TODO (Phase 12.5): Check refund policy eligibility
    // - Event date must be at least 7 days away for customer refunds
    // - Admins can override refund policy
    // This requires querying the reservation and event from projections

    // Create Payment store for this request
    let store = state.create_payment_store();

    // Build RefundPayment action
    let action = PaymentAction::RefundPayment {
        payment_id: payment_id_typed,
        amount: refund_amount,
        reason: request.reason.clone(),
    };

    // Send action to store (Store executes effects automatically)
    store
        .send(action)
        .await
        .map_err(|e| AppError::internal(format!("Failed to process refund: {e}")))?;

    let _ = ownership; // Ownership was used by extractor

    // Return success response
    Ok(Json(RefundPaymentResponse {
        payment_id,
        refund_amount: refund_amount.dollars() as f64,
        status: PaymentStatus::Refunded {
            amount: refund_amount,
        },
        message: "Refund processed successfully".to_string(),
    }))
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
    State(state): State<AppState>,
) -> Result<Json<ListPaymentsResponse>, AppError> {
    // Query payments through store/reducer pattern
    let customer_id = CustomerId::from_uuid(session.user_id.0);
    let store = state.create_payment_store();

    // Get all payments (limit 100 for now, TODO: add pagination query params)
    let result = store
        .send_and_wait_for(
            PaymentAction::ListCustomerPayments {
                customer_id,
                limit: 100,
                offset: 0,
            },
            |action| matches!(action, PaymentAction::CustomerPaymentsListed { .. }),
            Duration::from_secs(5),
        )
        .await
        .map_err(|e| AppError::internal(format!("Failed to query payments: {e}")))?;

    // Extract payments from result action
    let payments = match result {
        PaymentAction::CustomerPaymentsListed {
            customer_id: _,
            payments,
        } => payments,
        _ => {
            return Err(AppError::internal(
                "Unexpected response from payments query",
            ));
        }
    };

    let total = payments.len();

    // Convert domain Payments to API PaymentSummary
    let payment_summaries: Vec<PaymentSummary> = payments
        .into_iter()
        .map(|payment| {
            let payment_method_display = match &payment.payment_method {
                PaymentMethod::CreditCard { last_four } => format!("Credit Card (****{last_four})"),
                PaymentMethod::PayPal { email } => format!("PayPal ({email})"),
                PaymentMethod::ApplePay => "Apple Pay".to_string(),
            };

            PaymentSummary {
                id: *payment.id.as_uuid(),
                reservation_id: *payment.reservation_id.as_uuid(),
                amount: payment.amount.dollars() as f64,
                payment_method: payment_method_display,
                status: payment.status,
                created_at: payment.processed_at.unwrap_or_else(Utc::now),
            }
        })
        .collect();

    Ok(Json(ListPaymentsResponse {
        payments: payment_summaries,
        total,
    }))
}
