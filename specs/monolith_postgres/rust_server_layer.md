# Rust Server Layer Specification

> **Extends**: `bounded_contexts.md`, `architecture_fully_typed.md`
>
> This document specifies the thin Rust server layer that sits between
> HTTP clients and PostgreSQL, handling protocol translation, authentication,
> and side effect execution.

---

## 1. Philosophy: The Thin Shell

### 1.1 Core Principle

The Rust server is a **protocol translator and side-effect executor**, not a business logic engine:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         What Rust Owns                                   │
│                                                                          │
│  • HTTP/WebSocket protocol handling                                      │
│  • JWT signature validation (cryptography)                               │
│  • Session/cookie management                                             │
│  • Connection pooling to PostgreSQL                                      │
│  • Side effect execution (email, SMS, webhooks)                          │
│  • Observability (logging, metrics, tracing)                             │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────┐
│                       What PostgreSQL Owns                               │
│                                                                          │
│  • ALL business logic (validation, state transitions)                    │
│  • Event sourcing (global log, context events)                           │
│  • Projections (read models)                                             │
│  • Authorization (RLS, permission checks)                                │
│  • Cross-context communication (integration events)                      │
│  • Audit trail (actor_id in event metadata)                              │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### 1.2 Why This Split?

| Concern | Owner | Rationale |
|---------|-------|-----------|
| **Business logic** | PostgreSQL | Single source of truth, transactional, testable with SQL |
| **Cryptography** | Rust | Better libraries, easier key management, safer |
| **External I/O** | Rust | Async HTTP clients, retries, circuit breakers |
| **Protocol handling** | Rust | Type-safe parsing, streaming, WebSocket frames |

### 1.3 The Golden Rule

**Rust handlers should be boring.** If you're writing interesting code in a Rust handler,
it probably belongs in PostgreSQL.

```rust
// ✅ GOOD: Boring handler - just calls stored procedure
async fn submit_order(
    State(pool): State<PgPool>,
    identity: Identity,
    Json(req): Json<SubmitOrderRequest>,
) -> Result<Json<CommandResult>, ApiError> {
    let result = call_with_identity(&pool, &identity,
        "SELECT * FROM sales.submit_order($1, $2, $3)",
        (Uuid::new_v4().to_string(), req.customer_id, Json(&req.items))
    ).await?;
    Ok(Json(result))
}

// ❌ BAD: Interesting handler - business logic in Rust
async fn submit_order(...) -> Result<...> {
    // Validation should be in PostgreSQL!
    if req.items.is_empty() {
        return Err(ApiError::BadRequest("No items"));
    }
    // State checks should be in PostgreSQL!
    let existing = query_order(&pool, &req.order_id).await?;
    if existing.status != "draft" {
        return Err(ApiError::BadRequest("Order not in draft"));
    }
    // This complexity belongs in sales.submit_order()
}
```

---

## 2. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Clients                                         │
│                                                                              │
│     Browser/Mobile App              External Services                        │
│            │                              │                                  │
│            │ HTTPS                        │ HTTPS                            │
│            ▼                              ▼                                  │
├─────────────────────────────────────────────────────────────────────────────┤
│                           Rust Server                                        │
│                                                                              │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────────────────┐  │
│  │  HTTP Handlers  │  │ WebSocket Mgr   │  │  Background Task Workers    │  │
│  │                 │  │                 │  │                             │  │
│  │  /api/sales/*   │  │  /ws/events     │  │  • Outbox Poller            │  │
│  │  /api/inventory │  │                 │  │  • Email Sender             │  │
│  │  /api/shipping  │  │  Subscriptions  │  │  • SMS Sender               │  │
│  │  /api/auth/*    │  │  Broadcasting   │  │  • Webhook Executor         │  │
│  └────────┬────────┘  └────────┬────────┘  └──────────────┬──────────────┘  │
│           │                    │                          │                  │
│           │                    │                          │                  │
│  ┌────────┴────────────────────┴──────────────────────────┴──────────────┐  │
│  │                      Connection Layer                                  │  │
│  │                                                                        │  │
│  │  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────────────┐ │  │
│  │  │  Connection Pool │  │ LISTEN Connection │  │   Identity Context   │ │  │
│  │  │  (sqlx/deadpool) │  │  (dedicated)      │  │   (session vars)     │ │  │
│  │  └──────────────────┘  └──────────────────┘  └──────────────────────┘ │  │
│  └────────────────────────────────┬───────────────────────────────────────┘  │
│                                   │                                          │
└───────────────────────────────────┼──────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                            PostgreSQL                                        │
│                                                                              │
│  ┌─────────┐  ┌───────────┐  ┌───────────┐  ┌─────────┐  ┌───────────────┐  │
│  │ global  │  │   sales   │  │ inventory │  │shipping │  │  integration  │  │
│  │         │  │           │  │           │  │         │  │               │  │
│  │event_log│  │  events   │  │  events   │  │ events  │  │ domain_events │  │
│  │(JSONB)  │  │  (typed)  │  │  (typed)  │  │ (typed) │  │    outbox     │  │
│  └─────────┘  └───────────┘  └───────────┘  └─────────┘  └───────────────┘  │
│                                                                              │
│                              auth schema                                     │
│                     (users, sessions, permissions)                           │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. HTTP Server

### 3.1 Request/Response Flow

```
┌──────────────────────────────────────────────────────────────────────────┐
│                         HTTP Request Flow                                 │
│                                                                           │
│  1. Request arrives                                                       │
│     │                                                                     │
│     ▼                                                                     │
│  2. Middleware: Extract & validate JWT ─────────────────┐                │
│     │                                                    │                │
│     │ (401 if invalid)                                   │                │
│     ▼                                                    │                │
│  3. Handler: Deserialize JSON body ─────────────────────┤                │
│     │                                                    │                │
│     │ (400 if malformed)                                 │                │
│     ▼                                                    │                │
│  4. Set PostgreSQL session variables                     │                │
│     │  SET LOCAL app.user_id = '...'                     │                │
│     │  SET LOCAL app.tenant_id = '...'                   │                │
│     │  SET LOCAL app.roles = '[...]'                     │                │
│     ▼                                                    │                │
│  5. Call stored procedure                                │                │
│     │  SELECT * FROM context.command(...)                │                │
│     │                                                    │                │
│     ├─── Business error (P0001) ────────────────────────►│ 400            │
│     ├─── Auth error (P0401/P0403) ──────────────────────►│ 401/403        │
│     ├─── Conflict (P0409/23505) ────────────────────────►│ 409            │
│     ├─── Not found (P0404) ─────────────────────────────►│ 404            │
│     │                                                    │                │
│     ▼                                                    │                │
│  6. Serialize result to JSON                             │                │
│     │                                                    │                │
│     ▼                                                    │                │
│  7. Return 200 OK with body ◄────────────────────────────┘                │
│                                                                           │
└──────────────────────────────────────────────────────────────────────────┘
```

### 3.2 Endpoint Patterns

#### Command Endpoints (Write Operations)

```rust
// Pattern: POST /api/{context}/{aggregate}s/{command}
// Example: POST /api/sales/orders/submit

#[derive(Deserialize)]
struct SubmitOrderRequest {
    customer_id: String,
    items: Vec<OrderItem>,
}

#[derive(Serialize)]
struct CommandResult {
    stream_id: String,
    version: i32,
    global_event_id: i64,
}

async fn submit_order(
    State(app): State<AppState>,
    identity: Identity,
    Json(req): Json<SubmitOrderRequest>,
) -> Result<Json<CommandResult>, ApiError> {
    let result = app.db.call_with_identity(
        &identity,
        "SELECT * FROM sales.submit_order($1, $2, $3)",
        &[&Uuid::new_v4().to_string(), &req.customer_id, &Json(&req.items)],
    ).await?;

    Ok(Json(result))
}
```

#### Query Endpoints (Read Operations)

```rust
// Pattern: GET /api/{context}/{aggregate}s/{id}
// Example: GET /api/sales/orders/order-123

async fn get_order(
    State(app): State<AppState>,
    identity: Identity,
    Path(order_id): Path<String>,
) -> Result<Json<OrderProjection>, ApiError> {
    // RLS automatically filters by tenant
    let order = app.db.call_with_identity(
        &identity,
        "SELECT * FROM sales.orders_projection WHERE order_id = $1",
        &[&order_id],
    ).await?;

    order.ok_or(ApiError::NotFound)
}

// Pattern: GET /api/{context}/{aggregate}s?filters
// Example: GET /api/sales/orders?status=pending&limit=20

async fn list_orders(
    State(app): State<AppState>,
    identity: Identity,
    Query(params): Query<ListOrdersParams>,
) -> Result<Json<PaginatedResult<OrderSummary>>, ApiError> {
    let orders = app.db.call_with_identity(
        &identity,
        "SELECT * FROM sales.list_orders($1, $2, $3)",
        &[&params.status, &params.limit, &params.cursor],
    ).await?;

    Ok(Json(orders))
}
```

### 3.3 Error Handling

#### PostgreSQL Error Codes

```sql
-- Standard error codes (defined in PostgreSQL functions)
-- P0001 = Business rule violation → 400 Bad Request
-- P0401 = Authentication required → 401 Unauthorized
-- P0403 = Permission denied → 403 Forbidden
-- P0404 = Resource not found → 404 Not Found
-- P0409 = Conflict (business) → 409 Conflict
-- 23505 = Unique violation → 409 Conflict (version conflict)
-- 23503 = Foreign key violation → 400 Bad Request
```

#### Rust Error Mapping

```rust
#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized,
    Forbidden(String),
    NotFound,
    Conflict(String),
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Authentication required".into()),
            ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "Resource not found".into()),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            ApiError::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "Internal error".into()),
        };

        let body = Json(json!({ "error": message }));
        (status, body).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::Database(db) => {
                let code = db.code().map(|c| c.as_ref());
                let msg = db.message().to_string();

                match code {
                    Some("P0001") => ApiError::BadRequest(msg),
                    Some("P0401") => ApiError::Unauthorized,
                    Some("P0403") => ApiError::Forbidden(msg),
                    Some("P0404") => ApiError::NotFound,
                    Some("P0409") => ApiError::Conflict(msg),
                    Some("23505") => ApiError::Conflict("Version conflict, please retry".into()),
                    Some("23503") => ApiError::BadRequest(format!("Invalid reference: {msg}")),
                    _ => {
                        tracing::error!("Database error: {}", db);
                        ApiError::Internal
                    }
                }
            }
            sqlx::Error::RowNotFound => ApiError::NotFound,
            e => {
                tracing::error!("Unexpected database error: {}", e);
                ApiError::Internal
            }
        }
    }
}
```

### 3.4 Router Structure

```rust
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Health check (no auth)
        .route("/health", get(health_check))

        // Auth endpoints (special handling)
        .nest("/api/auth", auth_router())

        // Context-specific routes (require auth)
        .nest("/api/sales", sales_router())
        .nest("/api/inventory", inventory_router())
        .nest("/api/shipping", shipping_router())

        // WebSocket (auth via query param or first message)
        .route("/ws/events", get(ws_handler))

        // Middleware
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::new(Duration::from_secs(30)))
        .layer(CompressionLayer::new())

        .with_state(state)
}

fn sales_router() -> Router<AppState> {
    Router::new()
        // Commands
        .route("/orders/submit", post(submit_order))
        .route("/orders/:id/confirm", post(confirm_order))
        .route("/orders/:id/cancel", post(cancel_order))

        // Queries
        .route("/orders", get(list_orders))
        .route("/orders/:id", get(get_order))
        .route("/orders/:id/events", get(get_order_events))

        // Auth middleware for all routes
        .layer(middleware::from_fn(require_auth))
}
```

---

## 4. WebSocket Server

### 4.1 Connection Management

```rust
/// WebSocket connection state
struct WsConnection {
    /// Unique connection ID
    id: ConnectionId,

    /// Authenticated user identity
    identity: Identity,

    /// Subscribed streams/contexts
    subscriptions: HashSet<Subscription>,

    /// Sender for outgoing messages
    sender: mpsc::Sender<WsMessage>,
}

/// What a client can subscribe to
#[derive(Clone, Hash, Eq, PartialEq)]
enum Subscription {
    /// All events in a context
    Context(String),  // "sales", "inventory"

    /// Specific stream
    Stream(String),   // "order-123"

    /// Events matching a pattern
    Pattern { context: String, event_type: String },
}

/// WebSocket manager (shared state)
struct WsManager {
    /// Active connections by ID
    connections: DashMap<ConnectionId, WsConnection>,

    /// Broadcast channel from PostgreSQL notifications
    broadcast_rx: broadcast::Receiver<EventNotification>,
}
```

### 4.2 Subscription Protocol

```typescript
// Client → Server: Subscribe
{
    "type": "subscribe",
    "subscriptions": [
        { "context": "sales" },
        { "stream": "order-123" },
        { "context": "shipping", "event_type": "ShipmentCreated" }
    ]
}

// Server → Client: Subscription confirmed
{
    "type": "subscribed",
    "subscriptions": ["sales", "stream:order-123", "shipping:ShipmentCreated"]
}

// Client → Server: Unsubscribe
{
    "type": "unsubscribe",
    "subscriptions": ["sales"]
}

// Server → Client: Event notification
{
    "type": "event",
    "global_id": 12345,
    "context": "sales",
    "stream_id": "order-123",
    "version": 2,
    "event_type": "OrderConfirmed",
    "payload": { ... },
    "timestamp": "2024-01-15T10:30:00Z"
}

// Server → Client: Error
{
    "type": "error",
    "code": "invalid_subscription",
    "message": "Unknown context: foo"
}
```

### 4.3 PostgreSQL LISTEN Integration

```rust
/// Dedicated task for PostgreSQL LISTEN
async fn pg_notify_listener(
    pool: PgPool,
    broadcast_tx: broadcast::Sender<EventNotification>,
) {
    loop {
        match run_listener(&pool, &broadcast_tx).await {
            Ok(()) => {
                tracing::info!("LISTEN connection closed, reconnecting...");
            }
            Err(e) => {
                tracing::error!("LISTEN error: {}, reconnecting in 5s...", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn run_listener(
    pool: &PgPool,
    broadcast_tx: &broadcast::Sender<EventNotification>,
) -> Result<(), sqlx::Error> {
    // Get dedicated connection (not from pool)
    let mut conn = pool.acquire().await?;

    // Subscribe to all context notification channels
    sqlx::query("LISTEN global_events").execute(&mut *conn).await?;
    sqlx::query("LISTEN sales_events").execute(&mut *conn).await?;
    sqlx::query("LISTEN inventory_events").execute(&mut *conn).await?;
    sqlx::query("LISTEN shipping_events").execute(&mut *conn).await?;
    sqlx::query("LISTEN outbox_tasks").execute(&mut *conn).await?;

    tracing::info!("LISTEN connections established");

    // Listen for notifications
    let mut listener = conn.into_inner().into_listener();

    while let Some(notification) = listener.try_recv().await? {
        match notification.channel() {
            "outbox_tasks" => {
                // Signal outbox poller to check for new tasks
                // (handled separately)
            }
            channel => {
                // Parse event notification
                if let Ok(event) = serde_json::from_str::<EventNotification>(notification.payload()) {
                    // Broadcast to all WebSocket connections
                    // (they'll filter by subscription)
                    let _ = broadcast_tx.send(event);
                }
            }
        }
    }

    Ok(())
}
```

### 4.4 WebSocket Handler

```rust
async fn ws_handler(
    State(app): State<AppState>,
    ws: WebSocketUpgrade,
    Query(params): Query<WsAuthParams>,
) -> Result<Response, ApiError> {
    // Authenticate via query param token (or require auth message first)
    let identity = validate_ws_token(&params.token)?;

    Ok(ws.on_upgrade(move |socket| {
        handle_ws_connection(socket, identity, app.ws_manager.clone())
    }))
}

async fn handle_ws_connection(
    socket: WebSocket,
    identity: Identity,
    manager: Arc<WsManager>,
) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::channel(100);

    let conn_id = ConnectionId::new();
    let conn = WsConnection {
        id: conn_id,
        identity: identity.clone(),
        subscriptions: HashSet::new(),
        sender: tx,
    };

    manager.connections.insert(conn_id, conn);

    // Task to forward broadcast events to this connection
    let manager_clone = manager.clone();
    let forward_task = tokio::spawn(async move {
        let mut broadcast_rx = manager_clone.subscribe();

        while let Ok(event) = broadcast_rx.recv().await {
            // Check if this connection is subscribed to this event
            if let Some(conn) = manager_clone.connections.get(&conn_id) {
                if conn.is_subscribed_to(&event) {
                    let msg = serde_json::to_string(&event).unwrap();
                    if conn.sender.send(WsMessage::Text(msg)).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Task to send messages to WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Main loop: handle incoming messages
    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => {
                if let Ok(cmd) = serde_json::from_str::<WsCommand>(&text) {
                    handle_ws_command(&manager, conn_id, cmd).await;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    // Cleanup
    manager.connections.remove(&conn_id);
    forward_task.abort();
    send_task.abort();
}
```

---

## 5. Authentication

### 5.1 Authentication Strategies

```
┌─────────────────────────────────────────────────────────────────────────┐
│                     Authentication Strategies                            │
│                                                                          │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────────────┐  │
│  │   JWT Bearer    │  │  Session Cookie │  │      Magic Link         │  │
│  │                 │  │                 │  │                         │  │
│  │ Authorization:  │  │ Cookie:         │  │ GET /auth/verify?       │  │
│  │ Bearer eyJ...   │  │ session=abc123  │  │   token=xyz789          │  │
│  │                 │  │                 │  │                         │  │
│  │ Stateless       │  │ Server-side     │  │ Creates session,        │  │
│  │ Mobile/API      │  │ state           │  │ redirects with cookie   │  │
│  │ Short-lived     │  │ Web browsers    │  │ Passwordless login      │  │
│  └─────────────────┘  └─────────────────┘  └─────────────────────────┘  │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### 5.2 JWT Structure

```rust
/// JWT claims (what Rust extracts and validates)
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    /// Subject (user ID)
    sub: String,

    /// Tenant ID (for multi-tenancy)
    tenant_id: String,

    /// User roles
    roles: Vec<String>,

    /// Issued at
    iat: i64,

    /// Expires at
    exp: i64,

    /// JWT ID (for revocation)
    jti: String,
}

/// Validated identity (extracted from JWT or session)
#[derive(Clone, Debug)]
pub struct Identity {
    pub user_id: String,
    pub tenant_id: String,
    pub roles: Vec<String>,
    pub session_id: Option<String>,
}
```

### 5.3 JWT Validation (Rust)

```rust
/// Axum extractor for identity
#[async_trait]
impl<S> FromRequestParts<S> for Identity
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);

        // Try JWT from Authorization header first
        if let Some(auth) = parts.headers.get(AUTHORIZATION) {
            if let Ok(header) = auth.to_str() {
                if let Some(token) = header.strip_prefix("Bearer ") {
                    return validate_jwt(token, &app.jwt_keys);
                }
            }
        }

        // Fall back to session cookie
        let jar = CookieJar::from_headers(&parts.headers);
        if let Some(cookie) = jar.get("session") {
            return validate_session(cookie.value(), &app.db).await;
        }

        Err(ApiError::Unauthorized)
    }
}

fn validate_jwt(token: &str, keys: &JwtKeys) -> Result<Identity, ApiError> {
    let validation = Validation::new(Algorithm::RS256);

    let data = decode::<Claims>(token, &keys.decoding, &validation)
        .map_err(|e| {
            tracing::debug!("JWT validation failed: {}", e);
            ApiError::Unauthorized
        })?;

    // Check expiry (jsonwebtoken does this, but be explicit)
    let now = Utc::now().timestamp();
    if data.claims.exp < now {
        return Err(ApiError::Unauthorized);
    }

    Ok(Identity {
        user_id: data.claims.sub,
        tenant_id: data.claims.tenant_id,
        roles: data.claims.roles,
        session_id: None,
    })
}
```

### 5.4 Session Validation (PostgreSQL)

```rust
async fn validate_session(session_id: &str, db: &DbPool) -> Result<Identity, ApiError> {
    let session: Option<SessionRow> = sqlx::query_as(
        r#"
        SELECT user_id, tenant_id, roles, expires_at
        FROM auth.sessions
        WHERE session_id = $1 AND expires_at > now()
        "#
    )
    .bind(session_id)
    .fetch_optional(db)
    .await?;

    let session = session.ok_or(ApiError::Unauthorized)?;

    Ok(Identity {
        user_id: session.user_id,
        tenant_id: session.tenant_id,
        roles: session.roles,
        session_id: Some(session_id.to_string()),
    })
}
```

### 5.5 Magic Link Flow

```
┌──────────────────────────────────────────────────────────────────────────┐
│                         Magic Link Flow                                   │
│                                                                           │
│  1. User enters email                                                     │
│     POST /api/auth/magic-link { "email": "user@example.com" }            │
│                                                                           │
│  2. Rust calls PostgreSQL                                                 │
│     SELECT * FROM auth.create_magic_link($1, $2)                         │
│     → Creates token, stores in auth.magic_links                          │
│     → Returns { token, expires_at }                                       │
│                                                                           │
│  3. Rust sends email (via outbox)                                        │
│     Link: https://app.example.com/auth/verify?token=abc123               │
│                                                                           │
│  4. User clicks link                                                      │
│     GET /api/auth/verify?token=abc123                                    │
│                                                                           │
│  5. Rust calls PostgreSQL                                                 │
│     SELECT * FROM auth.verify_magic_link($1)                             │
│     → Validates token, creates session                                    │
│     → Returns { session_id, user_id, tenant_id, roles }                  │
│                                                                           │
│  6. Rust sets session cookie                                              │
│     Set-Cookie: session=xyz789; HttpOnly; Secure; SameSite=Lax           │
│                                                                           │
│  7. Redirect to app                                                       │
│     Location: /dashboard                                                  │
│                                                                           │
└──────────────────────────────────────────────────────────────────────────┘
```

```rust
async fn request_magic_link(
    State(app): State<AppState>,
    Json(req): Json<MagicLinkRequest>,
) -> Result<StatusCode, ApiError> {
    // Create magic link in PostgreSQL
    let result: MagicLinkResult = sqlx::query_as(
        "SELECT * FROM auth.create_magic_link($1, $2)"
    )
    .bind(&req.email)
    .bind(Duration::from_secs(3600).as_secs() as i64) // 1 hour expiry
    .fetch_one(&app.db)
    .await?;

    // Queue email via outbox (PostgreSQL inserts, we just return)
    // The magic link function already inserted into outbox.pending_tasks

    Ok(StatusCode::OK)
}

async fn verify_magic_link(
    State(app): State<AppState>,
    Query(params): Query<VerifyParams>,
) -> Result<Response, ApiError> {
    // Verify token and create session
    let session: SessionResult = sqlx::query_as(
        "SELECT * FROM auth.verify_magic_link($1)"
    )
    .bind(&params.token)
    .fetch_one(&app.db)
    .await?;

    // Set session cookie
    let cookie = Cookie::build(("session", session.session_id))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(Duration::from_secs(86400 * 30)) // 30 days
        .path("/")
        .build();

    // Redirect to app
    Ok((
        StatusCode::SEE_OTHER,
        [(header::SET_COOKIE, cookie.to_string())],
        [(header::LOCATION, "/dashboard")],
    ).into_response())
}
```

### 5.6 Auth Schema in PostgreSQL

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- AUTH SCHEMA
-- ═══════════════════════════════════════════════════════════════════════════

CREATE SCHEMA auth;

-- ───────────────────────────────────────────────────────────────────────────
-- Users (optional - might use external IdP)
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE auth.users (
    user_id         TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    email           TEXT UNIQUE NOT NULL,
    roles           TEXT[] NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ───────────────────────────────────────────────────────────────────────────
-- Sessions (server-side session storage)
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE auth.sessions (
    session_id      TEXT PRIMARY KEY,
    user_id         TEXT NOT NULL REFERENCES auth.users(user_id),
    tenant_id       TEXT NOT NULL,
    roles           TEXT[] NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL,
    last_active_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_sessions_user ON auth.sessions(user_id);
CREATE INDEX idx_sessions_expiry ON auth.sessions(expires_at);

-- ───────────────────────────────────────────────────────────────────────────
-- Magic Links
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE auth.magic_links (
    token           TEXT PRIMARY KEY,
    email           TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL,
    used_at         TIMESTAMPTZ
);

CREATE INDEX idx_magic_links_email ON auth.magic_links(email);

-- ───────────────────────────────────────────────────────────────────────────
-- Identity Context Helpers
-- ───────────────────────────────────────────────────────────────────────────

CREATE FUNCTION auth.current_user_id() RETURNS TEXT AS $$
    SELECT NULLIF(current_setting('app.user_id', true), '')
$$ LANGUAGE sql STABLE;

CREATE FUNCTION auth.current_tenant_id() RETURNS TEXT AS $$
    SELECT NULLIF(current_setting('app.tenant_id', true), '')
$$ LANGUAGE sql STABLE;

CREATE FUNCTION auth.current_roles() RETURNS TEXT[] AS $$
    SELECT COALESCE(
        ARRAY(SELECT jsonb_array_elements_text(
            current_setting('app.roles', true)::jsonb
        )),
        ARRAY[]::TEXT[]
    )
$$ LANGUAGE sql STABLE;

CREATE FUNCTION auth.has_role(role_name TEXT) RETURNS BOOLEAN AS $$
    SELECT role_name = ANY(auth.current_roles())
$$ LANGUAGE sql STABLE;

CREATE FUNCTION auth.require_authenticated() RETURNS VOID AS $$
BEGIN
    IF auth.current_user_id() IS NULL THEN
        RAISE EXCEPTION 'Authentication required'
            USING ERRCODE = 'P0401';
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION auth.require_role(role_name TEXT) RETURNS VOID AS $$
BEGIN
    PERFORM auth.require_authenticated();

    IF NOT auth.has_role(role_name) THEN
        RAISE EXCEPTION 'Permission denied: requires % role', role_name
            USING ERRCODE = 'P0403';
    END IF;
END;
$$ LANGUAGE plpgsql;

-- ───────────────────────────────────────────────────────────────────────────
-- Magic Link Functions
-- ───────────────────────────────────────────────────────────────────────────

CREATE TYPE auth.magic_link_result AS (
    token       TEXT,
    expires_at  TIMESTAMPTZ
);

CREATE FUNCTION auth.create_magic_link(
    p_email         TEXT,
    p_ttl_seconds   INTEGER DEFAULT 3600
) RETURNS auth.magic_link_result AS $$
DECLARE
    v_token TEXT;
    v_expires_at TIMESTAMPTZ;
    v_user auth.users;
BEGIN
    -- Check if user exists
    SELECT * INTO v_user FROM auth.users WHERE email = p_email;

    IF v_user IS NULL THEN
        RAISE EXCEPTION 'User not found'
            USING ERRCODE = 'P0404';
    END IF;

    -- Generate secure token
    v_token := encode(gen_random_bytes(32), 'base64url');
    v_expires_at := now() + (p_ttl_seconds || ' seconds')::interval;

    -- Store magic link
    INSERT INTO auth.magic_links (token, email, expires_at)
    VALUES (v_token, p_email, v_expires_at);

    -- Queue email via outbox
    INSERT INTO outbox.pending_tasks (task_type, payload, scheduled_for)
    VALUES (
        'send_email',
        jsonb_build_object(
            'template', 'magic_link',
            'to', p_email,
            'data', jsonb_build_object(
                'token', v_token,
                'expires_at', v_expires_at
            )
        ),
        now()
    );

    -- Notify outbox processor
    PERFORM pg_notify('outbox_tasks', 'new');

    RETURN (v_token, v_expires_at);
END;
$$ LANGUAGE plpgsql;

CREATE TYPE auth.session_result AS (
    session_id  TEXT,
    user_id     TEXT,
    tenant_id   TEXT,
    roles       TEXT[]
);

CREATE FUNCTION auth.verify_magic_link(
    p_token TEXT
) RETURNS auth.session_result AS $$
DECLARE
    v_link auth.magic_links;
    v_user auth.users;
    v_session_id TEXT;
BEGIN
    -- Find and validate magic link
    SELECT * INTO v_link
    FROM auth.magic_links
    WHERE token = p_token
      AND used_at IS NULL
      AND expires_at > now();

    IF v_link IS NULL THEN
        RAISE EXCEPTION 'Invalid or expired magic link'
            USING ERRCODE = 'P0401';
    END IF;

    -- Mark as used
    UPDATE auth.magic_links
    SET used_at = now()
    WHERE token = p_token;

    -- Get user
    SELECT * INTO v_user
    FROM auth.users
    WHERE email = v_link.email;

    -- Create session
    v_session_id := encode(gen_random_bytes(32), 'base64url');

    INSERT INTO auth.sessions (session_id, user_id, tenant_id, roles, expires_at)
    VALUES (
        v_session_id,
        v_user.user_id,
        v_user.tenant_id,
        v_user.roles,
        now() + interval '30 days'
    );

    RETURN (v_session_id, v_user.user_id, v_user.tenant_id, v_user.roles);
END;
$$ LANGUAGE plpgsql;
```

---

## 6. Authorization

### 6.1 Passing Identity to PostgreSQL

```rust
/// Extension trait for database operations with identity context
#[async_trait]
pub trait DbWithIdentity {
    /// Execute a query with identity context set
    async fn call_with_identity<T>(
        &self,
        identity: &Identity,
        query: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<T, sqlx::Error>
    where
        T: for<'r> FromRow<'r, PgRow> + Send + Unpin;

    /// Execute within a transaction with identity context
    async fn transaction_with_identity<T, F, Fut>(
        &self,
        identity: &Identity,
        f: F,
    ) -> Result<T, sqlx::Error>
    where
        F: FnOnce(Transaction<'_, Postgres>) -> Fut + Send,
        Fut: Future<Output = Result<T, sqlx::Error>> + Send;
}

#[async_trait]
impl DbWithIdentity for PgPool {
    async fn call_with_identity<T>(
        &self,
        identity: &Identity,
        query: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<T, sqlx::Error>
    where
        T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
    {
        let mut conn = self.acquire().await?;

        // Set session variables (LOCAL = transaction scoped)
        set_identity_context(&mut conn, identity).await?;

        // Execute the actual query
        sqlx::query_as(query)
            .bind_all(params)
            .fetch_one(&mut *conn)
            .await
    }

    async fn transaction_with_identity<T, F, Fut>(
        &self,
        identity: &Identity,
        f: F,
    ) -> Result<T, sqlx::Error>
    where
        F: FnOnce(Transaction<'_, Postgres>) -> Fut + Send,
        Fut: Future<Output = Result<T, sqlx::Error>> + Send,
    {
        let mut tx = self.begin().await?;

        // Set identity context for entire transaction
        set_identity_context(&mut *tx, identity).await?;

        // Execute user function
        let result = f(tx).await?;

        // Commit happens automatically when tx is dropped after Ok
        Ok(result)
    }
}

async fn set_identity_context(
    conn: &mut PgConnection,
    identity: &Identity,
) -> Result<(), sqlx::Error> {
    // set_config with 'true' for is_local makes it transaction-scoped
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(&identity.user_id)
        .execute(&mut *conn)
        .await?;

    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(&identity.tenant_id)
        .execute(&mut *conn)
        .await?;

    sqlx::query("SELECT set_config('app.roles', $1, true)")
        .bind(&serde_json::to_string(&identity.roles).unwrap())
        .execute(&mut *conn)
        .await?;

    Ok(())
}
```

### 6.2 Row Level Security for Tenant Isolation

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ROW LEVEL SECURITY
-- ═══════════════════════════════════════════════════════════════════════════

-- Enable RLS on projection tables
ALTER TABLE sales.orders_projection ENABLE ROW LEVEL SECURITY;
ALTER TABLE inventory.stock_projection ENABLE ROW LEVEL SECURITY;
ALTER TABLE shipping.shipments_projection ENABLE ROW LEVEL SECURITY;

-- Tenant isolation policies (SELECT, UPDATE, DELETE)
CREATE POLICY tenant_isolation_orders ON sales.orders_projection
    FOR ALL
    USING (tenant_id = auth.current_tenant_id());

CREATE POLICY tenant_isolation_stock ON inventory.stock_projection
    FOR ALL
    USING (tenant_id = auth.current_tenant_id());

CREATE POLICY tenant_isolation_shipments ON shipping.shipments_projection
    FOR ALL
    USING (tenant_id = auth.current_tenant_id());

-- Note: Event tables don't need RLS because they're accessed via functions
-- that already include authorization checks

-- For admin/support users who need cross-tenant access
CREATE POLICY admin_bypass_orders ON sales.orders_projection
    FOR SELECT
    USING (auth.has_role('admin'));

-- ═══════════════════════════════════════════════════════════════════════════
-- FUNCTION-LEVEL AUTHORIZATION
-- ═══════════════════════════════════════════════════════════════════════════

-- Example: Only sales role can submit orders
CREATE OR REPLACE FUNCTION sales.submit_order(
    p_order_id      TEXT,
    p_customer_id   TEXT,
    p_items         JSONB,
    p_timestamp     TIMESTAMPTZ DEFAULT now()
) RETURNS sales.command_result AS $$
DECLARE
    v_tenant_id TEXT := auth.current_tenant_id();
BEGIN
    -- Authorization
    PERFORM auth.require_role('sales');

    -- Business logic...
    RETURN global.append_event(
        p_stream_id := p_order_id,
        p_context := 'sales',
        p_aggregate := 'order',
        p_event_type := 'OrderSubmitted',
        p_payload := jsonb_build_object(
            'order_id', p_order_id,
            'customer_id', p_customer_id,
            'items', p_items,
            'tenant_id', v_tenant_id
        ),
        p_metadata := jsonb_build_object(
            'actor_id', auth.current_user_id(),
            'actor_type', 'user',
            'tenant_id', v_tenant_id
        ),
        p_expected_version := NULL,
        p_timestamp := p_timestamp
    );
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

-- Example: Only the order owner or admin can cancel
CREATE OR REPLACE FUNCTION sales.cancel_order(
    p_order_id      TEXT,
    p_reason        TEXT,
    p_timestamp     TIMESTAMPTZ DEFAULT now()
) RETURNS sales.command_result AS $$
DECLARE
    v_order sales.orders_projection;
    v_user_id TEXT := auth.current_user_id();
BEGIN
    PERFORM auth.require_authenticated();

    -- Get order (RLS already filters by tenant)
    SELECT * INTO v_order
    FROM sales.orders_projection
    WHERE order_id = p_order_id;

    IF v_order IS NULL THEN
        RAISE EXCEPTION 'Order not found'
            USING ERRCODE = 'P0404';
    END IF;

    -- Check ownership or admin role
    IF v_order.created_by != v_user_id AND NOT auth.has_role('admin') THEN
        RAISE EXCEPTION 'Not authorized to cancel this order'
            USING ERRCODE = 'P0403';
    END IF;

    -- Business logic...
    RETURN global.append_event(
        -- ... event details
    );
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;
```

### 6.3 Authorization Decision Matrix

| Operation | Required Auth | RLS Applied | Notes |
|-----------|--------------|-------------|-------|
| `sales.submit_order` | `sales` role | tenant via session | Function checks role |
| `sales.cancel_order` | authenticated + owner/admin | tenant via session | Function checks ownership |
| `SELECT orders_projection` | authenticated | tenant via RLS | RLS policy filters rows |
| `global.event_log` read | admin only | none | Admin function for audit |
| WebSocket subscribe | authenticated | events filtered by tenant | Rust filters events |

---

## 7. Side Effects and Background Tasks

### 7.1 The Outbox Pattern

PostgreSQL cannot reliably send emails, make HTTP calls, or interact with external services.
The **outbox pattern** provides transactional guarantees for side effects:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        Outbox Pattern Flow                               │
│                                                                          │
│  1. Command arrives at PostgreSQL                                        │
│     │                                                                    │
│     ▼                                                                    │
│  2. Business logic executes (within transaction)                         │
│     │                                                                    │
│     ├─► Events written to global.event_log                               │
│     │                                                                    │
│     └─► Side effects written to outbox.pending_tasks                     │
│         (same transaction = atomicity)                                   │
│     │                                                                    │
│     ▼                                                                    │
│  3. Transaction commits                                                  │
│     │                                                                    │
│     ├─► pg_notify('outbox_tasks', 'new')                                 │
│     │                                                                    │
│     ▼                                                                    │
│  4. Rust outbox worker receives notification                             │
│     │                                                                    │
│     ▼                                                                    │
│  5. Worker polls outbox.pending_tasks                                    │
│     │                                                                    │
│     ▼                                                                    │
│  6. Worker executes task (send email, etc.)                              │
│     │                                                                    │
│     ├─► Success: Mark task completed                                     │
│     │                                                                    │
│     └─► Failure: Increment attempts, schedule retry                      │
│         │                                                                │
│         └─► Max attempts exceeded: Move to dead letter queue             │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### 7.2 Outbox Schema

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- OUTBOX SCHEMA
-- ═══════════════════════════════════════════════════════════════════════════

CREATE SCHEMA outbox;

-- ───────────────────────────────────────────────────────────────────────────
-- Pending Tasks
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE outbox.pending_tasks (
    -- Identity
    id              BIGSERIAL PRIMARY KEY,

    -- Task definition
    task_type       TEXT NOT NULL,              -- 'send_email', 'send_sms', 'webhook'
    payload         JSONB NOT NULL,             -- Task-specific data

    -- Correlation (for debugging/tracing)
    correlation_id  TEXT,                       -- Request correlation ID
    causation_id    BIGINT,                     -- global.event_log.id that caused this
    tenant_id       TEXT,                       -- For tenant context

    -- Scheduling
    scheduled_for   TIMESTAMPTZ NOT NULL DEFAULT now(),
    priority        INTEGER NOT NULL DEFAULT 0, -- Higher = more urgent

    -- Processing state
    status          TEXT NOT NULL DEFAULT 'pending',
    locked_by       TEXT,                       -- Worker ID holding lock
    locked_until    TIMESTAMPTZ,                -- Lock expiry

    -- Retry handling
    attempts        INTEGER NOT NULL DEFAULT 0,
    max_attempts    INTEGER NOT NULL DEFAULT 3,
    last_error      TEXT,
    next_retry_at   TIMESTAMPTZ,

    -- Timestamps
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,

    -- Constraints
    CONSTRAINT valid_status CHECK (
        status IN ('pending', 'processing', 'completed', 'failed', 'dead')
    )
);

-- Index for claiming tasks
CREATE INDEX idx_outbox_claimable ON outbox.pending_tasks (scheduled_for, priority DESC)
    WHERE status = 'pending';

-- Index for finding stuck tasks
CREATE INDEX idx_outbox_stuck ON outbox.pending_tasks (locked_until)
    WHERE status = 'processing';

-- Index for dead letter queue
CREATE INDEX idx_outbox_dead ON outbox.pending_tasks (created_at)
    WHERE status = 'dead';

-- ───────────────────────────────────────────────────────────────────────────
-- Dead Letter Queue (for manual inspection)
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE outbox.dead_letters (
    id              BIGSERIAL PRIMARY KEY,
    original_id     BIGINT NOT NULL,
    task_type       TEXT NOT NULL,
    payload         JSONB NOT NULL,
    correlation_id  TEXT,
    causation_id    BIGINT,
    tenant_id       TEXT,
    attempts        INTEGER NOT NULL,
    last_error      TEXT,
    created_at      TIMESTAMPTZ NOT NULL,
    died_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ───────────────────────────────────────────────────────────────────────────
-- Task Completion Log (for auditing)
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE outbox.completed_tasks (
    id              BIGSERIAL PRIMARY KEY,
    original_id     BIGINT NOT NULL,
    task_type       TEXT NOT NULL,
    correlation_id  TEXT,
    causation_id    BIGINT,
    tenant_id       TEXT,
    attempts        INTEGER NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    completed_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    duration_ms     INTEGER
);

-- Partition by month for easy cleanup
-- CREATE TABLE outbox.completed_tasks_2024_01 PARTITION OF outbox.completed_tasks
--     FOR VALUES FROM ('2024-01-01') TO ('2024-02-01');

-- ───────────────────────────────────────────────────────────────────────────
-- Outbox Functions
-- ───────────────────────────────────────────────────────────────────────────

-- Claim a batch of tasks for processing
CREATE FUNCTION outbox.claim_tasks(
    p_worker_id     TEXT,
    p_batch_size    INTEGER DEFAULT 10,
    p_lock_duration INTERVAL DEFAULT '5 minutes'
) RETURNS SETOF outbox.pending_tasks AS $$
    UPDATE outbox.pending_tasks
    SET
        status = 'processing',
        locked_by = p_worker_id,
        locked_until = now() + p_lock_duration,
        started_at = now(),
        attempts = attempts + 1
    WHERE id IN (
        SELECT id
        FROM outbox.pending_tasks
        WHERE status = 'pending'
          AND scheduled_for <= now()
        ORDER BY priority DESC, scheduled_for ASC
        FOR UPDATE SKIP LOCKED
        LIMIT p_batch_size
    )
    RETURNING *
$$ LANGUAGE sql;

-- Mark task as completed
CREATE FUNCTION outbox.complete_task(
    p_task_id       BIGINT,
    p_worker_id     TEXT
) RETURNS VOID AS $$
DECLARE
    v_task outbox.pending_tasks;
BEGIN
    -- Get and lock the task
    SELECT * INTO v_task
    FROM outbox.pending_tasks
    WHERE id = p_task_id AND locked_by = p_worker_id
    FOR UPDATE;

    IF v_task IS NULL THEN
        RAISE EXCEPTION 'Task not found or not owned by worker';
    END IF;

    -- Move to completed log
    INSERT INTO outbox.completed_tasks (
        original_id, task_type, correlation_id, causation_id,
        tenant_id, attempts, created_at, completed_at, duration_ms
    ) VALUES (
        v_task.id, v_task.task_type, v_task.correlation_id, v_task.causation_id,
        v_task.tenant_id, v_task.attempts, v_task.created_at, now(),
        EXTRACT(MILLISECONDS FROM (now() - v_task.started_at))::integer
    );

    -- Delete from pending
    DELETE FROM outbox.pending_tasks WHERE id = p_task_id;
END;
$$ LANGUAGE plpgsql;

-- Mark task as failed (will retry or go to DLQ)
CREATE FUNCTION outbox.fail_task(
    p_task_id       BIGINT,
    p_worker_id     TEXT,
    p_error         TEXT
) RETURNS VOID AS $$
DECLARE
    v_task outbox.pending_tasks;
    v_retry_delay INTERVAL;
BEGIN
    SELECT * INTO v_task
    FROM outbox.pending_tasks
    WHERE id = p_task_id AND locked_by = p_worker_id
    FOR UPDATE;

    IF v_task IS NULL THEN
        RAISE EXCEPTION 'Task not found or not owned by worker';
    END IF;

    IF v_task.attempts >= v_task.max_attempts THEN
        -- Move to dead letter queue
        INSERT INTO outbox.dead_letters (
            original_id, task_type, payload, correlation_id, causation_id,
            tenant_id, attempts, last_error, created_at
        ) VALUES (
            v_task.id, v_task.task_type, v_task.payload, v_task.correlation_id,
            v_task.causation_id, v_task.tenant_id, v_task.attempts, p_error,
            v_task.created_at
        );

        DELETE FROM outbox.pending_tasks WHERE id = p_task_id;
    ELSE
        -- Calculate exponential backoff: 1s, 2s, 4s, 8s, 16s, ...
        v_retry_delay := (power(2, v_task.attempts - 1) || ' seconds')::interval;

        UPDATE outbox.pending_tasks
        SET
            status = 'pending',
            locked_by = NULL,
            locked_until = NULL,
            last_error = p_error,
            next_retry_at = now() + v_retry_delay,
            scheduled_for = now() + v_retry_delay
        WHERE id = p_task_id;
    END IF;
END;
$$ LANGUAGE plpgsql;

-- Release stuck tasks (for recovery)
CREATE FUNCTION outbox.release_stuck_tasks() RETURNS INTEGER AS $$
DECLARE
    v_count INTEGER;
BEGIN
    UPDATE outbox.pending_tasks
    SET
        status = 'pending',
        locked_by = NULL,
        locked_until = NULL
    WHERE status = 'processing'
      AND locked_until < now();

    GET DIAGNOSTICS v_count = ROW_COUNT;
    RETURN v_count;
END;
$$ LANGUAGE plpgsql;

-- Helper to queue a task (called from business logic)
CREATE FUNCTION outbox.queue_task(
    p_task_type     TEXT,
    p_payload       JSONB,
    p_correlation_id TEXT DEFAULT NULL,
    p_causation_id  BIGINT DEFAULT NULL,
    p_scheduled_for TIMESTAMPTZ DEFAULT now(),
    p_priority      INTEGER DEFAULT 0,
    p_max_attempts  INTEGER DEFAULT 3
) RETURNS BIGINT AS $$
DECLARE
    v_task_id BIGINT;
BEGIN
    INSERT INTO outbox.pending_tasks (
        task_type, payload, correlation_id, causation_id,
        tenant_id, scheduled_for, priority, max_attempts
    ) VALUES (
        p_task_type, p_payload, p_correlation_id, p_causation_id,
        auth.current_tenant_id(), p_scheduled_for, p_priority, p_max_attempts
    )
    RETURNING id INTO v_task_id;

    -- Hint to outbox processor
    PERFORM pg_notify('outbox_tasks', 'new');

    RETURN v_task_id;
END;
$$ LANGUAGE plpgsql;
```

### 7.3 Rust Task Executors

```rust
/// Task executor trait
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    /// Task type this executor handles
    fn task_type(&self) -> &'static str;

    /// Execute the task
    async fn execute(&self, payload: serde_json::Value) -> Result<(), TaskError>;
}

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("Temporary failure (will retry): {0}")]
    Temporary(String),

    #[error("Permanent failure (no retry): {0}")]
    Permanent(String),
}

/// Email task executor
pub struct EmailExecutor {
    smtp: lettre::SmtpTransport,
    from: String,
    templates: Tera,
}

#[async_trait]
impl TaskExecutor for EmailExecutor {
    fn task_type(&self) -> &'static str {
        "send_email"
    }

    async fn execute(&self, payload: serde_json::Value) -> Result<(), TaskError> {
        let task: EmailTask = serde_json::from_value(payload)
            .map_err(|e| TaskError::Permanent(format!("Invalid payload: {e}")))?;

        // Render template
        let body = self.templates
            .render(&task.template, &task.data)
            .map_err(|e| TaskError::Permanent(format!("Template error: {e}")))?;

        // Build email
        let email = Message::builder()
            .from(self.from.parse().unwrap())
            .to(task.to.parse().map_err(|e| TaskError::Permanent(format!("Invalid email: {e}")))?)
            .subject(&task.subject)
            .body(body)
            .map_err(|e| TaskError::Permanent(format!("Build error: {e}")))?;

        // Send (temporary errors on SMTP failure)
        self.smtp
            .send(&email)
            .map_err(|e| TaskError::Temporary(format!("SMTP error: {e}")))?;

        Ok(())
    }
}

#[derive(Deserialize)]
struct EmailTask {
    template: String,
    to: String,
    subject: String,
    data: serde_json::Value,
}

/// SMS task executor (e.g., Twilio)
pub struct SmsExecutor {
    client: reqwest::Client,
    account_sid: String,
    auth_token: String,
    from_number: String,
}

#[async_trait]
impl TaskExecutor for SmsExecutor {
    fn task_type(&self) -> &'static str {
        "send_sms"
    }

    async fn execute(&self, payload: serde_json::Value) -> Result<(), TaskError> {
        let task: SmsTask = serde_json::from_value(payload)
            .map_err(|e| TaskError::Permanent(format!("Invalid payload: {e}")))?;

        let response = self.client
            .post(format!(
                "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
                self.account_sid
            ))
            .basic_auth(&self.account_sid, Some(&self.auth_token))
            .form(&[
                ("From", &self.from_number),
                ("To", &task.to),
                ("Body", &task.body),
            ])
            .send()
            .await
            .map_err(|e| TaskError::Temporary(format!("HTTP error: {e}")))?;

        if response.status().is_success() {
            Ok(())
        } else if response.status().is_server_error() {
            Err(TaskError::Temporary(format!("Twilio error: {}", response.status())))
        } else {
            Err(TaskError::Permanent(format!("Twilio rejected: {}", response.status())))
        }
    }
}

#[derive(Deserialize)]
struct SmsTask {
    to: String,
    body: String,
}

/// Webhook task executor
pub struct WebhookExecutor {
    client: reqwest::Client,
}

#[async_trait]
impl TaskExecutor for WebhookExecutor {
    fn task_type(&self) -> &'static str {
        "webhook"
    }

    async fn execute(&self, payload: serde_json::Value) -> Result<(), TaskError> {
        let task: WebhookTask = serde_json::from_value(payload)
            .map_err(|e| TaskError::Permanent(format!("Invalid payload: {e}")))?;

        let mut request = self.client.post(&task.url);

        // Add headers
        for (key, value) in &task.headers {
            request = request.header(key, value);
        }

        // Add body
        request = request.json(&task.body);

        let response = request
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| TaskError::Temporary(format!("HTTP error: {e}")))?;

        if response.status().is_success() {
            Ok(())
        } else if response.status().is_server_error() || response.status() == 429 {
            // 5xx or rate limited = retry
            Err(TaskError::Temporary(format!("Webhook error: {}", response.status())))
        } else {
            // 4xx = permanent failure
            Err(TaskError::Permanent(format!("Webhook rejected: {}", response.status())))
        }
    }
}

#[derive(Deserialize)]
struct WebhookTask {
    url: String,
    headers: HashMap<String, String>,
    body: serde_json::Value,
}
```

### 7.4 Outbox Worker

```rust
/// Background worker that processes outbox tasks
pub struct OutboxWorker {
    pool: PgPool,
    worker_id: String,
    executors: HashMap<&'static str, Arc<dyn TaskExecutor>>,
    shutdown: broadcast::Receiver<()>,
}

impl OutboxWorker {
    pub fn new(
        pool: PgPool,
        executors: Vec<Arc<dyn TaskExecutor>>,
        shutdown: broadcast::Receiver<()>,
    ) -> Self {
        let worker_id = format!("worker-{}", Uuid::new_v4());
        let executors = executors
            .into_iter()
            .map(|e| (e.task_type(), e))
            .collect();

        Self {
            pool,
            worker_id,
            executors,
            shutdown,
        }
    }

    pub async fn run(mut self) {
        let mut poll_interval = tokio::time::interval(Duration::from_secs(5));
        let mut notify_rx = self.subscribe_to_notifications().await;

        loop {
            tokio::select! {
                // Shutdown signal
                _ = self.shutdown.recv() => {
                    tracing::info!("Outbox worker shutting down");
                    break;
                }

                // pg_notify hint (process immediately)
                _ = notify_rx.recv() => {
                    self.process_batch().await;
                }

                // Regular polling (catch any missed notifications)
                _ = poll_interval.tick() => {
                    self.process_batch().await;
                    self.release_stuck_tasks().await;
                }
            }
        }
    }

    async fn subscribe_to_notifications(&self) -> mpsc::Receiver<()> {
        let (tx, rx) = mpsc::channel(100);
        let pool = self.pool.clone();

        tokio::spawn(async move {
            // Simplified - in practice, use the LISTEN connection from earlier
            let mut conn = pool.acquire().await.unwrap();
            sqlx::query("LISTEN outbox_tasks").execute(&mut *conn).await.unwrap();

            // ... listen loop
        });

        rx
    }

    async fn process_batch(&self) {
        // Claim tasks
        let tasks: Vec<OutboxTask> = sqlx::query_as(
            "SELECT * FROM outbox.claim_tasks($1, $2)"
        )
        .bind(&self.worker_id)
        .bind(10i32)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        for task in tasks {
            self.process_task(task).await;
        }
    }

    async fn process_task(&self, task: OutboxTask) {
        let executor = match self.executors.get(task.task_type.as_str()) {
            Some(e) => e,
            None => {
                tracing::error!("No executor for task type: {}", task.task_type);
                self.fail_task(task.id, "No executor found").await;
                return;
            }
        };

        match executor.execute(task.payload.clone()).await {
            Ok(()) => {
                self.complete_task(task.id).await;
                tracing::info!(
                    task_type = %task.task_type,
                    task_id = task.id,
                    "Task completed"
                );
            }
            Err(TaskError::Temporary(msg)) => {
                self.fail_task(task.id, &msg).await;
                tracing::warn!(
                    task_type = %task.task_type,
                    task_id = task.id,
                    error = %msg,
                    "Task failed (will retry)"
                );
            }
            Err(TaskError::Permanent(msg)) => {
                // Mark as max attempts to force DLQ
                self.fail_task_permanent(task.id, &msg).await;
                tracing::error!(
                    task_type = %task.task_type,
                    task_id = task.id,
                    error = %msg,
                    "Task failed permanently"
                );
            }
        }
    }

    async fn complete_task(&self, task_id: i64) {
        let _ = sqlx::query("SELECT outbox.complete_task($1, $2)")
            .bind(task_id)
            .bind(&self.worker_id)
            .execute(&self.pool)
            .await;
    }

    async fn fail_task(&self, task_id: i64, error: &str) {
        let _ = sqlx::query("SELECT outbox.fail_task($1, $2, $3)")
            .bind(task_id)
            .bind(&self.worker_id)
            .bind(error)
            .execute(&self.pool)
            .await;
    }

    async fn fail_task_permanent(&self, task_id: i64, error: &str) {
        // Set attempts to max to force DLQ
        let _ = sqlx::query(
            "UPDATE outbox.pending_tasks SET attempts = max_attempts WHERE id = $1"
        )
        .bind(task_id)
        .execute(&self.pool)
        .await;

        self.fail_task(task_id, error).await;
    }

    async fn release_stuck_tasks(&self) {
        let released: i32 = sqlx::query_scalar("SELECT outbox.release_stuck_tasks()")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);

        if released > 0 {
            tracing::warn!("Released {} stuck tasks", released);
        }
    }
}
```

### 7.5 Queuing Tasks from Business Logic

```sql
-- Example: Order confirmation triggers email
CREATE OR REPLACE FUNCTION sales.confirm_order(
    p_order_id      TEXT,
    p_timestamp     TIMESTAMPTZ DEFAULT now()
) RETURNS sales.command_result AS $$
DECLARE
    v_result sales.command_result;
    v_order sales.orders_projection;
BEGIN
    PERFORM auth.require_role('sales');

    -- Get order details for email
    SELECT * INTO v_order
    FROM sales.orders_projection
    WHERE order_id = p_order_id;

    IF v_order IS NULL THEN
        RAISE EXCEPTION 'Order not found' USING ERRCODE = 'P0404';
    END IF;

    -- Append event
    v_result := global.append_event(
        p_stream_id := p_order_id,
        p_context := 'sales',
        p_aggregate := 'order',
        p_event_type := 'OrderConfirmed',
        -- ... rest of event
    );

    -- Queue confirmation email (same transaction!)
    PERFORM outbox.queue_task(
        p_task_type := 'send_email',
        p_payload := jsonb_build_object(
            'template', 'order_confirmed',
            'to', v_order.customer_email,
            'subject', 'Your order has been confirmed!',
            'data', jsonb_build_object(
                'order_id', p_order_id,
                'customer_name', v_order.customer_name,
                'items', v_order.items,
                'total', v_order.total
            )
        ),
        p_causation_id := v_result.global_event_id
    );

    RETURN v_result;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;
```

---

## 8. Connection Management

### 8.1 Pool Configuration

```rust
#[derive(Clone)]
pub struct DbConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_lifetime: Duration,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            url: std::env::var("DATABASE_URL").expect("DATABASE_URL required"),
            max_connections: 20,
            min_connections: 5,
            acquire_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(600),
            max_lifetime: Duration::from_secs(1800),
        }
    }
}

pub async fn create_pool(config: &DbConfig) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(config.min_connections)
        .acquire_timeout(config.acquire_timeout)
        .idle_timeout(Some(config.idle_timeout))
        .max_lifetime(Some(config.max_lifetime))
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                // Set default application name for monitoring
                sqlx::query("SET application_name = 'rust_server'")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&config.url)
        .await
}
```

### 8.2 Health Checks

```rust
pub async fn health_check(State(app): State<AppState>) -> impl IntoResponse {
    let db_healthy = check_database(&app.db).await;
    let outbox_healthy = check_outbox(&app.db).await;

    let status = if db_healthy && outbox_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let body = json!({
        "status": if status == StatusCode::OK { "healthy" } else { "unhealthy" },
        "checks": {
            "database": db_healthy,
            "outbox": outbox_healthy,
        }
    });

    (status, Json(body))
}

async fn check_database(pool: &PgPool) -> bool {
    sqlx::query("SELECT 1")
        .fetch_one(pool)
        .await
        .is_ok()
}

async fn check_outbox(pool: &PgPool) -> bool {
    // Check if outbox has old pending tasks (indicates stuck worker)
    let stuck_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM outbox.pending_tasks
         WHERE status = 'pending' AND scheduled_for < now() - interval '5 minutes'"
    )
    .fetch_one(pool)
    .await
    .unwrap_or(999);

    stuck_count < 100
}
```

---

## 9. Server Startup

### 9.1 Main Entry Point

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .json()
        .init();

    // Load configuration
    let config = Config::from_env()?;

    // Create database pool
    let pool = create_pool(&config.database).await?;

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await?;

    // Create broadcast channel for WebSocket
    let (ws_broadcast_tx, _) = broadcast::channel(1000);

    // Create shutdown channel
    let (shutdown_tx, _) = broadcast::channel(1);

    // Create task executors
    let executors: Vec<Arc<dyn TaskExecutor>> = vec![
        Arc::new(EmailExecutor::new(&config.email)?),
        Arc::new(SmsExecutor::new(&config.sms)?),
        Arc::new(WebhookExecutor::new()),
    ];

    // Spawn background tasks
    let pg_listener = tokio::spawn(pg_notify_listener(
        pool.clone(),
        ws_broadcast_tx.clone(),
    ));

    let outbox_worker = tokio::spawn(
        OutboxWorker::new(pool.clone(), executors, shutdown_tx.subscribe()).run()
    );

    // Create app state
    let state = AppState {
        db: pool,
        ws_broadcast: ws_broadcast_tx,
        jwt_keys: JwtKeys::from_env()?,
    };

    // Create router
    let app = create_router(state);

    // Start server
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("Starting server on {}", addr);

    let listener = TcpListener::bind(addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown_tx))
        .await?;

    // Wait for background tasks
    pg_listener.abort();
    outbox_worker.abort();

    tracing::info!("Server stopped");
    Ok(())
}

async fn shutdown_signal(shutdown_tx: broadcast::Sender<()>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received");
    let _ = shutdown_tx.send(());
}
```

---

## 10. Summary

### Architecture Layers

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           Rust Server                                    │
│                                                                          │
│  Layer 1: Protocol                                                       │
│  ├── HTTP request/response handling (axum)                               │
│  ├── WebSocket frame management                                          │
│  └── JSON serialization/deserialization                                  │
│                                                                          │
│  Layer 2: Security                                                       │
│  ├── JWT signature validation                                            │
│  ├── Session cookie management                                           │
│  └── Identity extraction and context setting                             │
│                                                                          │
│  Layer 3: Orchestration                                                  │
│  ├── Call stored procedures                                              │
│  ├── Map PostgreSQL errors to HTTP status                                │
│  └── Forward events to WebSocket clients                                 │
│                                                                          │
│  Layer 4: Side Effects                                                   │
│  ├── Outbox worker (poll & execute)                                      │
│  ├── Email sending (SMTP)                                                │
│  ├── SMS sending (Twilio)                                                │
│  └── Webhook delivery (HTTP)                                             │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                           PostgreSQL                                     │
│                                                                          │
│  Everything else:                                                        │
│  ├── Business logic (validation, state transitions)                      │
│  ├── Event sourcing (global log, context events)                         │
│  ├── Projections (read models)                                           │
│  ├── Authorization (RLS, permission checks)                              │
│  ├── Audit trail (actor_id in metadata)                                  │
│  └── Outbox (transactional side effect queue)                            │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### Ownership Summary

| Concern | Rust | PostgreSQL |
|---------|------|------------|
| HTTP parsing | ✅ | |
| JWT validation | ✅ | |
| Session storage | | ✅ |
| Identity context | Set variables | Read variables |
| Business validation | | ✅ |
| State transitions | | ✅ |
| Event storage | | ✅ |
| Authorization | | ✅ (RLS + functions) |
| Projections | | ✅ |
| Error codes | Map to HTTP | Define codes |
| WebSocket framing | ✅ | |
| Event notifications | Forward | pg_notify |
| Side effect queue | | ✅ (outbox table) |
| Side effect execution | ✅ | |
| Email/SMS/Webhook | ✅ | |

### Key Principles

1. **Rust handlers are boring**: If it's interesting, it belongs in PostgreSQL
2. **PostgreSQL is the source of truth**: Events, state, permissions all live there
3. **Outbox for reliability**: Side effects are transactional via outbox pattern
4. **RLS for tenant isolation**: Database-level enforcement, can't be bypassed
5. **Session variables for context**: Identity flows from Rust to PostgreSQL
6. **pg_notify for real-time**: Low latency hints, polling for reliability
