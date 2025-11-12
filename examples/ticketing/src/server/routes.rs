//! Router configuration for the ticketing system.
//!
//! Builds the complete Axum router with all endpoints.

use super::state::AppState;
use super::health::{health_check, readiness_check};
use axum::{
    routing::get,
    Router,
};

/// Build the complete Axum router.
///
/// Configures all routes including:
/// - Health checks
/// - Authentication endpoints (via framework's auth_router)
/// - Event management endpoints
/// - Reservation endpoints
/// - Payment endpoints
/// - Analytics endpoints
///
/// # Arguments
///
/// - `state`: Application state to share with handlers
///
/// # Returns
///
/// Configured Axum router ready to serve requests.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        // Health checks (no authentication)
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        // TODO: Add authentication routes (framework's auth_router)
        // TODO: Add API routes (events, reservations, payments, analytics)
        .with_state(state)
}
