# composable-rust-pg-gateway

PostgreSQL Gateway for Thin Server Applications - where PostgreSQL owns all business logic and Rust handles protocol translation, identity context, and side-effect execution.

## Overview

This crate implements the "thin server" architecture pattern where:

- **PostgreSQL** owns all business logic via stored procedures
- **Rust** handles protocol concerns (HTTP/WebSocket), identity propagation, and side-effects

```
┌─────────────┐     ┌─────────────────┐     ┌──────────────┐
│   Client    │────▶│   Rust Gateway  │────▶│  PostgreSQL  │
│ (HTTP/WS)   │◀────│  (This Crate)   │◀────│   (Logic)    │
└─────────────┘     └─────────────────┘     └──────────────┘
                           │
                           ▼
                    ┌─────────────┐
                    │ Side Effects│
                    │ (Email/SMS) │
                    └─────────────┘
```

## Features

| Feature | Description |
|---------|-------------|
| `http` (default) | HTTP API support with Axum |
| `websocket` | Real-time event streaming via WebSocket + `pg_notify` |
| `auth-handlers` | Built-in authentication handlers (magic link) |
| `tasks` | Background task execution framework |
| `tasks-email` | Email sending via SMTP (lettre) |
| `tasks-sms` | SMS sending (bring your own provider) |
| `tasks-webhook` | HTTP webhook delivery |
| `full` | All features enabled |

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
composable-rust-pg-gateway = { version = "0.1", features = ["full"] }
```

### Basic HTTP Handler

```rust
use composable_rust_pg_gateway::{
    ApiError, Identity, execute_with_identity,
};
use axum::{extract::State, Json};
use sqlx::PgPool;

async fn submit_order(
    State(pool): State<PgPool>,
    identity: Identity,
    Json(req): Json<SubmitOrderRequest>,
) -> Result<Json<OrderResult>, ApiError> {
    execute_with_identity(&pool, &identity, |conn| {
        Box::pin(async move {
            sqlx::query_as("SELECT * FROM sales.submit_order($1, $2)")
                .bind(&req.order_id)
                .bind(&req.customer_id)
                .fetch_one(conn)
                .await
        })
    })
    .await
    .map(Json)
    .map_err(Into::into)
}
```

### Creating a Pool

```rust
use composable_rust_pg_gateway::{create_pool, DbConfig};

let config = DbConfig::new("postgres://localhost/mydb")
    .with_max_connections(10)
    .with_connect_timeout_secs(30);

let pool = create_pool(&config).await?;
```

## PostgreSQL Error Code Mapping

PostgreSQL errors are automatically mapped to HTTP status codes:

| PostgreSQL Code | HTTP Status | Meaning |
|----------------|-------------|---------|
| `P0001` | 400 Bad Request | Validation error |
| `P0401` | 401 Unauthorized | Authentication required |
| `P0403` | 403 Forbidden | Permission denied |
| `P0404` | 404 Not Found | Resource not found |
| `P0409`, `23505` | 409 Conflict | Duplicate/conflict |
| `23503` | 400 Bad Request | Foreign key violation |
| `23502` | 400 Bad Request | Not null violation |
| `23514` | 400 Bad Request | Check constraint violation |

Use `RAISE EXCEPTION 'message' USING ERRCODE = 'P0404'` in PostgreSQL to trigger specific HTTP responses.

## Identity Context

User identity is automatically propagated to PostgreSQL via session variables:

```sql
-- These are set automatically by execute_with_identity()
SET LOCAL app.user_id = 'user-123';
SET LOCAL app.tenant_id = 'tenant-456';
SET LOCAL app.roles = '["admin", "user"]';
```

Your PostgreSQL functions can read these:

```sql
CREATE FUNCTION my_function() RETURNS void AS $$
DECLARE
    v_user_id text := current_setting('app.user_id', true);
    v_tenant_id text := current_setting('app.tenant_id', true);
BEGIN
    -- Use identity for authorization
    IF NOT has_permission(v_user_id, 'write') THEN
        RAISE EXCEPTION 'Permission denied' USING ERRCODE = 'P0403';
    END IF;
END;
$$ LANGUAGE plpgsql;
```

## Authentication

### Magic Link (Passwordless)

```rust
use axum::{Router, routing::{get, post}};
use composable_rust_pg_gateway::{
    request_magic_link, verify_magic_link, MagicLinkConfig,
};

let config = MagicLinkConfig::new()
    .with_redirect_url("/app/dashboard")
    .with_cookie_secure(true)
    .with_ttl_seconds(1800); // 30 minutes

let app = Router::new()
    .route("/auth/magic-link", post(request_magic_link))
    .route("/auth/verify", get(verify_magic_link))
    .with_state((pool, config));
```

Required PostgreSQL functions:

```sql
-- Create magic link token
CREATE FUNCTION auth.create_magic_link(
    p_email TEXT,
    p_ttl_seconds INTEGER
) RETURNS TABLE(token TEXT, expires_at TIMESTAMPTZ);

-- Verify token and create session
CREATE FUNCTION auth.verify_magic_link(
    p_token TEXT
) RETURNS TABLE(
    session_id TEXT,
    user_id TEXT,
    tenant_id TEXT,
    roles TEXT[]
);
```

### JWT Validation

```rust
use composable_rust_pg_gateway::{JwtConfig, JwtValidator};

// HS256 (symmetric)
let config = JwtConfig::from_secret("your-256-bit-secret")?;

// RS256 (asymmetric)
let config = JwtConfig::from_pem(std::fs::read_to_string("public.pem")?)?;

let validator = JwtValidator::new(config);
let claims = validator.validate(token)?;
```

## Health Checks

```rust
use axum::{Router, routing::get};
use composable_rust_pg_gateway::{health_check, health_check_with_outbox, HealthConfig};

// Basic health check (database only)
let app = Router::new()
    .route("/health", get(health_check))
    .with_state(pool);

// With outbox monitoring
let config = HealthConfig::new()
    .with_stuck_task_threshold(50)
    .with_stuck_duration_secs(300); // 5 minutes

let app = Router::new()
    .route("/health", get(health_check_with_outbox))
    .with_state((pool, config));
```

Response format:

```json
{
    "status": "healthy",
    "checks": {
        "database": true,
        "outbox": true
    }
}
```

## WebSocket Real-time Events

Stream PostgreSQL `pg_notify` events to clients:

```rust
use composable_rust_pg_gateway::{
    WsManager, WsState, ws_handler, pg_notify_listener,
};
use std::sync::Arc;

// Create WebSocket manager
let ws_manager = Arc::new(WsManager::new());

// Start pg_notify listener
let listener_handle = tokio::spawn(pg_notify_listener(
    pool.clone(),
    ws_manager.clone(),
    "events", // channel name
));

// Create router
let ws_state = WsState::new(ws_manager, validator);
let app = Router::new()
    .route("/ws", get(ws_handler))
    .with_state(ws_state);
```

### Client Protocol

```typescript
// Subscribe to events
ws.send(JSON.stringify({
    type: "subscribe",
    subscriptions: [
        { type: "context", value: "orders" },
        { type: "stream", value: "order-123" }
    ]
}));

// Receive events
// { type: "event", payload: { context: "orders", event_type: "OrderCreated", ... } }
```

## Background Tasks (Outbox Pattern)

Execute side-effects reliably using the outbox pattern:

```rust
use composable_rust_pg_gateway::{
    OutboxWorker, OutboxWorkerBuilder, OutboxConfig,
    EmailExecutor, EmailConfig,
    WebhookExecutor, WebhookConfig,
};

// Configure email executor
let email_config = EmailConfig::new()
    .with_smtp_host("smtp.example.com")
    .with_smtp_port(587)
    .with_from_address("noreply@example.com");

let email_executor = EmailExecutor::new(email_config)?;

// Configure webhook executor
let webhook_config = WebhookConfig::new()
    .with_timeout_secs(30)
    .with_max_retries(3);

let webhook_executor = WebhookExecutor::new(webhook_config);

// Build outbox worker
let worker = OutboxWorkerBuilder::new(pool.clone())
    .with_executor(email_executor)
    .with_executor(webhook_executor)
    .with_poll_interval_ms(1000)
    .with_batch_size(10)
    .build();

// Run worker (usually in a spawned task)
worker.run().await?;
```

### PostgreSQL Outbox Schema

```sql
CREATE SCHEMA outbox;

CREATE TABLE outbox.pending_tasks (
    id BIGSERIAL PRIMARY KEY,
    task_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    scheduled_for TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Queue a task from your business logic
INSERT INTO outbox.pending_tasks (task_type, payload)
VALUES ('email', '{"to": "user@example.com", "template": "welcome"}');
```

### Custom Task Executors

```rust
use composable_rust_pg_gateway::{TaskExecutor, TaskError};
use async_trait::async_trait;

struct MyCustomExecutor;

#[async_trait]
impl TaskExecutor for MyCustomExecutor {
    fn task_type(&self) -> &'static str {
        "my_custom_task"
    }

    async fn execute(&self, payload: serde_json::Value) -> Result<(), TaskError> {
        // Your logic here
        Ok(())
    }
}
```

## SMS with Custom Provider

```rust
use composable_rust_pg_gateway::{SmsExecutor, SmsProvider, SmsTask};
use async_trait::async_trait;

struct TwilioProvider {
    client: TwilioClient,
}

#[async_trait]
impl SmsProvider for TwilioProvider {
    async fn send(&self, to: &str, message: &str) -> Result<(), TaskError> {
        self.client.send_sms(to, message).await?;
        Ok(())
    }
}

let executor = SmsExecutor::new(TwilioProvider { client });
```

## Complete Application Example

```rust
use axum::{Router, routing::{get, post}};
use composable_rust_pg_gateway::{
    create_pool, DbConfig,
    health_check,
    request_magic_link, verify_magic_link, MagicLinkConfig,
    WsManager, ws_handler, pg_notify_listener, WsState,
    OutboxWorkerBuilder, EmailExecutor, EmailConfig,
    JwtConfig, JwtValidator,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Database pool
    let pool = create_pool(
        &DbConfig::from_env()?
    ).await?;

    // JWT validation
    let jwt_config = JwtConfig::from_env()?;
    let jwt_validator = Arc::new(JwtValidator::new(jwt_config));

    // WebSocket manager
    let ws_manager = Arc::new(WsManager::new());

    // Magic link config
    let magic_link_config = MagicLinkConfig::new()
        .with_redirect_url("/app")
        .with_cookie_secure(true);

    // Start background workers
    let pool_clone = pool.clone();
    tokio::spawn(async move {
        let email_config = EmailConfig::from_env().unwrap();
        let worker = OutboxWorkerBuilder::new(pool_clone)
            .with_executor(EmailExecutor::new(email_config).unwrap())
            .build();
        worker.run().await
    });

    // Start pg_notify listener
    tokio::spawn(pg_notify_listener(
        pool.clone(),
        ws_manager.clone(),
        "events",
    ));

    // Build router
    let app = Router::new()
        // Health
        .route("/health", get(health_check))
        // Auth
        .route("/auth/magic-link", post(request_magic_link))
        .route("/auth/verify", get(verify_magic_link))
        // WebSocket
        .route("/ws", get(ws_handler))
        .with_state((pool, magic_link_config, WsState::new(ws_manager, jwt_validator)));

    // Run server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}
```

## Required PostgreSQL Schema

See `specs/monolith_postgres/rust_server_layer.md` for complete SQL definitions including:

- Event store tables
- Outbox tables
- Auth functions
- Tenant isolation patterns

## License

MIT OR Apache-2.0
