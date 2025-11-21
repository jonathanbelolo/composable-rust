//! Axum web framework integration for Composable Rust.
//!
//! This crate provides integration between the Axum web framework and the
//! Composable Rust architecture, implementing the "Functional Core, Imperative Shell"
//! pattern.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │         Imperative Shell (Axum)         │  ← HTTP, JSON, cookies
//! │  - Request parsing                      │  ← Rate limiting, CORS
//! │  - Response serialization               │  ← Logging, metrics
//! ├─────────────────────────────────────────┤
//! │         Functional Core                 │
//! │  - Pure business logic (reducers)       │  ← Testable at memory speed
//! │  - State transformations                │  ← No I/O, no side effects
//! │  - Effect descriptions (values)         │  ← Composable, inspectable
//! └─────────────────────────────────────────┘
//! ```
//!
//! # Request Flow
//!
//! 1. **HTTP Request** arrives at Axum handler
//! 2. **Extract data** from request (JSON, headers, cookies)
//! 3. **Build Action** from extracted data
//! 4. **Dispatch** action through `Store`
//! 5. **Execute effects** (database, email, events)
//! 6. **Map result** to HTTP response
//! 7. **Return response** to client
//!
//! # Example
//!
//! ```ignore
//! use composable_rust_web::{AppState, AppError};
//! use axum::{Router, routing::post, Json};
//!
//! async fn handle_command(
//!     State(state): State<AppState>,
//!     Json(request): Json<CommandRequest>,
//! ) -> Result<Json<CommandResponse>, AppError> {
//!     // 1. Build action from request
//!     let action = AuthAction::RegisterUser { ... };
//!
//!     // 2. Dispatch through store
//!     state.auth_store.dispatch(action).await?;
//!
//!     // 3. Return response
//!     Ok(Json(CommandResponse { success: true }))
//! }
//!
//! let app = Router::new()
//!     .route("/api/v1/auth/register", post(handle_command))
//!     .with_state(app_state);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod error;
pub mod extractors;
pub mod handlers;
pub mod middleware;
pub mod state;

// Re-export key types for convenience
pub use error::AppError;
pub use extractors::{ClientIp, CorrelationId, UserAgent};
pub use middleware::{correlation_id_layer, CorrelationIdExt, CORRELATION_ID_HEADER};
pub use state::AppState;

/// Result type alias for web handlers.
pub type WebResult<T> = Result<T, AppError>;
