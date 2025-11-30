//! Router configuration for the ticketing system using next-generation handlers.
//!
//! All domain routes use the `BusinessLogic` + `Handler` pattern from `next::http`.

use crate::auth::setup::TicketingAuthStore;
use crate::config::Config;
use axum::extract::FromRef;
use std::sync::Arc;

/// Auth-specific state for authentication routes.
#[derive(Clone)]
pub struct AuthAppState {
    /// Auth store for session management
    pub auth_store: Arc<TicketingAuthStore>,
    /// Config for auth settings
    pub config: Arc<Config>,
}

impl FromRef<AuthAppState> for Arc<TicketingAuthStore> {
    fn from_ref(state: &AuthAppState) -> Self {
        state.auth_store.clone()
    }
}

impl FromRef<AuthAppState> for Arc<Config> {
    fn from_ref(state: &AuthAppState) -> Self {
        state.config.clone()
    }
}
