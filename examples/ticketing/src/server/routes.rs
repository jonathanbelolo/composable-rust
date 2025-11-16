//! Router configuration for the ticketing system.
//!
//! Builds the complete Axum router with all endpoints.

use super::health::{health_check, readiness_check};
use super::state::AppState;
use crate::api::{analytics, availability, events, payments, reservations, websocket};
use axum::{
    routing::{delete, get, post, put},
    Router,
};

/// Build the complete Axum router.
///
/// Configures all routes including:
/// - Health checks
/// - Authentication endpoints (via framework's `auth_router`)
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
        .route("/events/:id", delete(events::delete_event))
        // Availability queries (CQRS read side)
        .route(
            "/events/:id/availability",
            get(availability::get_event_availability),
        )
        .route(
            "/events/:id/sections/:section/availability",
            get(availability::get_section_availability),
        )
        .route(
            "/events/:id/total-available",
            get(availability::get_total_available),
        )
        // Reservation management (saga-coordinated)
        .route("/reservations", post(reservations::create_reservation))
        .route("/reservations", get(reservations::list_user_reservations))
        .route("/reservations/:id", get(reservations::get_reservation))
        .route(
            "/reservations/:id/cancel",
            post(reservations::cancel_reservation),
        )
        // Payment processing
        .route("/payments", post(payments::process_payment))
        .route("/payments", get(payments::list_user_payments))
        .route("/payments/:id", get(payments::get_payment))
        .route("/payments/:id/refund", post(payments::refund_payment))
        // Analytics and reporting
        .route(
            "/analytics/events/:id/sales",
            get(analytics::get_event_sales),
        )
        .route(
            "/analytics/events/:id/sections/popular",
            get(analytics::get_popular_sections),
        )
        .route("/analytics/revenue", get(analytics::get_total_revenue))
        .route(
            "/analytics/customers/top-spenders",
            get(analytics::get_top_spenders),
        )
        .route(
            "/analytics/customers/:id/profile",
            get(analytics::get_customer_profile),
        )
        // WebSocket endpoints
        .route(
            "/ws/availability/:event_id",
            get(websocket::availability_updates),
        )
        .route(
            "/ws/notifications",
            get(websocket::personal_notifications),
        );

    Router::new()
        // Health checks (no authentication)
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        // TODO: Add authentication routes (framework's auth_router)
        // API routes under /api prefix
        .nest("/api", api_routes)
        .with_state(state)
}
