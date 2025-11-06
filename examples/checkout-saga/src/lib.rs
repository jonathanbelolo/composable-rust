//! Checkout Saga Example
//!
//! Demonstrates multi-aggregate coordination with compensation using the saga pattern.
//!
//! # Architecture
//!
//! This example shows how to coordinate three aggregates (Order, Payment, Inventory)
//! in a checkout workflow with automatic compensation on failures.
//!
//! ```text
//! ┌──────────────┐
//! │  Customer    │
//! └──────┬───────┘
//!        │ InitiateCheckout
//!        ▼
//! ┌──────────────┐
//! │ CheckoutSaga │◄──── Coordinator
//! └──────┬───────┘
//!        │
//!        ├─► PlaceOrder ─────► Order Aggregate
//!        │                         │
//!        │◄── OrderPlaced ─────────┘
//!        │
//!        ├─► ProcessPayment ─► Payment Aggregate
//!        │                         │
//!        │◄── PaymentCompleted ────┘
//!        │       or PaymentFailed
//!        │
//!        ├─► ReserveInventory ► Inventory Aggregate
//!        │                         │
//!        │◄── InventoryReserved ───┘
//!        │       or InsufficientInventory
//!        │
//!        └─► CheckoutCompleted
//!
//! Compensation Flow (if Payment fails):
//! Payment Failed ─► CancelOrder ─► Order Aggregate
//!
//! Compensation Flow (if Inventory fails):
//! Insufficient Inventory ─► RefundPayment ─► Payment Aggregate
//!                        └─► CancelOrder ───► Order Aggregate
//! ```
//!
//! # Key Concepts Demonstrated
//!
//! - **Saga Pattern**: Multi-step workflows with compensation
//! - **Event-Driven Coordination**: Aggregates communicate via events
//! - **State Machine**: Saga tracks progress through states
//! - **Idempotency**: Handling duplicate events safely
//! - **Compensation**: Automatic rollback on failures
//!
//! # Usage
//!
//! ```
//! use checkout_saga::{CheckoutSaga, CheckoutSagaEnvironment, CheckoutSagaState, CheckoutAction};
//! use composable_rust_runtime::Store;
//! use composable_rust_testing::{test_clock, mocks::InMemoryEventBus};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create saga environment
//! let env = CheckoutSagaEnvironment {
//!     clock: test_clock(),
//!     event_bus: Arc::new(InMemoryEventBus::new()),
//! };
//!
//! // Create store
//! let store = Store::new(CheckoutSagaState::default(), CheckoutSaga::default(), env);
//!
//! // Initiate checkout
//! let _ = store.send(CheckoutAction::InitiateCheckout {
//!     customer_id: "customer-123".to_string(),
//!     order_total_cents: 10000, // $100.00
//!     items: vec!["item-1".to_string(), "item-2".to_string()],
//! }).await;
//!
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use composable_rust_core::effect::Effect;
use composable_rust_core::environment::Clock;
use composable_rust_core::event_bus::EventBus;
use composable_rust_core::reducer::Reducer;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

//
// ===== Payment Aggregate =====
//

/// Payment state machine
#[derive(Clone, Debug, Default, PartialEq)]
pub enum PaymentState {
    /// No payment initiated
    #[default]
    Idle,
    /// Payment processing
    Processing {
        /// Payment ID
        payment_id: String,
        /// Amount in cents
        amount_cents: u64,
    },
    /// Payment completed successfully
    Completed {
        /// Payment ID
        payment_id: String,
        /// Amount in cents
        amount_cents: u64,
    },
    /// Payment failed
    Failed {
        /// Payment ID
        payment_id: String,
        /// Failure reason
        reason: String,
    },
    /// Payment refunded
    Refunded {
        /// Payment ID
        payment_id: String,
        /// Amount in cents
        amount_cents: u64,
    },
}

/// Payment actions (commands and events)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PaymentAction {
    /// Command: Process a payment
    ProcessPayment {
        /// Payment ID
        payment_id: String,
        /// Amount in cents
        amount_cents: u64,
    },
    /// Event: Payment completed successfully
    PaymentCompleted {
        /// Payment ID
        payment_id: String,
    },
    /// Event: Payment failed
    PaymentFailed {
        /// Payment ID
        payment_id: String,
        /// Failure reason
        reason: String,
    },
    /// Command: Refund a payment
    RefundPayment {
        /// Payment ID
        payment_id: String,
    },
    /// Event: Payment refunded
    PaymentRefunded {
        /// Payment ID
        payment_id: String,
    },
}

/// Payment reducer
pub struct PaymentReducer;

impl Reducer for PaymentReducer {
    type State = PaymentState;
    type Action = PaymentAction;
    type Environment = ();

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> Vec<Effect<Self::Action>> {
        match (state.clone(), action) {
            // Process payment command
            (
                PaymentState::Idle,
                PaymentAction::ProcessPayment {
                    payment_id,
                    amount_cents,
                },
            ) => {
                *state = PaymentState::Processing {
                    payment_id: payment_id.clone(),
                    amount_cents,
                };

                // Simulate payment processing (always succeeds in this simplified example)
                vec![Effect::Future(Box::pin(async move {
                    // In real implementation: call payment gateway API
                    Some(PaymentAction::PaymentCompleted { payment_id })
                }))]
            },

            // Payment completed event
            (
                PaymentState::Processing {
                    payment_id,
                    amount_cents,
                },
                PaymentAction::PaymentCompleted {
                    payment_id: completed_id,
                },
            ) if payment_id == completed_id => {
                *state = PaymentState::Completed {
                    payment_id,
                    amount_cents,
                };
                vec![Effect::None]
            },

            // Payment failed event
            (
                PaymentState::Processing { payment_id, .. },
                PaymentAction::PaymentFailed {
                    payment_id: failed_id,
                    reason,
                },
            ) if payment_id == failed_id => {
                *state = PaymentState::Failed { payment_id, reason };
                vec![Effect::None]
            },

            // Refund payment command
            (
                PaymentState::Completed {
                    payment_id,
                    amount_cents,
                },
                PaymentAction::RefundPayment {
                    payment_id: refund_id,
                },
            ) if payment_id == refund_id => {
                *state = PaymentState::Refunded {
                    payment_id: payment_id.clone(),
                    amount_cents,
                };

                vec![Effect::Future(Box::pin(async move {
                    // In real implementation: call payment gateway refund API
                    Some(PaymentAction::PaymentRefunded { payment_id })
                }))]
            },

            // Payment refunded event
            (
                PaymentState::Refunded {
                    payment_id,
                    amount_cents,
                },
                PaymentAction::PaymentRefunded {
                    payment_id: refunded_id,
                },
            ) if payment_id == refunded_id => {
                // Already refunded, idempotent
                *state = PaymentState::Refunded {
                    payment_id,
                    amount_cents,
                };
                vec![Effect::None]
            },

            // Invalid transitions
            _ => vec![Effect::None],
        }
    }
}

//
// ===== Inventory Aggregate =====
//

/// Inventory state
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InventoryState {
    /// Items with their available quantities
    pub items: std::collections::HashMap<String, u32>,
    /// Reserved items (`reservation_id` -> items)
    pub reservations: std::collections::HashMap<String, Vec<String>>,
}

/// Inventory actions (commands and events)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum InventoryAction {
    /// Command: Add inventory
    AddInventory {
        /// Item ID
        item_id: String,
        /// Quantity to add
        quantity: u32,
    },
    /// Command: Reserve inventory
    ReserveInventory {
        /// Reservation ID
        reservation_id: String,
        /// Items to reserve
        items: Vec<String>,
    },
    /// Event: Inventory reserved successfully
    InventoryReserved {
        /// Reservation ID
        reservation_id: String,
    },
    /// Event: Insufficient inventory
    InsufficientInventory {
        /// Reservation ID
        reservation_id: String,
        /// Missing items
        missing_items: Vec<String>,
    },
    /// Command: Release reservation
    ReleaseInventory {
        /// Reservation ID
        reservation_id: String,
    },
    /// Event: Inventory released
    InventoryReleased {
        /// Reservation ID
        reservation_id: String,
    },
}

/// Inventory reducer
pub struct InventoryReducer;

impl Reducer for InventoryReducer {
    type State = InventoryState;
    type Action = InventoryAction;
    type Environment = ();

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> Vec<Effect<Self::Action>> {
        match action {
            // Add inventory command
            InventoryAction::AddInventory { item_id, quantity } => {
                *state.items.entry(item_id).or_insert(0) += quantity;
                vec![Effect::None]
            },

            // Reserve inventory command
            InventoryAction::ReserveInventory {
                reservation_id,
                items,
            } => {
                // Check if all items are available
                let mut missing_items = Vec::new();
                for item in &items {
                    if state.items.get(item).copied().unwrap_or(0) == 0 {
                        missing_items.push(item.clone());
                    }
                }

                if !missing_items.is_empty() {
                    // Insufficient inventory
                    return vec![Effect::Future(Box::pin(async move {
                        Some(InventoryAction::InsufficientInventory {
                            reservation_id,
                            missing_items,
                        })
                    }))];
                }

                // Reserve items
                for item in &items {
                    if let Some(quantity) = state.items.get_mut(item) {
                        *quantity = quantity.saturating_sub(1);
                    }
                }
                state.reservations.insert(reservation_id.clone(), items);

                vec![Effect::Future(Box::pin(async move {
                    Some(InventoryAction::InventoryReserved { reservation_id })
                }))]
            },

            // Event handlers (idempotent - no side effects)
            InventoryAction::InventoryReserved { .. }
            | InventoryAction::InsufficientInventory { .. }
            | InventoryAction::InventoryReleased { .. } => vec![Effect::None],

            // Release inventory command
            InventoryAction::ReleaseInventory { reservation_id } => {
                if let Some(items) = state.reservations.remove(&reservation_id) {
                    // Return items to inventory
                    for item in items {
                        *state.items.entry(item).or_insert(0) += 1;
                    }
                }

                vec![Effect::Future(Box::pin(async move {
                    Some(InventoryAction::InventoryReleased { reservation_id })
                }))]
            },
        }
    }
}

//
// ===== Checkout Saga =====
//

/// Checkout saga state machine
#[derive(Clone, Debug, Default, PartialEq)]
pub enum CheckoutSagaState {
    /// No checkout in progress
    #[default]
    Idle,
    /// Order being placed
    PlacingOrder {
        /// Customer ID
        customer_id: String,
        /// Order total in cents
        order_total_cents: u64,
        /// Items to purchase
        items: Vec<String>,
    },
    /// Payment being processed
    ProcessingPayment {
        /// Customer ID
        customer_id: String,
        /// Order ID
        order_id: String,
        /// Payment ID
        payment_id: String,
        /// Items to reserve
        items: Vec<String>,
    },
    /// Inventory being reserved
    ReservingInventory {
        /// Customer ID
        customer_id: String,
        /// Order ID
        order_id: String,
        /// Payment ID
        payment_id: String,
        /// Reservation ID
        reservation_id: String,
    },
    /// Checkout completed successfully
    Completed {
        /// Order ID
        order_id: String,
        /// Payment ID
        payment_id: String,
        /// Reservation ID
        reservation_id: String,
    },
    /// Checkout failed, compensation in progress
    Compensating {
        /// Order ID (if created)
        order_id: Option<String>,
        /// Payment ID (if created)
        payment_id: Option<String>,
        /// Failure reason
        reason: String,
    },
    /// Checkout failed and compensated
    Failed {
        /// Failure reason
        reason: String,
    },
}

/// Checkout saga actions
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CheckoutAction {
    /// Command: Initiate checkout
    InitiateCheckout {
        /// Customer ID
        customer_id: String,
        /// Order total in cents
        order_total_cents: u64,
        /// Items to purchase
        items: Vec<String>,
    },

    // Order events (from Order aggregate)
    /// Event: Order placed
    OrderPlaced {
        /// Order ID
        order_id: String,
    },
    /// Event: Order cancelled
    OrderCancelled {
        /// Order ID
        order_id: String,
    },

    // Payment events (from Payment aggregate)
    /// Event: Payment completed
    PaymentCompleted {
        /// Payment ID
        payment_id: String,
    },
    /// Event: Payment failed
    PaymentFailed {
        /// Payment ID
        payment_id: String,
        /// Failure reason
        reason: String,
    },
    /// Event: Payment refunded
    PaymentRefunded {
        /// Payment ID
        payment_id: String,
    },

    // Inventory events (from Inventory aggregate)
    /// Event: Inventory reserved
    InventoryReserved {
        /// Reservation ID
        reservation_id: String,
    },
    /// Event: Insufficient inventory
    InsufficientInventory {
        /// Reservation ID
        reservation_id: String,
    },
    /// Event: Inventory released
    InventoryReleased {
        /// Reservation ID
        reservation_id: String,
    },

    /// Event: Checkout completed
    CheckoutCompleted {
        /// Order ID
        order_id: String,
    },
    /// Event: Checkout failed
    CheckoutFailed {
        /// Failure reason
        reason: String,
    },
}

/// Checkout saga environment
#[derive(Clone)]
pub struct CheckoutSagaEnvironment<C, E>
where
    C: Clock,
    E: EventBus,
{
    /// Clock for timestamps
    pub clock: C,
    /// Event bus for cross-aggregate communication
    pub event_bus: Arc<E>,
}

/// Checkout saga reducer
#[derive(Clone)]
pub struct CheckoutSaga<C, E>
where
    C: Clock,
    E: EventBus,
{
    _phantom: std::marker::PhantomData<(C, E)>,
}

impl<C, E> Default for CheckoutSaga<C, E>
where
    C: Clock,
    E: EventBus,
{
    fn default() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<C, E> Reducer for CheckoutSaga<C, E>
where
    C: Clock,
    E: EventBus,
{
    type State = CheckoutSagaState;
    type Action = CheckoutAction;
    type Environment = CheckoutSagaEnvironment<C, E>;

    #[allow(clippy::too_many_lines)] // Saga complexity requires detailed state transitions
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> Vec<Effect<Self::Action>> {
        match (state.clone(), action) {
            // Initiate checkout
            (
                CheckoutSagaState::Idle,
                CheckoutAction::InitiateCheckout {
                    customer_id,
                    order_total_cents,
                    items,
                },
            ) => {
                *state = CheckoutSagaState::PlacingOrder {
                    customer_id: customer_id.clone(),
                    order_total_cents,
                    items: items.clone(),
                };

                // TODO: Dispatch PlaceOrder command to Order aggregate
                // For now, simulate immediate OrderPlaced event
                let order_id = format!("order-{}", uuid::Uuid::new_v4());
                vec![Effect::Future(Box::pin(async move {
                    Some(CheckoutAction::OrderPlaced { order_id })
                }))]
            },

            // Order placed successfully
            (
                CheckoutSagaState::PlacingOrder {
                    customer_id,
                    order_total_cents: _,
                    items,
                },
                CheckoutAction::OrderPlaced { order_id },
            ) => {
                let payment_id = format!("payment-{}", uuid::Uuid::new_v4());
                *state = CheckoutSagaState::ProcessingPayment {
                    customer_id,
                    order_id: order_id.clone(),
                    payment_id: payment_id.clone(),
                    items,
                };

                // TODO: Dispatch ProcessPayment command to Payment aggregate
                // For now, simulate immediate PaymentCompleted event
                vec![Effect::Future(Box::pin(async move {
                    Some(CheckoutAction::PaymentCompleted { payment_id })
                }))]
            },

            // Payment completed successfully
            (
                CheckoutSagaState::ProcessingPayment {
                    customer_id,
                    order_id,
                    payment_id,
                    items: _,
                },
                CheckoutAction::PaymentCompleted {
                    payment_id: completed_payment_id,
                },
            ) if payment_id == completed_payment_id => {
                let reservation_id = format!("reservation-{}", uuid::Uuid::new_v4());
                *state = CheckoutSagaState::ReservingInventory {
                    customer_id,
                    order_id: order_id.clone(),
                    payment_id: payment_id.clone(),
                    reservation_id: reservation_id.clone(),
                };

                // TODO: Dispatch ReserveInventory command to Inventory aggregate
                // For now, simulate immediate InventoryReserved event
                vec![Effect::Future(Box::pin(async move {
                    Some(CheckoutAction::InventoryReserved { reservation_id })
                }))]
            },

            // Payment failed - start compensation
            (
                CheckoutSagaState::ProcessingPayment {
                    order_id,
                    payment_id,
                    ..
                },
                CheckoutAction::PaymentFailed { reason, .. },
            ) => {
                *state = CheckoutSagaState::Compensating {
                    order_id: Some(order_id.clone()),
                    payment_id: Some(payment_id),
                    reason: reason.clone(),
                };

                // TODO: Dispatch CancelOrder command to Order aggregate
                vec![Effect::Future(Box::pin(async move {
                    Some(CheckoutAction::OrderCancelled { order_id })
                }))]
            },

            // Inventory reserved successfully - checkout complete!
            (
                CheckoutSagaState::ReservingInventory {
                    order_id,
                    payment_id,
                    reservation_id,
                    ..
                },
                CheckoutAction::InventoryReserved {
                    reservation_id: reserved_id,
                },
            ) if reservation_id == reserved_id => {
                *state = CheckoutSagaState::Completed {
                    order_id: order_id.clone(),
                    payment_id,
                    reservation_id,
                };

                vec![Effect::Future(Box::pin(async move {
                    Some(CheckoutAction::CheckoutCompleted { order_id })
                }))]
            },

            // Insufficient inventory - start compensation
            (
                CheckoutSagaState::ReservingInventory {
                    order_id,
                    payment_id,
                    ..
                },
                CheckoutAction::InsufficientInventory { .. },
            ) => {
                *state = CheckoutSagaState::Compensating {
                    order_id: Some(order_id.clone()),
                    payment_id: Some(payment_id.clone()),
                    reason: "Insufficient inventory".to_string(),
                };

                // TODO: Dispatch RefundPayment and CancelOrder commands
                vec![Effect::Future(Box::pin(async move {
                    Some(CheckoutAction::PaymentRefunded { payment_id })
                }))]
            },

            // Payment refunded - continue compensation
            (
                CheckoutSagaState::Compensating {
                    order_id, reason, ..
                },
                CheckoutAction::PaymentRefunded { .. },
            ) => {
                if let Some(order_id) = order_id {
                    let order_id_clone = order_id.clone();
                    *state = CheckoutSagaState::Compensating {
                        order_id: Some(order_id),
                        payment_id: None,
                        reason: reason.clone(),
                    };

                    vec![Effect::Future(Box::pin(async move {
                        Some(CheckoutAction::OrderCancelled {
                            order_id: order_id_clone,
                        })
                    }))]
                } else {
                    *state = CheckoutSagaState::Failed { reason };
                    vec![Effect::Future(Box::pin(async {
                        Some(CheckoutAction::CheckoutFailed {
                            reason: "Compensation completed".to_string(),
                        })
                    }))]
                }
            },

            // Order cancelled - compensation complete
            (
                CheckoutSagaState::Compensating { reason, .. },
                CheckoutAction::OrderCancelled { .. },
            ) => {
                *state = CheckoutSagaState::Failed {
                    reason: reason.clone(),
                };

                vec![Effect::Future(Box::pin(async move {
                    Some(CheckoutAction::CheckoutFailed { reason })
                }))]
            },

            // Terminal states and invalid transitions
            _ => vec![Effect::None],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_runtime::Store;
    use composable_rust_testing::{mocks::InMemoryEventBus, test_clock};

    #[tokio::test]
    async fn test_checkout_happy_path() {
        let env = CheckoutSagaEnvironment {
            clock: test_clock(),
            event_bus: Arc::new(InMemoryEventBus::new()),
        };

        let store = Store::new(CheckoutSagaState::default(), CheckoutSaga::default(), env);

        // Initiate checkout
        let _ = store
            .send(CheckoutAction::InitiateCheckout {
                customer_id: "customer-123".to_string(),
                order_total_cents: 10000,
                items: vec!["item-1".to_string(), "item-2".to_string()],
            })
            .await;

        // Simulate OrderPlaced event
        let _ = store
            .send(CheckoutAction::OrderPlaced {
                order_id: "order-123".to_string(),
            })
            .await;

        // Get payment_id from current state
        let payment_id = store
            .state(|s| {
                if let CheckoutSagaState::ProcessingPayment { payment_id, .. } = s {
                    payment_id.clone()
                } else {
                    String::new()
                }
            })
            .await;

        // Simulate PaymentCompleted event
        let _ = store
            .send(CheckoutAction::PaymentCompleted { payment_id })
            .await;

        // Get reservation_id from current state
        let reservation_id = store
            .state(|s| {
                if let CheckoutSagaState::ReservingInventory { reservation_id, .. } = s {
                    reservation_id.clone()
                } else {
                    String::new()
                }
            })
            .await;

        // Simulate InventoryReserved event
        let _ = store
            .send(CheckoutAction::InventoryReserved { reservation_id })
            .await;

        // Verify final state is Completed
        let is_completed = store
            .state(|s| matches!(s, CheckoutSagaState::Completed { .. }))
            .await;
        assert!(is_completed);
    }

    #[tokio::test]
    async fn test_payment_reducer() {
        let mut state = PaymentState::Idle;
        let reducer = PaymentReducer;

        // Process payment
        let effects = reducer.reduce(
            &mut state,
            PaymentAction::ProcessPayment {
                payment_id: "payment-123".to_string(),
                amount_cents: 10000,
            },
            &(),
        );

        assert!(matches!(state, PaymentState::Processing { .. }));
        assert_eq!(effects.len(), 1);
    }

    #[tokio::test]
    async fn test_inventory_reducer() {
        let mut state = InventoryState::default();
        let reducer = InventoryReducer;

        // Add inventory
        reducer.reduce(
            &mut state,
            InventoryAction::AddInventory {
                item_id: "item-1".to_string(),
                quantity: 10,
            },
            &(),
        );

        assert_eq!(state.items.get("item-1"), Some(&10));

        // Reserve inventory
        let effects = reducer.reduce(
            &mut state,
            InventoryAction::ReserveInventory {
                reservation_id: "res-1".to_string(),
                items: vec!["item-1".to_string()],
            },
            &(),
        );

        assert_eq!(state.items.get("item-1"), Some(&9));
        assert_eq!(effects.len(), 1);
    }

    #[tokio::test]
    async fn test_checkout_payment_failure() {
        let env = CheckoutSagaEnvironment {
            clock: test_clock(),
            event_bus: Arc::new(InMemoryEventBus::new()),
        };

        let store = Store::new(CheckoutSagaState::default(), CheckoutSaga::default(), env);

        // Initiate checkout
        let _ = store
            .send(CheckoutAction::InitiateCheckout {
                customer_id: "customer-123".to_string(),
                order_total_cents: 10000,
                items: vec!["item-1".to_string()],
            })
            .await;

        // Simulate OrderPlaced event
        let _ = store
            .send(CheckoutAction::OrderPlaced {
                order_id: "order-123".to_string(),
            })
            .await;

        // Get payment_id from current state
        let payment_id = store
            .state(|s| {
                if let CheckoutSagaState::ProcessingPayment { payment_id, .. } = s {
                    payment_id.clone()
                } else {
                    String::new()
                }
            })
            .await;

        // Simulate PaymentFailed event (triggers compensation)
        let _ = store
            .send(CheckoutAction::PaymentFailed {
                payment_id,
                reason: "Insufficient funds".to_string(),
            })
            .await;

        // Verify we're in compensating state
        let is_compensating = store
            .state(|s| matches!(s, CheckoutSagaState::Compensating { .. }))
            .await;
        assert!(is_compensating);

        // Simulate OrderCancelled event (compensation complete)
        let order_id = store
            .state(|s| {
                if let CheckoutSagaState::Compensating { order_id, .. } = s {
                    order_id.clone()
                } else {
                    None
                }
            })
            .await;

        if let Some(order_id) = order_id {
            let _ = store
                .send(CheckoutAction::OrderCancelled { order_id })
                .await;
        }

        // Verify final state is Failed
        let is_failed = store
            .state(|s| matches!(s, CheckoutSagaState::Failed { .. }))
            .await;
        assert!(is_failed);
    }

    #[tokio::test]
    async fn test_checkout_inventory_failure() {
        let env = CheckoutSagaEnvironment {
            clock: test_clock(),
            event_bus: Arc::new(InMemoryEventBus::new()),
        };

        let store = Store::new(CheckoutSagaState::default(), CheckoutSaga::default(), env);

        // Initiate checkout
        let _ = store
            .send(CheckoutAction::InitiateCheckout {
                customer_id: "customer-123".to_string(),
                order_total_cents: 10000,
                items: vec!["item-1".to_string(), "item-2".to_string()],
            })
            .await;

        // Simulate OrderPlaced
        let _ = store
            .send(CheckoutAction::OrderPlaced {
                order_id: "order-123".to_string(),
            })
            .await;

        // Simulate PaymentCompleted
        let payment_id = store
            .state(|s| {
                if let CheckoutSagaState::ProcessingPayment { payment_id, .. } = s {
                    payment_id.clone()
                } else {
                    String::new()
                }
            })
            .await;

        let _ = store
            .send(CheckoutAction::PaymentCompleted {
                payment_id: payment_id.clone(),
            })
            .await;

        // Simulate InsufficientInventory (triggers compensation)
        let reservation_id = store
            .state(|s| {
                if let CheckoutSagaState::ReservingInventory { reservation_id, .. } = s {
                    reservation_id.clone()
                } else {
                    String::new()
                }
            })
            .await;

        let _ = store
            .send(CheckoutAction::InsufficientInventory { reservation_id })
            .await;

        // Verify we're in compensating state
        let is_compensating = store
            .state(|s| matches!(s, CheckoutSagaState::Compensating { .. }))
            .await;
        assert!(is_compensating);

        // Simulate PaymentRefunded (compensation step 1)
        let _ = store
            .send(CheckoutAction::PaymentRefunded {
                payment_id: payment_id.clone(),
            })
            .await;

        // Simulate OrderCancelled (compensation step 2)
        let order_id = store
            .state(|s| {
                if let CheckoutSagaState::Compensating { order_id, .. } = s {
                    order_id.clone()
                } else {
                    None
                }
            })
            .await;

        if let Some(order_id) = order_id {
            let _ = store
                .send(CheckoutAction::OrderCancelled { order_id })
                .await;
        }

        // Verify final state is Failed
        let final_state = store.state(std::clone::Clone::clone).await;
        assert!(matches!(final_state, CheckoutSagaState::Failed { .. }));
    }

    #[tokio::test]
    async fn test_payment_refund_flow() {
        let mut state = PaymentState::Completed {
            payment_id: "payment-123".to_string(),
            amount_cents: 10000,
        };
        let reducer = PaymentReducer;

        // Refund payment (command)
        let effects = reducer.reduce(
            &mut state,
            PaymentAction::RefundPayment {
                payment_id: "payment-123".to_string(),
            },
            &(),
        );

        // Should transition to Refunded state immediately and issue effect
        assert!(matches!(state, PaymentState::Refunded { .. }));
        assert_eq!(effects.len(), 1);

        // Simulate refund completion event (idempotent)
        let effects = reducer.reduce(
            &mut state,
            PaymentAction::PaymentRefunded {
                payment_id: "payment-123".to_string(),
            },
            &(),
        );

        // Still Refunded, event is idempotent
        assert!(matches!(state, PaymentState::Refunded { .. }));
        assert_eq!(effects.len(), 1);
    }

    #[tokio::test]
    async fn test_inventory_insufficient() {
        let mut state = InventoryState::default();
        let reducer = InventoryReducer;

        // Add only 1 item
        reducer.reduce(
            &mut state,
            InventoryAction::AddInventory {
                item_id: "item-1".to_string(),
                quantity: 1,
            },
            &(),
        );

        // Try to reserve 2 items (should fail)
        let effects = reducer.reduce(
            &mut state,
            InventoryAction::ReserveInventory {
                reservation_id: "res-1".to_string(),
                items: vec!["item-1".to_string(), "item-2".to_string()],
            },
            &(),
        );

        // Verify insufficient inventory was detected
        assert_eq!(effects.len(), 1);
        // State should remain unchanged (no reservation made)
        assert_eq!(state.items.get("item-1"), Some(&1));
        assert!(state.reservations.is_empty());
    }

    #[tokio::test]
    async fn test_inventory_release() {
        let mut state = InventoryState::default();
        let reducer = InventoryReducer;

        // Add inventory and reserve
        reducer.reduce(
            &mut state,
            InventoryAction::AddInventory {
                item_id: "item-1".to_string(),
                quantity: 10,
            },
            &(),
        );

        reducer.reduce(
            &mut state,
            InventoryAction::ReserveInventory {
                reservation_id: "res-1".to_string(),
                items: vec!["item-1".to_string()],
            },
            &(),
        );

        assert_eq!(state.items.get("item-1"), Some(&9));

        // Release the reservation
        let effects = reducer.reduce(
            &mut state,
            InventoryAction::ReleaseInventory {
                reservation_id: "res-1".to_string(),
            },
            &(),
        );

        // Inventory should be restored
        assert_eq!(state.items.get("item-1"), Some(&10));
        assert!(state.reservations.is_empty());
        assert_eq!(effects.len(), 1);
    }
}
