# composable-rust-web

Generic Axum web framework integration for Composable Rust.

## Purpose

This crate provides **generic HTTP utilities** that work with ANY domain. It implements the "Functional Core, Imperative Shell" pattern, keeping HTTP concerns separate from business logic.

**Domain-specific handlers should NOT go in this crate.** They belong in domain crates (e.g., `composable-rust-auth`).

## What's Included

### 1. Generic Error Handling (`AppError`)

HTTP-friendly error type with constructor helpers:

```rust
use composable_rust_web::AppError;

// Pre-built constructors for common HTTP errors
AppError::bad_request("Invalid email format");
AppError::unauthorized("Invalid credentials");
AppError::forbidden("Insufficient permissions");
AppError::not_found("User", user_id);
AppError::conflict("Email already exists");
AppError::validation("Password must be at least 8 characters");
AppError::timeout("Request timed out");
AppError::internal("Database connection failed");
AppError::unavailable("Service temporarily unavailable");
```

Features:
- Automatic JSON error responses
- Logs 5xx errors for debugging
- Converts `anyhow::Error` automatically

### 2. HTTP Extractors

**CorrelationId**: Extract or generate request correlation IDs
```rust
use composable_rust_web::CorrelationId;

async fn handler(correlation_id: CorrelationId) -> String {
    format!("Request ID: {}", correlation_id.0)
}
```

**ClientIp**: Extract client IP from headers or connection
```rust
use composable_rust_web::ClientIp;

async fn handler(client_ip: ClientIp) -> String {
    format!("Client IP: {}", client_ip.0)
}
```

**UserAgent**: Extract User-Agent header
```rust
use composable_rust_web::UserAgent;

async fn handler(user_agent: UserAgent) -> String {
    format!("User-Agent: {}", user_agent.0)
}
```

### 3. Health Check Endpoints

Two health check patterns:

**Simple (liveness)**:
```rust
use composable_rust_web::handlers::health::health_check;

let app = Router::new()
    .route("/health", get(health_check));
```

**With Store diagnostics (readiness)**:
```rust
use composable_rust_web::handlers::health::health_check_with_store;
use std::sync::Arc;

let store = Arc::new(Store::new(state, reducer, env));

let app = Router::new()
    .route("/health/ready", get(health_check_with_store))
    .with_state(store);
```

## Usage Pattern

### 1. Define Your Domain State

```rust
// In your domain crate (e.g., auth/)
use composable_rust_runtime::Store;
use std::sync::Arc;

struct MyAppState {
    auth_store: Arc<Store<AuthState, AuthAction, AuthEnv, AuthReducer>>,
    orders_store: Arc<Store<OrderState, OrderAction, OrderEnv, OrderReducer>>,
}
```

### 2. Write Domain-Specific Handlers

```rust
// In your domain crate (e.g., auth/src/handlers/)
use composable_rust_web::{AppError, CorrelationId, ClientIp};
use axum::{extract::State, Json};

async fn login(
    State(state): State<Arc<MyAppState>>,
    correlation_id: CorrelationId,
    client_ip: ClientIp,
    Json(request): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    // 1. Build action from request
    let action = AuthAction::Login {
        correlation_id: correlation_id.0,
        email: request.email,
        password: request.password,
        client_ip: client_ip.0,
    };

    // 2. Use send_and_wait_for for request-response pattern
    let result = state.auth_store
        .send_and_wait_for(
            action,
            |a| matches!(a, AuthAction::SessionCreated { .. } | AuthAction::LoginFailed { .. }),
            Duration::from_secs(5),
        )
        .await
        .map_err(|e| AppError::timeout("Login timeout"))?;

    // 3. Map result to HTTP response
    match result {
        AuthAction::SessionCreated { session, .. } => {
            Ok(Json(LoginResponse { session }))
        }
        AuthAction::LoginFailed { reason, .. } => {
            Err(AppError::unauthorized(reason))
        }
        _ => Err(AppError::internal("Unexpected action")),
    }
}
```

### 3. Build Your Router

```rust
use axum::{Router, routing::post};

let app = Router::new()
    .route("/api/v1/auth/login", post(login))
    .route("/health", get(health_check))
    .with_state(Arc::new(app_state));
```

## Architecture

```text
┌─────────────────────────────────────────┐
│   Imperative Shell (Axum - THIS CRATE)  │  ← HTTP, JSON, cookies
│  - Request parsing                      │  ← Rate limiting, CORS
│  - Response serialization               │  ← Logging, metrics
├─────────────────────────────────────────┤
│         Functional Core (Domain)        │
│  - Pure business logic (reducers)       │  ← Testable at memory speed
│  - State transformations                │  ← No I/O, no side effects
│  - Effect descriptions (values)         │  ← Composable, inspectable
└─────────────────────────────────────────┘
```

## Request Flow

1. **HTTP Request** arrives at Axum handler
2. **Extract data** from request (JSON, headers, extractors)
3. **Build Action** from extracted data
4. **Dispatch** via `store.send_and_wait_for()`
5. **Execute effects** (database, email, events)
6. **Map result** to HTTP response
7. **Return** to client

## Key Principles

1. **Generic utilities only** - No domain-specific code
2. **HTTP concerns only** - Business logic stays in reducers
3. **Observable actions** - Use `send_and_wait_for()` for request-response
4. **Functional core, imperative shell** - HTTP is the shell

## When NOT to Use

- Domain-specific handlers → Put in domain crates
- Business logic → Put in reducers
- Database queries → Put in environment/effects
- Validation → Put in reducers or separate validation layer

## Testing

All utilities have comprehensive tests:

```bash
cargo test -p composable-rust-web
```

## Examples

See the auth crate for complete examples of domain-specific handlers using these utilities.

## License

MIT OR Apache-2.0
