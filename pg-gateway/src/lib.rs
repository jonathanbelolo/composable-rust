//! `PostgreSQL` Gateway for Thin Server Applications
//!
//! This crate provides infrastructure for building "thin server" applications where
//! `PostgreSQL` owns all business logic and Rust is responsible for:
//!
//! - **Protocol Translation**: HTTP/WebSocket to `PostgreSQL` function calls
//! - **Identity Context**: Propagating user identity to `PostgreSQL` session variables
//! - **Side-Effect Execution**: Background tasks (email, SMS, webhooks) via outbox pattern
//! - **Real-time Events**: WebSocket streaming of `PostgreSQL` `pg_notify` events
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐     ┌─────────────────┐     ┌──────────────┐
//! │   Client    │────▶│   Rust Gateway  │────▶│  PostgreSQL  │
//! │ (HTTP/WS)   │◀────│  (This Crate)   │◀────│   (Logic)    │
//! └─────────────┘     └─────────────────┘     └──────────────┘
//!                            │
//!                            ▼
//!                     ┌─────────────┐
//!                     │ Side Effects│
//!                     │ (Email/SMS) │
//!                     └─────────────┘
//! ```
//!
//! # Features
//!
//! - **`http`** (default): HTTP API support with Axum
//! - **`websocket`**: Real-time event streaming via WebSocket
//! - **`auth-handlers`**: Built-in authentication handlers (magic link)
//! - **`tasks`**: Background task execution framework
//! - **`tasks-email`**: Email sending via SMTP
//! - **`tasks-sms`**: SMS sending (bring your own provider)
//! - **`tasks-webhook`**: HTTP webhook delivery
//! - **`full`**: All features enabled
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use composable_rust_pg_gateway::{
//!     ApiError, Identity, DbConfig,
//!     execute_with_identity,
//! };
//! use axum::{extract::State, Json};
//! use std::sync::Arc;
//!
//! async fn submit_order(
//!     State(app): State<Arc<AppState>>,
//!     identity: Identity,
//!     Json(req): Json<SubmitOrderRequest>,
//! ) -> Result<Json<CommandResult>, ApiError> {
//!     execute_with_identity(&app.pool, &identity, |conn| {
//!         Box::pin(async move {
//!             sqlx::query_as("SELECT * FROM sales.submit_order($1, $2)")
//!                 .bind(&req.order_id)
//!                 .bind(&req.customer_id)
//!                 .fetch_one(conn)
//!                 .await
//!         })
//!     })
//!     .await
//!     .map(Json)
//!     .map_err(Into::into)
//! }
//! ```
//!
//! # `PostgreSQL` Error Code Mapping
//!
//! `PostgreSQL` errors are automatically mapped to HTTP status codes:
//!
//! | Code | HTTP Status | Meaning |
//! |------|-------------|---------|
//! | P0001 | 400 | Bad Request (validation) |
//! | P0401 | 401 | Unauthorized |
//! | P0403 | 403 | Forbidden |
//! | P0404 | 404 | Not Found |
//! | P0409, 23505 | 409 | Conflict |
//! | 23503 | 400 | Foreign key violation |
//!
//! # Identity Context
//!
//! User identity is propagated to `PostgreSQL` via session variables:
//!
//! ```sql
//! -- These are set automatically by execute_with_identity()
//! SET LOCAL app.user_id = 'user-123';
//! SET LOCAL app.tenant_id = 'tenant-456';
//! SET LOCAL app.roles = '["admin", "user"]';
//! ```
//!
//! `PostgreSQL` functions can read these with `current_setting()`:
//!
//! ```sql
//! CREATE FUNCTION my_function() RETURNS void AS $$
//! DECLARE
//!     v_user_id text := current_setting('app.user_id', true);
//!     v_tenant_id text := current_setting('app.tenant_id', true);
//! BEGIN
//!     -- Use identity in your logic
//! END;
//! $$ LANGUAGE plpgsql;
//! ```

// ═══════════════════════════════════════════════════════════════════════════
// Core Modules (Always Available)
// ═══════════════════════════════════════════════════════════════════════════

mod error;
pub mod identity;
pub mod pool;

pub use error::ApiError;
pub use pool::{DbConfig, create_pool};

// Re-export commonly used identity types at crate root
pub use identity::{Claims, Identity, execute_with_identity, set_identity_context};

// ═══════════════════════════════════════════════════════════════════════════
// JWT and Auth (Requires auth-handlers feature)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "auth-handlers")]
pub use identity::{JwtConfig, JwtError, JwtValidator};

// ═══════════════════════════════════════════════════════════════════════════
// HTTP Extractors (Requires http + auth-handlers features)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(all(feature = "http", feature = "auth-handlers"))]
pub use identity::{IdentityConfig, IdentityState, OptionalIdentity};

// ═══════════════════════════════════════════════════════════════════════════
// WebSocket Feature
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "websocket")]
pub mod websocket;

#[cfg(feature = "websocket")]
pub use websocket::{
    ClientMessage, ConnectionId, EventNotification, ServerMessage, Subscription,
    SubscriptionRequest, WsConnection, WsManager, WsState, pg_notify_listener, ws_handler,
};

// ═══════════════════════════════════════════════════════════════════════════
// Tasks Feature
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "tasks")]
pub mod tasks;

#[cfg(feature = "tasks")]
pub use tasks::{
    OutboxConfig, OutboxTask, OutboxWorker, OutboxWorkerBuilder, TaskError, TaskExecutor,
};

#[cfg(feature = "tasks-email")]
pub use tasks::builtins::{EmailConfig, EmailExecutor, EmailTask};

#[cfg(feature = "tasks-sms")]
pub use tasks::builtins::{ConsoleSmsProvider, DynSmsExecutor, SmsExecutor, SmsProvider, SmsTask};

#[cfg(feature = "tasks-webhook")]
pub use tasks::builtins::{WebhookConfig, WebhookExecutor, WebhookTask};

// ═══════════════════════════════════════════════════════════════════════════
// Auth Handlers Feature
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "auth-handlers")]
pub mod auth;

#[cfg(feature = "auth-handlers")]
pub use auth::{
    MagicLinkConfig, MagicLinkConfigBuilder, MagicLinkRequest, MagicLinkResponse, VerifyParams,
    request_magic_link, verify_magic_link,
};

// ═══════════════════════════════════════════════════════════════════════════
// Health Checks (Requires http feature)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "http")]
pub use pool::{
    HealthChecks, HealthConfig, HealthResponse, HealthStatus, check_database, check_outbox,
    health_check, health_check_with_outbox,
};
