//! Authentication handlers for the thin-server pattern.
//!
//! These handlers call `PostgreSQL` stored procedures for all business logic.
//! Rust is only responsible for:
//!
//! - HTTP request/response handling
//! - Cookie management
//! - Redirects
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐     ┌─────────────────┐     ┌──────────────┐
//! │   Client    │────▶│  Auth Handlers  │────▶│  PostgreSQL  │
//! │  (Browser)  │◀────│                 │◀────│  auth.*()    │
//! └─────────────┘     └─────────────────┘     └──────────────┘
//!                            │
//!                            ▼
//!                     Set-Cookie header
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use axum::{Router, routing::{get, post}};
//! use composable_rust_pg_gateway::auth::{
//!     request_magic_link, verify_magic_link, MagicLinkConfig,
//! };
//!
//! let config = MagicLinkConfig::new()
//!     .with_redirect_url("/app/dashboard")
//!     .with_cookie_secure(true);
//!
//! let app = Router::new()
//!     .route("/auth/magic-link", post(request_magic_link))
//!     .route("/auth/verify", get(verify_magic_link))
//!     .with_state(app_state);
//! ```
//!
//! # `PostgreSQL` Schema Requirements
//!
//! These handlers expect the following `PostgreSQL` functions to exist:
//!
//! - `auth.create_magic_link(email TEXT, ttl_seconds INTEGER)` → `(token, expires_at)`
//! - `auth.verify_magic_link(token TEXT)` → `(session_id, user_id, tenant_id, roles)`
//!
//! See `specs/monolith_postgres/rust_server_layer.md` Section 5.6 for the SQL definitions.

mod magic_link;

pub use magic_link::{
    MagicLinkConfig, MagicLinkConfigBuilder, MagicLinkRequest, MagicLinkResponse, VerifyParams,
    request_magic_link, verify_magic_link,
};
