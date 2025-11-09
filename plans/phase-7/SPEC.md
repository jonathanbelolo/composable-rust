# Phase 7 Specification: Axum Web Framework Integration

**Version**: 2.0
**Status**: Design Complete
**Author**: Composable Rust Team
**Date**: 2024-11-09

---

## Table of Contents

1. [Overview](#overview)
2. [Architecture Principles](#architecture-principles)
3. [Store Action Broadcasting](#store-action-broadcasting)
4. [Integration Patterns](#integration-patterns)
5. [Correlation IDs](#correlation-ids)
6. [WebSocket Streaming](#websocket-streaming)
7. [Generic Web Utilities](#generic-web-utilities)
8. [Error Handling](#error-handling)
9. [Performance Considerations](#performance-considerations)
10. [Testing Strategy](#testing-strategy)

---

## Overview

### Purpose

Phase 7 integrates the composable-rust architecture with HTTP servers by adding **action observation** to the Store and providing **generic web utilities** for Axum. This enables request-response patterns and real-time event streaming while maintaining the functional core principles.

### Key Innovation: Action Broadcasting

The Store gains the ability to **broadcast all actions** it processes, enabling:
- **Request-Response**: HTTP handlers wait for specific terminal actions
- **Event Streaming**: WebSockets stream actions to connected clients
- **Observability**: External systems monitor action flow
- **Zero coupling**: Domain logic never knows about HTTP

### Goals

1. ✅ **Generic Design**: Works with ANY domain (not just auth)
2. ✅ **Production Ready**: Battle-tested patterns for real systems
3. ✅ **Framework Agnostic**: Patterns apply to Actix, Rocket, Warp, etc.
4. ✅ **Functional Purity**: No HTTP coupling in reducers
5. ✅ **Performance**: Lock-free broadcasting with minimal overhead

### Non-Goals

- ❌ Build a batteries-included web framework
- ❌ Abstract over all web frameworks (focus on Axum as reference)
- ❌ Provide admin UI or dashboard
- ❌ Support GraphQL, gRPC (future phases)

---

## Architecture Principles

### 1. Functional Core, Imperative Shell

The architecture maintains strict separation:

```
┌─────────────────────────────────────────────────────┐
│         Imperative Shell (Axum)                     │
│  ┌──────────────────────────────────────────────┐   │
│  │ HTTP Request → Action                        │   │  ← Parse, validate
│  │ Action → Store → Reducer → Effects           │   │  ← Dispatch
│  │ Wait for terminal action (broadcast)         │   │  ← Observe
│  │ Terminal Action → HTTP Response              │   │  ← Serialize
│  └──────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────┤
│         Functional Core                             │
│  ┌──────────────────────────────────────────────┐   │
│  │ Reducer: (State, Action) → (State, Effects)  │   │  ← Pure logic
│  │ Effects execute → produce more Actions       │   │  ← Side effects
│  │ Actions broadcast to observers               │   │  ← Observable
│  └──────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────┘
```

**Critical Insight**: The reducer never knows it's embedded in an HTTP server. The HTTP layer observes actions and maps them to responses based on domain semantics.

### 2. Request Flow with Action Observation

```rust
// HTTP handler
async fn launch_missiles(
    State(store): State<Arc<MissileStore>>,
    Json(req): Json<LaunchRequest>,
) -> Result<Response, AppError> {
    // 1. Build domain action
    let action = MissileAction::InitiateLaunch {
        correlation_id: Uuid::new_v4(),
        target: req.target,
        authorization: req.auth_code,
    };

    // 2. Send and wait for terminal action
    let result = store.send_and_wait_for(
        action,
        |a| matches!(a,
            MissileAction::MissilesLaunched { .. } |
            MissileAction::LaunchAborted { .. } |
            MissileAction::LaunchFailed { .. }
        ),
        Duration::from_secs(30),
    ).await?;

    // 3. Map domain action → HTTP response (imperative shell's job)
    match result {
        MissileAction::MissilesLaunched { missile_ids, .. } => {
            Ok((StatusCode::OK, Json(json!({
                "status": "launched",
                "missile_ids": missile_ids
            }))).into_response())
        }
        MissileAction::LaunchAborted { reason, .. } => {
            Ok((StatusCode::CONFLICT, Json(json!({
                "error": "launch_aborted",
                "reason": reason
            }))).into_response())
        }
        MissileAction::LaunchFailed { error, .. } => {
            Ok((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
                "error": "launch_failed",
                "details": error
            }))).into_response())
        }
        _ => unreachable!("Predicate ensures only terminal actions match"),
    }
}
```

**Key Points**:
- Reducer knows only: `MissileAction::InitiateLaunch`, `MissilesLaunched`, etc.
- HTTP handler knows: "200 OK", "409 Conflict", JSON serialization
- Clean separation maintained

### 3. The Feedback Loop with Broadcasting

```
HTTP Request
    ↓
┌───────────────────────────────────────┐
│ Action: InitiateLaunch                │
└───────────────┬───────────────────────┘
                ↓
┌───────────────────────────────────────┐
│ Reducer → (State, Effects)            │
└───────────────┬───────────────────────┘
                ↓
┌───────────────────────────────────────┐
│ Effect::Future executes saga          │
│   - Checks authorization              │
│   - Coordinates with other systems    │
│   - Eventually produces action:       │
│     MissilesLaunched                  │
└───────────────┬───────────────────────┘
                ↓
┌───────────────────────────────────────┐
│ Action broadcast to ALL observers:    │
│   - HTTP handler (waiting)     ✓      │
│   - WebSocket clients          ✓      │
│   - Metrics/logging            ✓      │
└───────────────┬───────────────────────┘
                ↓
┌───────────────────────────────────────┐
│ Action auto-fed back to Store         │
│ (existing feedback loop)               │
└───────────────────────────────────────┘
                ↓
            (Cycle continues)
```

**Dual Purpose of Broadcasting**:
1. **Observation**: External systems watch actions
2. **Feedback**: Actions loop back to reducer (unchanged from current architecture)

---

## Store Action Broadcasting

### Design

Add action broadcasting to the Store using `tokio::sync::broadcast`:

```rust
// In runtime/src/lib.rs
pub struct Store<S, A, E, R>
where
    R: Reducer<State = S, Action = A, Environment = E>,
{
    state: Arc<RwLock<S>>,
    reducer: R,
    environment: E,
    retry_policy: RetryPolicy,
    dlq: DeadLetterQueue<String>,
    shutdown: Arc<AtomicBool>,
    pending_effects: Arc<AtomicUsize>,

    // NEW: Action broadcasting for observation
    action_broadcast: broadcast::Sender<A>,
}
```

### Implementation Details

**1. Broadcast on every action**

Modify effect execution in `execute_effect_internal`:

```rust
// Effect::Future execution (around line 1820)
if let Some(action) = fut.await {
    // Broadcast to observers (HTTP handlers, WebSockets, metrics)
    let _ = self.action_broadcast.send(action.clone());

    // Auto-feedback (existing behavior)
    let _ = store.send(action).await;
}
```

**2. Add send_and_wait_for method**

```rust
impl<S, A, E, R> Store<S, A, E, R>
where
    R: Reducer<State = S, Action = A, Environment = E> + Send + Sync + 'static,
    A: Clone + Send + 'static,
    S: Send + Sync + 'static,
    E: Send + Sync + 'static,
{
    /// Send an action and wait for a matching result action.
    ///
    /// This method is designed for request-response patterns (HTTP, RPC).
    /// It subscribes to the action broadcast, sends the initial action,
    /// then waits for an action matching the predicate.
    ///
    /// # Arguments
    ///
    /// - `action`: The initial action to send
    /// - `predicate`: Function to test if an action is the terminal result
    /// - `timeout`: Maximum time to wait
    ///
    /// # Returns
    ///
    /// The first action matching the predicate, or timeout error.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let result = store.send_and_wait_for(
    ///     OrderAction::PlaceOrder { items, customer_id },
    ///     |a| matches!(a, OrderAction::OrderPlaced { .. } | OrderAction::OrderFailed { .. }),
    ///     Duration::from_secs(10),
    /// ).await?;
    ///
    /// match result {
    ///     OrderAction::OrderPlaced { order_id } => { /* success */ },
    ///     OrderAction::OrderFailed { reason } => { /* failure */ },
    ///     _ => unreachable!(),
    /// }
    /// ```
    pub async fn send_and_wait_for<F>(
        &self,
        action: A,
        predicate: F,
        timeout: Duration,
    ) -> Result<A, StoreError>
    where
        F: Fn(&A) -> bool,
    {
        // Subscribe BEFORE sending to avoid race condition
        let mut rx = self.action_broadcast.subscribe();

        // Send the initial action
        self.send(action).await?;

        // Wait for matching action with timeout
        tokio::time::timeout(timeout, async {
            loop {
                match rx.recv().await {
                    Ok(action) if predicate(&action) => return Ok(action),
                    Ok(_) => continue,  // Not the action we want, keep waiting
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        // Slow consumer, some actions were dropped
                        // Continue waiting - if terminal action was dropped, timeout will catch it
                        tracing::warn!(skipped, "Action observer lagged, {} actions skipped", skipped);
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(StoreError::ChannelClosed);
                    }
                }
            }
        })
        .await
        .map_err(|_| StoreError::Timeout)?
    }

    /// Subscribe to all actions from this store.
    ///
    /// This method is designed for event streaming (WebSockets, SSE).
    /// Returns a receiver that gets a clone of every action processed.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut rx = store.subscribe_actions();
    /// while let Ok(action) = rx.recv().await {
    ///     // Stream to WebSocket client
    ///     ws.send(serde_json::to_string(&action)?).await?;
    /// }
    /// ```
    pub fn subscribe_actions(&self) -> broadcast::Receiver<A> {
        self.action_broadcast.subscribe()
    }
}
```

**3. Configure capacity**

```rust
impl<S, A, E, R> Store<S, A, E, R> {
    /// Create a new store with custom action broadcast capacity.
    ///
    /// Default capacity is 16. Increase for high-throughput scenarios
    /// with many slow observers.
    pub fn with_broadcast_capacity(
        initial_state: S,
        reducer: R,
        environment: E,
        capacity: usize,
    ) -> Self {
        let (action_broadcast, _) = broadcast::channel(capacity);

        Self {
            state: Arc::new(RwLock::new(initial_state)),
            reducer,
            environment,
            retry_policy: RetryPolicy::default(),
            dlq: DeadLetterQueue::default(),
            shutdown: Arc::new(AtomicBool::new(false)),
            pending_effects: Arc::new(AtomicUsize::new(0)),
            action_broadcast,
        }
    }
}
```

### Performance Characteristics

- **Broadcast overhead**: ~100ns per action (lock-free atomic operations)
- **Memory overhead**:
  - Per Store: ~24 bytes (one Sender)
  - Per observer: ~40 bytes (one Receiver)
  - Channel buffer: capacity × sizeof(Action) (default: 16 × ~256 bytes = 4KB)
- **Lagging behavior**: Slow observers skip old actions, continue receiving new ones
- **Zero observers**: No overhead when no one is listening (optimized fast path)

### Error Handling

```rust
pub enum StoreError {
    // ... existing variants

    /// Timeout waiting for terminal action
    Timeout,

    /// Action broadcast channel closed (store shutting down)
    ChannelClosed,
}
```

---

## Integration Patterns

### Pattern 1: Fire-and-Forget (202 Accepted)

For long-running operations where the client doesn't wait:

```rust
async fn start_background_job(
    State(store): State<Arc<Store>>,
    Json(req): Json<JobRequest>,
) -> Result<Response, AppError> {
    let action = JobAction::StartJob {
        job_id: Uuid::new_v4(),
        params: req.params,
    };

    // Just dispatch, don't wait
    store.send(action).await?;

    // Return immediately
    Ok((StatusCode::ACCEPTED, Json(json!({
        "status": "processing",
        "job_id": job_id,
        "poll_url": format!("/api/jobs/{}", job_id)
    }))).into_response())
}
```

**Use Cases**:
- File uploads
- Batch operations
- Async notifications

**UX**: Client polls status endpoint or receives WebSocket updates

### Pattern 2: Wait for Completion (200 OK)

For operations that must complete before responding:

```rust
async fn process_payment(
    State(store): State<Arc<Store>>,
    Json(req): Json<PaymentRequest>,
) -> Result<Response, AppError> {
    let correlation_id = Uuid::new_v4();

    let action = PaymentAction::ProcessPayment {
        correlation_id,
        amount: req.amount,
        customer_id: req.customer_id,
    };

    // Wait for terminal action (max 30s)
    let result = store.send_and_wait_for(
        action,
        |a| match a {
            PaymentAction::PaymentCompleted { correlation_id: id, .. } => id == &correlation_id,
            PaymentAction::PaymentFailed { correlation_id: id, .. } => id == &correlation_id,
            _ => false,
        },
        Duration::from_secs(30),
    ).await?;

    // Map domain action → HTTP response
    match result {
        PaymentAction::PaymentCompleted { transaction_id, .. } => {
            Ok((StatusCode::OK, Json(json!({
                "status": "completed",
                "transaction_id": transaction_id
            }))).into_response())
        }
        PaymentAction::PaymentFailed { reason, .. } => {
            Ok((StatusCode::PAYMENT_REQUIRED, Json(json!({
                "error": "payment_failed",
                "reason": reason
            }))).into_response())
        }
        _ => unreachable!("Predicate ensures only terminal actions"),
    }
}
```

**Use Cases**:
- Payment processing
- Order placement
- User registration

**Timeout Handling**: If saga takes > 30s, return 503 Service Unavailable

### Pattern 3: Query (Read-Only)

For reads, bypass the reducer and query projections directly:

```rust
async fn get_user_profile(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> Result<Json<UserProfile>, AppError> {
    // Read from projection (CQRS query side)
    let profile = state.user_projection
        .get_user(user_id)
        .await
        .ok_or_else(|| AppError::not_found("User", user_id))?;

    Ok(Json(profile))
}
```

**Use Cases**:
- Get user profile
- List orders
- Retrieve dashboard data

**Note**: Reads don't go through the reducer (CQRS separation)

### Pattern 4: Partial Wait (Acknowledged + Background)

Acknowledge immediately, but wait for first milestone:

```rust
async fn send_verification_email(
    State(store): State<Arc<Store>>,
    Json(req): Json<EmailRequest>,
) -> Result<Response, AppError> {
    let correlation_id = Uuid::new_v4();

    let action = EmailAction::SendVerification {
        correlation_id,
        email: req.email,
    };

    // Wait for "email accepted by SMTP" (not delivered)
    let result = store.send_and_wait_for(
        action,
        |a| matches!(a, EmailAction::EmailAccepted { .. } | EmailAction::EmailFailed { .. }),
        Duration::from_secs(5),
    ).await?;

    match result {
        EmailAction::EmailAccepted { .. } => {
            Ok((StatusCode::OK, Json(json!({
                "status": "sent",
                "message": "Verification email sent"
            }))).into_response())
        }
        EmailAction::EmailFailed { reason, .. } => {
            Ok((StatusCode::BAD_GATEWAY, Json(json!({
                "error": "email_failed",
                "reason": reason
            }))).into_response())
        }
        _ => unreachable!(),
    }
}
```

**Use Cases**:
- Email sending (wait for SMTP accept, not delivery)
- File processing (wait for upload, not processing)
- Multi-phase operations

---

## Correlation IDs

### Purpose

Correlation IDs enable matching responses to requests in concurrent scenarios.

### Pattern

```rust
// Add correlation_id to all command actions
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OrderAction {
    // Command with correlation
    PlaceOrder {
        correlation_id: Uuid,
        customer_id: CustomerId,
        items: Vec<LineItem>,
    },

    // Event includes correlation from originating command
    OrderPlaced {
        correlation_id: Uuid,
        order_id: OrderId,
        timestamp: DateTime<Utc>,
    },

    OrderFailed {
        correlation_id: Uuid,
        reason: String,
    },
}
```

### Predicate Filtering

```rust
// Wait for action matching correlation_id
let result = store.send_and_wait_for(
    action.clone(),
    |a| {
        // Extract correlation_id from action (pattern match)
        let action_correlation = match a {
            OrderAction::OrderPlaced { correlation_id, .. } => Some(correlation_id),
            OrderAction::OrderFailed { correlation_id, .. } => Some(correlation_id),
            _ => None,
        };

        // Match if correlation_id matches ours
        action_correlation == Some(&action.correlation_id())
    },
    timeout,
).await?;
```

### Macro Support (Phase 7.1)

```rust
// Future: Derive correlation ID support
#[derive(Action, Clone, Debug)]
#[action(correlation = "correlation_id")]
pub enum OrderAction {
    PlaceOrder {
        #[correlation]
        correlation_id: Uuid,
        customer_id: CustomerId,
    },

    OrderPlaced {
        #[correlation]
        correlation_id: Uuid,
        order_id: OrderId,
    },
}

// Auto-generates:
impl OrderAction {
    fn correlation_id(&self) -> Option<&Uuid> { ... }
}
```

---

## WebSocket Streaming

### Pattern

Stream all actions from a store to a WebSocket client:

```rust
async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(store): State<Arc<Store>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        let (mut sender, _receiver) = socket.split();

        // Subscribe to all actions from store
        let mut rx = store.subscribe_actions();

        // Stream actions as JSON
        while let Ok(action) = rx.recv().await {
            let json = serde_json::to_string(&action).unwrap();

            if sender.send(Message::Text(json)).await.is_err() {
                break;  // Client disconnected
            }
        }
    })
}
```

### Filtered Streaming

Stream only relevant actions to each client:

```rust
async fn user_events_websocket(
    ws: WebSocketUpgrade,
    State(store): State<Arc<Store>>,
    SessionGuard(session): SessionGuard,
) -> impl IntoResponse {
    let user_id = session.user_id;

    ws.on_upgrade(move |socket| async move {
        let (mut sender, _receiver) = socket.split();
        let mut rx = store.subscribe_actions();

        // Filter: only actions for this user
        while let Ok(action) = rx.recv().await {
            if action.user_id() == Some(&user_id) {
                let json = serde_json::to_string(&action).unwrap();
                let _ = sender.send(Message::Text(json)).await;
            }
        }
    })
}
```

### Client-Side Usage

```typescript
const ws = new WebSocket('wss://api.example.com/ws/events');

ws.onmessage = (event) => {
  const action = JSON.parse(event.data);

  switch (action.type) {
    case 'OrderPlaced':
      showNotification('Order placed!');
      updateOrderList(action.order_id);
      break;
    case 'PaymentCompleted':
      showNotification('Payment successful!');
      break;
    // ... handle other actions
  }
};
```

---

## Generic Web Utilities

### Scope of `web` Crate

The `web` crate provides **generic utilities** for ANY domain:

```rust
// web/src/lib.rs

/// Generic error type for web handlers
pub struct AppError { ... }

/// Generic state container
pub struct AppState<S, A, E, R> {
    pub store: Arc<Store<S, A, E, R>>,
    pub config: Config,
}

/// Generic extractors
pub mod extractors {
    /// Extract correlation ID from request headers
    pub struct CorrelationId(pub Uuid);

    /// Extract IP address from request
    pub struct ClientIp(pub IpAddr);

    /// Extract user agent
    pub struct UserAgent(pub String);
}

/// Health check endpoint (works for any store)
pub async fn health_check<S, A, E, R>(
    State(state): State<AppState<S, A, E, R>>,
) -> impl IntoResponse {
    let health = state.store.health();
    let status = match health.status {
        HealthStatus::Healthy => StatusCode::OK,
        HealthStatus::Degraded => StatusCode::OK,
        HealthStatus::Unhealthy => StatusCode::SERVICE_UNAVAILABLE,
    };
    (status, Json(health))
}
```

### Domain-Specific Handlers

Auth-specific HTTP handlers live in the `auth` crate, NOT `web`:

```rust
// auth/src/handlers/magic_link.rs

pub async fn send_magic_link(
    State(store): State<Arc<Store<AuthState, AuthAction, AuthEnvironment>>>,
    Json(req): Json<SendMagicLinkRequest>,
) -> Result<Json<SendMagicLinkResponse>, AppError> {
    let correlation_id = Uuid::new_v4();

    let action = AuthAction::SendMagicLink {
        correlation_id,
        email: req.email,
        redirect_url: req.redirect_url,
    };

    let result = store.send_and_wait_for(
        action,
        |a| matches!(a,
            AuthAction::MagicLinkSent { .. } |
            AuthAction::MagicLinkFailed { .. }
        ),
        Duration::from_secs(10),
    ).await?;

    match result {
        AuthAction::MagicLinkSent { .. } => {
            Ok(Json(SendMagicLinkResponse { success: true }))
        }
        AuthAction::MagicLinkFailed { reason, .. } => {
            Err(AppError::bad_request(reason))
        }
        _ => unreachable!(),
    }
}
```

### Router Composition

```rust
// auth/src/router.rs

pub fn auth_router<S>(
    store: Arc<Store<AuthState, AuthAction, AuthEnvironment>>,
) -> Router<S> {
    Router::new()
        .route("/magic-link/send", post(handlers::send_magic_link))
        .route("/magic-link/verify", get(handlers::verify_magic_link))
        .route("/oauth/:provider/authorize", get(handlers::oauth_authorize))
        .route("/oauth/:provider/callback", get(handlers::oauth_callback))
        .with_state(store)
}
```

```rust
// main.rs or server setup

let app = Router::new()
    .route("/health", get(web::health_check))
    .nest("/api/v1/auth", auth::router(auth_store))
    .nest("/api/v1/orders", orders::router(order_store))
    .nest("/api/v1/payments", payments::router(payment_store));
```

---

## Error Handling

### AppError Design

```rust
// web/src/error.rs

pub struct AppError {
    status: StatusCode,
    message: String,
    code: String,
    source: Option<anyhow::Error>,
}

impl AppError {
    pub fn bad_request(msg: impl Into<String>) -> Self { ... }
    pub fn unauthorized(msg: impl Into<String>) -> Self { ... }
    pub fn not_found(resource: impl Display, id: impl Display) -> Self { ... }
    pub fn timeout(msg: impl Into<String>) -> Self { ... }
    pub fn internal(msg: impl Into<String>) -> Self { ... }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // Log server errors
        if self.status.is_server_error() {
            tracing::error!(
                status = %self.status,
                code = %self.code,
                message = %self.message,
                error = ?self.source,
                "Internal server error"
            );
        }

        let body = json!({
            "code": self.code,
            "message": self.message,
        });

        (self.status, Json(body)).into_response()
    }
}
```

### Timeout Handling

```rust
match store.send_and_wait_for(action, predicate, timeout).await {
    Ok(result) => { /* success */ },
    Err(StoreError::Timeout) => {
        // Saga is still running, but we can't wait longer
        // Return 503 or 202 depending on use case
        Err(AppError::timeout("Operation is taking longer than expected"))
    }
    Err(e) => Err(e.into()),
}
```

---

## Performance Considerations

### 1. Action Cloning Overhead

**Impact**: Every action is cloned for broadcasting
**Mitigation**:
- Actions already required to be `Clone` (for effect feedback)
- Most actions are small (~256 bytes)
- Clone is cheap for Copy types and Rc/Arc

**Benchmark**: ~50ns per clone for typical action

### 2. Broadcast Channel Performance

**Characteristics**:
- Lock-free atomic operations
- ~100ns overhead per action
- Scales linearly with number of observers

**Capacity Tuning**:
```rust
// Default: 16 actions
let store = Store::new(state, reducer, env);

// High throughput: 256 actions
let store = Store::with_broadcast_capacity(state, reducer, env, 256);
```

### 3. Slow Observer Handling

**Scenario**: WebSocket client can't keep up with action rate

**Behavior**:
- Channel fills up (16 actions)
- New action drops oldest
- Observer gets `RecvError::Lagged(skipped)`
- Observer continues receiving newer actions

**Decision**: Acceptable. Slow clients fall behind, fast clients stay current.

### 4. Memory Overhead

**Per Store**:
- broadcast::Sender: ~24 bytes
- Default capacity: 16 × 256 bytes = 4KB

**Per Observer** (HTTP request or WebSocket):
- broadcast::Receiver: ~40 bytes

**Total** (1000 concurrent requests):
- 1000 × 40 bytes = 40KB (negligible)

### 5. Zero-Observer Optimization

**When no one is listening**:
- `broadcast::send()` returns immediately (no cloning occurs if no receivers)
- Near-zero overhead

**Benchmark**: <10ns when zero observers

---

## Testing Strategy

### 1. Unit Tests (Pure Business Logic)

Test reducers without HTTP:

```rust
#[test]
fn test_place_order_reducer() {
    let mut state = OrderState::default();
    let env = MockEnvironment::new();
    let reducer = OrderReducer;

    let effects = reducer.reduce(
        &mut state,
        OrderAction::PlaceOrder { ... },
        &env,
    );

    assert!(matches!(effects[0], Effect::Database(_)));
    assert_eq!(state.orders.len(), 1);
}
```

### 2. Integration Tests (Store + HTTP)

Test full request-response flow:

```rust
#[tokio::test]
async fn test_place_order_http() {
    let store = Arc::new(test_store());
    let app = Router::new()
        .route("/orders", post(place_order_handler))
        .with_state(store);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/orders")
                .body(json!({ "items": [...] }))
                .unwrap()
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}
```

### 3. WebSocket Tests

Test event streaming:

```rust
#[tokio::test]
async fn test_websocket_streaming() {
    let store = Arc::new(test_store());

    // Start WebSocket client
    let mut ws = WebSocketClient::connect("ws://localhost/events").await;

    // Trigger action
    store.send(OrderAction::PlaceOrder { ... }).await;

    // Assert event received
    let msg = ws.recv().await.unwrap();
    let action: OrderAction = serde_json::from_str(&msg).unwrap();
    assert!(matches!(action, OrderAction::OrderPlaced { .. }));
}
```

### 4. Timeout Tests

```rust
#[tokio::test(start_paused = true)]
async fn test_timeout_handling() {
    let store = Arc::new(slow_store());  // Saga takes 60s

    let result = store.send_and_wait_for(
        SlowAction::Start,
        |a| matches!(a, SlowAction::Complete),
        Duration::from_secs(5),  // Timeout after 5s
    ).await;

    assert!(matches!(result, Err(StoreError::Timeout)));
}
```

---

## Implementation Phases

### Sprint 7.1: Foundation (2 days)
- Add `action_broadcast` to Store
- Implement `send_and_wait_for`
- Implement `subscribe_actions`
- Add `StoreError::Timeout` and `StoreError::ChannelClosed`
- Write unit tests for broadcasting

### Sprint 7.2: Web Crate Foundation (1 day)
- Create `web` crate with generic utilities
- Implement `AppError` with `IntoResponse`
- Create generic extractors (CorrelationId, ClientIp, UserAgent)
- Implement health check endpoint

### Sprint 7.3: Auth HTTP Handlers (3 days)
- Magic Link endpoints (send, verify)
- OAuth endpoints (authorize, callback)
- Passkey endpoints (register/begin, register/complete, login/begin, login/complete)
- Session endpoints (me, refresh, logout)

### Sprint 7.4: WebSocket Streaming (2 days)
- Implement WebSocket endpoint
- Add filtered streaming
- Client reconnection handling
- Integration tests

### Sprint 7.5: Example Application (2 days)
- Complete reference app showing all patterns
- Docker Compose setup
- Load testing
- Documentation

**Total**: 10 days

---

## Success Criteria

- ✅ Store can broadcast actions to multiple observers
- ✅ HTTP handlers can wait for terminal actions
- ✅ WebSocket clients can stream actions in real-time
- ✅ Timeout handling works correctly
- ✅ Zero coupling between domain logic and HTTP
- ✅ Correlation IDs enable concurrent request handling
- ✅ Performance: <200ns overhead per action with broadcasting
- ✅ All tests passing (unit, integration, WebSocket)
- ✅ Example app demonstrates all patterns
- ✅ Documentation complete

---

## Future Enhancements (Phase 8+)

- GraphQL integration (same patterns apply)
- gRPC support (bidirectional streaming)
- Server-Sent Events (SSE) as alternative to WebSocket
- Correlation ID macros/derives
- Action replay for debugging
- Distributed tracing integration (OpenTelemetry)
