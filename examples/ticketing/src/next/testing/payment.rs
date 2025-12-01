//! In-memory payment projection infrastructure for testing.
//!
//! This module provides:
//! - [`InMemoryPaymentProjections`]: Shared store for payment projection data
//! - [`InMemoryPaymentProjector`]: Writes to the store when events are persisted
//! - [`InMemoryPaymentQueryFetcher`]: Reads from the store for command validation

use std::collections::HashMap;
use std::sync::Arc;

use composable_rust_next::{
    FetchResult, ProjectionError, ProjectionQueries, Projector, QueryFetcher, SerializedEvent,
};
use tokio::sync::RwLock;

use crate::next::payment::{PaymentCommand, PaymentDto, PaymentDtoStatus, PaymentEvent};
use crate::types::{CustomerId, Money, PaymentId, ReservationId};

// ═══════════════════════════════════════════════════════════════════════════
// Shared Projection Store
// ═══════════════════════════════════════════════════════════════════════════

/// In-memory payment data.
#[derive(Debug, Clone)]
pub struct PaymentProjectionData {
    /// Payment ID
    pub id: PaymentId,
    /// Reservation ID
    pub reservation_id: ReservationId,
    /// Customer ID
    pub customer_id: CustomerId,
    /// Amount
    pub amount: Money,
    /// Payment method (stored as string)
    pub payment_method: String,
    /// Status
    pub status: PaymentDtoStatus,
    /// Transaction ID (if succeeded)
    pub transaction_id: Option<String>,
    /// Failure reason (if failed)
    pub failure_reason: Option<String>,
    /// Refund amount (if refunded)
    pub refund_amount: Option<Money>,
    /// Refund reason (if refunded)
    pub refund_reason: Option<String>,
}

impl From<PaymentProjectionData> for PaymentDto {
    fn from(data: PaymentProjectionData) -> Self {
        Self {
            id: data.id,
            reservation_id: data.reservation_id,
            customer_id: data.customer_id,
            amount: data.amount,
            payment_method: data.payment_method,
            status: data.status,
            transaction_id: data.transaction_id,
            failure_reason: data.failure_reason,
            refund_amount: data.refund_amount,
            refund_reason: data.refund_reason,
        }
    }
}

/// In-memory projection store for payment data.
///
/// This store is shared between:
/// - [`InMemoryPaymentProjector`]: Writes when events are projected
/// - [`InMemoryPaymentQueryFetcher`]: Reads when preparing commands
///
/// # Thread Safety
///
/// Uses `Arc<RwLock<...>>` for safe concurrent access.
#[derive(Debug, Clone, Default)]
pub struct InMemoryPaymentProjections {
    /// Payment data keyed by payment_id
    payments: Arc<RwLock<HashMap<PaymentId, PaymentProjectionData>>>,
}

impl InMemoryPaymentProjections {
    /// Create a new empty projection store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get payment DTO by ID.
    ///
    /// Returns `None` if payment doesn't exist.
    pub async fn get_payment(&self, payment_id: PaymentId) -> Option<PaymentDto> {
        let payments = self.payments.read().await;
        payments.get(&payment_id).cloned().map(Into::into)
    }

    /// List payments by customer.
    pub async fn list_by_customer(&self, customer_id: CustomerId) -> Vec<PaymentDto> {
        let payments = self.payments.read().await;
        payments
            .values()
            .filter(|p| p.customer_id == customer_id)
            .cloned()
            .map(Into::into)
            .collect()
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Write Methods (used by projector)
    // ═══════════════════════════════════════════════════════════════════════

    /// Insert a new payment (from PaymentProcessed event).
    pub(crate) async fn insert_payment(
        &self,
        payment_id: PaymentId,
        reservation_id: ReservationId,
        customer_id: CustomerId,
        amount: Money,
        payment_method: String,
    ) {
        let mut payments = self.payments.write().await;
        payments.insert(
            payment_id,
            PaymentProjectionData {
                id: payment_id,
                reservation_id,
                customer_id,
                amount,
                payment_method,
                status: PaymentDtoStatus::Processing,
                transaction_id: None,
                failure_reason: None,
                refund_amount: None,
                refund_reason: None,
            },
        );
    }

    /// Mark payment as succeeded.
    pub(crate) async fn mark_succeeded(&self, payment_id: PaymentId, transaction_id: String) {
        let mut payments = self.payments.write().await;
        if let Some(payment) = payments.get_mut(&payment_id) {
            payment.status = PaymentDtoStatus::Succeeded;
            payment.transaction_id = Some(transaction_id);
        }
    }

    /// Mark payment as failed.
    pub(crate) async fn mark_failed(&self, payment_id: PaymentId, reason: String) {
        let mut payments = self.payments.write().await;
        if let Some(payment) = payments.get_mut(&payment_id) {
            payment.status = PaymentDtoStatus::Failed;
            payment.failure_reason = Some(reason);
        }
    }

    /// Mark payment as refunded.
    pub(crate) async fn mark_refunded(&self, payment_id: PaymentId, amount: Money, reason: String) {
        let mut payments = self.payments.write().await;
        if let Some(payment) = payments.get_mut(&payment_id) {
            payment.status = PaymentDtoStatus::Refunded;
            payment.refund_amount = Some(amount);
            payment.refund_reason = Some(reason);
        }
    }
}

impl ProjectionQueries for InMemoryPaymentProjections {
    type Error = std::convert::Infallible;
}

// ═══════════════════════════════════════════════════════════════════════════
// Projector Implementation
// ═══════════════════════════════════════════════════════════════════════════

/// In-memory projector that writes to [`InMemoryPaymentProjections`].
///
/// This projector deserializes payment events and updates the shared
/// projection store, enabling the query fetcher to read the latest state.
#[derive(Debug, Clone)]
pub struct InMemoryPaymentProjector {
    projections: InMemoryPaymentProjections,
}

impl InMemoryPaymentProjector {
    /// Create a new projector with the given shared store.
    #[must_use]
    pub fn new(projections: InMemoryPaymentProjections) -> Self {
        Self { projections }
    }
}

impl Projector for InMemoryPaymentProjector {
    fn project(
        &self,
        events: &[SerializedEvent],
    ) -> impl std::future::Future<Output = Result<(), ProjectionError>> + Send {
        let events_owned: Vec<SerializedEvent> = events.to_vec();
        let projections = self.projections.clone();

        async move {
            for event in events_owned {
                // Skip events that aren't payment events
                if !event.event_type.starts_with("Payment") {
                    continue;
                }

                let payment_event: PaymentEvent =
                    bincode::deserialize(&event.payload).map_err(|e| {
                        ProjectionError::Deserialization(format!(
                            "failed to deserialize payment event: {e}"
                        ))
                    })?;

                match payment_event {
                    PaymentEvent::PaymentProcessed {
                        payment_id,
                        reservation_id,
                        customer_id,
                        amount,
                        payment_method,
                        ..
                    } => {
                        let method_str = format!("{payment_method:?}");
                        projections
                            .insert_payment(
                                payment_id,
                                reservation_id,
                                customer_id,
                                amount,
                                method_str,
                            )
                            .await;
                    }

                    PaymentEvent::PaymentSucceeded {
                        payment_id,
                        transaction_id,
                        ..
                    } => {
                        projections.mark_succeeded(payment_id, transaction_id).await;
                    }

                    PaymentEvent::PaymentFailed {
                        payment_id, reason, ..
                    } => {
                        projections.mark_failed(payment_id, reason).await;
                    }

                    PaymentEvent::PaymentRefunded {
                        payment_id,
                        amount,
                        reason,
                        ..
                    } => {
                        projections.mark_refunded(payment_id, amount, reason).await;
                    }
                }
            }

            Ok(())
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Query Fetcher Implementation
// ═══════════════════════════════════════════════════════════════════════════

/// In-memory query fetcher that reads from [`InMemoryPaymentProjections`].
///
/// This fetcher populates the `fetched` field in payment commands with
/// data from the shared projection store.
#[derive(Debug, Clone)]
pub struct InMemoryPaymentQueryFetcher {
    projections: InMemoryPaymentProjections,
}

impl InMemoryPaymentQueryFetcher {
    /// Create a new query fetcher with the given shared store.
    #[must_use]
    pub fn new(projections: InMemoryPaymentProjections) -> Self {
        Self { projections }
    }
}

impl<PQ: ProjectionQueries> QueryFetcher<PaymentCommand, PQ> for InMemoryPaymentQueryFetcher {
    type Error = std::convert::Infallible;

    fn fetch(
        &self,
        input: PaymentCommand,
        _projections: &PQ, // Ignored - we use our own projections
    ) -> impl std::future::Future<Output = Result<FetchResult<PaymentCommand>, Self::Error>> + Send
    {
        let projections = self.projections.clone();

        async move {
            let prepared = match input {
                // ═══════════════════════════════════════════════════════════
                // Write Commands - need validation data
                // ═══════════════════════════════════════════════════════════
                PaymentCommand::ProcessPayment {
                    payment_id,
                    reservation_id,
                    customer_id,
                    amount,
                    payment_method,
                    fetched: _,
                } => {
                    let fetched = projections.get_payment(payment_id).await;
                    PaymentCommand::ProcessPayment {
                        payment_id,
                        reservation_id,
                        customer_id,
                        amount,
                        payment_method,
                        fetched,
                    }
                }

                PaymentCommand::RefundPayment {
                    payment_id,
                    amount,
                    reason,
                    fetched: _,
                } => {
                    let fetched = projections.get_payment(payment_id).await;
                    PaymentCommand::RefundPayment {
                        payment_id,
                        amount,
                        reason,
                        fetched,
                    }
                }

                PaymentCommand::SimulatePaymentFailure { payment_id, reason } => {
                    // No fetched data needed for simulation
                    PaymentCommand::SimulatePaymentFailure { payment_id, reason }
                }

                // ═══════════════════════════════════════════════════════════
                // Query Commands - need read data
                // ═══════════════════════════════════════════════════════════
                PaymentCommand::GetPayment { payment_id, fetched: _ } => {
                    let fetched = projections.get_payment(payment_id).await;
                    PaymentCommand::GetPayment { payment_id, fetched }
                }

                PaymentCommand::ListCustomerPayments {
                    customer_id,
                    fetched: _,
                } => {
                    let fetched = projections.list_by_customer(customer_id).await;
                    PaymentCommand::ListCustomerPayments { customer_id, fetched }
                }
            };

            Ok(FetchResult::new_entity(prepared))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Test Harness
// ═══════════════════════════════════════════════════════════════════════════

use composable_rust_next::{
    testing::{InMemoryEventBus, InMemoryEventStore},
    Clock, Handler, HandlerEnvironment, HandlerError, HandleResult, MetadataContext,
    NoOpCallExecutor,
};

use crate::next::payment::{PaymentBusinessLogic, PaymentError, PaymentResponse};

/// Test environment that uses our custom projector type.
#[derive(Clone)]
pub struct PaymentTestEnvironment<C: Clock> {
    /// The clock
    pub clock: C,
    /// Shared event store
    pub event_store: InMemoryEventStore,
    /// Projector that writes to shared projections
    pub projector: InMemoryPaymentProjector,
    /// Event bus for broadcasts
    pub event_bus: InMemoryEventBus,
    /// Shared projections
    pub projections: InMemoryPaymentProjections,
    /// Metadata context
    pub metadata: MetadataContext,
}

impl<C: Clock + Send + Sync> HandlerEnvironment for PaymentTestEnvironment<C> {
    type Clock = C;
    type EventStore = InMemoryEventStore;
    type Projector = InMemoryPaymentProjector;
    type EventBus = InMemoryEventBus;
    type Projections = InMemoryPaymentProjections;

    fn clock(&self) -> &Self::Clock {
        &self.clock
    }

    fn event_store(&self) -> &Self::EventStore {
        &self.event_store
    }

    fn projector(&self) -> Option<&Self::Projector> {
        Some(&self.projector)
    }

    fn event_bus(&self) -> Option<&Self::EventBus> {
        Some(&self.event_bus)
    }

    fn broadcast_topic(&self) -> &str {
        "payment-events"
    }

    fn projections(&self) -> &Self::Projections {
        &self.projections
    }

    fn metadata(&self) -> &MetadataContext {
        &self.metadata
    }
}

/// Fully-wired test harness for Payment handler integration tests.
pub struct PaymentTestHarness {
    projections: InMemoryPaymentProjections,
    handler: Handler<
        PaymentBusinessLogic,
        NoOpCallExecutor,
        InMemoryPaymentQueryFetcher,
        PaymentTestEnvironment<composable_rust_next::FixedClock>,
    >,
    event_store: InMemoryEventStore,
    event_bus: InMemoryEventBus,
}

impl PaymentTestHarness {
    /// Create a new test harness with all infrastructure wired together.
    #[must_use]
    pub fn new() -> Self {
        Self::with_clock(composable_rust_next::FixedClock::new(chrono::Utc::now()))
    }

    /// Create a test harness with a specific clock.
    #[must_use]
    pub fn with_clock(clock: composable_rust_next::FixedClock) -> Self {
        let projections = InMemoryPaymentProjections::new();
        let projector = InMemoryPaymentProjector::new(projections.clone());
        let query_fetcher = InMemoryPaymentQueryFetcher::new(projections.clone());
        let event_store = InMemoryEventStore::new();
        let event_bus = InMemoryEventBus::new();

        let env = PaymentTestEnvironment {
            clock,
            event_store: event_store.clone(),
            projector,
            event_bus: event_bus.clone(),
            projections: projections.clone(),
            metadata: MetadataContext::new(),
        };

        let handler = Handler::new(PaymentBusinessLogic, NoOpCallExecutor, query_fetcher, env);

        Self {
            projections,
            handler,
            event_store,
            event_bus,
        }
    }

    /// Handle a payment command through the full pipeline.
    pub async fn handle(
        &self,
        command: PaymentCommand,
    ) -> Result<HandleResult<PaymentResponse>, HandlerError<PaymentError>> {
        self.handler.handle(command).await
    }

    /// Get payment DTO (for test assertions).
    pub async fn get_payment(&self, payment_id: PaymentId) -> Option<PaymentDto> {
        self.projections.get_payment(payment_id).await
    }

    /// List payments by customer (for test assertions).
    pub async fn list_by_customer(&self, customer_id: CustomerId) -> Vec<PaymentDto> {
        self.projections.list_by_customer(customer_id).await
    }

    /// Get total event count in the event store (for test assertions).
    #[must_use]
    pub fn total_event_count(&self) -> usize {
        self.event_store.total_event_count()
    }

    /// Get published events from the event bus (for test assertions).
    #[must_use]
    pub fn published_events(&self) -> Vec<(String, SerializedEvent)> {
        self.event_bus.published_events()
    }
}

impl Default for PaymentTestHarness {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::types::PaymentMethod;
    use chrono::Utc;

    #[tokio::test]
    async fn projections_store_payment() {
        let projections = InMemoryPaymentProjections::new();
        let payment_id = PaymentId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

        projections
            .insert_payment(
                payment_id,
                reservation_id,
                customer_id,
                Money::from_dollars(100),
                "CreditCard".to_string(),
            )
            .await;

        let dto = projections.get_payment(payment_id).await.unwrap();
        assert_eq!(dto.id, payment_id);
        assert_eq!(dto.status, PaymentDtoStatus::Processing);
        assert_eq!(dto.amount, Money::from_dollars(100));
    }

    #[tokio::test]
    async fn projections_track_succeeded_payment() {
        let projections = InMemoryPaymentProjections::new();
        let payment_id = PaymentId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

        // Insert payment
        projections
            .insert_payment(
                payment_id,
                reservation_id,
                customer_id,
                Money::from_dollars(100),
                "CreditCard".to_string(),
            )
            .await;

        // Mark succeeded
        projections
            .mark_succeeded(payment_id, "txn_123".to_string())
            .await;

        let dto = projections.get_payment(payment_id).await.unwrap();
        assert_eq!(dto.status, PaymentDtoStatus::Succeeded);
        assert_eq!(dto.transaction_id, Some("txn_123".to_string()));
    }

    #[tokio::test]
    async fn projections_track_failed_payment() {
        let projections = InMemoryPaymentProjections::new();
        let payment_id = PaymentId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

        // Insert payment
        projections
            .insert_payment(
                payment_id,
                reservation_id,
                customer_id,
                Money::from_dollars(100),
                "CreditCard".to_string(),
            )
            .await;

        // Mark failed
        projections
            .mark_failed(payment_id, "Card declined".to_string())
            .await;

        let dto = projections.get_payment(payment_id).await.unwrap();
        assert_eq!(dto.status, PaymentDtoStatus::Failed);
        assert_eq!(dto.failure_reason, Some("Card declined".to_string()));
    }

    #[tokio::test]
    async fn projections_list_by_customer() {
        let projections = InMemoryPaymentProjections::new();
        let customer_id = CustomerId::new();
        let other_customer_id = CustomerId::new();

        // Insert payments for our customer
        projections
            .insert_payment(
                PaymentId::new(),
                ReservationId::new(),
                customer_id,
                Money::from_dollars(100),
                "CreditCard".to_string(),
            )
            .await;

        projections
            .insert_payment(
                PaymentId::new(),
                ReservationId::new(),
                customer_id,
                Money::from_dollars(50),
                "PayPal".to_string(),
            )
            .await;

        // Insert payment for other customer
        projections
            .insert_payment(
                PaymentId::new(),
                ReservationId::new(),
                other_customer_id,
                Money::from_dollars(200),
                "CreditCard".to_string(),
            )
            .await;

        // List should only return our customer's payments
        let payments = projections.list_by_customer(customer_id).await;
        assert_eq!(payments.len(), 2);
    }

    #[tokio::test]
    async fn projector_deserializes_and_updates_store() {
        let projections = InMemoryPaymentProjections::new();
        let projector = InMemoryPaymentProjector::new(projections.clone());

        let payment_id = PaymentId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

        // Create serialized events
        let processed_event = PaymentEvent::PaymentProcessed {
            payment_id,
            reservation_id,
            customer_id,
            amount: Money::from_dollars(100),
            payment_method: PaymentMethod::CreditCard {
                last_four: "4242".to_string(),
            },
            processed_at: Utc::now(),
        };

        let succeeded_event = PaymentEvent::PaymentSucceeded {
            payment_id,
            transaction_id: "txn_abc".to_string(),
            succeeded_at: Utc::now(),
        };

        let events = vec![
            SerializedEvent {
                event_type: "PaymentProcessed".to_string(),
                payload: bincode::serialize(&processed_event).unwrap(),
                metadata: None,
                version: None,
            },
            SerializedEvent {
                event_type: "PaymentSucceeded".to_string(),
                payload: bincode::serialize(&succeeded_event).unwrap(),
                metadata: None,
                version: None,
            },
        ];

        // Project
        projector.project(&events).await.unwrap();

        // Verify
        let dto = projections.get_payment(payment_id).await.unwrap();
        assert_eq!(dto.status, PaymentDtoStatus::Succeeded);
        assert_eq!(dto.transaction_id, Some("txn_abc".to_string()));
    }

    #[tokio::test]
    async fn query_fetcher_populates_fetched_field() {
        let projections = InMemoryPaymentProjections::new();
        let payment_id = PaymentId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

        // Pre-seed projections
        projections
            .insert_payment(
                payment_id,
                reservation_id,
                customer_id,
                Money::from_dollars(100),
                "CreditCard".to_string(),
            )
            .await;
        projections
            .mark_succeeded(payment_id, "txn_xyz".to_string())
            .await;

        let fetcher = InMemoryPaymentQueryFetcher::new(projections.clone());

        // Create refund command with fetched: None
        let command = PaymentCommand::RefundPayment {
            payment_id,
            amount: Money::from_dollars(50),
            reason: "Customer request".to_string(),
            fetched: None,
        };

        // Fetch should populate fetched
        let result = fetcher.fetch(command, &projections).await.unwrap();

        match result.input {
            PaymentCommand::RefundPayment { fetched, .. } => {
                let dto = fetched.expect("fetched should be populated");
                assert_eq!(dto.status, PaymentDtoStatus::Succeeded);
                assert_eq!(dto.amount, Money::from_dollars(100));
            }
            _ => panic!("Expected RefundPayment command"),
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Handler Integration Tests (Level 2)
    // ═══════════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn harness_process_payment_full_flow() {
        let harness = PaymentTestHarness::new();
        let payment_id = PaymentId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

        // Process payment
        let result = harness
            .handle(PaymentCommand::ProcessPayment {
                payment_id,
                reservation_id,
                customer_id,
                amount: Money::from_dollars(100),
                payment_method: PaymentMethod::CreditCard {
                    last_four: "4242".to_string(),
                },
                fetched: None,
            })
            .await
            .expect("ProcessPayment should succeed");

        assert!(result.is_command());
        // Payment processing emits multiple events: PaymentProcessed, PaymentSucceeded
        assert!(result.event_count() >= 1);

        // Verify payment exists in projections
        let dto = harness.get_payment(payment_id).await.unwrap();
        assert_eq!(dto.id, payment_id);
        assert_eq!(dto.reservation_id, reservation_id);
        assert_eq!(dto.customer_id, customer_id);
        assert_eq!(dto.amount, Money::from_dollars(100));
    }

    #[tokio::test]
    async fn harness_duplicate_payment_fails() {
        let harness = PaymentTestHarness::new();
        let payment_id = PaymentId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

        // First payment succeeds
        harness
            .handle(PaymentCommand::ProcessPayment {
                payment_id,
                reservation_id,
                customer_id,
                amount: Money::from_dollars(100),
                payment_method: PaymentMethod::CreditCard {
                    last_four: "4242".to_string(),
                },
                fetched: None,
            })
            .await
            .unwrap();

        // Second payment with same ID should fail
        let result = harness
            .handle(PaymentCommand::ProcessPayment {
                payment_id,
                reservation_id,
                customer_id,
                amount: Money::from_dollars(100),
                payment_method: PaymentMethod::CreditCard {
                    last_four: "4242".to_string(),
                },
                fetched: None,
            })
            .await;

        assert!(result.is_err());
        match result {
            Err(HandlerError::Business(PaymentError::AlreadyExists(_))) => {}
            other => panic!("Expected AlreadyExists error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn harness_refund_payment_flow() {
        let harness = PaymentTestHarness::new();
        let payment_id = PaymentId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

        // Process payment first
        harness
            .handle(PaymentCommand::ProcessPayment {
                payment_id,
                reservation_id,
                customer_id,
                amount: Money::from_dollars(100),
                payment_method: PaymentMethod::CreditCard {
                    last_four: "4242".to_string(),
                },
                fetched: None,
            })
            .await
            .unwrap();

        // Refund
        let result = harness
            .handle(PaymentCommand::RefundPayment {
                payment_id,
                amount: Money::from_dollars(50),
                reason: "Customer request".to_string(),
                fetched: None,
            })
            .await
            .expect("Refund should succeed");

        assert!(result.is_command());

        // Verify refund in projection
        let dto = harness.get_payment(payment_id).await.unwrap();
        assert_eq!(dto.status, PaymentDtoStatus::Refunded);
        assert_eq!(dto.refund_amount, Some(Money::from_dollars(50)));
        assert_eq!(dto.refund_reason, Some("Customer request".to_string()));
    }

    #[tokio::test]
    async fn harness_refund_nonexistent_payment_fails() {
        let harness = PaymentTestHarness::new();
        let payment_id = PaymentId::new();

        // Try to refund a payment that doesn't exist
        let result = harness
            .handle(PaymentCommand::RefundPayment {
                payment_id,
                amount: Money::from_dollars(50),
                reason: "Customer request".to_string(),
                fetched: None,
            })
            .await;

        assert!(result.is_err());
        match result {
            Err(HandlerError::Business(PaymentError::NotFound(_))) => {}
            other => panic!("Expected NotFound error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn harness_list_customer_payments() {
        let harness = PaymentTestHarness::new();
        let customer_id = CustomerId::new();

        // Create multiple payments for the same customer
        for i in 0..3 {
            harness
                .handle(PaymentCommand::ProcessPayment {
                    payment_id: PaymentId::new(),
                    reservation_id: ReservationId::new(),
                    customer_id,
                    amount: Money::from_dollars(100 + i * 50),
                    payment_method: PaymentMethod::CreditCard {
                        last_four: format!("424{i}"),
                    },
                    fetched: None,
                })
                .await
                .unwrap();
        }

        // List customer payments
        let payments = harness.list_by_customer(customer_id).await;
        assert_eq!(payments.len(), 3);
    }

    #[tokio::test]
    async fn harness_events_are_broadcast() {
        let harness = PaymentTestHarness::new();

        harness
            .handle(PaymentCommand::ProcessPayment {
                payment_id: PaymentId::new(),
                reservation_id: ReservationId::new(),
                customer_id: CustomerId::new(),
                amount: Money::from_dollars(100),
                payment_method: PaymentMethod::CreditCard {
                    last_four: "4242".to_string(),
                },
                fetched: None,
            })
            .await
            .unwrap();

        // Check event bus
        let published = harness.published_events();
        assert!(!published.is_empty());
        assert_eq!(published[0].0, "payment-events"); // topic
    }

    #[tokio::test]
    async fn projections_track_refunded_payment() {
        let projections = InMemoryPaymentProjections::new();
        let payment_id = PaymentId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

        // Insert payment and mark as succeeded (prerequisite for refund)
        projections
            .insert_payment(
                payment_id,
                reservation_id,
                customer_id,
                Money::from_dollars(100),
                "CreditCard".to_string(),
            )
            .await;
        projections
            .mark_succeeded(payment_id, "txn_123".to_string())
            .await;

        // Mark refunded
        projections
            .mark_refunded(payment_id, Money::from_dollars(50), "Customer request".to_string())
            .await;

        let dto = projections.get_payment(payment_id).await.unwrap();
        assert_eq!(dto.status, PaymentDtoStatus::Refunded);
        assert_eq!(dto.refund_amount, Some(Money::from_dollars(50)));
        assert_eq!(dto.refund_reason, Some("Customer request".to_string()));
    }

    #[tokio::test]
    async fn projector_skips_non_payment_events() {
        let projections = InMemoryPaymentProjections::new();
        let projector = InMemoryPaymentProjector::new(projections.clone());

        // Create a non-payment event
        let fake_event = SerializedEvent {
            event_type: "InventoryInitialized".to_string(), // Not a payment event
            payload: vec![1, 2, 3], // Invalid payload
            metadata: None,
            version: None,
        };

        // Project should succeed (skip the event, not fail)
        let result = projector.project(&[fake_event]).await;
        assert!(
            result.is_ok(),
            "Projector should skip non-payment events without error"
        );
    }

    #[tokio::test]
    async fn projector_handles_failed_payment() {
        let projections = InMemoryPaymentProjections::new();
        let projector = InMemoryPaymentProjector::new(projections.clone());

        let payment_id = PaymentId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

        // First, process the payment
        let processed_event = PaymentEvent::PaymentProcessed {
            payment_id,
            reservation_id,
            customer_id,
            amount: Money::from_dollars(100),
            payment_method: PaymentMethod::CreditCard {
                last_four: "4242".to_string(),
            },
            processed_at: Utc::now(),
        };

        let failed_event = PaymentEvent::PaymentFailed {
            payment_id,
            reason: "Insufficient funds".to_string(),
            failed_at: Utc::now(),
        };

        let events = vec![
            SerializedEvent {
                event_type: "PaymentProcessed".to_string(),
                payload: bincode::serialize(&processed_event).unwrap(),
                metadata: None,
                version: None,
            },
            SerializedEvent {
                event_type: "PaymentFailed".to_string(),
                payload: bincode::serialize(&failed_event).unwrap(),
                metadata: None,
                version: None,
            },
        ];

        // Project both events
        projector.project(&events).await.unwrap();

        // Verify final state is Failed
        let dto = projections.get_payment(payment_id).await.unwrap();
        assert_eq!(dto.status, PaymentDtoStatus::Failed);
        assert_eq!(dto.failure_reason, Some("Insufficient funds".to_string()));
    }

    #[tokio::test]
    async fn query_fetcher_returns_none_for_missing_payment() {
        let projections = InMemoryPaymentProjections::new();
        let fetcher = InMemoryPaymentQueryFetcher::new(projections.clone());
        let payment_id = PaymentId::new();

        // Create command for non-existent payment
        let command = PaymentCommand::GetPayment {
            payment_id,
            fetched: None,
        };

        // Fetch should succeed but with fetched = None
        let result = fetcher.fetch(command, &projections).await.unwrap();

        match result.input {
            PaymentCommand::GetPayment { fetched, .. } => {
                assert!(fetched.is_none(), "fetched should be None for missing payment");
            }
            _ => panic!("Expected GetPayment command"),
        }
    }
}
