# Order Processing Example

**Complete event-sourced order processing system demonstrating Composable Rust patterns.**

## Overview

This example demonstrates a production-ready order processing system built with event sourcing, HTTP APIs, and WebSocket support. It showcases:

- **Event Sourcing** - All state changes persisted as immutable events
- **HTTP API** - REST endpoints for order management
- **WebSocket** - Real-time order updates for connected clients
- **Request-Response Pattern** - `send_and_wait_for()` for synchronous API calls
- **Domain-Driven Design** - Clean separation of concerns

## Features

| Feature | Status | Description |
|---------|--------|-------------|
| Create Order | ✅ | Place new orders with line items |
| Cancel Order | ✅ | Cancel existing orders |
| Ship Order | ✅ | Mark orders as shipped |
| Event Replay | ✅ | Reconstruct state from events |
| HTTP API | ✅ | REST endpoints (`http` feature) |
| WebSocket | ✅ | Real-time updates (`http` feature) |
| PostgreSQL | ✅ | Persistent event store (`postgres` feature) |
| In-Memory | ✅ | Fast testing without database |

## Quick Start

### Run with In-Memory Store (No Database)

```bash
cargo run --bin order-processing
```

### Run with PostgreSQL

```bash
# Start PostgreSQL
docker compose up -d postgres

# Run with postgres feature
cargo run --bin order-processing --features postgres
```

### Run HTTP API with WebSocket

```bash
# Run server with HTTP and WebSocket support
cargo run --bin order-processing --features http

# In another terminal, interact with API:
curl http://localhost:3000/health

# Place an order
curl -X POST http://localhost:3000/api/v1/orders \
  -H "Content-Type: application/json" \
  -d '{
    "customer_id": "cust-123",
    "items": [
      {
        "product_id": "prod-1",
        "name": "Widget",
        "quantity": 2,
        "unit_price_cents": 1000
      }
    ]
  }'

# Cancel an order
curl -X POST http://localhost:3000/api/v1/orders/{order_id}/cancel

# Ship an order
curl -X POST http://localhost:3000/api/v1/orders/{order_id}/ship \
  -H "Content-Type: application/json" \
  -d '{"tracking_number": "TRACK123"}'
```

### WebSocket Client

```javascript
const ws = new WebSocket('ws://localhost:3000/ws');

// Receive real-time order updates
ws.onmessage = (event) => {
  const message = JSON.parse(event.data);

  if (message.type === 'event') {
    console.log('Order event:', message.action);
  }
};

// Send commands
ws.send(JSON.stringify({
  type: 'command',
  action: {
    PlaceOrder: {
      customer_id: 'cust-123',
      items: [/* ... */]
    }
  }
}));
```

## Architecture

### Domain Model

```
Order State Machine:
┌────────┐  PlaceOrder   ┌────────┐
│        │──────────────>│        │
│  Draft │               │ Placed │
│        │<──────────────│        │
└────────┘   (Initial)   └───┬────┘
                             │
                  ┌──────────┴───────────┐
                  │                      │
             CancelOrder            ShipOrder
                  │                      │
                  v                      v
            ┌──────────┐           ┌─────────┐
            │Cancelled │           │ Shipped │
            └──────────┘           └─────────┘
```

### Project Structure

```
order-processing/
├── src/
│   ├── types.rs       # Domain types (OrderId, OrderState, OrderAction, OrderEvent)
│   ├── reducer.rs     # Business logic (state transitions)
│   ├── handlers.rs    # HTTP request handlers (http feature)
│   ├── router.rs      # Axum router setup (http feature)
│   ├── lib.rs         # Public API and utilities
│   └── main.rs        # Application entry point
├── tests/
│   └── integration/   # Integration tests
└── Cargo.toml
```

### Core Types

#### State

```rust
#[derive(State, Clone, Debug, Default)]
pub struct OrderState {
    pub orders: HashMap<OrderId, Order>,
}

pub struct Order {
    pub id: OrderId,
    pub customer_id: CustomerId,
    pub items: Vec<OrderItem>,
    pub status: OrderStatus,
    pub total_cents: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub enum OrderStatus {
    Draft,
    Placed,
    Shipped { tracking_number: String, shipped_at: DateTime<Utc> },
    Cancelled { reason: String, cancelled_at: DateTime<Utc> },
}
```

#### Actions

```rust
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum OrderAction {
    // Commands
    PlaceOrder { customer_id: CustomerId, items: Vec<OrderItem> },
    CancelOrder { order_id: OrderId },
    ShipOrder { order_id: OrderId, tracking_number: String },

    // Events
    OrderPlaced { order_id: OrderId, customer_id: CustomerId, items: Vec<OrderItem>, total_cents: u64 },
    OrderCancelled { order_id: OrderId, reason: String },
    OrderShipped { order_id: OrderId, tracking_number: String },

    // Errors
    OrderNotFound { order_id: OrderId },
    InvalidOrderState { order_id: OrderId, current_status: String },
}
```

#### Events (Persisted)

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum OrderEvent {
    OrderPlaced { order_id: OrderId, customer_id: CustomerId, items: Vec<OrderItem>, total_cents: u64 },
    OrderCancelled { order_id: OrderId, reason: String },
    OrderShipped { order_id: OrderId, tracking_number: String },
}
```

### Reducer Logic

```rust
impl Reducer for OrderReducer {
    type State = OrderState;
    type Action = OrderAction;
    type Environment = OrderEnvironment;

    fn reduce(
        &self,
        state: &mut OrderState,
        action: OrderAction,
        env: &OrderEnvironment,
    ) -> Vec<Effect<OrderAction>> {
        match action {
            // Command: Place Order
            OrderAction::PlaceOrder { customer_id, items } => {
                let order_id = OrderId::new(generate_id());
                let total_cents = items.iter().map(|i| i.quantity * i.unit_price_cents).sum();

                // Create order in state
                state.orders.insert(order_id.clone(), Order {
                    id: order_id.clone(),
                    customer_id: customer_id.clone(),
                    items: items.clone(),
                    status: OrderStatus::Placed,
                    total_cents,
                    created_at: env.clock.now(),
                    updated_at: env.clock.now(),
                });

                // Persist event and broadcast
                vec![
                    Effect::AppendEvents {
                        stream_id: StreamId::new(&format!("order-{}", order_id)),
                        events: vec![serialize(&OrderEvent::OrderPlaced {
                            order_id: order_id.clone(),
                            customer_id,
                            items,
                            total_cents,
                        })],
                        expected_version: None,
                    },
                    Effect::Future(Box::pin(async move {
                        Some(OrderAction::OrderPlaced { order_id, customer_id, items, total_cents })
                    })),
                ]
            }

            // Command: Cancel Order
            OrderAction::CancelOrder { order_id } => {
                match state.orders.get_mut(&order_id) {
                    Some(order) => {
                        match order.status {
                            OrderStatus::Placed => {
                                let reason = "Cancelled by customer".to_string();
                                order.status = OrderStatus::Cancelled {
                                    reason: reason.clone(),
                                    cancelled_at: env.clock.now(),
                                };
                                order.updated_at = env.clock.now();

                                vec![
                                    Effect::AppendEvents {
                                        stream_id: StreamId::new(&format!("order-{}", order_id)),
                                        events: vec![serialize(&OrderEvent::OrderCancelled {
                                            order_id: order_id.clone(),
                                            reason: reason.clone(),
                                        })],
                                        expected_version: None,
                                    },
                                    Effect::Future(Box::pin(async move {
                                        Some(OrderAction::OrderCancelled { order_id, reason })
                                    })),
                                ]
                            }
                            _ => {
                                vec![Effect::Future(Box::pin({
                                    let status = format!("{:?}", order.status);
                                    async move {
                                        Some(OrderAction::InvalidOrderState {
                                            order_id,
                                            current_status: status,
                                        })
                                    }
                                }))]
                            }
                        }
                    }
                    None => {
                        vec![Effect::Future(Box::pin(async move {
                            Some(OrderAction::OrderNotFound { order_id })
                        }))]
                    }
                }
            }

            // Events are idempotent - already applied to state
            OrderAction::OrderPlaced { .. } |
            OrderAction::OrderCancelled { .. } |
            OrderAction::OrderShipped { .. } => vec![Effect::None],

            _ => vec![],
        }
    }
}
```

## HTTP API Reference

### Base URL

```
http://localhost:3000
```

### Endpoints

#### GET /health

Health check endpoint.

**Response**: `200 OK`
```json
{
  "status": "ok"
}
```

---

#### POST /api/v1/orders

Place a new order.

**Request**:
```json
{
  "customer_id": "cust-123",
  "items": [
    {
      "product_id": "prod-1",
      "name": "Widget",
      "quantity": 2,
      "unit_price_cents": 1000
    }
  ]
}
```

**Response**: `201 Created`
```json
{
  "order_id": "order-abc123",
  "status": "placed",
  "total_cents": 2000
}
```

---

#### POST /api/v1/orders/:order_id/cancel

Cancel an existing order.

**Response**: `200 OK`
```json
{
  "order_id": "order-abc123",
  "status": "cancelled"
}
```

**Errors**:
- `404 Not Found` - Order doesn't exist
- `400 Bad Request` - Order cannot be cancelled (already shipped/cancelled)

---

#### POST /api/v1/orders/:order_id/ship

Mark order as shipped.

**Request**:
```json
{
  "tracking_number": "TRACK123"
}
```

**Response**: `200 OK`
```json
{
  "order_id": "order-abc123",
  "status": "shipped",
  "tracking_number": "TRACK123"
}
```

---

#### GET /ws

WebSocket endpoint for real-time order updates.

See [WebSocket Protocol](#websocket-protocol) below.

## WebSocket Protocol

### Message Format

All messages are JSON with a `type` field:

```typescript
type WsMessage =
  | { type: "command", action: OrderAction }  // Client → Server
  | { type: "event", action: OrderAction }    // Server → Client
  | { type: "error", message: string }        // Server → Client
  | { type: "ping" }                           // Keepalive
  | { type: "pong" }                           // Keepalive response
```

### Example: Place Order via WebSocket

**Client sends**:
```json
{
  "type": "command",
  "action": {
    "PlaceOrder": {
      "customer_id": "cust-123",
      "items": [
        {
          "product_id": "prod-1",
          "name": "Widget",
          "quantity": 1,
          "unit_price_cents": 1000
        }
      ]
    }
  }
}
```

**Server broadcasts**:
```json
{
  "type": "event",
  "action": {
    "OrderPlaced": {
      "order_id": "order-xyz",
      "customer_id": "cust-123",
      "items": [/* ... */],
      "total_cents": 1000
    }
  }
}
```

All connected clients receive the event in real-time.

## Testing

### Run Unit Tests

```bash
cargo test
```

### Run Integration Tests (Requires PostgreSQL)

```bash
# Start PostgreSQL
docker compose up -d postgres

# Run tests
cargo test --features postgres --test integration
```

### Property-Based Tests

The example includes proptest-based property tests:

```rust
proptest! {
    #[test]
    fn test_order_total_calculation(items in prop::collection::vec(order_item_strategy(), 1..10)) {
        let total: u64 = items.iter().map(|i| i.quantity * i.unit_price_cents).sum();
        assert!(total > 0);
    }
}
```

## Configuration

### Environment Variables

```bash
# Database (postgres feature)
DATABASE_URL=postgresql://postgres:postgres@localhost:5432/composable_rust

# HTTP Server (http feature)
HOST=0.0.0.0
PORT=3000

# Logging
RUST_LOG=order_processing=debug,composable_rust_runtime=info
```

## Event Sourcing Details

### Event Stream Format

Each order has its own event stream:

```
Stream ID: order-{order_id}
Events:
  1. OrderPlaced { order_id, customer_id, items, total_cents }
  2. OrderShipped { order_id, tracking_number }
```

### State Reconstruction

State is reconstructed from events on startup:

```rust
async fn load_state(event_store: &impl EventStore) -> OrderState {
    let mut state = OrderState::default();

    // Load all order streams
    for stream_id in event_store.list_streams().await? {
        let events = event_store.load_events(&stream_id, None).await?;

        for event_data in events {
            let event: OrderEvent = deserialize(&event_data)?;
            apply_event(&mut state, event);
        }
    }

    state
}
```

### Optimistic Concurrency

Events can use version checks to prevent concurrent modifications:

```rust
Effect::AppendEvents {
    stream_id: StreamId::new(&format!("order-{}", order_id)),
    events: vec![/* ... */],
    expected_version: Some(current_version), // Fail if version doesn't match
}
```

## Design Patterns

### Request-Response Pattern

HTTP handlers use `send_and_wait_for()` for synchronous responses:

```rust
async fn place_order_handler(
    State(store): State<Arc<Store</* ... */>>>,
    Json(request): Json<PlaceOrderRequest>,
) -> Result<Json<PlaceOrderResponse>, AppError> {
    let result = store
        .send_and_wait_for(
            OrderAction::PlaceOrder { /* ... */ },
            |a| matches!(a, OrderAction::OrderPlaced { .. }),
            Duration::from_secs(5),
        )
        .await?;

    match result {
        OrderAction::OrderPlaced { order_id, .. } => {
            Ok(Json(PlaceOrderResponse { order_id }))
        }
        _ => Err(AppError::internal("Unexpected action")),
    }
}
```

### Event Broadcasting

All actions are automatically broadcast to WebSocket clients:

```rust
// In router.rs
let app = Router::new()
    .route("/ws", get(websocket::handle::<OrderState, OrderAction, OrderEnv, OrderReducer>))
    .with_state(store);
```

The Store's `subscribe_actions()` method provides a broadcast channel that the WebSocket handler uses.

## Deployment

### Docker

```dockerfile
FROM rust:1.85 as builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin order-processing --features postgres,http

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libpq5
COPY --from=builder /app/target/release/order-processing /usr/local/bin/
CMD ["order-processing"]
```

### Docker Compose

```yaml
services:
  postgres:
    image: postgres:16
    environment:
      POSTGRES_PASSWORD: postgres
      POSTGRES_DB: composable_rust
    ports:
      - "5432:5432"

  order-processing:
    build: .
    environment:
      DATABASE_URL: postgresql://postgres:postgres@postgres:5432/composable_rust
      RUST_LOG: info
    ports:
      - "3000:3000"
    depends_on:
      - postgres
```

## Performance

- **Event append**: ~1ms (PostgreSQL)
- **State reconstruction**: ~100μs per event
- **HTTP request**: ~10ms end-to-end
- **WebSocket broadcast**: ~1ms per client

## Further Reading

- [Getting Started Guide](../../docs/getting-started.md) - Framework tutorial
- [Event Sourcing Guide](../../docs/projections.md) - Event sourcing patterns
- [WebSocket Guide](../../docs/websocket.md) - Real-time communication
- [Cookbook](../../docs/cookbook.md) - Common patterns and recipes

## License

MIT OR Apache-2.0
