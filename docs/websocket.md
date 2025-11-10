# WebSocket Real-Time Communication

This guide covers how to add real-time bidirectional communication to your composable-rust applications using WebSockets.

## Table of Contents

- [Overview](#overview)
- [Architecture](#architecture)
- [Adding WebSocket Support](#adding-websocket-support)
- [Message Protocol](#message-protocol)
- [Client Integration](#client-integration)
- [Use Cases](#use-cases)
- [Testing](#testing)
- [Troubleshooting](#troubleshooting)

## Overview

The `composable-rust-web` crate provides production-ready WebSocket support that integrates seamlessly with the Store's action broadcasting system.

### Key Features

- **Bidirectional Communication**: Send commands to server, receive real-time events
- **Type-Safe**: Same action types as HTTP endpoints
- **Zero Additional State**: Leverages Store's built-in action broadcasting
- **Multiple Clients**: Each client gets independent event stream
- **Consistent Logic**: Same reducers power HTTP and WebSocket
- **Scalable**: Non-blocking async architecture

### When to Use WebSockets

WebSockets are ideal for:

- **Real-time dashboards** - Live order status, metrics, analytics
- **Notifications** - Instant alerts when events occur
- **Collaborative features** - Multiple users seeing live updates
- **Progress tracking** - Long-running operations with status updates
- **Chat/messaging** - Instant message delivery

Use HTTP for:

- **Simple queries** - One-time data fetches
- **File uploads** - Large payloads
- **RESTful APIs** - Public APIs following REST conventions
- **Cacheable requests** - Benefit from HTTP caching

## Architecture

### How It Works

```text
┌─────────────────────────────────────────────────────────────┐
│                       Client (Browser)                       │
│  WebSocket.send() ─────────────────> Receive events         │
└────────────┬───────────────────────────────┬────────────────┘
             │                               │
             │ Command (JSON)                │ Event (JSON)
             ▼                               ▲
┌─────────────────────────────────────────────────────────────┐
│                  WebSocket Handler (Axum)                    │
│  - Parse Command → Action                                    │
│  - Subscribe to Store broadcasts                             │
│  - Stream Events → Client                                    │
└────────────┬───────────────────────────────┬────────────────┘
             │                               │
             │ Action                        │ Action Broadcast
             ▼                               ▲
┌─────────────────────────────────────────────────────────────┐
│                     Store (Runtime)                          │
│  - Dispatch actions through reducers                         │
│  - Broadcast all actions from effects                        │
│  - Multiple WebSocket clients subscribe independently        │
└─────────────────────────────────────────────────────────────┘
```

### Store Broadcasting

The Store includes built-in action broadcasting:

```rust
// Subscribe to all actions produced by effects
let mut action_rx = store.subscribe_actions();

// Receive actions in real-time
while let Ok(action) = action_rx.recv().await {
    // Send to WebSocket client
    send_to_client(action).await;
}
```

**Key Point**: Only actions produced by effects are broadcast, not the initial commands. This prevents feedback loops.

## Adding WebSocket Support

### Step 1: Add WebSocket Handler to Router

```rust
use axum::{Router, routing::get};
use composable_rust_web::handlers::websocket;

pub fn create_router(store: Arc<Store<S, A, E, R>>) -> Router {
    Router::new()
        // HTTP endpoints
        .route("/orders", post(handlers::place_order))
        .route("/orders/:id", get(handlers::get_order))
        // WebSocket endpoint
        .route("/ws", get(websocket::handle::<S, A, E, R>))
        .with_state(store)
}
```

That's it! The generic `websocket::handle` function works with any Store type.

### Step 2: Enable WebSocket Feature (Already Enabled)

The `ws` feature is already enabled in `composable-rust-web`:

```toml
[dependencies]
composable-rust-web = "0.1"  # WebSocket support included
```

### Complete Example

From `examples/order-processing/src/router.rs`:

```rust
use composable_rust_web::handlers::websocket;
use axum::{Router, routing::{get, post}};

pub fn order_router(
    store: Arc<Store<OrderState, OrderAction, OrderEnvironment, OrderReducer>>,
) -> Router {
    Router::new()
        // HTTP endpoints
        .route("/orders", post(handlers::place_order))
        .route("/orders/:id", get(handlers::get_order))
        .route("/orders/:id/cancel", post(handlers::cancel_order))
        .route("/orders/:id/ship", post(handlers::ship_order))
        // WebSocket endpoint for real-time events
        .route(
            "/ws",
            get(websocket::handle::<OrderState, OrderAction, OrderEnvironment, OrderReducer>),
        )
        .with_state(store)
}
```

## Message Protocol

WebSocket communication uses JSON messages with a tagged envelope format.

### Message Types

All messages include a `type` field:

| Type      | Direction       | Purpose                |
|-----------|-----------------|------------------------|
| `command` | Client → Server | Execute an action      |
| `event`   | Server → Client | Broadcast an action    |
| `error`   | Server → Client | Error occurred         |
| `ping`    | Bidirectional   | Keep connection alive  |
| `pong`    | Bidirectional   | Respond to ping        |

### Command Messages (Client → Server)

Send commands to execute actions:

```json
{
  "type": "command",
  "action": {
    "PlaceOrder": {
      "customer_id": "cust-123",
      "items": [{
        "product_id": "prod-1",
        "name": "Widget",
        "quantity": 2,
        "unit_price_cents": 1999
      }]
    }
  }
}
```

The `action` field matches your Action enum's serde serialization.

### Event Messages (Server → Client)

Receive real-time events:

```json
{
  "type": "event",
  "action": {
    "OrderPlaced": {
      "order_id": "ord-456",
      "customer_id": "cust-123",
      "status": "pending",
      "total_cents": 3998
    }
  }
}
```

**Important**: You'll receive events for ALL actions from effects, not just the ones you triggered.

### Error Messages (Server → Client)

Errors that occur during processing:

```json
{
  "type": "error",
  "message": "Invalid action format: missing required field 'customer_id'"
}
```

### Keep-Alive (Ping/Pong)

WebSocket connections are kept alive automatically by Axum. You can also manually ping:

```json
{
  "type": "ping"
}
```

Server responds with:

```json
{
  "type": "pong"
}
```

## Client Integration

### JavaScript/TypeScript Client

Basic WebSocket client:

```javascript
// Connect to WebSocket
const ws = new WebSocket('ws://localhost:3000/api/v1/ws');

// Handle connection open
ws.onopen = () => {
  console.log('Connected to server');

  // Send a command
  const command = {
    type: 'command',
    action: {
      PlaceOrder: {
        customer_id: 'cust-123',
        items: [{
          product_id: 'prod-1',
          name: 'Widget',
          quantity: 2,
          unit_price_cents: 1999
        }]
      }
    }
  };

  ws.send(JSON.stringify(command));
};

// Handle incoming events
ws.onmessage = (event) => {
  const message = JSON.parse(event.data);

  switch (message.type) {
    case 'event':
      console.log('Event received:', message.action);
      handleAction(message.action);
      break;

    case 'error':
      console.error('Error:', message.message);
      break;

    case 'pong':
      console.log('Pong received');
      break;
  }
};

// Handle errors
ws.onerror = (error) => {
  console.error('WebSocket error:', error);
};

// Handle connection close
ws.onclose = () => {
  console.log('Disconnected from server');
  // Implement reconnection logic here
};

// Helper function to handle different action types
function handleAction(action) {
  if (action.OrderPlaced) {
    updateOrderList(action.OrderPlaced);
  } else if (action.OrderShipped) {
    showNotification(`Order ${action.OrderShipped.order_id} shipped!`);
  } else if (action.OrderCancelled) {
    removeOrder(action.OrderCancelled.order_id);
  }
}
```

### React Hook Example

```typescript
import { useEffect, useState, useCallback } from 'react';

interface OrderAction {
  OrderPlaced?: {
    order_id: string;
    customer_id: string;
    status: string;
    total_cents: number;
  };
  OrderShipped?: {
    order_id: string;
    tracking_number: string;
  };
  OrderCancelled?: {
    order_id: string;
    reason: string;
  };
}

interface WsMessage {
  type: 'event' | 'error';
  action?: OrderAction;
  message?: string;
}

export function useOrderWebSocket(url: string) {
  const [ws, setWs] = useState<WebSocket | null>(null);
  const [events, setEvents] = useState<OrderAction[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const socket = new WebSocket(url);

    socket.onopen = () => {
      console.log('WebSocket connected');
      setError(null);
    };

    socket.onmessage = (event) => {
      const message: WsMessage = JSON.parse(event.data);

      if (message.type === 'event' && message.action) {
        setEvents((prev) => [...prev, message.action]);
      } else if (message.type === 'error') {
        setError(message.message || 'Unknown error');
      }
    };

    socket.onerror = () => {
      setError('WebSocket connection error');
    };

    socket.onclose = () => {
      console.log('WebSocket disconnected');
      // Implement reconnection with exponential backoff
      setTimeout(() => {
        setWs(null);
      }, 5000);
    };

    setWs(socket);

    return () => {
      socket.close();
    };
  }, [url]);

  const sendCommand = useCallback((action: OrderAction) => {
    if (ws && ws.readyState === WebSocket.OPEN) {
      const command = {
        type: 'command',
        action,
      };
      ws.send(JSON.stringify(command));
    }
  }, [ws]);

  return { events, error, sendCommand };
}

// Usage in component
function OrderDashboard() {
  const { events, error, sendCommand } = useOrderWebSocket('ws://localhost:3000/api/v1/ws');

  const placeOrder = () => {
    sendCommand({
      PlaceOrder: {
        customer_id: 'cust-123',
        items: [/* ... */]
      }
    });
  };

  return (
    <div>
      {error && <div className="error">{error}</div>}
      <button onClick={placeOrder}>Place Order</button>
      <div>
        <h2>Real-time Events:</h2>
        {events.map((event, i) => (
          <div key={i}>{JSON.stringify(event)}</div>
        ))}
      </div>
    </div>
  );
}
```

### Command-Line Testing with `wscat`

Install wscat:

```bash
npm install -g wscat
```

Connect and send commands:

```bash
# Connect
wscat -c ws://localhost:3000/api/v1/ws

# You'll see a prompt: >

# Send a command (paste this JSON)
> {"type":"command","action":{"PlaceOrder":{"customer_id":"cust-123","items":[{"product_id":"prod-1","name":"Widget","quantity":1,"unit_price_cents":1000}]}}}

# You'll receive events in real-time:
< {"type":"event","action":{"OrderPlaced":{"order_id":"ord-456","customer_id":"cust-123","status":"pending","total_cents":1000}}}
```

## Use Cases

### 1. Real-Time Order Dashboard

Display live order status updates:

```rust
// Server automatically broadcasts order events
smallvec![AuthEffect::PublishEvent(OrderPlaced { ... })]

// All connected dashboards receive the event instantly
```

Client shows:
- Orders being placed in real-time
- Status changes (pending → processing → shipped)
- Cancellations
- Inventory updates

### 2. Live Notifications

Send instant notifications to users:

```rust
// Reducer emits notification event
smallvec![AuthEffect::PublishEvent(NotificationCreated {
    user_id,
    message: "Your order has shipped!",
    priority: Priority::High,
})]
```

Client:
- Shows toast notification
- Plays sound
- Updates notification badge

### 3. Collaborative Editing

Multiple users see each other's changes:

```rust
// User A makes a change
DocumentUpdated { doc_id, changes, user_id }

// User B receives event immediately
// User C receives event immediately
```

### 4. Progress Tracking

Long-running operations with status updates:

```rust
// Initial command
BatchProcessStarted { batch_id, total_items: 1000 }

// Progress updates
BatchProcessProgress { batch_id, processed: 250, total: 1000 }
BatchProcessProgress { batch_id, processed: 500, total: 1000 }
BatchProcessProgress { batch_id, processed: 750, total: 1000 }

// Completion
BatchProcessCompleted { batch_id, succeeded: 990, failed: 10 }
```

Client shows progress bar updating in real-time.

### 5. Live Metrics & Analytics

Real-time system metrics:

```rust
MetricsUpdated {
    timestamp,
    active_users: 1234,
    requests_per_second: 567,
    error_rate: 0.01,
}
```

Client renders live graphs and dashboards.

## Testing

### Unit Testing (No WebSocket)

Test the underlying reducer logic:

```rust
#[tokio::test]
async fn test_order_placement() {
    let store = Store::new(/* ... */);

    // Test reducer directly
    let effects = reducer.reduce(
        &mut state,
        OrderAction::PlaceOrder { /* ... */ },
        &env,
    );

    assert!(matches!(effects[0], OrderEffect::PublishEvent(_)));
}
```

### Integration Testing with Real WebSocket

```rust
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::test]
async fn test_websocket_order_flow() {
    // Start server
    let server = spawn_test_server();

    // Connect WebSocket client
    let (mut ws, _) = connect_async("ws://localhost:3000/ws")
        .await
        .expect("Failed to connect");

    // Send command
    let command = serde_json::json!({
        "type": "command",
        "action": {
            "PlaceOrder": {
                "customer_id": "test",
                "items": [/*...*/]
            }
        }
    });

    ws.send(Message::Text(command.to_string())).await.unwrap();

    // Receive event
    let msg = ws.next().await.unwrap().unwrap();
    let event: WsMessage = serde_json::from_str(&msg.to_string()).unwrap();

    assert_eq!(event.type_field, "event");
    assert!(matches!(event.action, Some(OrderAction::OrderPlaced { .. })));
}
```

### Load Testing

Test multiple concurrent WebSocket connections:

```bash
# Using websocat for load testing
for i in {1..100}; do
  websocat ws://localhost:3000/ws &
done
```

Or use specialized tools like:
- [Artillery](https://artillery.io/) - Load testing with WebSocket support
- [k6](https://k6.io/) - Modern load testing tool

## Troubleshooting

### Connection Refused

**Problem**: "Connection refused" when connecting to WebSocket

**Solutions**:
1. Verify server is running: `curl http://localhost:3000/health`
2. Check port is correct: WebSocket uses same port as HTTP
3. Check firewall rules

### WebSocket Upgrade Failed

**Problem**: "Failed to upgrade connection"

**Solutions**:
1. Ensure using `ws://` (not `http://`) or `wss://` (not `https://`)
2. Check reverse proxy configuration (nginx/Caddy) - must support WebSocket upgrade
3. Verify `Connection: Upgrade` and `Upgrade: websocket` headers

### Messages Not Received

**Problem**: Client doesn't receive events

**Solutions**:
1. Check client is subscribed before events occur
2. Verify action serialization matches expected format
3. Enable debug logging: `RUST_LOG=composable_rust_web=debug`
4. Check for JSON serialization errors in server logs

### Connection Drops

**Problem**: WebSocket disconnects after a few minutes

**Solutions**:
1. Implement ping/pong keep-alive (already built-in)
2. Add reconnection logic in client
3. Check reverse proxy timeout settings (nginx `proxy_read_timeout`)
4. Monitor server resource usage (memory, CPU)

### CORS Issues

**Problem**: WebSocket connection fails with CORS error

**Solutions**:

```rust
use tower_http::cors::{CorsLayer, Any};

let app = Router::new()
    .route("/ws", get(websocket::handle::<_, _, _, _>))
    .layer(CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any));
```

For production, use specific origins:

```rust
.allow_origin("https://app.example.com".parse::<HeaderValue>().unwrap())
```

### Reconnection Strategy

Implement exponential backoff:

```javascript
class WebSocketClient {
  constructor(url) {
    this.url = url;
    this.reconnectDelay = 1000;  // Start with 1 second
    this.maxReconnectDelay = 30000;  // Max 30 seconds
    this.connect();
  }

  connect() {
    this.ws = new WebSocket(this.url);

    this.ws.onopen = () => {
      console.log('Connected');
      this.reconnectDelay = 1000;  // Reset delay on successful connection
    };

    this.ws.onclose = () => {
      console.log('Disconnected, reconnecting in', this.reconnectDelay, 'ms');
      setTimeout(() => this.connect(), this.reconnectDelay);

      // Exponential backoff
      this.reconnectDelay = Math.min(this.reconnectDelay * 2, this.maxReconnectDelay);
    };

    this.ws.onmessage = (event) => {
      // Handle messages
    };
  }

  send(data) {
    if (this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(data));
    } else {
      console.warn('WebSocket not connected, message queued');
      // Implement message queue here
    }
  }
}
```

## Best Practices

### Server-Side

1. **Rate Limiting**: Protect against WebSocket abuse
2. **Authentication**: Verify user identity on connection
3. **Message Validation**: Validate all incoming commands
4. **Resource Limits**: Limit connections per user/IP
5. **Graceful Shutdown**: Close connections cleanly on server shutdown

### Client-Side

1. **Reconnection**: Always implement reconnection with backoff
2. **Message Queue**: Queue messages when disconnected
3. **Error Handling**: Handle all error cases
4. **Heartbeat**: Send periodic pings to detect dead connections
5. **Memory Management**: Clean up event listeners on unmount

### Security

1. **Use WSS**: Always use `wss://` in production (encrypted)
2. **Authenticate**: Verify user on connection (use session cookies or JWT)
3. **Authorize**: Check permissions before dispatching actions
4. **Validate**: Validate all incoming messages
5. **Rate Limit**: Prevent abuse with rate limiting

## Performance

### Scalability

- **Connection Pooling**: Each WebSocket is a long-lived TCP connection
- **Memory**: ~1-5 KB per connection
- **CPU**: Minimal overhead with Tokio's async runtime
- **Broadcast**: Efficient with `tokio::sync::broadcast` channel

### Benchmarks

On a modern server (4 cores, 8GB RAM):

- **10,000 concurrent connections**: ~50MB RAM
- **100 messages/sec**: ~10% CPU
- **1,000 messages/sec**: ~50% CPU

### Optimization Tips

1. **Filter Events**: Only send relevant events to each client
2. **Compress**: Use per-message deflate for large messages
3. **Batch**: Group multiple small events into one message
4. **Throttle**: Limit broadcast frequency for high-frequency events

## Next Steps

- **See [Getting Started](./getting-started.md)** for complete application setup
- **See [Saga Patterns](./saga-patterns.md)** for complex workflows
- **See [Observability](./observability.md)** for monitoring WebSocket connections

## Examples

Complete working examples:

- `examples/order-processing/` - WebSocket-enabled order system
- `web/src/handlers/websocket.rs` - WebSocket handler implementation
- `runtime/tests/broadcasting.rs` - Action broadcasting tests
