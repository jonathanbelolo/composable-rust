//! # Composable Rust Authentication & Authorization
//!
//! This crate provides composable, type-safe authentication and authorization
//! primitives that integrate natively with the Composable Rust architecture.
//!
//! ## Features
//!
//! - **Passwordless-first**: WebAuthn, magic links, OAuth2/OIDC
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
//! ## Example: OAuth2 Login
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
#![deny(clippy::unimplemented)]

// Public modules
pub mod actions;
pub mod effects;
pub mod environment;
pub mod error;
pub mod providers;
pub mod reducers;
pub mod state;

// Re-export main types for convenience
pub use actions::AuthAction;
pub use effects::AuthEffect;
pub use error::{AuthError, Result};
pub use state::{AuthState, Session, SessionId, TokenPair, UserId};
