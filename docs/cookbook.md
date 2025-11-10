# Pattern Cookbook

**Common patterns and recipes for Composable Rust applications.**

## Overview

This cookbook provides ready-to-use patterns for common scenarios when building applications with Composable Rust. Each pattern includes working code examples and explains when to use it.

## Table of Contents

1. [Request-Response Pattern](#request-response-pattern)
2. [Event-Driven Workflows](#event-driven-workflows)
3. [Saga Compensation](#saga-compensation)
4. [Projection Updates](#projection-updates)
5. [Real-Time Notifications](#real-time-notifications)
6. [Batch Operations](#batch-operations)
7. [Idempotency](#idempotency)
8. [Error Handling](#error-handling)
9. [Testing Patterns](#testing-patterns)
10. [Performance Optimization](#performance-optimization)

---

## Request-Response Pattern

**When**: HTTP APIs, synchronous workflows, user-facing operations

**Pattern**: Use `send_and_wait_for()` to dispatch a command and wait for a specific result event.

```rust
use composable_rust_runtime::Store;
use std::time::Duration;

async fn place_order_handler(
    State(store): State<Arc<Store<OrderState, OrderAction, OrderEnv, OrderReducer>>>,
    Json(request): Json<PlaceOrderRequest>,
) -> Result<Json<PlaceOrderResponse>, AppError> {
    // 1. Build command action
    let action = OrderAction::PlaceOrder {
        customer_id: request.customer_id,
        items: request.items,
    };

    // 2. Wait for result event
    let result = store
        .send_and_wait_for(
            action,
            |a| matches!(
                a,
                OrderAction::OrderPlaced { .. } | OrderAction::OrderFailed { .. }
            ),
            Duration::from_secs(5),
        )
        .await
        .map_err(|_| AppError::timeout("Order placement timeout"))?;

    // 3. Map to HTTP response
    match result {
        OrderAction::OrderPlaced { order_id, .. } => {
            Ok(Json(PlaceOrderResponse { order_id }))
        }
        OrderAction::OrderFailed { reason, .. } => {
            Err(AppError::bad_request(reason))
        }
        _ => Err(AppError::internal("Unexpected action")),
    }
}
```

**Key Points**:
- Command and event are both actions
- Timeout prevents indefinite waiting
- Pattern maps domain events to HTTP responses

---

## Event-Driven Workflows

**When**: Background processing, asynchronous operations, multi-step workflows

**Pattern**: Chain effects together, each step produces the next action.

```rust
impl Reducer for OrderReducer {
    fn reduce(
        &self,
        state: &mut OrderState,
        action: OrderAction,
        env: &OrderEnv,
    ) -> SmallVec<[Effect<OrderAction>; 4]> {
        match action {
            // Step 1: Order placed
            OrderAction::PlaceOrder { customer_id, items } => {
                let order_id = generate_id();
                state.orders.insert(order_id.clone(), Order::new(customer_id, items));

                vec![
                    Effect::AppendEvents {
                        stream_id: StreamId::new(&order_id),
                        events: vec![serialize(&OrderPlaced { order_id: order_id.clone() })],
                        expected_version: None,
                    },
                    Effect::Future(Box::pin({
                        let order_id = order_id.clone();
                        async move {
                            // Triggers next step
                            Some(OrderAction::CheckInventory { order_id })
                        }
                    })),
                ]
            }

            // Step 2: Check inventory
            OrderAction::CheckInventory { order_id } => {
                smallvec![Effect::Future(Box::pin({
                    let inventory = env.inventory.clone();
                    let order_id = order_id.clone();
                    async move {
                        match inventory.check_availability(&order_id).await {
                            Ok(true) => Some(OrderAction::InventoryAvailable { order_id }),
                            Ok(false) => Some(OrderAction::InventoryUnavailable { order_id }),
                            Err(e) => Some(OrderAction::InventoryCheckFailed { order_id, error: e.to_string() }),
                        }
                    }
                }))]
            }

            // Step 3: Process based on inventory
            OrderAction::InventoryAvailable { order_id } => {
                smallvec![Effect::Future(Box::pin({
                    let order_id = order_id.clone();
                    async move {
                        Some(OrderAction::ChargePayment { order_id })
                    }
                }))]
            }

            // Handle failures
            OrderAction::InventoryUnavailable { order_id } => {
                state.orders.get_mut(&order_id).unwrap().status = OrderStatus::Cancelled;
                smallvec![Effect::None]
            }

            _ => vec![],
        }
    }
}
```

**Key Points**:
- Each step produces an effect that triggers the next action
- Failures are handled as domain events
- State is updated incrementally at each step

---

## Saga Compensation

**When**: Multi-aggregate coordination, distributed transactions, rollback scenarios

**Pattern**: Track completed steps, compensate in reverse order on failure.

```rust
#[derive(Clone, Debug)]
struct SagaState {
    current_step: SagaStep,
    completed_steps: Vec<SagaStep>,
    order_id: Option<String>,
    payment_id: Option<String>,
    inventory_reservation_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum SagaStep {
    CreateOrder,
    ReserveInventory,
    ChargePayment,
    Completed,
}

impl Reducer for CheckoutSagaReducer {
    fn reduce(
        &self,
        state: &mut SagaState,
        action: SagaAction,
        env: &SagaEnv,
    ) -> SmallVec<[Effect<SagaAction>; 4]> {
        match action {
            // Forward flow
            SagaAction::OrderCreated { order_id } => {
                state.order_id = Some(order_id.clone());
                state.completed_steps.push(SagaStep::CreateOrder);
                state.current_step = SagaStep::ReserveInventory;

                smallvec![Effect::Future(Box::pin(async move {
                    Some(SagaAction::ReserveInventory { order_id })
                }))]
            }

            SagaAction::InventoryReserved { reservation_id } => {
                state.inventory_reservation_id = Some(reservation_id);
                state.completed_steps.push(SagaStep::ReserveInventory);
                state.current_step = SagaStep::ChargePayment;

                smallvec![Effect::Future(Box::pin({
                    let order_id = state.order_id.clone().unwrap();
                    async move {
                        Some(SagaAction::ChargePayment { order_id })
                    }
                }))]
            }

            // Failure triggers compensation
            SagaAction::PaymentFailed { reason } => {
                self.compensate(state)
            }

            _ => vec![],
        }
    }
}

impl CheckoutSagaReducer {
    fn compensate(&self, state: &SagaState) -> SmallVec<[Effect<SagaAction>; 4]> {
        let mut effects = vec![];

        // Compensate in reverse order
        for step in state.completed_steps.iter().rev() {
            match step {
                SagaStep::ReserveInventory => {
                    if let Some(reservation_id) = &state.inventory_reservation_id {
                        effects.push(Effect::Future(Box::pin({
                            let reservation_id = reservation_id.clone();
                            async move {
                                Some(SagaAction::ReleaseInventory { reservation_id })
                            }
                        })));
                    }
                }
                SagaStep::CreateOrder => {
                    if let Some(order_id) = &state.order_id {
                        effects.push(Effect::Future(Box::pin({
                            let order_id = order_id.clone();
                            async move {
                                Some(SagaAction::CancelOrder { order_id })
                            }
                        })));
                    }
                }
                _ => {}
            }
        }

        effects
    }
}
```

**Key Points**:
- Track completed steps for compensation
- Compensate in reverse order
- Each step stores IDs needed for compensation
- See [`docs/saga-patterns.md`](./saga-patterns.md) for more examples

---

## Projection Updates

**When**: Read models, denormalized views, query optimization

**Pattern**: Update projections from events, handle idempotency with timestamps.

```rust
impl Projection for CustomerOrderProjection {
    type Event = OrderEvent;
    type Error = ProjectionError;

    async fn handle_event(
        &self,
        event: &Self::Event,
        metadata: &EventMetadata,
    ) -> Result<(), Self::Error> {
        match event {
            OrderEvent::OrderPlaced { customer_id, order_id, total, .. } => {
                // Idempotent upsert with timestamp check
                self.store.upsert_with_timestamp(
                    &format!("customer:{customer_id}"),
                    |existing| {
                        let mut data = existing.unwrap_or_else(|| json!({
                            "customer_id": customer_id,
                            "total_orders": 0,
                            "total_spent_cents": 0,
                            "orders": [],
                            "last_updated": null,
                        }));

                        // Only update if event is newer
                        if should_update(&data, metadata.timestamp) {
                            data["total_orders"] = json!(data["total_orders"].as_u64().unwrap() + 1);
                            data["total_spent_cents"] = json!(data["total_spent_cents"].as_u64().unwrap() + total);
                            data["orders"].as_array_mut().unwrap().push(json!(order_id));
                            data["last_updated"] = json!(metadata.timestamp.to_rfc3339());
                        }

                        data
                    },
                ).await?;

                Ok(())
            }
            _ => Ok(()),
        }
    }
}

fn should_update(data: &serde_json::Value, event_timestamp: DateTime<Utc>) -> bool {
    data.get("last_updated")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|existing| event_timestamp > existing.with_timezone(&Utc))
        .unwrap_or(true)
}
```

**Key Points**:
- Projections are eventually consistent
- Use timestamps for idempotency
- Handle out-of-order events gracefully
- See [`docs/projections.md`](./projections.md) for comprehensive guide

---

## Real-Time Notifications

**When**: User notifications, WebSocket updates, email alerts

**Pattern**: Subscribe to action stream, filter relevant events, send notifications.

```rust
async fn start_notification_service(
    store: Arc<Store<OrderState, OrderAction, OrderEnv, OrderReducer>>,
    email_provider: Arc<dyn EmailProvider>,
) {
    let mut rx = store.subscribe_actions();

    tokio::spawn(async move {
        while let Ok(action) = rx.recv().await {
            match action {
                OrderAction::OrderPlaced { customer_id, order_id, .. } => {
                    // Send confirmation email
                    if let Err(e) = email_provider
                        .send_order_confirmation(&customer_id, &order_id)
                        .await
                    {
                        warn!("Failed to send confirmation email: {}", e);
                        // Don't fail - email is best-effort
                    }
                }

                OrderAction::OrderShipped { customer_id, tracking_number, .. } => {
                    // Send tracking email
                    if let Err(e) = email_provider
                        .send_tracking_notification(&customer_id, &tracking_number)
                        .await
                    {
                        warn!("Failed to send tracking email: {}", e);
                    }
                }

                _ => {}
            }
        }
    });
}
```

**Key Points**:
- Notifications are fire-and-forget (eventual consistency)
- Failures are logged but don't halt the system
- Use `subscribe_actions()` to listen to all events
- See [`docs/email-providers.md`](./email-providers.md) for email setup

---

## Batch Operations

**When**: Bulk imports, data migrations, performance optimization

**Pattern**: Use `append_batch()` for efficient bulk event storage.

```rust
async fn import_orders(
    event_store: &PostgresEventStore,
    orders: Vec<OrderImport>,
) -> Result<()> {
    let batches: Vec<EventBatch> = orders
        .into_iter()
        .map(|order| EventBatch {
            stream_id: StreamId::new(&format!("order-{}", order.id)),
            events: vec![
                serialize(&OrderPlaced {
                    order_id: order.id,
                    customer_id: order.customer_id,
                    items: order.items,
                }),
            ],
            expected_version: None,
        })
        .collect();

    // 10-100x faster than individual appends
    event_store.append_batch(batches).await?;

    Ok(())
}
```

**Key Points**:
- `append_batch()` is 10-100x faster than individual appends
- Use for migrations, imports, bulk operations
- Maintains atomicity per stream
- See [`docs/production-database.md`](./production-database.md) for details

---

## Idempotency

**When**: Retries, at-least-once delivery, duplicate prevention

**Pattern**: Use event version checks or deduplication IDs.

### Version-Based Idempotency

```rust
Effect::AppendEvents {
    stream_id: StreamId::new(&order_id),
    events: vec![serialize(&OrderPlaced { order_id: order_id.clone() })],
    expected_version: Some(0), // Fail if not first event
}
```

### ID-Based Deduplication

```rust
struct OrderState {
    processed_request_ids: HashSet<String>,
}

impl Reducer for OrderReducer {
    fn reduce(&self, state: &mut OrderState, action: OrderAction, env: &OrderEnv) -> SmallVec<[Effect<OrderAction>; 4]> {
        match action {
            OrderAction::PlaceOrder { request_id, .. } => {
                // Check if already processed
                if state.processed_request_ids.contains(&request_id) {
                    return smallvec![Effect::None]; // Idempotent - already processed
                }

                // Process and mark as seen
                state.processed_request_ids.insert(request_id.clone());

                // ... continue with order placement
                vec![/* ... */]
            }
            _ => vec![],
        }
    }
}
```

**Key Points**:
- Version checks prevent duplicate events
- Request IDs enable idempotent commands
- Critical for at-least-once delivery semantics

---

## Error Handling

**When**: External service failures, transient errors, domain validation

**Pattern**: Model errors as domain events, use retry policies for transient failures.

### Domain Errors as Events

```rust
match action {
    OrderAction::PlaceOrder { customer_id, items } => {
        // Validate in reducer
        if items.is_empty() {
            return smallvec![Effect::Future(Box::pin(async move {
                Some(OrderAction::OrderRejected {
                    reason: "Order must contain at least one item".into(),
                })
            }))];
        }

        // ... continue processing
    }
}
```

### Retry Policies for Transient Failures

```rust
Effect::Future(Box::pin({
    let retry_policy = env.retry_policy.clone();
    let payment_gateway = env.payment_gateway.clone();
    async move {
        let result = retry_policy
            .retry(|| payment_gateway.charge(&payment))
            .await;

        match result {
            Ok(charge_id) => Some(OrderAction::PaymentCharged { charge_id }),
            Err(e) => Some(OrderAction::PaymentFailed { reason: e.to_string() }),
        }
    }
}))
```

**Key Points**:
- Domain errors are modeled as actions/events
- Transient failures use retry policies
- Circuit breakers prevent cascading failures
- See [`docs/error-handling.md`](./error-handling.md) for complete guide

---

## Testing Patterns

**When**: Unit tests, integration tests, end-to-end tests

### Unit Testing (Fastest)

```rust
#[test]
fn test_order_placement() {
    let mut state = OrderState::default();
    let reducer = OrderReducer;
    let env = OrderEnv {
        clock: test_clock(),
        event_store: InMemoryEventStore::new(),
    };

    let effects = reducer.reduce(
        &mut state,
        OrderAction::PlaceOrder {
            customer_id: "cust-1".into(),
            items: vec![/* ... */],
        },
        &env,
    );

    assert_eq!(state.orders.len(), 1);
    assert!(matches!(effects[0], Effect::AppendEvents { .. }));
}
```

### Integration Testing (Fast)

```rust
#[tokio::test]
async fn test_order_workflow() {
    let store = Store::new(
        OrderState::default(),
        OrderReducer,
        OrderEnv {
            clock: test_clock(),
            event_store: InMemoryEventStore::new(),
            event_bus: InMemoryEventBus::new(),
        },
    );

    store.send(OrderAction::PlaceOrder { /* ... */ }).await.unwrap();

    let orders = store.state(|s| s.orders.len()).await;
    assert_eq!(orders, 1);
}
```

**Key Points**:
- Use `InMemoryEventStore` and `InMemoryEventBus` for fast tests
- Use `FixedClock` for deterministic time
- Test reducers directly (fastest) or with Store (integration)
- See [`docs/testing/README.md`](../testing/README.md) for comprehensive guide

---

## Performance Optimization

**When**: High-throughput scenarios, latency-sensitive operations

### Use SmallVec for Effects

```rust
use smallvec::{smallvec, SmallVec};

fn reduce(...) -> SmallVec<[Effect<OrderAction>; 4]> {
    // Most common case: 1-2 effects (stack allocation)
    smallvec![
        Effect::AppendEvents { /* ... */ },
        Effect::PublishEvent { /* ... */ },
    ]
}
```

### Batch Event Appends

```rust
// Instead of multiple individual appends:
for order in orders {
    event_store.append_events(/* ... */).await?;
}

// Use batch append:
let batches = orders.into_iter().map(|o| EventBatch { /* ... */ }).collect();
event_store.append_batch(batches).await?; // 10-100x faster
```

### Optimize State Access

```rust
// Read-only access (allows concurrent reads)
let count = store.state(|s| s.orders.len()).await;

// Avoid unnecessary clones
let order_id = store.state(|s| s.orders.keys().next().cloned()).await;
```

**Key Points**:
- SmallVec avoids heap allocation for common cases
- Batch operations for bulk work
- Minimize state access frequency
- See [`runtime/benches/`](../runtime/benches/) for benchmarks

---

## Further Reading

- [Getting Started Guide](./getting-started.md) - Complete tutorial
- [Saga Patterns](./saga-patterns.md) - Multi-aggregate coordination
- [Projections Guide](./projections.md) - Read model patterns
- [Error Handling](./error-handling.md) - Comprehensive error handling
- [Testing Guide](../testing/README.md) - Test utilities and patterns
- [Production Database](./production-database.md) - Performance optimization

## License

MIT OR Apache-2.0
