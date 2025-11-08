//! Authentication reducers.
//!
//! This module contains pure reducer functions for authentication.
//!
//! Reducers are pure functions: `(State, Action, Environment) → (State, Effects)`.

pub mod oauth;

// Re-export
pub use oauth::OAuthReducer;
