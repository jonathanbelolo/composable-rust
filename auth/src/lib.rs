//! # Composable Rust Authentication & Authorization
//!
//! This crate provides composable, type-safe authentication and authorization
//! primitives that integrate natively with the Composable Rust architecture.
//!
//! ## Features
//!
//! - **Passwordless-first**: `WebAuthn`, magic links, `OAuth2`/OIDC
//! - **Composable**: Mix and match auth strategies
//! - **Type-safe**: Compile-time guarantees for permissions
//! - **Event-sourced**: Complete audit trail
//! - **Testable**: Auth logic runs at memory speed
//!
//! ## Architecture
//!
//! Authentication is implemented as reducers and effects:
//!
//! ```text
//! Action → Reducer → (State, Effects) → Effect Execution → More Actions
//! ```
//!
//! ## Example: `OAuth2` Login
//!
//! ```rust,ignore
//! use composable_rust_auth::*;
//!
//! // 1. Initiate OAuth login
//! let effects = reducer.reduce(
//!     &mut state,
//!     AuthAction::InitiateOAuth { provider: OAuthProvider::Google },
//!     &env,
//! );
//!
//! // 2. Execute effects (redirect to Google)
//! // 3. Handle callback
//! let effects = reducer.reduce(
//!     &mut state,
//!     AuthAction::OAuthCallback { code, state },
//!     &env,
//! );
//!
//! // 4. Session created
//! assert!(state.session.is_some());
//! ```

#![deny(missing_docs)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
#![deny(clippy::todo)]
// Auth code has legitimate complexity - allow pedantic warnings
#![allow(clippy::cognitive_complexity)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::type_complexity)]
#![deny(clippy::unimplemented)]

// Public modules
pub mod actions;
pub mod config;
pub mod constants;
pub mod effects;
pub mod environment;
pub mod error;
pub mod events;
pub mod providers;
pub mod reducers;
pub mod state;
pub mod stores;
pub mod utils;

// HTTP handlers and router (requires axum feature)
#[cfg(feature = "axum")]
pub mod handlers;

#[cfg(feature = "axum")]
pub mod router;

// Projection system (optional, requires postgres feature)
#[cfg(feature = "postgres")]
pub mod projection;

// Mock providers for testing
#[cfg(any(test, feature = "test-utils"))]
pub mod mocks;

// Re-export main types for convenience
pub use actions::AuthAction;
pub use effects::AuthEffect;
pub use environment::AuthEnvironment;
pub use error::{AuthError, Result};
pub use reducers::AuthReducer;
pub use state::{AuthState, Session, SessionId, TokenPair, UserId};
