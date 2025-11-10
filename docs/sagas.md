# Saga Pattern in Composable Rust

## Table of Contents
- [Overview](#overview)
- [What is a Saga?](#what-is-a-saga)
- [Sagas as Reducers](#sagas-as-reducers)
- [State Machine Approach](#state-machine-approach)
- [Compensation Pattern](#compensation-pattern)
- [Timeout Handling](#timeout-handling)
- [Best Practices](#best-practices)
- [Complete Example](#complete-example)

## Overview

Composable Rust implements sagas using the same `Reducer` trait as regular aggregates. **Sagas don't require special framework support** - they're just reducers with state machines that coordinate multiple aggregates through events.

This approach provides:
- **Type safety**: Compiler-verified state transitions
- **Testability**: Pure functions, easy to test
- **Composability**: Sagas work like any other reducer
- **Transparency**: All coordination logic visible in one place

## What is a Saga?

A saga is a **long-running distributed transaction** broken into multiple steps, where each step:
1. Performs a local transaction in an aggregate
2. Publishes an event on success
3. Has a **compensating action** to undo the step if needed

### Saga vs. Aggregate

| Aspect | Aggregate | Saga |
|--------|-----------|------|
| **Purpose** | Enforce business invariants for a single entity | Coordinate multiple aggregates |
| **State** | Domain entity state | Workflow coordination state |
| **Actions** | Commands and events for one entity | Cross-aggregate events and commands |
| **Consistency** | Strong consistency within aggregate | Eventual consistency across aggregates |

## Sagas as Reducers

In Composable Rust, sagas are just reducers:

```rust
pub struct CheckoutSaga;

impl Reducer for CheckoutSaga {
    type State = CheckoutSagaState;
    type Action = CheckoutAction;
    type Environment = CheckoutSagaEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match (state, action) {
            // State machine transitions...
        }
    }
}
```

**Key insight**: The saga state machine lives in the `reduce()` function's pattern matching.

## State Machine Approach

### Saga State

Saga state tracks workflow progress:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckoutSagaState {
    /// Current step in the workflow
    pub status: SagaStatus,

    /// IDs for compensation (undo operations)
    pub order_id: Option<OrderId>,
    pub payment_id: Option<PaymentId>,
    pub reservation_id: Option<ReservationId>,

    /// Tracking for debugging
    pub completed_steps: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SagaStatus {
    Idle,
    PlacingOrder,
    ProcessingPayment,
    ReservingInventory,
    Completed,
    Compensating,
    Failed,
}
```

### State Transitions

State transitions are explicit in the pattern matching:

```rust
fn reduce(&self, state: &mut State, action: Action, env: &Env) -> SmallVec<[Effect<Action>; 4]> {
    match (&state.status, action) {
        // Happy path: Idle → PlacingOrder
        (SagaStatus::Idle, CheckoutAction::InitiateCheckout { cart, customer }) => {
            state.status = SagaStatus::PlacingOrder;
            smallvec![Effect::DispatchCommand(PlaceOrder { cart, customer })]
        }

        // Happy path: PlacingOrder → ProcessingPayment
        (SagaStatus::PlacingOrder, CheckoutAction::OrderPlaced { order_id }) => {
            state.order_id = Some(order_id);
            state.status = SagaStatus::ProcessingPayment;
            state.completed_steps.push("OrderPlaced".to_string());
            smallvec![Effect::DispatchCommand(ProcessPayment { order_id })]
        }

        // Error path: payment failure triggers compensation
        (SagaStatus::ProcessingPayment, CheckoutAction::PaymentFailed { .. }) => {
            state.status = SagaStatus::Compensating;
            smallvec![Effect::DispatchCommand(CancelOrder { order_id: state.order_id })]
        }

        // More transitions...
    }
}
```

### Benefits of Explicit States

1. **Compiler verification**: Can't transition to invalid states
2. **Clear flow**: All transitions visible in one place
3. **Easy debugging**: State tells you exactly where the saga is
4. **Idempotency**: Can handle duplicate events safely

## Compensation Pattern

When a step fails, sagas must **compensate** (undo) completed steps.

### Compensation Strategies

#### 1. Semantic Compensation

Undo business operations (preferred):

```rust
// Forward operation
Effect::DispatchCommand(ProcessPayment { amount })

// Compensation
Effect::DispatchCommand(RefundPayment { payment_id })
```

#### 2. Reversal Compensation

Reverse the operation exactly:

```rust
// Forward operation
Effect::DispatchCommand(ReserveInventory { items })

// Compensation
Effect::DispatchCommand(ReleaseInventory { reservation_id })
```

### Compensation Flow

```rust
match (&state.status, action) {
    // Payment fails → compensate by cancelling order
    (SagaStatus::ProcessingPayment, CheckoutAction::PaymentFailed { error }) => {
        state.status = SagaStatus::Compensating;
        smallvec![
            Effect::DispatchCommand(CancelOrder { order_id: state.order_id }),
        ]
    }

    // Inventory fails → compensate by refunding payment AND cancelling order
    (SagaStatus::ReservingInventory, CheckoutAction::InsufficientInventory { .. }) => {
        state.status = SagaStatus::Compensating;
        smallvec![
            Effect::DispatchCommand(RefundPayment { payment_id: state.payment_id }),
            Effect::DispatchCommand(CancelOrder { order_id: state.order_id }),
        ]
    }

    // All compensations complete → mark saga as failed
    (SagaStatus::Compensating, CheckoutAction::OrderCancelled { .. }) => {
        state.status = SagaStatus::Failed;
        state.completed_steps.push("Compensated".to_string());
        smallvec![Effect::None]
    }
}
```

### Compensation Best Practices

1. **Idempotent compensations**: Can be applied multiple times safely
2. **Compensation order**: Undo steps in reverse order when possible
3. **Track progress**: Store IDs needed for compensation in saga state
4. **Graceful degradation**: Some compensations may fail (manual intervention needed)

## Timeout Handling

Sagas must handle steps that never complete.

### Using Delay Effect

```rust
// Start operation with timeout
smallvec![
    Effect::DispatchCommand(ReserveInventory { items }),
    delay! {
        duration: Duration::from_secs(30),
        action: CheckoutAction::InventoryTimeout
    },
]

// Handle timeout
(SagaStatus::ReservingInventory, CheckoutAction::InventoryTimeout) => {
    // Treat timeout as failure → compensate
    state.status = SagaStatus::Compensating;
    smallvec![
        Effect::DispatchCommand(RefundPayment { payment_id: state.payment_id }),
        Effect::DispatchCommand(CancelOrder { order_id: state.order_id }),
    ]
}

// On success, the timeout effect is cancelled automatically
(SagaStatus::ReservingInventory, CheckoutAction::InventoryReserved { .. }) => {
    state.status = SagaStatus::Completed;
    smallvec![Effect::None]
}
```

### Timeout Best Practices

1. **Reasonable timeouts**: Consider normal operation times + buffer
2. **Idempotent handling**: Success after timeout should be idempotent
3. **Monitoring**: Track timeout rates for operational visibility
4. **Graceful degradation**: Timeouts trigger compensation, not crashes

## Best Practices

### 1. Design for Idempotency

Events may be delivered multiple times (at-least-once semantics):

```rust
// Use IDs to detect duplicates
(SagaStatus::Completed, CheckoutAction::InventoryReserved { .. }) => {
    // Already completed, ignore duplicate event
    smallvec![Effect::None]
}
```

### 2. Store Compensation Data

Keep IDs and data needed for compensation:

```rust
pub struct CheckoutSagaState {
    pub order_id: Option<OrderId>,      // For CancelOrder
    pub payment_id: Option<PaymentId>,  // For RefundPayment
    pub reservation_id: Option<ReservationId>, // For ReleaseInventory
}
```

### 3. Explicit State Transitions

Make all transitions explicit:

```rust
// Good: Clear transition
(SagaStatus::ProcessingPayment, CheckoutAction::PaymentCompleted { .. }) => {
    state.status = SagaStatus::ReservingInventory;
    // ...
}

// Bad: Implicit state
(_, CheckoutAction::PaymentCompleted { .. }) => {
    // Which state are we coming from?
}
```

### 4. Test All Paths

Test happy path, failures, and compensations:

```rust
#[tokio::test]
async fn test_checkout_payment_failure() {
    // Arrange: Create saga, initiate checkout
    // Act: Payment fails
    // Assert: Order cancelled, saga in Failed state
}

#[tokio::test]
async fn test_checkout_inventory_failure() {
    // Arrange: Order placed, payment completed
    // Act: Inventory fails
    // Assert: Payment refunded, order cancelled
}
```

### 5. Monitor Saga Progress

Track saga states and completion rates:

```rust
// Add tracing
tracing::info!(
    saga_id = %state.saga_id,
    status = ?state.status,
    completed_steps = ?state.completed_steps,
    "Saga state transition"
);
```

### 6. Handle Partial Failures

Not all compensations may succeed:

```rust
(SagaStatus::Compensating, CheckoutAction::RefundFailed { error }) => {
    // Log for manual intervention
    tracing::error!(
        saga_id = %state.saga_id,
        error = %error,
        "Compensation failed - manual intervention required"
    );

    // Mark as failed with note
    state.status = SagaStatus::Failed;
    state.completed_steps.push(format!("RefundFailed: {error}"));
    smallvec![Effect::None]
}
```

## Complete Example

See `examples/checkout-saga/` for a complete working example featuring:

- **3 aggregates**: Order, Payment, Inventory
- **1 saga**: `CheckoutSaga` coordinating the three
- **Happy path**: Order → Payment → Inventory → Success
- **Failure paths**:
  - Payment fails → Cancel order
  - Inventory fails → Refund payment + Cancel order
- **Event bus integration**: Events flow through `InMemoryEventBus`
- **Comprehensive tests**: Happy path + all failure scenarios

### Running the Example

```bash
# Run all saga tests
cargo test -p checkout-saga

# Run specific failure test
cargo test -p checkout-saga test_checkout_payment_failure
```

## Summary

Sagas in Composable Rust:

1. **Are just reducers** with state machines
2. **Coordinate multiple aggregates** via events
3. **Handle failures** with compensation logic
4. **Support timeouts** via `Effect::Delay`
5. **Don't need special framework support**

This approach provides type-safe, testable, composable workflow coordination without framework magic.

## Next Steps

- Read [Event Bus Guide](event-bus.md) for event routing patterns
- Read [Redpanda Setup](redpanda-setup.md) for production deployment
- Study `examples/checkout-saga/` for complete implementation
- Review Phase 3 TODO for advanced patterns
