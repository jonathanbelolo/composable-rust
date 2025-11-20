//! PostgreSQL-backed payments projection.
//!
//! This projection maintains a denormalized view of payment data in PostgreSQL,
//! enabling fast queries like "List all payments for Customer X" or
//! "Show payment status for Reservation Y".
//!
//! # Architecture
//!
//! - **Storage**: PostgreSQL with custom queryable tables
//! - **Checkpointing**: Uses framework's `PostgresProjectionCheckpoint`
//! - **CQRS**: Separate database from event store

use crate::aggregates::PaymentAction;
use crate::projections::TicketingEvent;
use crate::types::{CustomerId, Payment, PaymentId, PaymentMethod, PaymentStatus, ReservationId};
use chrono::{DateTime, Utc};
use composable_rust_core::projection::{Projection, ProjectionError, Result};
use sqlx::PgPool;
use std::sync::Arc;

/// PostgreSQL-backed payments projection.
///
/// Maintains real-time view of payment data with proper idempotency
/// and crash recovery via checkpointing.
#[derive(Clone)]
pub struct PostgresPaymentsProjection {
    pool: Arc<PgPool>,
}

impl PostgresPaymentsProjection {
    /// Create a new PostgreSQL-backed projection.
    ///
    /// # Arguments
    ///
    /// - `pool`: Connection pool for projection database (separate from event store)
    #[must_use]
    pub const fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    /// Get payment by ID.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn get_payment(&self, payment_id: &PaymentId) -> Result<Option<Payment>> {
        let row: Option<(
            sqlx::types::Uuid,
            sqlx::types::Uuid,
            sqlx::types::Uuid,
            i64,
            String,
            String,
            String,
            sqlx::types::JsonValue,
            Option<DateTime<Utc>>,
        )> = sqlx::query_as(
            "SELECT payment_id, reservation_id, customer_id, amount_cents, currency,
                    status, payment_method_type, payment_method_details, processed_at
             FROM payments_projection
             WHERE payment_id = $1",
        )
        .bind(payment_id.as_uuid())
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to query payment: {e}")))?;

        let Some((
            payment_id_db,
            reservation_id_db,
            customer_id_db,
            amount_cents,
            _currency,
            status_str,
            payment_method_type,
            payment_method_details,
            processed_at,
        )) = row
        else {
            return Ok(None);
        };

        // Reconstruct Payment
        let payment_id = PaymentId::from_uuid(payment_id_db);
        let reservation_id = ReservationId::from_uuid(reservation_id_db);
        let customer_id = CustomerId::from_uuid(customer_id_db);

        #[allow(clippy::cast_sign_loss)] // Amount is always positive in our domain
        let amount = crate::types::Money::from_cents(amount_cents as u64);

        let status = Self::parse_status(&status_str)?;
        let payment_method = Self::parse_payment_method(&payment_method_type, &payment_method_details)?;

        Ok(Some(Payment {
            id: payment_id,
            reservation_id,
            customer_id,
            amount,
            status,
            payment_method,
            processed_at,
        }))
    }

    /// List payments for a customer.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn list_customer_payments(
        &self,
        customer_id: &CustomerId,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Payment>> {
        let rows: Vec<(
            sqlx::types::Uuid,
            sqlx::types::Uuid,
            sqlx::types::Uuid,
            i64,
            String,
            String,
            String,
            sqlx::types::JsonValue,
            Option<DateTime<Utc>>,
        )> = sqlx::query_as(
            "SELECT payment_id, reservation_id, customer_id, amount_cents, currency,
                    status, payment_method_type, payment_method_details, processed_at
             FROM payments_projection
             WHERE customer_id = $1
             ORDER BY created_at DESC
             LIMIT $2 OFFSET $3",
        )
        .bind(customer_id.as_uuid())
        .bind(limit)
        .bind(offset)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to list customer payments: {e}")))?;

        rows.into_iter()
            .map(
                |(payment_id, reservation_id, customer_id, amount_cents, _currency, status_str, payment_method_type, payment_method_details, processed_at)| {
                    let payment_id = PaymentId::from_uuid(payment_id);
                    let reservation_id = ReservationId::from_uuid(reservation_id);
                    let customer_id = CustomerId::from_uuid(customer_id);

                    #[allow(clippy::cast_sign_loss)]
                    let amount = crate::types::Money::from_cents(amount_cents as u64);

                    let status = Self::parse_status(&status_str)?;
                    let payment_method = Self::parse_payment_method(&payment_method_type, &payment_method_details)?;

                    Ok(Payment {
                        id: payment_id,
                        reservation_id,
                        customer_id,
                        amount,
                        status,
                        payment_method,
                        processed_at,
                    })
                },
            )
            .collect()
    }

    /// List payments for a reservation.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    pub async fn list_reservation_payments(
        &self,
        reservation_id: &ReservationId,
    ) -> Result<Vec<Payment>> {
        let rows: Vec<(
            sqlx::types::Uuid,
            sqlx::types::Uuid,
            sqlx::types::Uuid,
            i64,
            String,
            String,
            String,
            sqlx::types::JsonValue,
            Option<DateTime<Utc>>,
        )> = sqlx::query_as(
            "SELECT payment_id, reservation_id, customer_id, amount_cents, currency,
                    status, payment_method_type, payment_method_details, processed_at
             FROM payments_projection
             WHERE reservation_id = $1
             ORDER BY created_at DESC",
        )
        .bind(reservation_id.as_uuid())
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| ProjectionError::Storage(format!("Failed to list reservation payments: {e}")))?;

        rows.into_iter()
            .map(
                |(payment_id, reservation_id, customer_id, amount_cents, _currency, status_str, payment_method_type, payment_method_details, processed_at)| {
                    let payment_id = PaymentId::from_uuid(payment_id);
                    let reservation_id = ReservationId::from_uuid(reservation_id);
                    let customer_id = CustomerId::from_uuid(customer_id);

                    #[allow(clippy::cast_sign_loss)]
                    let amount = crate::types::Money::from_cents(amount_cents as u64);

                    let status = Self::parse_status(&status_str)?;
                    let payment_method = Self::parse_payment_method(&payment_method_type, &payment_method_details)?;

                    Ok(Payment {
                        id: payment_id,
                        reservation_id,
                        customer_id,
                        amount,
                        status,
                        payment_method,
                        processed_at,
                    })
                },
            )
            .collect()
    }

    /// Parse payment status from string.
    fn parse_status(status_str: &str) -> Result<PaymentStatus> {
        match status_str {
            "Pending" => Ok(PaymentStatus::Pending),
            "Authorized" => Ok(PaymentStatus::Authorized),
            "Captured" => Ok(PaymentStatus::Captured),
            s if s.starts_with("Failed:") => {
                let reason = s.strip_prefix("Failed:").unwrap_or("Unknown").to_string();
                Ok(PaymentStatus::Failed { reason })
            }
            s if s.starts_with("Refunded:") => {
                // Parse refunded amount from status string (format: "Refunded:12345")
                let amount_str = s.strip_prefix("Refunded:").unwrap_or("0");
                let cents = amount_str.parse::<u64>().map_err(|e| {
                    ProjectionError::EventProcessing(format!("Invalid refund amount: {e}"))
                })?;
                Ok(PaymentStatus::Refunded {
                    amount: crate::types::Money::from_cents(cents),
                })
            }
            _ => Err(ProjectionError::EventProcessing(format!(
                "Unknown payment status: {status_str}"
            ))),
        }
    }

    /// Parse payment method from type and details.
    fn parse_payment_method(
        method_type: &str,
        details: &sqlx::types::JsonValue,
    ) -> Result<PaymentMethod> {
        match method_type {
            "CreditCard" => {
                let last_four = details
                    .get("last_four")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ProjectionError::EventProcessing(
                            "Missing last_four in credit card details".to_string(),
                        )
                    })?
                    .to_string();
                Ok(PaymentMethod::CreditCard { last_four })
            }
            "PayPal" => {
                let email = details
                    .get("email")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ProjectionError::EventProcessing(
                            "Missing email in PayPal details".to_string(),
                        )
                    })?
                    .to_string();
                Ok(PaymentMethod::PayPal { email })
            }
            "ApplePay" => Ok(PaymentMethod::ApplePay),
            _ => Err(ProjectionError::EventProcessing(format!(
                "Unknown payment method: {method_type}"
            ))),
        }
    }

    /// Serialize payment method to type and JSON details.
    fn serialize_payment_method(method: &PaymentMethod) -> (String, sqlx::types::JsonValue) {
        match method {
            PaymentMethod::CreditCard { last_four } => (
                "CreditCard".to_string(),
                serde_json::json!({ "last_four": last_four }),
            ),
            PaymentMethod::PayPal { email } => {
                ("PayPal".to_string(), serde_json::json!({ "email": email }))
            }
            PaymentMethod::ApplePay => ("ApplePay".to_string(), serde_json::json!({})),
        }
    }

    /// Serialize payment status to string for storage.
    fn serialize_status(status: &PaymentStatus) -> String {
        match status {
            PaymentStatus::Pending => "Pending".to_string(),
            PaymentStatus::Authorized => "Authorized".to_string(),
            PaymentStatus::Captured => "Captured".to_string(),
            PaymentStatus::Failed { reason } => format!("Failed:{reason}"),
            PaymentStatus::Refunded { amount } => format!("Refunded:{}", amount.cents()),
        }
    }
}

impl Projection for PostgresPaymentsProjection {
    type Event = TicketingEvent;

    fn name(&self) -> &'static str {
        "payments_projection"
    }

    async fn apply_event(&self, event: &Self::Event) -> Result<()> {
        match event {
            // Payment processed: create new payment record
            TicketingEvent::Payment(PaymentAction::PaymentProcessed {
                payment_id,
                reservation_id,
                amount,
                payment_method,
                processed_at,
            }) => {
                // Query customer_id from reservations projection
                let customer_id: sqlx::types::Uuid = sqlx::query_scalar(
                    "SELECT customer_id FROM reservations_projection WHERE id = $1"
                )
                .bind(reservation_id.as_uuid())
                .fetch_one(self.pool.as_ref())
                .await
                .map_err(|e| {
                    ProjectionError::Storage(format!(
                        "Failed to query customer_id for reservation {reservation_id}: {e}"
                    ))
                })?;

                let (method_type, method_details) =
                    Self::serialize_payment_method(payment_method);

                #[allow(clippy::cast_possible_wrap)] // Amount is within i64 range
                sqlx::query(
                    "INSERT INTO payments_projection
                     (payment_id, reservation_id, customer_id, amount_cents, currency,
                      status, payment_method_type, payment_method_details,
                      transaction_id, processed_at, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NULL, $9, $9, $9)
                     ON CONFLICT (payment_id) DO NOTHING",
                )
                .bind(payment_id.as_uuid())
                .bind(reservation_id.as_uuid())
                .bind(customer_id)
                .bind(amount.cents() as i64)
                .bind("USD")
                .bind("Pending")
                .bind(&method_type)
                .bind(method_details)
                .bind(processed_at)
                .execute(self.pool.as_ref())
                .await
                .map_err(|e| {
                    ProjectionError::Storage(format!("Failed to insert payment: {e}"))
                })?;

                Ok(())
            }

            // Payment succeeded: update status and add transaction ID
            TicketingEvent::Payment(PaymentAction::PaymentSucceeded {
                payment_id,
                transaction_id,
            }) => {
                sqlx::query(
                    "UPDATE payments_projection
                     SET status = $2, transaction_id = $3, updated_at = NOW()
                     WHERE payment_id = $1",
                )
                .bind(payment_id.as_uuid())
                .bind("Captured")
                .bind(transaction_id)
                .execute(self.pool.as_ref())
                .await
                .map_err(|e| {
                    ProjectionError::Storage(format!("Failed to update payment status: {e}"))
                })?;

                Ok(())
            }

            // Payment failed: update status with failure reason
            TicketingEvent::Payment(PaymentAction::PaymentFailed {
                payment_id,
                reason,
                failed_at,
            }) => {
                let status = Self::serialize_status(&PaymentStatus::Failed {
                    reason: reason.clone(),
                });

                sqlx::query(
                    "UPDATE payments_projection
                     SET status = $2, failure_reason = $3, failed_at = $4, updated_at = NOW()
                     WHERE payment_id = $1",
                )
                .bind(payment_id.as_uuid())
                .bind(&status)
                .bind(reason)
                .bind(failed_at)
                .execute(self.pool.as_ref())
                .await
                .map_err(|e| {
                    ProjectionError::Storage(format!("Failed to update payment failure: {e}"))
                })?;

                Ok(())
            }

            // Payment refunded: update status with refund amount
            TicketingEvent::Payment(PaymentAction::PaymentRefunded {
                payment_id,
                amount,
                reason,
                refunded_at,
            }) => {
                let status = Self::serialize_status(&PaymentStatus::Refunded {
                    amount: *amount,
                });

                #[allow(clippy::cast_possible_wrap)]
                sqlx::query(
                    "UPDATE payments_projection
                     SET status = $2, refund_amount_cents = $3, refund_reason = $4,
                         refunded_at = $5, updated_at = NOW()
                     WHERE payment_id = $1",
                )
                .bind(payment_id.as_uuid())
                .bind(&status)
                .bind(amount.cents() as i64)
                .bind(reason)
                .bind(refunded_at)
                .execute(self.pool.as_ref())
                .await
                .map_err(|e| {
                    ProjectionError::Storage(format!("Failed to update payment refund: {e}"))
                })?;

                Ok(())
            }

            // Ignore other events
            _ => Ok(()),
        }
    }
}

// ============================================================================
// PaymentProjectionQuery Implementation
// ============================================================================

#[async_trait::async_trait]
impl crate::aggregates::payment::PaymentProjectionQuery for PostgresPaymentsProjection {
    async fn load_payment(&self, payment_id: &PaymentId) -> std::result::Result<Option<Payment>, String> {
        self.get_payment(payment_id)
            .await
            .map_err(|e| format!("Failed to load payment: {e}"))
    }

    async fn load_customer_payments(&self, customer_id: &CustomerId, limit: usize, offset: usize) -> std::result::Result<Vec<Payment>, String> {
        // Convert usize to i64 for SQL query
        #[allow(clippy::cast_possible_wrap)]
        let limit_i64 = limit as i64;
        #[allow(clippy::cast_possible_wrap)]
        let offset_i64 = offset as i64;

        // Call the existing list_customer_payments method
        self.list_customer_payments(customer_id, limit_i64, offset_i64)
            .await
            .map_err(|e| format!("Failed to load customer payments: {e}"))
    }
}
