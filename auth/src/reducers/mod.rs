//! Authentication reducers.
//!
//! This module contains pure reducer functions for authentication.
//!
//! Reducers are pure functions: `(State, Action, Environment) → (State, Effects)`.

pub mod magic_link;
pub mod oauth;

// Re-export
pub use magic_link::MagicLinkReducer;
pub use oauth::OAuthReducer;
