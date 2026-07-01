//! Identity management and authentication.
//!
//! This module provides complete identity management for the thin server pattern:
//!
//! - **Types**: [`Identity`], [`Claims`] for representing authenticated users
//! - **JWT Validation**: [`JwtValidator`], [`JwtConfig`] for token validation
//! - **Context Propagation**: [`set_identity_context`], [`execute_with_identity`]
//!   for passing identity to `PostgreSQL`
//! - **Axum Extractors**: [`Identity`] and [`OptionalIdentity`] for HTTP handlers
//!
//! # Architecture
//!
//! ```text
//! HTTP Request
//!     │
//!     ▼
//! ┌─────────────────────────────────────────┐
//! │           Axum Extractor                 │
//! │  ┌───────────────────────────────────┐  │
//! │  │ 1. Check Authorization: Bearer    │  │
//! │  │    └─► JwtValidator.validate()    │  │
//! │  │                                   │  │
//! │  │ 2. Check session cookie           │  │
//! │  │    └─► Query auth.sessions        │  │
//! │  │                                   │  │
//! │  │ 3. No credentials → 401           │  │
//! │  └───────────────────────────────────┘  │
//! └─────────────────────────────────────────┘
//!     │
//!     ▼ Identity
//! ┌─────────────────────────────────────────┐
//! │           Handler                        │
//! │  execute_with_identity(pool, &id, |c| { │
//! │      // PostgreSQL now has:              │
//! │      // - app.user_id                    │
//! │      // - app.tenant_id                  │
//! │      // - app.roles                      │
//! │  })                                      │
//! └─────────────────────────────────────────┘
//! ```
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use composable_rust_pg_gateway::identity::{
//!     Identity, IdentityConfig, JwtConfig,
//!     execute_with_identity,
//! };
//! use axum::{routing::post, Router, Json};
//! use sqlx::PgPool;
//!
//! // Setup
//! let jwt_config = JwtConfig::from_env()?;
//! let identity_config = IdentityConfig::new(pool.clone(), jwt_config);
//!
//! // Handler
//! async fn submit_order(
//!     State(pool): State<PgPool>,
//!     identity: Identity,
//!     Json(req): Json<OrderRequest>,
//! ) -> Result<Json<OrderResult>, ApiError> {
//!     execute_with_identity(&pool, &identity, |conn| {
//!         Box::pin(async move {
//!             sqlx::query_as("SELECT * FROM sales.submit_order($1)")
//!                 .bind(&req.order_id)
//!                 .fetch_one(conn)
//!                 .await
//!         })
//!     })
//!     .await
//!     .map(Json)
//!     .map_err(Into::into)
//! }
//!
//! // Router
//! let app = Router::new()
//!     .route("/orders/submit", post(submit_order))
//!     .with_state(identity_config);
//! ```

mod context;
mod types;

// JWT module requires auth-handlers feature
#[cfg(feature = "auth-handlers")]
mod jwt;

// Extractor requires both http (for Axum) and auth-handlers (for JWT)
#[cfg(all(feature = "http", feature = "auth-handlers"))]
mod extractor;

// ═══════════════════════════════════════════════════════════════════════════
// Core Types (Always Available)
// ═══════════════════════════════════════════════════════════════════════════

pub use context::ExecuteError;
pub use context::{execute_with_identity, execute_with_identity_raw, set_identity_context};
pub use types::{Claims, Identity};

// ═══════════════════════════════════════════════════════════════════════════
// JWT (Requires auth-handlers feature)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "auth-handlers")]
pub use jwt::{JwtConfig, JwtError, JwtValidator};

// ═══════════════════════════════════════════════════════════════════════════
// Axum Extractors (Requires http + auth-handlers features)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(all(feature = "http", feature = "auth-handlers"))]
pub use extractor::{IdentityConfig, IdentityState, OptionalIdentity};
