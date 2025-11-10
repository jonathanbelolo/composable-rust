# Consistency Patterns in Composable Rust

**Critical Architectural Guidance for Event-Driven Systems**

> ⚠️ **Required Reading**: This document is essential for all developers building sagas, projections, and event-driven workflows. The patterns described here prevent common architectural mistakes that lead to race conditions, data inconsistencies, and hard-to-debug production issues.

---

## Table of Contents

1. [Overview](#overview)
2. [The Consistency Spectrum](#the-consistency-spectrum)
3. [When to Use Projections vs Event Store](#when-to-use-projections-vs-event-store)
4. [Saga Patterns: Avoiding Projection Dependencies](#saga-patterns-avoiding-projection-dependencies)
5. [Event Design for Workflows](#event-design-for-workflows)
6. [Read-After-Write Patterns](#read-after-write-patterns)
7. [Testing Patterns](#testing-patterns)
8. [Decision Tree](#decision-tree)
9. [Common Pitfalls](#common-pitfalls)
10. [Architecture Decision Records](#architecture-decision-records)

---

## Overview

Composable Rust uses **CQRS** (Command Query Responsibility Segregation) and **Event Sourcing**, which means:

- **Events** are the source of truth (append-only, immutable)
- **Event Store** provides strong consistency (read-your-writes)
- **Projections** are eventually consistent (10-100ms lag)

This separation enables massive scalability but requires careful architectural decisions.

### Key Principle

> **Projections are for queries, not for workflows.**
>
> If a saga or command needs to make a decision based on data, it must NOT query a projection. Projections lag behind events and will cause race conditions.

---

## The Consistency Spectrum

Understanding consistency models is critical for architectural decisions:

```
Strong Consistency          Eventually Consistent         No Consistency
        │                            │                         │
        ▼                            ▼                         ▼
   Event Store                  Projections               Cached Data
   (immediate)                  (10-100ms lag)           (arbitrary lag)
```

### Strong Consistency (Event Store)

**Properties**:
- Read-your-writes guaranteed
- Linearizable (events have total order)
- Single source of truth
- Suitable for decision-making

**Use When**:
- Commands that depend on previous writes
- Saga decision points
- Critical business logic
- Anything requiring current state

**Example**: Checking account balance before withdrawal

```rust
// ✅ GOOD: Read from event store (current balance)
let events = event_store.load_events(&account_stream_id).await?;
let account = Account::from_events(events);

if account.balance >= withdrawal_amount {
    // Safe to process withdrawal
}
```

### Eventual Consistency (Projections)

**Properties**:
- Lag behind events (10-100ms typical)
- Optimized for queries
- May return stale data
- NOT suitable for decision-making

**Use When**:
- UI display (customer list, order history)
- Search interfaces
- Analytics and reports
- Non-critical queries

**Example**: Displaying customer order history

```rust
// ✅ GOOD: Query projection for display
let orders = projection.get_customer_orders(&customer_id).await?;
// Lag is acceptable for UI display
```

---

## When to Use Projections vs Event Store

### ✅ Use Projections For

**Read-heavy, non-critical queries**:

```rust
// ✅ UI display - eventual consistency is fine
async fn get_customer_dashboard(&self, customer_id: &str) -> Result<Dashboard> {
    let orders = self.order_projection.get_customer_orders(customer_id).await?;
    let total_spent = self.analytics_projection.get_lifetime_value(customer_id).await?;

    Ok(Dashboard {
        recent_orders: orders,
        total_spent,
        // This data can be 10-100ms stale
    })
}
```

**Characteristics**:
- 10-100ms staleness is acceptable
- Read performance is critical (10K+ reads/sec)
- Complex queries (joins, aggregations, full-text search)
- Denormalized data for efficiency

**Examples**:
- Customer order history (UI)
- Product search (e-commerce)
- Analytics dashboards
- Admin reports
- Email templates (using last-known data)

### ❌ DON'T Use Projections For

**Critical workflows and decision-making**:

```rust
// ❌ BAD: Saga queries projection immediately after write
async fn handle_order_placed(&mut self, event: OrderPlacedEvent) -> Result<()> {
    // ❌ Projection might not be updated yet!
    let order = self.projection.get_order(&event.order_id).await?;

    // ❌ Race condition: projection may not have the order yet
    if order.is_none() {
        return Err("Order not found".into()); // False negative!
    }

    // Continue with saga...
}
```

**Why This Fails**:
1. Event is published to event bus
2. Saga receives event immediately
3. Projection is still processing (10-100ms lag)
4. Saga queries projection → **data not there yet**
5. Saga fails or makes wrong decision

**Characteristics**:
- Immediate consistency required
- Commands that read-then-write
- Saga decision points
- Critical business logic
- Anything where correctness depends on current state

**Examples**:
- Account balance checks (withdrawal, transfer)
- Inventory reservation (prevent overselling)
- Saga compensation decisions
- Command validation against current state
- Read-after-write within same transaction

### ✅ Use Event Store For

**Current state reconstruction for decision-making**:

```rust
// ✅ GOOD: Read current state from event store
async fn handle_order_placed(&mut self, event: OrderPlacedEvent) -> Result<()> {
    // Saga carries all needed data from the event
    self.state.order_id = event.order_id.clone();
    self.state.order_total = event.total;
    self.state.items = event.items.clone();

    // Next step: charge payment (no query needed!)
    self.charge_payment(self.state.order_total).await
}
```

Or when you really need current state:

```rust
// ✅ GOOD: Read from event store when you need current state
async fn process_withdrawal(&self, account_id: &str, amount: Money) -> Result<()> {
    // Load current account state from events
    let stream_id = format!("account-{}", account_id);
    let events = self.event_store.load_events(&stream_id).await?;
    let account = Account::from_events(events);

    // Now we have current balance - safe to make decision
    if account.balance >= amount {
        // Process withdrawal...
    } else {
        return Err(InsufficientFunds);
    }

    Ok(())
}
```

---

## Saga Patterns: Avoiding Projection Dependencies

### The Golden Rule

> **Sagas should NEVER query projections.**
>
> If a saga needs data, it should either:
> 1. Carry the data in saga state
> 2. Read from event store (for current state)
> 3. Receive data in events

### Pattern 1: Carry State Through Saga

**The correct pattern** - saga has all data it needs:

```rust
#[derive(Clone, Debug)]
struct CheckoutSagaState {
    // All data carried from order creation
    order_id: OrderId,
    customer_id: CustomerId,
    items: Vec<LineItem>,           // Full item details
    order_total: Money,              // Pre-calculated
    shipping_address: Address,       // Complete address
    payment_method: PaymentMethod,   // Full details

    // Saga progress tracking
    payment_confirmed: bool,
    inventory_reserved: bool,
    shipping_scheduled: bool,
}

impl CheckoutSaga {
    async fn handle_order_placed(&mut self, event: OrderPlacedEvent) -> Vec<Effect<SagaAction>> {
        // Event carries ALL data we need
        self.state.order_id = event.order_id;
        self.state.customer_id = event.customer_id;
        self.state.items = event.items;           // ✅ Full items, not just IDs
        self.state.order_total = event.total;     // ✅ Pre-calculated
        self.state.shipping_address = event.shipping_address;  // ✅ Complete
        self.state.payment_method = event.payment_method;      // ✅ All details

        // Next step: charge payment
        // ✅ No projection query needed - we have everything!
        vec![Effect::Future(Box::pin(async move {
            self.payment_service.charge(
                self.state.payment_method,
                self.state.order_total
            ).await?;
            Some(SagaAction::PaymentCharged { order_id: self.state.order_id })
        }))]
    }

    async fn handle_payment_charged(&mut self, event: PaymentChargedEvent) -> Vec<Effect<SagaAction>> {
        self.state.payment_confirmed = true;

        // Reserve inventory
        // ✅ We have items from initial state - no query!
        vec![Effect::Future(Box::pin(async move {
            self.inventory_service.reserve_items(
                &self.state.order_id,
                &self.state.items  // ✅ Already in saga state
            ).await?;
            Some(SagaAction::InventoryReserved { order_id: self.state.order_id })
        }))]
    }

    async fn handle_payment_failed(&mut self, event: PaymentFailedEvent) -> Vec<Effect<SagaAction>> {
        // Compensate: cancel order
        // ✅ We have order_id - no query needed
        vec![Effect::PublishEvent(OrderAction::CancelOrder {
            order_id: self.state.order_id.clone(),
            reason: "Payment failed".to_string(),
        })]
    }
}
```

**Key Points**:
- Saga state contains ALL data needed for the entire workflow
- Events carry complete data (fat events)
- No projection queries at any step
- Saga can make decisions immediately

### Pattern 2: Read from Event Store (When Needed)

When saga needs current state of an aggregate:

```rust
impl TransferSaga {
    async fn handle_transfer_initiated(&mut self, event: TransferInitiatedEvent) -> Result<Vec<Effect<SagaAction>>> {
        // Need current balance to validate transfer
        let from_stream = format!("account-{}", event.from_account_id);
        let events = self.event_store.load_events(&from_stream).await?;
        let from_account = Account::from_events(events);

        // ✅ Now we have current balance
        if from_account.balance >= event.amount {
            // Proceed with transfer
            Ok(vec![
                Effect::PublishEvent(AccountAction::Debit {
                    account_id: event.from_account_id,
                    amount: event.amount,
                }),
            ])
        } else {
            // Insufficient funds - compensate
            Ok(vec![
                Effect::PublishEvent(TransferAction::TransferFailed {
                    transfer_id: event.transfer_id,
                    reason: "Insufficient funds".to_string(),
                }),
            ])
        }
    }
}
```

**When to Use**:
- Saga needs to check current state of an aggregate
- Can't carry all data upfront (e.g., account balance changes over time)
- Need to validate against current conditions

**Trade-offs**:
- Slower than carrying data (event store query required)
- Still strongly consistent (read-your-writes)
- Good for infrequent checks in saga

### ❌ Anti-Pattern: Querying Projections in Sagas

**DO NOT DO THIS**:

```rust
// ❌ WRONG: Saga depends on projection
impl CheckoutSaga {
    async fn handle_order_placed(&mut self, event: OrderPlacedEvent) -> Result<Vec<Effect<SagaAction>>> {
        // ❌ BAD: Projection might not be updated yet
        let order = self.projection.get_order(&event.order_id).await?;

        // ❌ Race condition - projection lags behind
        match order {
            Some(order) => {
                // ❌ This might fail even though order was just created
                self.process_order(order).await
            }
            None => {
                // ❌ False negative - order exists but projection not updated
                Err("Order not found".into())
            }
        }
    }
}
```

**Why This Fails**:
```
Time    Event Store          Event Bus           Projection          Saga
────────────────────────────────────────────────────────────────────────────
T+0ms   OrderPlaced saved    ─────────→         Not updated yet     ─────→ Saga triggered
T+10ms                                           Processing event
T+20ms                                                               ❌ Query projection
T+30ms                                                               ❌ Order not found!
T+40ms                                           Order saved         (Too late)
```

**The Problem**: Projection lag (10-100ms) creates a race condition where saga executes before projection is updated.

---

## Event Design for Workflows

### Fat Events vs Thin Events

The design of your events determines whether sagas can work without querying projections.

#### ❌ Thin Events (Anti-Pattern)

Events with minimal data force consumers to query:

```rust
// ❌ BAD: Thin event with only IDs
#[derive(Clone, Serialize, Deserialize)]
pub struct OrderPlacedEvent {
    pub order_id: OrderId,
    pub timestamp: DateTime<Utc>,
    // ❌ No other data - consumers must query to get details
}

// This forces consumers to query:
async fn handle_order_placed(&mut self, event: OrderPlacedEvent) -> Result<()> {
    // ❌ Must query to get order details
    let order = self.projection.get_order(&event.order_id).await?;  // Race condition!

    // Now process...
}
```

**Problems**:
- Forces projection queries (race conditions)
- Tight coupling between services
- Higher latency (extra queries)
- Can't process events in isolation

#### ✅ Fat Events (Correct Pattern)

Events with complete data enable independent processing:

```rust
// ✅ GOOD: Fat event with all needed data
#[derive(Clone, Serialize, Deserialize)]
pub struct OrderPlacedEvent {
    pub order_id: OrderId,
    pub customer_id: CustomerId,
    pub items: Vec<LineItem>,              // ✅ Full item details
    pub subtotal: Money,                   // ✅ Pre-calculated
    pub tax: Money,                        // ✅ Pre-calculated
    pub total: Money,                      // ✅ Pre-calculated
    pub shipping_address: Address,         // ✅ Complete address
    pub billing_address: Address,          // ✅ Complete address
    pub payment_method: PaymentMethod,     // ✅ Full payment details
    pub discount_code: Option<String>,     // ✅ Applied discount
    pub timestamp: DateTime<Utc>,
}

// Consumers can process without queries:
async fn handle_order_placed(&mut self, event: OrderPlacedEvent) -> Result<()> {
    // ✅ All data is in the event - no queries needed!
    self.state.order_total = event.total;
    self.state.items = event.items;
    self.state.shipping_address = event.shipping_address;

    // Process immediately...
}
```

**Benefits**:
- No projection queries (no race conditions)
- Events are self-contained
- Can process events in isolation
- Lower latency (no extra queries)
- Better testability

### Design Guideline

> **Include in events any data that downstream consumers will need.**
>
> If a saga will need it, include it in the event.

**Checklist**:
- [ ] Saga decision-making data (amounts, status, etc.)
- [ ] Data for other aggregates (customer info for order)
- [ ] Pre-calculated values (totals, tax, etc.)
- [ ] Complete addresses (not just IDs)
- [ ] Denormalized lookups (product names, not just product IDs)

### Trade-offs

| Aspect | Thin Events | Fat Events |
|--------|-------------|------------|
| Event size | Smaller (100-500 bytes) | Larger (1-5 KB) |
| Storage cost | Lower | Higher |
| Processing | Requires queries | Self-contained |
| Latency | Higher (queries) | Lower (no queries) |
| Race conditions | Common | Eliminated |
| **Recommendation** | ❌ Don't use | ✅ Use for workflows |

**Verdict**: For critical workflows, **always use fat events**. Storage is cheap, race conditions are expensive.

---

## Read-After-Write Patterns

When a client creates something and immediately wants to use it, you need to handle eventual consistency carefully.

### Pattern 1: Return Data from Command (Recommended)

The command returns the created data directly:

```rust
// ✅ GOOD: Return data immediately from command
#[async_trait]
impl OrderService {
    async fn place_order(&self, cmd: PlaceOrderCommand) -> Result<Order> {
        let order = Order::new(cmd.customer_id, cmd.items);

        // Save events
        self.event_store.append_events(&order.stream_id(), order.events()).await?;

        // ✅ Return the created order
        Ok(order)
    }
}

// Client gets data immediately, no query needed:
async fn checkout_handler(order_service: &OrderService, cmd: PlaceOrderCommand) -> Result<CheckoutResponse> {
    // ✅ Get order directly from command
    let order = order_service.place_order(cmd).await?;

    // ✅ Use it immediately - no projection query
    Ok(CheckoutResponse {
        order_id: order.id,
        total: order.total,
        status: order.status,
    })
}
```

**Benefits**:
- Immediate access to created data
- No race condition
- No projection query needed
- Simple and efficient

### Pattern 2: Read from Event Store

When you can't return data from command, read from event store:

```rust
async fn get_current_account_balance(account_id: &str) -> Result<Money> {
    // ✅ Read from event store (always current)
    let stream_id = format!("account-{}", account_id);
    let events = event_store.load_events(&stream_id).await?;
    let account = Account::from_events(events);

    Ok(account.balance)
}
```

**When to Use**:
- Need current state immediately after write
- Can't return data from command
- Strong consistency required

**Trade-offs**:
- Slower than returning from command
- Requires event replay
- Good with snapshots (faster replay)

### Pattern 3: Accept Eventual Consistency (UI Only)

For non-critical UI, accept the lag:

```rust
// Client accepts eventual consistency
async fn place_order_handler(order_service: &OrderService, cmd: PlaceOrderCommand) -> Result<Response> {
    let order_id = order_service.place_order(cmd).await?;

    // ✅ UI shows "Processing..." state
    Ok(Response {
        order_id,
        message: "Your order is being processed. You'll receive a confirmation email shortly.",
    })
}

// UI polls or uses websockets for updates:
async fn poll_order_status(order_id: &str) -> Result<OrderStatus> {
    // Query projection - eventual consistency is OK
    let order = projection.get_order(order_id).await?;
    Ok(order.map(|o| o.status).unwrap_or(OrderStatus::Processing))
}
```

**When to Use**:
- UI display only
- Non-critical information
- Can show loading/processing state

**UI Strategies**:
- Show "Processing..." placeholder
- Optimistic UI updates (show expected state)
- Polling (check every few seconds)
- WebSocket updates (push when ready)

### Pattern 4: Real-Time Updates with WebSockets (Recommended for Modern UIs)

The best way to handle eventual consistency in modern UIs is **real-time event streaming**:

```rust
use composable_rust_web::handlers::websocket;
use axum::{Router, routing::get};

// Server: Add WebSocket endpoint to your router
pub fn order_router(
    store: Arc<Store<OrderState, OrderAction, OrderEnvironment, OrderReducer>>,
) -> Router {
    Router::new()
        // HTTP endpoints for commands
        .route("/orders", post(handlers::place_order))
        .route("/orders/:id", get(handlers::get_order))
        // WebSocket for real-time events
        .route("/ws", get(websocket::handle::<OrderState, OrderAction, _, _>))
        .with_state(store)
}
```

**Client-side JavaScript**:

```javascript
// 1. Connect to WebSocket
const ws = new WebSocket('ws://localhost:3000/api/v1/ws');

// 2. Send commands through WebSocket
function placeOrder(orderData) {
  ws.send(JSON.stringify({
    type: "command",
    action: {
      PlaceOrder: orderData
    }
  }));

  // ✅ Show optimistic UI immediately
  showOrderProcessing(orderData);
}

// 3. Receive real-time events
ws.onmessage = (event) => {
  const message = JSON.parse(event.data);

  if (message.type === "event") {
    const action = message.action;

    // ✅ Update UI immediately when event arrives
    if (action.OrderPlaced) {
      updateOrderStatus(action.OrderPlaced.order_id, 'pending');
    } else if (action.OrderShipped) {
      updateOrderStatus(action.OrderShipped.order_id, 'shipped');
      showNotification('Your order has shipped!');
    }
  }
};
```

**How It Works**:

1. **Client sends command** → WebSocket → Store dispatch
2. **Store processes action** → State updated → Events saved
3. **Store broadcasts action** → All WebSocket clients receive it
4. **Clients update UI** in real-time (10-50ms)

**Benefits over Polling**:
- **Lower latency**: ~10-50ms vs 2-5 seconds for polling
- **Less load**: No repeated HTTP requests
- **Better UX**: Instant updates, live collaboration
- **Efficient**: Push-based, not pull-based

**Real-World Example** (from `examples/order-processing`):

```rust
// Server automatically broadcasts all actions
let store = Store::new(
    OrderState::new(),
    OrderReducer::new(),
    environment,
);

// Any action dispatched to the store is broadcast to all WebSocket clients:
store.send(OrderAction::PlaceOrder { /* ... */ }).await?;
// ↓
// All connected WebSocket clients receive:
// {"type": "event", "action": {"OrderPlaced": {...}}}
```

**When to Use**:
- Real-time dashboards (order tracking, analytics)
- Collaborative editing (multiple users see changes)
- Live notifications (order shipped, payment completed)
- Progress tracking (saga steps, processing status)
- Chat/messaging features

**See Also**:
- [WebSocket Guide](./websocket.md) - Complete WebSocket implementation guide
- [Order Processing Example](../examples/order-processing/) - Working WebSocket integration

### Pattern 5: Email Notifications with Eventual Consistency

Email notifications naturally work well with eventual consistency because they are **asynchronous and non-blocking**:

```rust
use composable_rust_auth::providers::EmailProvider;

// Email effect in reducer
impl Reducer for OrderReducer {
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> Vec<Effect<Self::Action>> {
        match action {
            OrderAction::OrderShipped { order_id, tracking_number } => {
                // Update state
                state.status = OrderStatus::Shipped;
                state.tracking_number = Some(tracking_number.clone());

                // ✅ Send email notification (async, eventual)
                vec![
                    Effect::Future(Box::pin({
                        let email = env.email_provider.clone();
                        let customer_email = state.customer_email.clone();
                        async move {
                            email.send_security_alert(
                                &customer_email,
                                "Your order has shipped!",
                                &format!("Tracking: {tracking_number}"),
                            ).await.ok();
                            None // No follow-up action
                        }
                    })),
                    Effect::PublishEvent(OrderAction::OrderShipped {
                        order_id,
                        tracking_number,
                    }),
                ]
            }
            // ...
        }
    }
}
```

**Key Characteristics**:
- **Fire-and-forget**: Email delivery doesn't block the workflow
- **Eventual**: Email arrives seconds/minutes later (acceptable)
- **Best-effort**: Failed emails don't fail the command
- **Non-critical**: User gets email eventually, or checks UI

**Environment Setup**:

```rust
use composable_rust_auth::providers::{SmtpEmailProvider, ConsoleEmailProvider};

// Development: Console email (logs to terminal)
let email_provider = ConsoleEmailProvider::new();

// Production: Real SMTP email
let email_provider = SmtpEmailProvider::new(
    env::var("SMTP_SERVER")?,
    env::var("SMTP_PORT")?.parse()?,
    env::var("SMTP_USERNAME")?,
    env::var("SMTP_PASSWORD")?,
    env::var("FROM_EMAIL")?,
    env::var("FROM_NAME")?,
)?;

let environment = OrderEnvironment {
    email_provider,
    event_store,
    // ... other dependencies
};
```

**Email Design Guidelines**:

1. **Include complete data in email** (don't query projections):
   ```rust
   // ❌ BAD: Query projection for email data
   async fn send_order_confirmation(order_id: &str) -> Result<()> {
       let order = projection.get_order(order_id).await?; // Race!
       email.send_order_confirmation(&order).await?;
   }

   // ✅ GOOD: Saga state has all email data
   async fn send_order_confirmation(state: &SagaState) -> Result<()> {
       email.send_order_confirmation(
           &state.customer_email,
           &state.order_id,
           &state.items,          // Already in saga state
           &state.shipping_address, // Already in saga state
       ).await?;
   }
   ```

2. **Make emails idempotent** (users might receive duplicates):
   ```text
   Subject: [Order #12345] Shipped

   Your order #12345 has shipped!

   Track your package: https://example.com/track/ABC123

   (If you've already received this email, please discard it)
   ```

3. **Use emails for confirmation, not coordination**:
   - ✅ "Your order has been placed" (confirmation)
   - ✅ "Your order has shipped" (notification)
   - ❌ "Click here to complete your order" (coordination - use UI instead)

**Email vs WebSocket**:

| Notification Type | Use Email | Use WebSocket |
|-------------------|-----------|---------------|
| User offline | ✅ Yes | ❌ No |
| Important record | ✅ Yes | ❌ No |
| Real-time critical | ❌ No | ✅ Yes |
| Active user in UI | ⚠️ Both | ✅ Yes |
| User must act | ✅ Yes (with link) | ⚠️ Maybe |

**Best Practice**: Use **both** for important events:
- WebSocket → Immediate UI update (10-50ms)
- Email → Durable notification + offline users

**See Also**:
- [Email Providers Guide](./email-providers.md) - Complete email setup and configuration
- [Auth Example](../auth/) - Email in magic link authentication

---

## Testing Patterns

Testing systems with eventual consistency requires different strategies.

### Pattern 1: Test Sagas Without Projections

Sagas should never depend on projections, so they shouldn't be in saga tests:

```rust
#[tokio::test]
async fn test_checkout_saga_happy_path() {
    // Arrange: Create saga with mock services
    let payment_service = MockPaymentService::new();
    let inventory_service = MockInventoryService::new();
    let saga = CheckoutSaga::new(payment_service, inventory_service);

    // Act: Event with all needed data (fat event)
    let event = OrderPlacedEvent {
        order_id: OrderId::new("order-1"),
        customer_id: CustomerId::new("cust-1"),
        items: vec![
            LineItem { product_id: "prod-1".into(), quantity: 2, price: Money::from_dollars(10) }
        ],
        total: Money::from_dollars(20),
        shipping_address: Address { /* ... */ },
        payment_method: PaymentMethod::CreditCard { /* ... */ },
        timestamp: Utc::now(),
    };

    // Saga processes without any projection queries
    let effects = saga.handle(event).await?;

    // Assert: Saga state and effects
    assert_eq!(saga.state.order_total, Money::from_dollars(20));
    assert!(matches!(effects[0], Effect::Future(_)));  // Payment charge

    // ✅ No projection used - fast, deterministic test
}
```

**Key Points**:
- No `InMemoryProjectionStore` in saga tests
- Saga gets all data from events
- Fast (memory-only, no I/O)
- Deterministic (no timing issues)

### Pattern 2: Test Projections Separately

Test projections in isolation:

```rust
#[tokio::test]
async fn test_order_projection_updates() {
    // Arrange: In-memory projection store
    let store = Arc::new(InMemoryProjectionStore::new());
    let projection = CustomerOrderHistoryProjection::new(store.clone());
    let mut harness = ProjectionTestHarness::new(projection, store);

    // Act: Apply events
    harness.given_events(vec![
        OrderAction::OrderPlaced {
            order_id: OrderId::new("order-1"),
            customer_id: CustomerId::new("cust-1"),
            items: vec![/* ... */],
            total: Money::from_dollars(99),
            timestamp: Utc::now(),
        },
        OrderAction::OrderShipped {
            order_id: OrderId::new("order-1"),
            tracking: "TRACK123".to_string(),
            timestamp: Utc::now(),
        },
    ]).await?;

    // Assert: Projection state
    harness.then_contains("order:order-1").await?;

    let data = harness.get_data("order:order-1").await?;
    assert!(data.is_some());

    // ✅ Eventual consistency is OK - this is a projection test
}
```

**Key Points**:
- Test projection update logic
- Eventual consistency is expected
- Use `InMemoryProjectionStore` for speed
- Integration tests use real PostgreSQL

### Pattern 3: Integration Tests with Timing

When testing the full system, account for projection lag:

```rust
#[tokio::test]
async fn test_end_to_end_order_flow() {
    // Arrange: Full system (event store, event bus, projections)
    let event_store = PostgresEventStore::new(pool.clone()).await?;
    let projection_manager = ProjectionManager::new(/* ... */);

    // Start projection manager
    let handle = tokio::spawn(async move {
        projection_manager.start().await
    });

    // Act: Place order
    let order_id = order_service.place_order(PlaceOrderCommand {
        customer_id: "cust-1".into(),
        items: vec![/* ... */],
    }).await?;

    // ⏰ Wait for projection to catch up (test-only)
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Assert: Query projection
    let orders = projection.get_customer_orders("cust-1").await?;
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].id, order_id);

    // Cleanup
    handle.abort();
}
```

**Key Points**:
- Full system integration test
- Sleep to wait for projection catch-up (test-only!)
- Tests eventual consistency behavior
- Slower (real I/O, timing-dependent)

### Test Pyramid for Eventual Consistency

```
                  ▲
                 ╱ ╲
                ╱   ╲         E2E Tests (Few)
               ╱     ╲        - Full system
              ╱───────╲       - Account for lag
             ╱         ╲      - Slow but realistic
            ╱───────────╲
           ╱             ╲    Integration Tests (Some)
          ╱   Projection  ╲   - Real databases
         ╱      Tests      ╲  - Test catch-up
        ╱─────────────────────╲
       ╱                       ╲
      ╱      Saga Tests         ╲  Unit Tests (Many)
     ╱    (No Projections)       ╲ - Fast, deterministic
    ╱─────────────────────────────╲ - No I/O
   ╱                               ╲
  ╱───────────────────────────────╲
 ╱          Reducer Tests           ╲
╱       (Pure functions)              ╲
───────────────────────────────────────
```

**Recommendations**:
- **80% Unit tests**: Reducers, sagas (no projections)
- **15% Integration tests**: Projections with real DBs
- **5% E2E tests**: Full system with timing

---

## Decision Tree

When you need to query data, follow this decision tree:

```
Need to query data?
├─ Is this in a saga or critical workflow?
│  ├─ Yes → DON'T use projection
│  │      ├─ Option 1: Carry state through saga (recommended)
│  │      ├─ Option 2: Read from event store (if needed)
│  │      └─ Option 3: Return data from command
│  └─ No → Is eventual consistency acceptable?
│         ├─ Yes → Use projection (UI, reports, search)
│         │      └─ Choose backend: PostgreSQL (complex queries)
│         │                         Redis (cache, counters)
│         │                         Elasticsearch (full-text search)
│         └─ No → Read from event store
│                └─ Consider snapshots for performance
```

### Quick Reference Table

| Use Case | Solution | Consistency | Latency |
|----------|----------|-------------|---------|
| Saga decision | Carry state in saga | Strong | Low |
| Saga needs current state | Read event store | Strong | Medium |
| Command validation | Read event store | Strong | Medium |
| UI display | Query projection | Eventual | Low |
| Search | Query projection (Elasticsearch) | Eventual | Low |
| Analytics | Query projection (PostgreSQL) | Eventual | Low |
| Cache | Query projection (Redis) | Eventual | Very low |
| Read after write (critical) | Return from command | Strong | Low |

---

## Common Pitfalls

### ❌ Pitfall 1: Saga Queries Projection After Write

**Problem**: Race condition between event and projection update

```rust
// ❌ WRONG
async fn handle_order_placed(&mut self, event: OrderPlacedEvent) -> Result<()> {
    // Projection not updated yet!
    let order = self.projection.get_order(&event.order_id).await?;
    // ... use order data
}
```

**Fix**: Carry data in saga state

```rust
// ✅ CORRECT
async fn handle_order_placed(&mut self, event: OrderPlacedEvent) -> Result<()> {
    // Event has all data we need
    self.state.order_id = event.order_id;
    self.state.order_total = event.total;
    self.state.items = event.items;
    // ... continue with saga
}
```

### ❌ Pitfall 2: Command Reads Projection Then Writes

**Problem**: Projection might be stale

```rust
// ❌ WRONG
async fn withdraw(&self, account_id: &str, amount: Money) -> Result<()> {
    // Projection might be stale!
    let account = self.projection.get_account(account_id).await?;

    if account.balance >= amount {
        // ❌ Balance might have changed!
        self.process_withdrawal(account_id, amount).await?;
    }
}
```

**Fix**: Read from event store

```rust
// ✅ CORRECT
async fn withdraw(&self, account_id: &str, amount: Money) -> Result<()> {
    // Read current state from event store
    let events = self.event_store.load_events(&stream_id).await?;
    let account = Account::from_events(events);

    if account.balance >= amount {
        // Balance is current
        self.process_withdrawal(account_id, amount).await?;
    }
}
```

### ❌ Pitfall 3: Events Only Contain IDs

**Problem**: Forces consumers to query

```rust
// ❌ WRONG: Thin event
struct OrderPlacedEvent {
    order_id: String,
    // Missing: items, total, addresses, etc.
}

// Forces this:
async fn handle(&mut self, event: OrderPlacedEvent) {
    let order = self.projection.get_order(&event.order_id).await?;  // Race!
}
```

**Fix**: Use fat events

```rust
// ✅ CORRECT: Fat event
struct OrderPlacedEvent {
    order_id: String,
    customer_id: String,
    items: Vec<LineItem>,     // Full details
    total: Money,              // Pre-calculated
    shipping_address: Address, // Complete
    // ... all data downstream needs
}

// Now consumers can process without queries:
async fn handle(&mut self, event: OrderPlacedEvent) {
    // All data is here!
    self.state.total = event.total;
}
```

### ❌ Pitfall 4: Testing with Real Projections in Unit Tests

**Problem**: Slow, flaky tests

```rust
// ❌ WRONG: Saga test with real projection
#[tokio::test]
async fn test_checkout_saga() {
    let postgres = setup_postgres().await;  // Slow!
    let projection = PostgresProjectionStore::new(postgres);
    let saga = CheckoutSaga::new(projection);  // Shouldn't need this!

    // Test is slow and flaky
}
```

**Fix**: Saga shouldn't depend on projections at all

```rust
// ✅ CORRECT: Saga test without projections
#[tokio::test]
async fn test_checkout_saga() {
    let saga = CheckoutSaga::new();  // No projection needed!

    let event = OrderPlacedEvent { /* all data */ };
    saga.handle(event).await?;

    // Fast, deterministic test
}
```

---

## Architecture Decision Records

### ADR-001: Why Sagas Don't Query Projections

**Context**: Sagas need to coordinate multiple aggregates and make decisions based on data.

**Decision**: Sagas MUST NOT query projections for decision-making.

**Rationale**:
1. **Race conditions**: Projections lag behind events (10-100ms). Saga receives event immediately but projection isn't updated yet.
2. **Correctness**: Saga decisions must be based on current state, not stale data.
3. **Testability**: Sagas without projection dependencies are faster and more deterministic to test.

**Consequences**:
- Events must be "fat" (include all data downstream needs)
- Saga state carries data through the workflow
- When saga needs current state, it reads from event store (not projection)
- Storage cost is higher (larger events) but correctness is ensured

**Status**: Accepted

### ADR-002: Event Design Philosophy (Fat Events)

**Context**: Events can be "thin" (IDs only) or "fat" (complete data).

**Decision**: Use fat events for critical workflows.

**Rationale**:
1. **Independence**: Consumers can process events without additional queries
2. **No race conditions**: All data is in the event, no need to query projections
3. **Performance**: No extra queries = lower latency
4. **Testability**: Events are self-contained, easier to test

**Trade-offs**:
- **Storage**: Fat events use more storage (1-5 KB vs 100-500 bytes)
- **Network**: Larger events = more bandwidth
- **Worth it**: Storage is cheap, race conditions are expensive

**Guidelines**:
- Include any data that downstream consumers will need
- Pre-calculate totals, taxes, etc.
- Include complete addresses, not just IDs
- Denormalize lookups (product names, not just product IDs)

**Status**: Accepted

### ADR-003: When to Use Event Store vs Projections

**Context**: Both event store and projections can be queried for data.

**Decision**: Use event store for decision-making, projections for display.

**Decision Matrix**:

| Characteristic | Use Event Store | Use Projection |
|----------------|-----------------|----------------|
| Consistency needed | Strong | Eventual OK |
| Use case | Commands, sagas | UI, search, analytics |
| Latency tolerance | N/A (needs current) | 10-100ms acceptable |
| Query complexity | Simple (by stream ID) | Complex (joins, full-text) |
| Read volume | Low-medium | High |

**Consequences**:
- Commands that validate against current state read from event store
- Sagas carry state or read from event store (never projections)
- UI queries use projections (eventual consistency is acceptable)
- Analytics and reports use projections

**Status**: Accepted

---

## Summary

### ✅ Do This

1. **Use projections for queries, event store for workflows**
2. **Carry state through sagas** (don't query)
3. **Use fat events** (include all downstream needs)
4. **Return data from commands** (avoid read-after-write)
5. **Test sagas without projections** (fast, deterministic)
6. **Document lag tolerance** (what can be stale?)

### ❌ Don't Do This

1. ❌ Query projections in sagas
2. ❌ Use projections for command validation
3. ❌ Use thin events for critical workflows
4. ❌ Assume read-after-write works with projections
5. ❌ Test with real projections in unit tests
6. ❌ Ignore eventual consistency in system design

---

## Further Reading

### Composable Rust Documentation

- [WebSocket Guide](./websocket.md) - Real-time event streaming with WebSockets
- [Email Providers Guide](./email-providers.md) - Email notifications with SMTP and console providers
- [Saga Patterns Guide](./saga-patterns.md) - Detailed saga implementation patterns
- [Event Design Guidelines](./event-design-guidelines.md) - Event schema best practices
- [Projections Guide](./projections.md) - Complete projection documentation
- [Getting Started](./getting-started.md) - Complete tutorial for building your first app

### Working Examples

- [Order Processing Example](../examples/order-processing/) - HTTP API + WebSocket + Event Sourcing
- [Checkout Saga Example](../examples/checkout-saga/) - Multi-aggregate coordination
- [Auth Example](../auth/) - Authentication with email providers and magic links

### External Resources

- [CQRS Pattern](https://martinfowler.com/bliki/CQRS.html) - Martin Fowler's CQRS introduction
- [Event Sourcing](https://martinfowler.com/eaaDev/EventSourcing.html) - Event Sourcing fundamentals
- [Eventual Consistency](https://www.allthingsdistributed.com/2008/12/eventually_consistent.html) - Werner Vogels on eventual consistency

---

**Last Updated**: 2025-01-09
**Status**: ✅ Production Ready
