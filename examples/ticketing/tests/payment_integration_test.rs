//! Payment integration tests.
//!
//! Tests complete payment flows including refunds, failures, and edge cases.
//! Uses ReducerTest for unit-level testing of payment aggregate business logic.
//!
//! Run with: `cargo test --test payment_integration_test`

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use ticketing::aggregates::payment::{PaymentAction, PaymentReducer, PaymentEnvironment};
use ticketing::types::{
    CustomerId, Money, Payment, PaymentId, PaymentMethod, PaymentState, PaymentStatus, ReservationId,
};
use composable_rust_core::environment::SystemClock;
use composable_rust_core::reducer::Reducer;
use composable_rust_core::stream::StreamId;
use composable_rust_testing::{assertions, mocks::{InMemoryEventBus, InMemoryEventStore}, ReducerTest};
use std::sync::Arc;

// Mock projection query for tests
#[derive(Clone)]
struct MockPaymentQuery;

impl ticketing::aggregates::payment::PaymentProjectionQuery for MockPaymentQuery {
    fn load_payment(
        &self,
        _payment_id: &PaymentId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Payment>, String>> + Send + '_>> {
        // Return None for tests - state will be built from events
        Box::pin(async move { Ok(None) })
    }
}

fn create_test_env() -> PaymentEnvironment {
    PaymentEnvironment::new(
        Arc::new(SystemClock),
        Arc::new(InMemoryEventStore::new()),
        Arc::new(InMemoryEventBus::new()),
        StreamId::new("payment-test"),
        Arc::new(MockPaymentQuery),
    )
}

/// Test 1: Successful Payment Flow
///
/// Verifies that a payment can be processed successfully and transitions to Captured state.
#[tokio::test]
async fn test_successful_payment_flow() {
    println!("🧪 Test 1: Successful Payment Flow");

    let payment_id = PaymentId::new();
    let reservation_id = ReservationId::new();
    let amount = Money::from_dollars(100);

    ReducerTest::new(PaymentReducer::new())
        .with_env(create_test_env())
        .given_state(PaymentState::new())
        .when_action(PaymentAction::ProcessPayment {
            payment_id,
            reservation_id,
            amount,
            payment_method: PaymentMethod::CreditCard {
                last_four: "4242".to_string(),
            },
        })
        .then_state(move |state| {
            assert_eq!(state.count(), 1);
            let payment = state.get(&payment_id).unwrap();
            assert_eq!(payment.status, PaymentStatus::Captured);
            assert_eq!(payment.amount, amount);
            assert_eq!(payment.reservation_id, reservation_id);
        })
        .then_effects(|effects| {
            // Should return 4 effects:
            // 2 for PaymentProcessed (AppendEvents + PublishEvent)
            // 2 for PaymentSucceeded (AppendEvents + PublishEvent)
            assert_eq!(effects.len(), 4);
        })
        .run();

    println!("  ✅ Payment processed successfully");
}

/// Test 2: Payment Refund Flow
///
/// Verifies that a completed payment can be refunded.
#[tokio::test]
async fn test_payment_refund_flow() {
    println!("🧪 Test 2: Payment Refund Flow");

    let payment_id = PaymentId::new();
    let reservation_id = ReservationId::new();
    let amount = Money::from_dollars(100);

    ReducerTest::new(PaymentReducer::new())
        .with_env(create_test_env())
        .given_state({
            let mut state = PaymentState::new();
            let mut payment = Payment::new(
                payment_id,
                reservation_id,
                CustomerId::new(),
                amount,
                PaymentMethod::CreditCard {
                    last_four: "4242".to_string(),
                },
            );
            payment.status = PaymentStatus::Captured;
            state.payments.insert(payment_id, payment);
            state
        })
        .when_action(PaymentAction::RefundPayment {
            payment_id,
            amount,
            reason: "Customer requested refund".to_string(),
        })
        .then_state(move |state| {
            let payment = state.get(&payment_id).unwrap();
            assert!(matches!(
                payment.status,
                PaymentStatus::Refunded { amount: refund_amount } if refund_amount == amount
            ));
        })
        .then_effects(|effects| {
            // Should return 2 effects: AppendEvents + PublishEvent
            assert_eq!(effects.len(), 2);
        })
        .run();

    println!("  ✅ Payment refunded successfully");
}

/// Test 3: Cannot Refund Non-Captured Payment
///
/// Verifies that attempting to refund a payment that isn't captured is rejected.
#[tokio::test]
async fn test_cannot_refund_non_captured_payment() {
    println!("🧪 Test 3: Cannot Refund Non-Captured Payment");

    let payment_id = PaymentId::new();
    let amount = Money::from_dollars(100);

    ReducerTest::new(PaymentReducer::new())
        .with_env(create_test_env())
        .given_state({
            let mut state = PaymentState::new();
            // Payment is Pending, not Captured
            let payment = Payment::new(
                payment_id,
                ReservationId::new(),
                CustomerId::new(),
                amount,
                PaymentMethod::CreditCard {
                    last_four: "4242".to_string(),
                },
            );
            state.payments.insert(payment_id, payment);
            state
        })
        .when_action(PaymentAction::RefundPayment {
            payment_id,
            amount,
            reason: "Test refund".to_string(),
        })
        .then_state(move |state| {
            let payment = state.get(&payment_id).unwrap();
            // Should still be Pending
            assert_eq!(payment.status, PaymentStatus::Pending);
            // Should have validation error
            assert!(state.last_error.is_some());
            assert!(state
                .last_error
                .as_ref()
                .unwrap()
                .contains("Cannot refund"));
        })
        .then_effects(assertions::assert_no_effects)
        .run();

    println!("  ✅ Refund rejected for non-captured payment");
}

/// Test 4: Cannot Refund Non-existent Payment
///
/// Verifies that attempting to refund a payment that doesn't exist is rejected.
#[tokio::test]
async fn test_cannot_refund_nonexistent_payment() {
    println!("🧪 Test 4: Cannot Refund Non-existent Payment");

    let payment_id = PaymentId::new();
    let amount = Money::from_dollars(100);

    ReducerTest::new(PaymentReducer::new())
        .with_env(create_test_env())
        .given_state(PaymentState::new()) // No payment in state
        .when_action(PaymentAction::RefundPayment {
            payment_id,
            amount,
            reason: "Test refund".to_string(),
        })
        .then_state(move |_state| {
            // State should remain empty (no payment)
            // Note: The reducer validates payment exists, so this will trigger ValidationFailed
        })
        .then_effects(assertions::assert_no_effects)
        .run();

    println!("  ✅ Refund rejected for non-existent payment");
}

/// Test 5: Cannot Refund Already Refunded Payment
///
/// Verifies idempotency - cannot refund the same payment twice.
#[tokio::test]
async fn test_cannot_refund_twice() {
    println!("🧪 Test 5: Cannot Refund Already Refunded Payment");

    let payment_id = PaymentId::new();
    let amount = Money::from_dollars(100);

    ReducerTest::new(PaymentReducer::new())
        .with_env(create_test_env())
        .given_state({
            let mut state = PaymentState::new();
            let mut payment = Payment::new(
                payment_id,
                ReservationId::new(),
                CustomerId::new(),
                amount,
                PaymentMethod::CreditCard {
                    last_four: "4242".to_string(),
                },
            );
            // Payment is already refunded
            payment.status = PaymentStatus::Refunded { amount };
            state.payments.insert(payment_id, payment);
            state
        })
        .when_action(PaymentAction::RefundPayment {
            payment_id,
            amount,
            reason: "Second refund attempt".to_string(),
        })
        .then_state(move |state| {
            let payment = state.get(&payment_id).unwrap();
            // Should still be Refunded (no state change)
            assert!(matches!(
                payment.status,
                PaymentStatus::Refunded { .. }
            ));
            // Should have validation error
            assert!(state.last_error.is_some());
        })
        .then_effects(assertions::assert_no_effects)
        .run();

    println!("  ✅ Second refund rejected (idempotency)");
}

/// Test 6: Payment Failure Handling
///
/// Verifies that failed payments are handled correctly using SimulatePaymentFailure.
#[tokio::test]
async fn test_payment_failure_handling() {
    println!("🧪 Test 6: Payment Failure Handling");

    let payment_id = PaymentId::new();
    let reservation_id = ReservationId::new();

    ReducerTest::new(PaymentReducer::new())
        .with_env(create_test_env())
        .given_state(PaymentState::new())
        .when_action(PaymentAction::SimulatePaymentFailure {
            payment_id,
            reservation_id,
            reason: "Insufficient funds".to_string(),
        })
        .then_state(move |state| {
            // State should have error recorded
            assert!(state.last_error.is_some());
            assert!(state
                .last_error
                .as_ref()
                .unwrap()
                .contains("Insufficient funds"));
        })
        .then_effects(|effects| {
            // Should return 2 effects: AppendEvents + PublishEvent for PaymentFailed
            assert_eq!(effects.len(), 2);
        })
        .run();

    println!("  ✅ Payment failure handled correctly");
}

/// Test 7: Credit Card Payment Method
///
/// Verifies that credit card payment method works successfully.
#[tokio::test]
async fn test_credit_card_payment_method() {
    println!("🧪 Test 7: Credit Card Payment Method");

    let payment_id = PaymentId::new();
    let reservation_id = ReservationId::new();
    let payment_method = PaymentMethod::CreditCard {
        last_four: "4242".to_string(),
    };

    ReducerTest::new(PaymentReducer::new())
        .with_env(create_test_env())
        .given_state(PaymentState::new())
        .when_action(PaymentAction::ProcessPayment {
            payment_id,
            reservation_id,
            amount: Money::from_dollars(50),
            payment_method: payment_method.clone(),
        })
        .then_state(move |state| {
            let payment = state.get(&payment_id).unwrap();
            assert_eq!(payment.status, PaymentStatus::Captured);
            assert_eq!(payment.payment_method, payment_method);
        })
        .then_effects(|effects| {
            assert_eq!(effects.len(), 4);
        })
        .run();

    println!("  ✅ Credit card payment processed successfully");
}

/// Test 8: Process and Then Refund Full Flow
///
/// Verifies the complete lifecycle: process payment → capture → refund.
#[tokio::test]
async fn test_full_payment_lifecycle() {
    println!("🧪 Test 8: Full Payment Lifecycle (Process → Refund)");

    let payment_id = PaymentId::new();
    let reservation_id = ReservationId::new();
    let amount = Money::from_dollars(75);

    let reducer = PaymentReducer::new();
    let env = create_test_env();

    // Step 1: Process payment
    let mut state = PaymentState::new();
    let _effects = reducer.reduce(
        &mut state,
        PaymentAction::ProcessPayment {
            payment_id,
            reservation_id,
            amount,
            payment_method: PaymentMethod::CreditCard {
                last_four: "5555".to_string(),
            },
        },
        &env,
    );

    // Verify payment is captured
    assert_eq!(state.count(), 1);
    let payment = state.get(&payment_id).unwrap();
    assert_eq!(payment.status, PaymentStatus::Captured);

    // Step 2: Refund payment
    let _refund_effects = reducer.reduce(
        &mut state,
        PaymentAction::RefundPayment {
            payment_id,
            amount,
            reason: "Event cancelled".to_string(),
        },
        &env,
    );

    // Verify payment is refunded
    let payment = state.get(&payment_id).unwrap();
    assert!(matches!(
        payment.status,
        PaymentStatus::Refunded { amount: refund_amount } if refund_amount == amount
    ));

    println!("  ✅ Full payment lifecycle completed successfully");
}
