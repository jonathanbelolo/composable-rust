# pg-gateway Integration Guide

> **For**: Domain-Driven Design AI Agent
>
> **Purpose**: Wire pg-gateway (Rust thin shell) to PostgreSQL (business logic owner)

---

## 1. Overview

`pg-gateway` is a Rust crate that translates HTTP/WebSocket protocols to PostgreSQL function calls. **PostgreSQL owns all business logic**; Rust handles:

- Protocol translation (HTTP → SQL)
- Identity context propagation
- JWT/session validation (cryptography)
- Side-effect execution (email, SMS, webhooks)
- Real-time events (WebSocket)

**Golden Rule**: Rust handlers are boring. They call PostgreSQL functions and map errors.

---

## 2. Required PostgreSQL Infrastructure

### 2.1 Auth Schema

```sql
-- Required schema
CREATE SCHEMA IF NOT EXISTS auth;

-- Sessions table (for cookie-based auth)
CREATE TABLE auth.sessions (
    session_id  TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    tenant_id   TEXT NOT NULL,
    roles       TEXT[] NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_sessions_user ON auth.sessions(user_id);
CREATE INDEX idx_sessions_expires ON auth.sessions(expires_at);

-- Magic link tokens table
CREATE TABLE auth.magic_link_tokens (
    token       TEXT PRIMARY KEY,
    email       TEXT NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_magic_link_email ON auth.magic_link_tokens(email);
CREATE INDEX idx_magic_link_expires ON auth.magic_link_tokens(expires_at);

-- Users table (customize for your domain)
CREATE TABLE auth.users (
    user_id     TEXT PRIMARY KEY,
    email       TEXT UNIQUE NOT NULL,
    tenant_id   TEXT NOT NULL,
    roles       TEXT[] NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_users_email ON auth.users(email);
CREATE INDEX idx_users_tenant ON auth.users(tenant_id);
```

### 2.2 Required Functions (Magic Link Auth)

```sql
-- Function: auth.create_magic_link(email, ttl_seconds)
-- Returns: (token TEXT, expires_at TIMESTAMPTZ)
-- Errors: P0001 if email invalid, P0404 if user not found (optional - can silently succeed)

CREATE FUNCTION auth.create_magic_link(
    p_email TEXT,
    p_ttl_seconds INTEGER DEFAULT 600
) RETURNS TABLE(token TEXT, expires_at TIMESTAMPTZ) AS $$
DECLARE
    v_token TEXT;
    v_expires TIMESTAMPTZ;
BEGIN
    -- Generate secure token
    v_token := encode(gen_random_bytes(32), 'hex');
    v_expires := now() + (p_ttl_seconds || ' seconds')::interval;

    -- Store token (implement your storage)
    INSERT INTO auth.magic_link_tokens (token, email, expires_at)
    VALUES (v_token, p_email, v_expires);

    -- Queue email via outbox (PostgreSQL handles this)
    INSERT INTO outbox.pending_tasks (task_type, payload)
    VALUES ('email', jsonb_build_object(
        'to', p_email,
        'template', 'magic_link',
        'data', jsonb_build_object('token', v_token)
    ));

    RETURN QUERY SELECT v_token, v_expires;
END;
$$ LANGUAGE plpgsql;


-- Function: auth.verify_magic_link(token)
-- Returns: (session_id TEXT, user_id TEXT, tenant_id TEXT, roles TEXT[])
-- Errors: P0401 if token invalid/expired

CREATE FUNCTION auth.verify_magic_link(
    p_token TEXT
) RETURNS TABLE(session_id TEXT, user_id TEXT, tenant_id TEXT, roles TEXT[]) AS $$
DECLARE
    v_email TEXT;
    v_user_id TEXT;
    v_tenant_id TEXT;
    v_roles TEXT[];
    v_session_id TEXT;
BEGIN
    -- Validate and consume token
    DELETE FROM auth.magic_link_tokens
    WHERE token = p_token AND expires_at > now()
    RETURNING email INTO v_email;

    IF v_email IS NULL THEN
        RAISE EXCEPTION 'Invalid or expired token' USING ERRCODE = 'P0401';
    END IF;

    -- Get or create user (implement your logic)
    SELECT u.user_id, u.tenant_id, u.roles
    INTO v_user_id, v_tenant_id, v_roles
    FROM auth.users u WHERE u.email = v_email;

    -- Create session
    v_session_id := encode(gen_random_bytes(32), 'hex');
    INSERT INTO auth.sessions (session_id, user_id, tenant_id, roles, expires_at)
    VALUES (v_session_id, v_user_id, v_tenant_id, v_roles, now() + interval '30 days');

    RETURN QUERY SELECT v_session_id, v_user_id, v_tenant_id, v_roles;
END;
$$ LANGUAGE plpgsql;
```

---

## 3. Identity Context

### 3.1 Session Variables

pg-gateway propagates identity to PostgreSQL via session variables:

```sql
-- Set by pg-gateway automatically (via execute_with_identity)
SET LOCAL app.user_id = 'user-123';
SET LOCAL app.tenant_id = 'tenant-456';
SET LOCAL app.roles = '["admin","user"]';  -- JSON array
```

### 3.2 Reading Identity in Functions

```sql
CREATE FUNCTION your_function() RETURNS void AS $$
DECLARE
    v_user_id TEXT := current_setting('app.user_id', true);
    v_tenant_id TEXT := current_setting('app.tenant_id', true);
    v_roles TEXT[] := (
        SELECT array_agg(r)
        FROM jsonb_array_elements_text(
            current_setting('app.roles', true)::jsonb
        ) AS r
    );
BEGIN
    -- Use identity for authorization, audit, RLS
END;
$$ LANGUAGE plpgsql;
```

### 3.3 Row-Level Security

```sql
-- Enable RLS on tables
ALTER TABLE sales.orders ENABLE ROW LEVEL SECURITY;

-- Policy using identity context
CREATE POLICY tenant_isolation ON sales.orders
    USING (tenant_id = current_setting('app.tenant_id', true));
```

---

## 4. Error Code Mapping

PostgreSQL errors are mapped to HTTP status codes:

| PostgreSQL Code | HTTP Status | When to Use |
|-----------------|-------------|-------------|
| `P0001` | 400 Bad Request | Validation failures |
| `P0401` | 401 Unauthorized | Authentication required/failed |
| `P0403` | 403 Forbidden | Permission denied |
| `P0404` | 404 Not Found | Resource doesn't exist |
| `P0409` | 409 Conflict | Business conflict (duplicate, invalid state) |
| `23505` | 409 Conflict | Unique constraint (optimistic locking) |
| `23503` | 400 Bad Request | Foreign key violation |

### Usage in Functions

```sql
-- Validation error
IF p_quantity <= 0 THEN
    RAISE EXCEPTION 'Quantity must be positive' USING ERRCODE = 'P0001';
END IF;

-- Authorization error
IF NOT has_permission(v_user_id, 'orders:write') THEN
    RAISE EXCEPTION 'Permission denied' USING ERRCODE = 'P0403';
END IF;

-- Not found
IF v_order IS NULL THEN
    RAISE EXCEPTION 'Order not found' USING ERRCODE = 'P0404';
END IF;

-- Conflict
IF v_order.status != 'draft' THEN
    RAISE EXCEPTION 'Order already submitted' USING ERRCODE = 'P0409';
END IF;
```

---

## 5. Handler Patterns

### 5.1 Command Handler (Write Operations)

```rust
use composable_rust_pg_gateway::{execute_with_identity, ApiError, Identity};

async fn submit_order(
    State(pool): State<PgPool>,
    identity: Identity,
    Json(req): Json<SubmitOrderRequest>,
) -> Result<Json<CommandResult>, ApiError> {
    // Rust: just call PostgreSQL
    execute_with_identity(&pool, &identity, |conn| {
        Box::pin(async move {
            sqlx::query_as(
                "SELECT * FROM sales.submit_order($1, $2, $3)"
            )
            .bind(&req.order_id)
            .bind(&req.customer_id)
            .bind(sqlx::types::Json(&req.items))
            .fetch_one(conn)
            .await
        })
    })
    .await
    .map(Json)
}
```

### 5.2 Query Handler (Read Operations)

```rust
async fn get_order(
    State(pool): State<PgPool>,
    identity: Identity,
    Path(order_id): Path<String>,
) -> Result<Json<Order>, ApiError> {
    execute_with_identity(&pool, &identity, |conn| {
        Box::pin(async move {
            sqlx::query_as(
                "SELECT * FROM sales.orders_projection WHERE order_id = $1"
            )
            .bind(&order_id)
            .fetch_optional(conn)
            .await
        })
    })
    .await?
    .ok_or(ApiError::NotFound)
    .map(Json)
}
```

---

## 6. Configuration

### 6.1 Database Pool

```rust
use composable_rust_pg_gateway::{DbConfig, create_pool};

// Option 1: From environment (DATABASE_URL, DB_MAX_CONNECTIONS, etc.)
let config = DbConfig::from_env()?;
let pool = create_pool(&config).await?;

// Option 2: Builder pattern with explicit URL
let config = DbConfig::with_url("postgres://user:pass@localhost/db")
    .max_connections(20)
    .connect_timeout(30)
    .idle_timeout(600);
let pool = create_pool(&config).await?;
```

### 6.2 JWT Validation

```rust
use composable_rust_pg_gateway::JwtConfig;

// From environment (JWT_SECRET, JWT_PUBLIC_KEY, JWT_AUDIENCE, JWT_ISSUER)
let jwt_config = JwtConfig::from_env()?;

// Or configure explicitly
let jwt_config = JwtConfig::with_secret("your-secret-key")
    .with_audience("my-app")      // Validate aud claim
    .with_issuer("my-issuer");    // Validate iss claim
```

### 6.3 Magic Link Auth

```rust
use composable_rust_pg_gateway::MagicLinkConfig;
use cookie::SameSite;

let magic_link_config = MagicLinkConfig::builder()
    .cookie_name("session")
    .cookie_secure(true)              // HTTPS only
    .cookie_same_site(SameSite::Lax)
    .ttl_seconds(600)                 // 10 minute token validity
    .redirect_url("/dashboard")       // Post-login redirect
    .use_host_prefix(true)            // __Host- prefix for subdomain security
    .build();

// Or use fluent API
let magic_link_config = MagicLinkConfig::new()
    .with_cookie_secure(true)
    .with_redirect_url("/dashboard")
    .with_host_prefix(true);          // Enables __Host-session cookie
```

### 6.4 Cargo Features

Enable features in `Cargo.toml`:

```toml
[dependencies]
composable-rust-pg-gateway = { version = "0.1", features = ["full"] }

# Or pick specific features:
# - "http" (default): HTTP API support with Axum
# - "auth-handlers": Magic link handlers, JWT validation
# - "websocket": Real-time event streaming
# - "tasks": Background task execution framework
# - "tasks-email": Email sending via SMTP
# - "tasks-webhook": HTTP webhook delivery
# - "full": All features
```

---

## 7. Security: Rate Limiting

⚠️ **CRITICAL**: The magic link handlers do NOT implement rate limiting. You MUST add rate limiting to prevent:

- Email flooding attacks
- Token brute-forcing
- Resource exhaustion

### Recommended Limits

| Endpoint | Per-IP | Per-Email | Rationale |
|----------|--------|-----------|-----------|
| `/api/auth/magic-link` | 10/min | 3/hour | Prevent email spam |
| `/api/auth/verify` | 30/min | - | Prevent brute-force |

### Implementation Options

1. **Reverse Proxy** (recommended): nginx, Cloudflare, or cloud load balancer
2. **Tower Middleware**: Use `tower_governor` crate for Rust-native limiting

```rust
// Example with tower_governor
use tower_governor::{GovernorLayer, GovernorConfigBuilder};

let governor_config = GovernorConfigBuilder::default()
    .per_second(10)
    .burst_size(20)
    .finish()?;

Router::new()
    .route("/api/auth/magic-link", post(request_magic_link))
    .layer(GovernorLayer { config: governor_config })
```

3. **PostgreSQL-backed**: Store request counts in `auth.rate_limits` table for persistent, distributed limiting

---

## 8. Wiring Checklist

When generating a system, ensure:

### PostgreSQL Side

- [ ] `auth` schema exists
- [ ] `auth.sessions` table exists with required columns
- [ ] `auth.magic_link_tokens` table exists
- [ ] `auth.users` table exists (or equivalent)
- [ ] `auth.create_magic_link(email, ttl_seconds)` function exists
- [ ] `auth.verify_magic_link(token)` function exists
- [ ] All domain functions raise correct error codes (P0001, P0401, etc.)
- [ ] Functions read identity via `current_setting('app.user_id', true)` etc.
- [ ] RLS policies use `current_setting('app.tenant_id', true)` for isolation

### Rust Side

- [ ] `pg-gateway` dependency with required features (`auth-handlers`, etc.)
- [ ] `DbConfig` configured (pool size, timeouts)
- [ ] `JwtConfig` with audience/issuer if using JWT
- [ ] `MagicLinkConfig` for auth endpoints (consider `use_host_prefix(true)`)
- [ ] Routes use `Identity` extractor for protected endpoints
- [ ] Handlers use `execute_with_identity()` for all queries
- [ ] Health check endpoint at `/health`
- [ ] **Rate limiting on auth endpoints** (tower_governor or reverse proxy)

### Integration

- [ ] PostgreSQL listens on expected port
- [ ] `DATABASE_URL` environment variable set
- [ ] `JWT_SECRET` or `JWT_PUBLIC_KEY_PEM` set for JWT validation
- [ ] Outbox worker running for email/SMS/webhook tasks

---

## 9. Complete Router Example

```rust
use axum::{routing::{get, post}, Router, extract::FromRef};
use composable_rust_pg_gateway::{
    health_check, create_pool, DbConfig,
    request_magic_link, verify_magic_link, MagicLinkConfig,
    IdentityConfig, JwtConfig, Identity,
};
use sqlx::PgPool;

// Application state that works with extractors
#[derive(Clone)]
struct AppState {
    pool: PgPool,
    magic_link_config: MagicLinkConfig,
    identity_config: IdentityConfig,
}

// Enable extractors to access components from state
impl FromRef<AppState> for PgPool {
    fn from_ref(state: &AppState) -> Self {
        state.pool.clone()
    }
}

impl FromRef<AppState> for MagicLinkConfig {
    fn from_ref(state: &AppState) -> Self {
        state.magic_link_config.clone()
    }
}

impl FromRef<AppState> for IdentityConfig {
    fn from_ref(state: &AppState) -> Self {
        state.identity_config.clone()
    }
}

async fn build_app() -> Result<Router, Box<dyn std::error::Error>> {
    // Database
    let config = DbConfig::from_env()?;
    let pool = create_pool(&config).await?;

    // Auth config
    let jwt_config = JwtConfig::from_env()?;
    let magic_link_config = MagicLinkConfig::new()
        .with_host_prefix(true);  // Enable __Host- prefix
    let identity_config = IdentityConfig::new(pool.clone(), jwt_config);

    // State
    let state = AppState { pool, magic_link_config, identity_config };

    Ok(Router::new()
        // Health (no auth)
        .route("/health", get(health_check))

        // Auth routes (no auth required)
        .route("/api/auth/magic-link", post(request_magic_link))
        .route("/api/auth/verify", get(verify_magic_link))

        // Domain routes (require Identity extractor)
        .route("/api/sales/orders", post(submit_order))
        .route("/api/sales/orders/{id}", get(get_order))

        .with_state(state))
}
```

---

## 10. Key Principles

1. **PostgreSQL owns logic**: Validation, authorization, state transitions, events
2. **Rust is a protocol adapter**: HTTP parsing, cryptography, side effects
3. **Identity flows automatically**: `execute_with_identity()` sets session vars
4. **Errors are codes**: Use P0001/P0401/P0403/P0404/P0409 consistently
5. **Handlers are boring**: Call function, return result, done

When in doubt: put the logic in PostgreSQL, not Rust.
