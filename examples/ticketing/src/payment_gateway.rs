//! Mock payment gateway for development and testing.
//!
//! This module provides a simplified payment gateway interface compatible with
//! services like Stripe, `PayPal`, and Apple Pay. In production, this would be
//! replaced with actual payment service integrations.

use crate::types::{Money, PaymentMethod, PaymentId};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Payment gateway result
pub type GatewayResult<T> = Result<T, PaymentGatewayError>;

/// Payment gateway error
#[derive(Debug, Clone)]
pub enum PaymentGatewayError {
    /// Card declined
    CardDeclined {
        /// Decline reason
        reason: String
    },
    /// Insufficient funds
    InsufficientFunds,
    /// Invalid payment method
    InvalidPaymentMethod {
        /// Invalid reason
        reason: String
    },
    /// Gateway timeout
    Timeout,
    /// Other error
    Other {
        /// Error message
        message: String
    },
}

impl std::fmt::Display for PaymentGatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CardDeclined { reason } => write!(f, "Card declined: {reason}"),
            Self::InsufficientFunds => write!(f, "Insufficient funds"),
            Self::InvalidPaymentMethod { reason } => write!(f, "Invalid payment method: {reason}"),
            Self::Timeout => write!(f, "Gateway timeout"),
            Self::Other { message } => write!(f, "Payment error: {message}"),
        }
    }
}

impl std::error::Error for PaymentGatewayError {}

/// Payment gateway transaction result
#[derive(Debug, Clone)]
pub struct PaymentTransaction {
    /// Payment ID (internal)
    pub payment_id: PaymentId,
    /// Gateway transaction ID
    pub transaction_id: String,
    /// Amount charged
    pub amount: Money,
    /// Payment method used
    pub payment_method: PaymentMethod,
}

/// Payment gateway trait
///
/// Abstraction over payment processors like Stripe, `PayPal`, Apple Pay, etc.
pub trait PaymentGateway: Send + Sync {
    /// Process a payment
    ///
    /// # Errors
    ///
    /// Returns error if payment fails
    fn process_payment(
        &self,
        payment_id: PaymentId,
        amount: Money,
        payment_method: PaymentMethod,
    ) -> Pin<Box<dyn Future<Output = GatewayResult<PaymentTransaction>> + Send>>;

    /// Refund a payment
    ///
    /// # Errors
    ///
    /// Returns error if refund fails
    fn refund_payment(
        &self,
        transaction_id: &str,
        amount: Money,
    ) -> Pin<Box<dyn Future<Output = GatewayResult<String>> + Send>>;
}

/// Mock payment gateway (always succeeds for development)
///
/// This gateway simulates successful payment processing for all requests.
/// In production, replace with real gateway integrations.
#[derive(Clone, Debug)]
pub struct MockPaymentGateway;

impl MockPaymentGateway {
    /// Creates a new mock payment gateway
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Creates an Arc-wrapped instance for sharing
    #[must_use]
    pub fn shared() -> Arc<dyn PaymentGateway> {
        Arc::new(Self::new())
    }
}

impl Default for MockPaymentGateway {
    fn default() -> Self {
        Self::new()
    }
}

impl PaymentGateway for MockPaymentGateway {
    fn process_payment(
        &self,
        payment_id: PaymentId,
        amount: Money,
        payment_method: PaymentMethod,
    ) -> Pin<Box<dyn Future<Output = GatewayResult<PaymentTransaction>> + Send>> {
        Box::pin(async move {
            // Simulate network delay
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            // Generate mock transaction ID
            let transaction_id = format!("mock_txn_{}", uuid::Uuid::new_v4());

            tracing::info!(
                payment_id = %payment_id.as_uuid(),
                amount = amount.cents(),
                transaction_id = %transaction_id,
                "Mock payment processed successfully"
            );

            Ok(PaymentTransaction {
                payment_id,
                transaction_id,
                amount,
                payment_method,
            })
        })
    }

    fn refund_payment(
        &self,
        transaction_id: &str,
        amount: Money,
    ) -> Pin<Box<dyn Future<Output = GatewayResult<String>> + Send>> {
        let transaction_id = transaction_id.to_string();
        Box::pin(async move {
            // Simulate network delay
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            // Generate mock refund ID
            let refund_id = format!("mock_refund_{}", uuid::Uuid::new_v4());

            tracing::info!(
                transaction_id = %transaction_id,
                amount = amount.cents(),
                refund_id = %refund_id,
                "Mock refund processed successfully"
            );

            Ok(refund_id)
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_payment_success() {
        let gateway = MockPaymentGateway::new();
        let payment_id = PaymentId::new();
        let amount = Money::from_dollars(100);
        let payment_method = PaymentMethod::CreditCard {
            last_four: "4242".to_string(),
        };

        let result = gateway
            .process_payment(payment_id, amount, payment_method)
            .await;

        assert!(result.is_ok());
        let transaction = result.unwrap();
        assert_eq!(transaction.payment_id, payment_id);
        assert_eq!(transaction.amount, amount);
        assert!(transaction.transaction_id.starts_with("mock_txn_"));
    }

    #[tokio::test]
    async fn test_mock_refund_success() {
        let gateway = MockPaymentGateway::new();
        let amount = Money::from_dollars(100);

        let result = gateway.refund_payment("txn_123", amount).await;

        assert!(result.is_ok());
        let refund_id = result.unwrap();
        assert!(refund_id.starts_with("mock_refund_"));
    }
}
