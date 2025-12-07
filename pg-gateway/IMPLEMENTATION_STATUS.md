# `pg-gateway` Implementation Status

> **Last Updated**: December 7, 2025 (Phase 6 complete - all phases done)

## Overview

The `composable-rust-pg-gateway` crate implements the "thin server" architecture from `specs/monolith_postgres/rust_server_layer.md`. In this pattern, PostgreSQL owns all business logic via stored procedures, while Rust handles:

- **Protocol Translation**: HTTP/WebSocket → PostgreSQL function calls
- **Identity Context**: Propagating user identity to PostgreSQL session variables
- **Side-Effect Execution**: Background tasks (email, SMS, webhooks) via outbox pattern
- **Real-time Events**: WebSocket streaming of PostgreSQL `pg_notify` events

## Architecture

```
┌─────────────┐     ┌─────────────────┐     ┌──────────────┐
│   Client    │────▶│   Rust Gateway  │────▶│  PostgreSQL  │
│ (HTTP/WS)   │◀────│  (pg-gateway)   │◀────│   (Logic)    │
└─────────────┘     └─────────────────┘     └──────────────┘
                           │
                           ▼
                    ┌─────────────┐
                    │ Side Effects│
                    │ (Email/SMS) │
                    └─────────────┘
```

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Integration** | Standalone crate | Not integrated with `next` crate patterns; complements existing `web`/`auth` crates |
| **Task System** | Generic `TaskExecutor` trait | Users can implement custom executors; PostgreSQL-specific `OutboxWorker` provided |
| **Features** | Modular feature flags | Pick only what you need; minimize dependencies |
| **Error Mapping** | PostgreSQL codes → HTTP | P0xxx custom codes for semantic HTTP responses |

## Module Structure

```
pg-gateway/
├── Cargo.toml
├── IMPLEMENTATION_STATUS.md    ← You are here
├── src/
│   ├── lib.rs                  ✅ Main exports + feature gates
│   │
│   ├── error.rs                ✅ ApiError with PostgreSQL code mapping
│   │
│   ├── pool/
│   │   ├── mod.rs              ✅ create_pool function
│   │   ├── config.rs           ✅ DbConfig with builder pattern
│   │   └── health.rs           ✅ Health check endpoints (Phase 5)
│   │
│   ├── identity/               ✅ Phase 2
│   │   ├── mod.rs              ✅ Feature-gated exports
│   │   ├── types.rs            ✅ Identity, Claims structs
│   │   ├── extractor.rs        ✅ Axum Identity extractor
│   │   ├── jwt.rs              ✅ JWT validation
│   │   └── context.rs          ✅ set_identity_context, execute_with_identity
│   │
│   ├── tasks/                  ✅ Phase 3
│   │   ├── mod.rs              ✅ Feature-gated exports
│   │   ├── executor.rs         ✅ TaskExecutor trait
│   │   ├── error.rs            ✅ TaskError enum
│   │   ├── outbox.rs           ✅ OutboxWorker + OutboxConfig
│   │   └── builtins/
│   │       ├── mod.rs          ✅ Built-in executors
│   │       ├── email.rs        ✅ EmailExecutor + EmailConfig
│   │       ├── sms.rs          ✅ SmsExecutor + SmsProvider trait
│   │       └── webhook.rs      ✅ WebhookExecutor + WebhookConfig
│   │
│   ├── websocket/              ✅ Phase 4
│   │   ├── mod.rs              ✅ Feature-gated exports
│   │   ├── connection.rs       ✅ WsConnection, ConnectionId
│   │   ├── manager.rs          ✅ WsManager with DashMap
│   │   ├── subscription.rs     ✅ Subscription enum
│   │   ├── notification.rs     ✅ EventNotification
│   │   ├── protocol.rs         ✅ ClientMessage, ServerMessage
│   │   ├── handler.rs          ✅ ws_handler, WsState
│   │   └── listener.rs         ✅ pg_notify LISTEN task
│   │
│   └── auth/                   ✅ Phase 5
│       ├── mod.rs              ✅ Feature-gated exports
│       └── magic_link.rs       ✅ Request/verify handlers + MagicLinkConfig
```

## Feature Flags

```toml
[features]
default = ["http"]
http = ["dep:axum", "dep:tower", "dep:tower-http"]
websocket = ["http", "dep:dashmap"]
auth-handlers = ["http", "dep:jsonwebtoken", "dep:cookie"]
tasks = []
tasks-email = ["tasks", "dep:lettre", "dep:tera"]
tasks-sms = ["tasks"]
tasks-webhook = ["tasks", "dep:reqwest"]
full = ["http", "websocket", "auth-handlers", "tasks", "tasks-email", "tasks-sms", "tasks-webhook"]
```

---

## Implementation Phases

### Phase 1: Foundation ✅ COMPLETE

**Status**: All items complete, 10 tests passing, 0 clippy warnings

| Item | Status | Notes |
|------|--------|-------|
| `Cargo.toml` with feature flags | ✅ | All features defined |
| `src/lib.rs` module structure | ✅ | Feature-gated exports |
| `src/error.rs` - `ApiError` | ✅ | PostgreSQL code mapping |
| `src/pool/mod.rs` - `create_pool` | ✅ | Async pool creation |
| `src/pool/config.rs` - `DbConfig` | ✅ | Builder pattern |
| Add to workspace | ✅ | In `members` and `default-members` |

**Key Types Implemented**:

```rust
// PostgreSQL error codes → HTTP status codes
pub enum ApiError {
    BadRequest(String),   // P0001, 23503, 23502, 23514
    Unauthorized,         // P0401
    Forbidden(String),    // P0403
    NotFound,            // P0404
    Conflict(String),    // P0409, 23505
    Internal,
}

// Database configuration
pub struct DbConfig {
    pub database_url: String,
    pub max_connections: u32,
    pub connect_timeout_secs: u64,
    pub idle_timeout_secs: u64,
}
```

---

### Phase 2: Identity & Context ✅ COMPLETE

**Status**: All items complete, 37 tests passing, 0 clippy warnings

**Feature Flag Note**: The extractor module requires BOTH `http` AND `auth-handlers` features.
This is intentional - the extractors use JWT validation which depends on `auth-handlers`.
Users who want HTTP support without identity extraction can enable just `http`.

| Item | Status | Description |
|------|--------|-------------|
| `identity/types.rs` | ✅ | `Identity`, `Claims` structs with role helpers |
| `identity/jwt.rs` | ✅ | JWT validation (HS256, RS256), config from env |
| `identity/extractor.rs` | ✅ | Axum `Identity` + `OptionalIdentity` extractors |
| `identity/context.rs` | ✅ | `set_identity_context`, `execute_with_identity` |
| `identity/mod.rs` | ✅ | Feature-gated exports |

**Key Types Implemented**:

```rust
// Core identity types
pub struct Identity {
    pub user_id: String,
    pub tenant_id: String,
    pub roles: Vec<String>,
    pub session_id: Option<String>,
}

pub struct Claims {
    pub sub: String,
    pub tenant_id: String,
    pub roles: Vec<String>,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
}

// JWT validation (requires auth-handlers feature)
pub struct JwtConfig { /* HS256 or RS256 */ }
pub struct JwtValidator { /* validates tokens */ }

// Context propagation
pub async fn set_identity_context(conn: &mut PgConnection, identity: &Identity) -> Result<(), sqlx::Error>;
pub async fn execute_with_identity<T, F>(pool: &PgPool, identity: &Identity, f: F) -> Result<T, sqlx::Error>;

// Axum extractors (requires http feature)
impl FromRequestParts for Identity { /* JWT or session cookie */ }
pub struct OptionalIdentity(pub Option<Identity>);
pub struct IdentityConfig { /* pool + jwt_validator */ }
```

---

### Phase 3: Background Tasks ✅ COMPLETE

**Status**: All items complete, 25 tests passing, 0 clippy warnings

| Item | Status | Description |
|------|--------|-------------|
| `tasks/executor.rs` | ✅ | `TaskExecutor` trait with async execute |
| `tasks/error.rs` | ✅ | `TaskError` enum (Temporary/Permanent) |
| `tasks/outbox.rs` | ✅ | `OutboxWorker` + `OutboxWorkerBuilder` |
| `tasks/builtins/email.rs` | ✅ | `EmailExecutor` + `EmailConfig` + Tera templates |
| `tasks/builtins/sms.rs` | ✅ | `SmsExecutor` + `SmsProvider` trait + `ConsoleSmsProvider` |
| `tasks/builtins/webhook.rs` | ✅ | `WebhookExecutor` + `WebhookConfig` + retry logic |

**Key Types Implemented**:

```rust
pub trait TaskExecutor: Send + Sync {
    fn task_type(&self) -> &'static str;
    async fn execute(&self, payload: serde_json::Value) -> Result<(), TaskError>;
}

pub enum TaskError {
    Temporary(String),  // Will retry
    Permanent(String),  // No retry, move to dead letter
}

pub struct OutboxWorker {
    pool: PgPool,
    executors: HashMap<String, Arc<dyn TaskExecutor>>,
    config: OutboxConfig,
}

pub struct EmailExecutor { /* SMTP via lettre, Tera templates */ }
pub struct SmsExecutor<P: SmsProvider> { /* Bring your own provider */ }
pub struct WebhookExecutor { /* HTTP with reqwest */ }
```

---

### Phase 4: WebSocket ✅ COMPLETE

**Status**: All items complete, 34 tests passing, 0 clippy warnings

| Item | Status | Description |
|------|--------|-------------|
| `websocket/connection.rs` | ✅ | `WsConnection`, `ConnectionId` with tenant isolation |
| `websocket/manager.rs` | ✅ | `WsManager` with DashMap + broadcast channel |
| `websocket/subscription.rs` | ✅ | `Subscription` enum (Context, Stream, Pattern) |
| `websocket/notification.rs` | ✅ | `EventNotification` + `RawNotification` |
| `websocket/protocol.rs` | ✅ | `ClientMessage`, `ServerMessage` JSON protocol |
| `websocket/handler.rs` | ✅ | `ws_handler`, `WsState` Axum handler |
| `websocket/listener.rs` | ✅ | `pg_notify_listener` with auto-reconnect |

**Key Types Implemented**:

```rust
pub enum Subscription {
    Context(String),                              // All events in context
    Stream(String),                               // Specific stream
    Pattern { context: String, event_type: String }, // Filtered
}

pub struct EventNotification {
    global_id: i64,
    context: String,
    stream_id: String,
    version: i32,
    event_type: String,
    payload: serde_json::Value,
    timestamp: DateTime<Utc>,
    tenant_id: Option<String>,  // For tenant isolation
}

pub struct WsManager {
    connections: DashMap<ConnectionId, WsConnection>,
    broadcast_tx: broadcast::Sender<EventNotification>,
}

// Client → Server messages
pub enum ClientMessage {
    Subscribe { subscriptions: Vec<SubscriptionRequest> },
    Unsubscribe { subscriptions: Vec<String> },
    Ping,
}

// Server → Client messages
pub enum ServerMessage {
    Subscribed { subscriptions: Vec<String> },
    Unsubscribed { subscriptions: Vec<String> },
    Event(EventPayload),
    Pong,
    Error { code: String, message: String },
}
```

---

### Phase 5: Auth Handlers ✅ COMPLETE

**Status**: All items complete, 118 tests passing, 0 clippy warnings

| Item | Status | Description |
|------|--------|-------------|
| `auth/mod.rs` | ✅ | Feature-gated exports |
| `auth/magic_link.rs` | ✅ | Request/verify magic link handlers |
| `pool/health.rs` | ✅ | Health check endpoints (database + outbox) |

**Key Types Implemented**:

```rust
// Magic link authentication (requires auth-handlers feature)
pub struct MagicLinkConfig {
    pub cookie_name: String,        // default: "session"
    pub cookie_secure: bool,        // default: true
    pub cookie_http_only: bool,     // default: true
    pub cookie_same_site: SameSite, // default: Lax
    pub cookie_max_age_days: i64,   // default: 30
    pub redirect_url: String,       // default: "/dashboard"
    pub ttl_seconds: i64,           // default: 3600 (1 hour)
}

pub async fn request_magic_link(
    State(pool): State<PgPool>,
    State(config): State<MagicLinkConfig>,
    Json(req): Json<MagicLinkRequest>,
) -> Result<(StatusCode, Json<MagicLinkResponse>), ApiError>;

pub async fn verify_magic_link(
    State(pool): State<PgPool>,
    State(config): State<MagicLinkConfig>,
    Query(params): Query<VerifyParams>,
) -> Result<Response, ApiError>;

// Health checks (requires http feature)
pub enum HealthStatus { Healthy, Unhealthy }
pub struct HealthResponse { status: HealthStatus, checks: HealthChecks }
pub struct HealthConfig { monitor_outbox: bool, stuck_task_threshold: i64 }

pub async fn health_check(State(pool): State<PgPool>) -> impl IntoResponse;
pub async fn health_check_with_outbox(
    State(pool): State<PgPool>,
    State(config): State<HealthConfig>,
) -> impl IntoResponse;
pub async fn check_database(pool: &PgPool) -> bool;
pub async fn check_outbox(pool: &PgPool, threshold: i64) -> bool;
```

---

### Phase 6: Documentation & Integration ✅ COMPLETE

**Status**: All items complete, 41 tests passing (26 unit + 15 integration), 0 clippy warnings

| Item | Status | Description |
|------|--------|-------------|
| README.md | ✅ | Comprehensive usage documentation |
| Integration tests | ✅ | With testcontainers (15 tests) |
| Example application | ✅ | `thin-server-example` reference implementation |

**Key Deliverables**:

- **README.md**: Complete documentation including quick start, feature flags, error mapping, identity context, authentication, health checks, WebSocket, and background tasks examples
- **Integration Tests** (`tests/integration_tests.rs`):
  - Pool creation and configuration tests
  - Health check endpoint tests
  - Identity context propagation tests (`set_identity_context`, `execute_with_identity`)
  - PostgreSQL error code mapping tests (P0001, P0401, P0403, P0404, P0409, 23505)
- **Example Application** (`examples/thin-server/`):
  - Demonstrates thin server pattern
  - Endpoints: `/health`, `/api/items` (POST), `/api/items/{id}` (GET)
  - Shows `execute_with_identity` pattern
  - Includes SQL examples for required PostgreSQL functions

---

## Usage Example (Target API)

```rust
use composable_rust_pg_gateway::{
    ApiError, Identity, DbConfig,
    execute_with_identity,
    TaskExecutor, OutboxWorker, EmailExecutor,
    WsManager, pg_notify_listener,
};
use axum::{extract::State, Json};
use std::sync::Arc;

async fn submit_order(
    State(app): State<Arc<AppState>>,
    identity: Identity,
    Json(req): Json<SubmitOrderRequest>,
) -> Result<Json<CommandResult>, ApiError> {
    execute_with_identity(&app.pool, &identity, |conn| {
        Box::pin(async move {
            sqlx::query_as("SELECT * FROM sales.submit_order($1, $2, $3)")
                .bind(&req.order_id)
                .bind(&req.customer_id)
                .bind(&Json(&req.items))
                .fetch_one(conn)
                .await
        })
    })
    .await
    .map(Json)
    .map_err(Into::into)
}
```

---

## Dependencies

| Dependency | Feature | Purpose |
|------------|---------|---------|
| `sqlx` | always | PostgreSQL client |
| `axum` | `http` | HTTP framework |
| `tower` | `http` | Middleware |
| `tower-http` | `http` | HTTP utilities |
| `dashmap` | `websocket` | Concurrent connections map |
| `jsonwebtoken` | `auth-handlers` | JWT validation |
| `cookie` | `auth-handlers` | Cookie handling |
| `lettre` | `tasks-email` | SMTP email |
| `tera` | `tasks-email` | Email templates |
| `reqwest` | `tasks-webhook` | HTTP webhooks |

---

## Reference Files

| File | Purpose |
|------|---------|
| `specs/monolith_postgres/rust_server_layer.md` | Authoritative specification |
| `web/src/error.rs` | Pattern reference for error handling |
| `postgres-next/src/lib.rs` | Pattern reference for sqlx usage |
| `.claude/plans/twinkling-conjuring-fog.md` | Original implementation plan |

---

## Progress Summary

| Phase | Status | Progress |
|-------|--------|----------|
| Phase 1: Foundation | ✅ Complete | 100% |
| Phase 2: Identity | ✅ Complete | 100% |
| Phase 3: Tasks | ✅ Complete | 100% |
| Phase 4: WebSocket | ✅ Complete | 100% |
| Phase 5: Auth | ✅ Complete | 100% |
| Phase 6: Docs | ✅ Complete | 100% |

**Overall**: 100% complete (All 6 phases done)

**Test Summary**: 41 tests passing (26 unit + 15 integration), 0 clippy warnings

---

## Getting Started

See the [README.md](./README.md) for complete usage documentation, or explore the [thin-server-example](../examples/thin-server/) for a working reference implementation.
