# Saga Patterns in Composable Rust

**Practical Guide to Building Reliable Distributed Transactions**

> 📖 **Companion Document**: This guide provides detailed saga implementation patterns. For consistency fundamentals, see [Consistency Patterns](./consistency-patterns.md).

---

## Table of Contents

1. [Overview](#overview)
2. [Saga Basics](#saga-basics)
3. [State Management Patterns](#state-management-patterns)
4. [Compensation Patterns](#compensation-patterns)
5. [Error Handling](#error-handling)
6. [Testing Patterns](#testing-patterns)
7. [Real-World Examples](#real-world-examples)
8. [Anti-Patterns](#anti-patterns)
9. [Best Practices](#best-practices)

---

## Overview

A **saga** is a pattern for managing distributed transactions across multiple aggregates. Sagas coordinate long-running workflows that involve multiple services or aggregates, with built-in compensation for failures.

### Key Characteristics

- **Long-running**: May take seconds, minutes, or hours
- **Eventually consistent**: Not ACID transactions
- **Compensatable**: Can undo completed steps
- **Event-driven**: React to events, publish new events
- **Stateful**: Track progress through the workflow

### When to Use Sagas

✅ **Use sagas for**:
- Multi-aggregate coordination (order + payment + inventory)
- Long-running business processes (order fulfillment, days)
- Workflows with external services (payment gateway, shipping)
- Processes requiring compensation (refunds, rollbacks)

❌ **Don't use sagas for**:
- Single aggregate operations (use reducers)
- Simple sequential operations (use Effect::Sequential)
- ACID transactions (use database transactions)

---

## Saga Basics

### The Saga Lifecycle

```
1. Initiate     → Saga receives trigger event
2. Execute      → Each step executes sequentially or in parallel
3. Decide       → Each step outcome determines next action
4. Complete     → All steps succeed → saga ends
   OR
   Compensate   → Any step fails → undo completed steps
```

### Saga as a Reducer

In Composable Rust, sagas are just reducers with state machines:

```rust
use composable_rust_core::{Effect, Reducer};
use serde::{Deserialize, Serialize};

/// Saga state tracks progress through workflow
#[derive(Clone, Debug)]
pub struct CheckoutSagaState {
    // Order data (carried from initial event)
    order_id: OrderId,
    customer_id: CustomerId,
    items: Vec<LineItem>,
    order_total: Money,

    // Progress tracking
    step: CheckoutStep,
    payment_confirmed: bool,
    inventory_reserved: bool,
    shipping_scheduled: bool,

    // Compensation tracking
    completed_steps: Vec<CompletedStep>,
}

#[derive(Clone, Debug, PartialEq)]
enum CheckoutStep {
    Started,
    ChargingPayment,
    ReservingInventory,
    SchedulingShipment,
    Completed,
    Failed,
    Compensating,
}

/// Saga actions (events from external services)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CheckoutSagaAction {
    // Trigger
    OrderPlaced {
        order_id: OrderId,
        customer_id: CustomerId,
        items: Vec<LineItem>,
        total: Money,
        timestamp: DateTime<Utc>,
    },

    // Success events
    PaymentCharged { order_id: OrderId },
    InventoryReserved { order_id: OrderId },
    ShipmentScheduled { order_id: OrderId, tracking: String },

    // Failure events
    PaymentFailed { order_id: OrderId, reason: String },
    InventoryUnavailable { order_id: OrderId, reason: String },
    ShipmentFailed { order_id: OrderId, reason: String },

    // Compensation events
    PaymentRefunded { order_id: OrderId },
    InventoryReleased { order_id: OrderId },
}

/// Saga reducer implements state machine
pub struct CheckoutSagaReducer;

impl Reducer for CheckoutSagaReducer {
    type State = CheckoutSagaState;
    type Action = CheckoutSagaAction;
    type Environment = CheckoutSagaEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match (state.step.clone(), action) {
            // Step 1: Order placed → charge payment
            (CheckoutStep::Started, CheckoutSagaAction::OrderPlaced { order_id, customer_id, items, total, .. }) => {
                state.order_id = order_id.clone();
                state.customer_id = customer_id;
                state.items = items.clone();
                state.order_total = total;
                state.step = CheckoutStep::ChargingPayment;

                smallvec![Effect::Database(/* charge payment */)]
            }

            // Step 2: Payment charged → reserve inventory
            (CheckoutStep::ChargingPayment, CheckoutSagaAction::PaymentCharged { .. }) => {
                state.payment_confirmed = true;
                state.completed_steps.push(CompletedStep::PaymentCharged);
                state.step = CheckoutStep::ReservingInventory;

                smallvec![Effect::Database(/* reserve inventory */)]
            }

            // Step 3: Inventory reserved → schedule shipment
            (CheckoutStep::ReservingInventory, CheckoutSagaAction::InventoryReserved { .. }) => {
                state.inventory_reserved = true;
                state.completed_steps.push(CompletedStep::InventoryReserved);
                state.step = CheckoutStep::SchedulingShipment;

                smallvec![Effect::Database(/* schedule shipment */)]
            }

            // Step 4: Shipment scheduled → complete
            (CheckoutStep::SchedulingShipment, CheckoutSagaAction::ShipmentScheduled { tracking, .. }) => {
                state.shipping_scheduled = true;
                state.step = CheckoutStep::Completed;

                smallvec![Effect::PublishEvent(OrderAction::OrderShipped {
                    order_id: state.order_id.clone(),
                    tracking,
                    timestamp: env.clock.now(),
                })]
            }

            // Compensation: Payment failed → cancel order
            (CheckoutStep::ChargingPayment, CheckoutSagaAction::PaymentFailed { reason, .. }) => {
                state.step = CheckoutStep::Failed;

                smallvec![Effect::PublishEvent(OrderAction::OrderCancelled {
                    order_id: state.order_id.clone(),
                    reason,
                    timestamp: env.clock.now(),
                })]
            }

            // Compensation: Inventory failed → refund payment
            (CheckoutStep::ReservingInventory, CheckoutSagaAction::InventoryUnavailable { .. }) => {
                state.step = CheckoutStep::Compensating;

                // Refund payment (undo completed step)
                smallvec![Effect::Database(/* refund payment */)]
            }

            _ => smallvec![Effect::None],
        }
    }
}
```

**Key Points**:
- Saga is a reducer with state machine
- State tracks progress and completed steps
- Pattern matching on `(current_step, event)` determines transitions
- Compensation is just another path through the state machine

---

## State Management Patterns

### Pattern 1: Carry All Data Upfront

**Best for**: Workflows where all data is known at start

```rust
#[derive(Clone, Debug)]
pub struct TransferSagaState {
    // All data from initial command
    transfer_id: TransferId,
    from_account_id: AccountId,
    to_account_id: AccountId,
    amount: Money,
    initiated_at: DateTime<Utc>,

    // Progress
    from_account_debited: bool,
    to_account_credited: bool,

    // Compensation
    debit_transaction_id: Option<TransactionId>,
}

impl Reducer for TransferSagaReducer {
    fn reduce(&self, state: &mut State, action: Action, env: &Env) -> SmallVec<[Effect<Action>; 4]> {
        match action {
            TransferAction::TransferInitiated { transfer_id, from_account_id, to_account_id, amount, .. } => {
                // ✅ Carry all data in state
                state.transfer_id = transfer_id;
                state.from_account_id = from_account_id.clone();
                state.to_account_id = to_account_id.clone();
                state.amount = amount;

                // Debit from account
                smallvec![Effect::PublishEvent(AccountAction::Debit {
                    account_id: from_account_id,
                    amount,
                    reference: format!("transfer-{}", transfer_id),
                })]
            }

            TransferAction::AccountDebited { transaction_id, .. } => {
                state.from_account_debited = true;
                state.debit_transaction_id = Some(transaction_id);

                // Credit to account (using state data)
                smallvec![Effect::PublishEvent(AccountAction::Credit {
                    account_id: state.to_account_id.clone(),  // From state!
                    amount: state.amount,                      // From state!
                    reference: format!("transfer-{}", state.transfer_id),
                })]
            }

            // ... rest of saga
        }
    }
}
```

**Benefits**:
- No queries needed
- All data available at each step
- Fast and deterministic

**When to Use**:
- Order checkout (items, addresses known upfront)
- Money transfer (accounts, amount known)
- Batch processing (all items known)

### Pattern 2: Query Event Store When Needed

**Best for**: Workflows that need current state

```rust
impl Reducer for RefundSagaReducer {
    fn reduce(&self, state: &mut State, action: Action, env: &Env) -> SmallVec<[Effect<Action>; 4]> {
        match action {
            RefundAction::RefundInitiated { order_id, .. } => {
                state.order_id = order_id.clone();

                // Need current order state to process refund
                smallvec![Effect::Future(Box::pin(async move {
                    // ✅ Query event store for current state
                    let stream_id = format!("order-{}", order_id);
                    let events = env.event_store.load_events(&stream_id).await?;
                    let order = Order::from_events(events);

                    // Calculate refund based on current state
                    let refund_amount = order.calculate_refund();

                    Some(RefundAction::RefundAmountCalculated {
                        order_id,
                        amount: refund_amount,
                    })
                }))]
            }

            RefundAction::RefundAmountCalculated { amount, .. } => {
                state.refund_amount = amount;

                // Process refund
                smallvec![Effect::Database(/* process refund */)]
            }

            // ... rest of saga
        }
    }
}
```

**When to Use**:
- Need current aggregate state
- State changes during saga
- Can't carry all data upfront

**Trade-offs**:
- Slower (event store query)
- Still strongly consistent
- Good for infrequent checks

### Pattern 3: Incremental State Building

**Best for**: Workflows that accumulate data

```rust
#[derive(Clone, Debug)]
pub struct OrderFulfillmentState {
    order_id: OrderId,

    // Accumulated data from each step
    payment_details: Option<PaymentDetails>,
    inventory_allocation: Option<InventoryAllocation>,
    shipping_label: Option<ShippingLabel>,
    warehouse_assignment: Option<WarehouseId>,
}

impl Reducer for OrderFulfillmentReducer {
    fn reduce(&self, state: &mut State, action: Action, env: &Env) -> SmallVec<[Effect<Action>; 4]> {
        match action {
            FulfillmentAction::PaymentConfirmed { payment_details, .. } => {
                // ✅ Accumulate data from each step
                state.payment_details = Some(payment_details);

                // Next step uses accumulated data
                smallvec![Effect::Database(/* allocate inventory */)]
            }

            FulfillmentAction::InventoryAllocated { allocation, warehouse_id, .. } => {
                state.inventory_allocation = Some(allocation);
                state.warehouse_assignment = Some(warehouse_id);

                // Generate shipping label using accumulated data
                smallvec![Effect::Future(Box::pin(async move {
                    let label = env.shipping_service.generate_label(
                        state.warehouse_assignment.unwrap(),
                        state.inventory_allocation.as_ref().unwrap(),
                    ).await?;

                    Some(FulfillmentAction::ShippingLabelGenerated { label })
                }))]
            }

            // ... continue building state
        }
    }
}
```

**When to Use**:
- Each step produces data needed by later steps
- Progressive workflow (build up information)
- Can't know all data upfront

---

## Compensation Patterns

### Pattern 1: Sequential Compensation (Reverse Order)

Compensate in reverse order of execution:

```rust
#[derive(Clone, Debug)]
pub struct CheckoutSagaState {
    completed_steps: Vec<CompletedStep>,
}

#[derive(Clone, Debug)]
enum CompletedStep {
    PaymentCharged { transaction_id: String },
    InventoryReserved { reservation_id: String },
    ShipmentScheduled { shipment_id: String },
}

impl Reducer for CheckoutSagaReducer {
    fn reduce(&self, state: &mut State, action: Action, env: &Env) -> SmallVec<[Effect<Action>; 4]> {
        match action {
            CheckoutSagaAction::InventoryUnavailable { .. } => {
                // ✅ Compensate in reverse order
                let compensations = state.completed_steps.iter().rev().map(|step| {
                    match step {
                        CompletedStep::PaymentCharged { transaction_id } => {
                            Effect::Database(/* refund payment */)
                        }
                        CompletedStep::InventoryReserved { reservation_id } => {
                            Effect::Database(/* release inventory */)
                        }
                        CompletedStep::ShipmentScheduled { shipment_id } => {
                            Effect::Database(/* cancel shipment */)
                        }
                    }
                }).collect();

                compensations
            }

            // ... other handlers
        }
    }
}
```

**Benefits**:
- Natural unwinding (like stack unwinding)
- Correct dependency order
- Matches developer intuition

### Pattern 2: Parallel Compensation

Compensate independent steps in parallel:

```rust
impl Reducer for CheckoutSagaReducer {
    fn reduce(&self, state: &mut State, action: Action, env: &Env) -> SmallVec<[Effect<Action>; 4]> {
        match action {
            CheckoutSagaAction::ShipmentFailed { .. } => {
                // ✅ Compensate independent steps in parallel
                smallvec![Effect::Parallel(vec![
                    // These can happen concurrently
                    Effect::Database(/* refund payment */),
                    Effect::Database(/* release inventory */),
                    Effect::Database(/* notify customer */),
                ])]
            }

            // ... other handlers
        }
    }
}
```

**When to Use**:
- Compensation steps are independent
- Want faster compensation
- Steps don't depend on each other

### Pattern 3: Idempotent Compensation

Make compensation safe to retry:

```rust
async fn compensate_payment(transaction_id: &str) -> Result<()> {
    // ✅ Idempotent: Check if already refunded
    if payment_service.is_refunded(transaction_id).await? {
        return Ok(()); // Already compensated
    }

    // Process refund
    payment_service.refund(transaction_id).await?;
    Ok(())
}

async fn compensate_inventory(reservation_id: &str) -> Result<()> {
    // ✅ Idempotent: Release only if still reserved
    if inventory_service.is_reserved(reservation_id).await? {
        inventory_service.release(reservation_id).await?;
    }
    Ok(())
}
```

**Why Important**:
- Compensation may be retried (network failures)
- Avoid double refunds
- Prevent inventory errors

---

## Error Handling

### Pattern 1: Retry with Exponential Backoff

```rust
use std::time::Duration;

impl Reducer for CheckoutSagaReducer {
    fn reduce(&self, state: &mut State, action: Action, env: &Env) -> SmallVec<[Effect<Action>; 4]> {
        match action {
            CheckoutSagaAction::PaymentServiceUnavailable { attempt, .. } => {
                if attempt < 3 {
                    // ✅ Retry with exponential backoff
                    let delay = Duration::from_millis(100 * 2_u64.pow(attempt));

                    smallvec![Effect::Delay(
                        delay,
                        Box::new(CheckoutSagaAction::RetryPayment {
                            order_id: state.order_id.clone(),
                            attempt: attempt + 1,
                        }),
                    )]
                } else {
                    // Max retries exceeded → compensate
                    smallvec![Effect::PublishEvent(CheckoutSagaAction::PaymentFailed {
                        order_id: state.order_id.clone(),
                        reason: "Payment service unavailable after 3 attempts".to_string(),
                    })]
                }
            }

            // ... other handlers
        }
    }
}
```

**When to Use**:
- Transient failures (network errors, service unavailable)
- External services (payment gateway, shipping API)

**Backoff Schedule**:
- Attempt 1: 100ms
- Attempt 2: 200ms
- Attempt 3: 400ms

### Pattern 2: Timeout Handling

```rust
impl Reducer for CheckoutSagaReducer {
    fn reduce(&self, state: &mut State, action: Action, env: &Env) -> SmallVec<[Effect<Action>; 4]> {
        match action {
            CheckoutSagaAction::PaymentCharged { .. } => {
                state.payment_confirmed = true;

                // ✅ Start timeout for next step
                vec![
                    Effect::Database(/* reserve inventory */),
                    Effect::Delay(
                        Duration::from_secs(30),
                        Box::new(CheckoutSagaAction::InventoryTimeout {
                            order_id: state.order_id.clone(),
                        }),
                    ),
                ]
            }

            CheckoutSagaAction::InventoryReserved { .. } => {
                // Success - timeout no longer needed
                state.inventory_reserved = true;
                vec![/* continue */]
            }

            CheckoutSagaAction::InventoryTimeout { .. } => {
                // ✅ Timeout fired → compensate
                if !state.inventory_reserved {
                    smallvec![Effect::PublishEvent(CheckoutSagaAction::InventoryUnavailable {
                        order_id: state.order_id.clone(),
                        reason: "Timeout waiting for inventory".to_string(),
                    })]
                } else {
                    smallvec![Effect::None] // Already succeeded, ignore timeout
                }
            }

            // ... other handlers
        }
    }
}
```

### Pattern 3: Dead Letter Queue (DLQ)

```rust
impl Reducer for CheckoutSagaReducer {
    fn reduce(&self, state: &mut State, action: Action, env: &Env) -> SmallVec<[Effect<Action>; 4]> {
        match action {
            CheckoutSagaAction::UnrecoverableError { error, .. } => {
                // ✅ Send to DLQ for manual intervention
                vec![
                    Effect::Future(Box::pin(async move {
                        env.dlq.send(DLQMessage {
                            saga_id: state.order_id.clone(),
                            saga_type: "CheckoutSaga",
                            error: error.clone(),
                            state: serde_json::to_value(&state)?,
                            timestamp: env.clock.now(),
                        }).await?;

                        Some(CheckoutSagaAction::SentToDLQ { order_id: state.order_id.clone() })
                    })),
                    // Notify ops team
                    Effect::PublishEvent(AlertAction::SagaFailure {
                        saga_id: state.order_id.clone(),
                        error,
                    }),
                ]
            }

            // ... other handlers
        }
    }
}
```

---

## Testing Patterns

### Pattern 1: Test Happy Path

```rust
#[tokio::test]
async fn test_checkout_saga_happy_path() {
    // Arrange
    let env = test_environment();
    let mut state = CheckoutSagaState::default();
    let reducer = CheckoutSagaReducer;

    // Act & Assert: Step 1 - Order placed
    let effects = reducer.reduce(
        &mut state,
        CheckoutSagaAction::OrderPlaced {
            order_id: OrderId::new("order-1"),
            customer_id: CustomerId::new("cust-1"),
            items: vec![test_line_item()],
            total: Money::from_dollars(100),
            timestamp: Utc::now(),
        },
        &env,
    );

    assert_eq!(state.step, CheckoutStep::ChargingPayment);
    assert!(matches!(effects[0], Effect::Database(_)));

    // Act & Assert: Step 2 - Payment charged
    let effects = reducer.reduce(
        &mut state,
        CheckoutSagaAction::PaymentCharged {
            order_id: OrderId::new("order-1"),
        },
        &env,
    );

    assert_eq!(state.step, CheckoutStep::ReservingInventory);
    assert!(state.payment_confirmed);

    // Continue testing each step...
}
```

### Pattern 2: Test Compensation Flow

```rust
#[tokio::test]
async fn test_checkout_saga_payment_failure_compensation() {
    // Arrange
    let env = test_environment();
    let mut state = CheckoutSagaState::default();
    let reducer = CheckoutSagaReducer;

    // Setup: Get to payment step
    reducer.reduce(
        &mut state,
        CheckoutSagaAction::OrderPlaced { /* ... */ },
        &env,
    );

    // Act: Payment fails
    let effects = reducer.reduce(
        &mut state,
        CheckoutSagaAction::PaymentFailed {
            order_id: OrderId::new("order-1"),
            reason: "Card declined".to_string(),
        },
        &env,
    );

    // Assert: Saga compensates
    assert_eq!(state.step, CheckoutStep::Failed);
    assert!(matches!(
        effects[0],
        Effect::PublishEvent(OrderAction::OrderCancelled { .. })
    ));
}
```

### Pattern 3: Test Timeout Scenarios

```rust
#[tokio::test]
async fn test_checkout_saga_inventory_timeout() {
    // Arrange
    let env = test_environment();
    let mut state = CheckoutSagaState {
        step: CheckoutStep::ReservingInventory,
        payment_confirmed: true,
        inventory_reserved: false,
        ..Default::default()
    };
    let reducer = CheckoutSagaReducer;

    // Act: Timeout fires
    let effects = reducer.reduce(
        &mut state,
        CheckoutSagaAction::InventoryTimeout {
            order_id: OrderId::new("order-1"),
        },
        &env,
    );

    // Assert: Saga compensates
    assert!(matches!(
        effects[0],
        Effect::PublishEvent(CheckoutSagaAction::InventoryUnavailable { .. })
    ));
}
```

---

## Real-World Examples

### Example 1: E-Commerce Checkout Saga

Complete implementation in `examples/checkout-saga/`:

```rust
// See examples/checkout-saga/src/lib.rs for full implementation

pub struct CheckoutSaga {
    // Dependencies
    payment_service: Arc<dyn PaymentService>,
    inventory_service: Arc<dyn InventoryService>,
    shipping_service: Arc<dyn ShippingService>,
}

// Workflow:
// 1. Order placed → charge payment
// 2. Payment confirmed → reserve inventory
// 3. Inventory reserved → schedule shipment
// 4. Shipment scheduled → complete order
//
// Compensation (if any step fails):
// - Refund payment
// - Release inventory
// - Cancel shipment
// - Mark order as cancelled
```

### Example 2: Money Transfer Saga

```rust
// Transfer money between accounts with compensation

pub struct TransferSaga {
    from_account_id: AccountId,
    to_account_id: AccountId,
    amount: Money,
    debit_transaction_id: Option<TransactionId>,
    credit_transaction_id: Option<TransactionId>,
}

// Workflow:
// 1. Debit from_account
// 2. Credit to_account
// 3. Mark transfer complete
//
// Compensation (if credit fails):
// - Reverse debit (credit from_account)
```

---

## Anti-Patterns

### ❌ Anti-Pattern 1: Querying Projections

**Don't do this**:

```rust
// ❌ WRONG: Query projection in saga
async fn handle_order_placed(&mut self, event: OrderPlacedEvent) -> Result<()> {
    let order = self.projection.get_order(&event.order_id).await?;  // Race!
    self.process_order(order).await
}
```

**Do this instead**:

```rust
// ✅ CORRECT: Carry data in event
async fn handle_order_placed(&mut self, event: OrderPlacedEvent) -> Result<()> {
    self.state.order_total = event.total;  // From event!
    self.state.items = event.items;         // From event!
    self.process_order().await
}
```

### ❌ Anti-Pattern 2: Global State

**Don't do this**:

```rust
// ❌ WRONG: Saga depends on global state
static CURRENT_ORDERS: Mutex<HashMap<OrderId, Order>> = Mutex::new(HashMap::new());

impl Saga {
    async fn handle_event(&mut self, event: Event) {
        let orders = CURRENT_ORDERS.lock().unwrap();
        let order = orders.get(&self.order_id).unwrap();  // Global state!
    }
}
```

**Do this instead**:

```rust
// ✅ CORRECT: Saga carries its own state
pub struct SagaState {
    order: Order,  // Saga owns its data
}
```

### ❌ Anti-Pattern 3: Synchronous Waiting

**Don't do this**:

```rust
// ❌ WRONG: Synchronous wait in saga
async fn handle_payment_sent(&mut self) -> Result<()> {
    loop {
        let status = self.payment_service.check_status().await?;
        if status.is_complete() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;  // Polling!
    }
}
```

**Do this instead**:

```rust
// ✅ CORRECT: Event-driven (wait for event)
fn reduce(&self, state: &mut State, action: Action) -> SmallVec<[Effect<Action>; 4]> {
    match action {
        SagaAction::PaymentSent { .. } => {
            // Just wait for PaymentConfirmed event
            smallvec![Effect::None]
        }
        SagaAction::PaymentConfirmed { .. } => {
            // Event arrived → continue
            vec![/* next step */]
        }
    }
}
```

---

## Best Practices

### ✅ Do This

1. **Carry data in saga state** - Don't query projections
2. **Make compensation idempotent** - Safe to retry
3. **Track completed steps** - Know what to compensate
4. **Use timeouts** - Don't wait forever
5. **Test compensation paths** - Not just happy path
6. **Log saga progress** - Debugging and monitoring
7. **Use fat events** - Include all data downstream needs
8. **Handle retries** - Transient failures are common

### ❌ Don't Do This

1. ❌ Query projections for decision-making
2. ❌ Use global state
3. ❌ Synchronous polling (use events)
4. ❌ Forget compensation
5. ❌ Ignore timeouts
6. ❌ Test only happy path
7. ❌ Use thin events
8. ❌ Assume operations always succeed

---

## Summary

**Saga Pattern Checklist**:

- [ ] Saga is a reducer with state machine
- [ ] State carries all needed data (or queries event store)
- [ ] Never queries projections
- [ ] Tracks completed steps for compensation
- [ ] Compensation is idempotent
- [ ] Has timeout handling
- [ ] Has retry logic for transient failures
- [ ] Uses fat events with complete data
- [ ] Tests both happy path and compensation
- [ ] Logs progress for debugging

---

## Further Reading

- [Consistency Patterns](./consistency-patterns.md) - Fundamental consistency concepts
- [Event Design Guidelines](./event-design-guidelines.md) - How to design events for sagas
- [Checkout Saga Example](../examples/checkout-saga/) - Complete working example
- [Saga Pattern](https://microservices.io/patterns/data/saga.html) - Original saga pattern
- [Process Manager vs Saga](https://www.enterpriseintegrationpatterns.com/patterns/messaging/ProcessManager.html) - Pattern comparison

---

**Last Updated**: 2025-01-07
**Status**: ✅ Production Ready
