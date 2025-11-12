//! Router configuration for the ticketing system.
//!
//! Builds the complete Axum router with all endpoints.

use super::state::AppState;
use super::health::{health_check, readiness_check};
use crate::api::events;
use axum::{
    routing::{get, post, put, delete},
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
    // API routes
    let api_routes = Router::new()
        // Event management
        .route("/events", post(events::create_event))
        .route("/events", get(events::list_events))
        .route("/events/:id", get(events::get_event))
        .route("/events/:id", put(events::update_event))
        .route("/events/:id", delete(events::delete_event));
        // TODO: Add availability routes
        // TODO: Add reservation routes
        // TODO: Add payment routes
        // TODO: Add analytics routes

    Router::new()
        // Health checks (no authentication)
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        // TODO: Add authentication routes (framework's auth_router)
        // API routes under /api prefix
        .nest("/api", api_routes)
        .with_state(state)
}
