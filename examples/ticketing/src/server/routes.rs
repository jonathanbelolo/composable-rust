//! Router configuration for the ticketing system using next-generation handlers.
//!
//! All domain routes use the `BusinessLogic` + `Handler` pattern from `next::http`.

use crate::auth::handlers;
use crate::auth::setup::TicketingAuthStore;
use crate::config::Config;
use crate::next::http::{
    analytics_routes, availability_routes, events_v2_routes, payment_routes, query_routes,
    reservation_query_routes, reservation_routes, AnalyticsAppState, FullQueryAppState,
    NextAppState, QueryAppState, ReservationAppState, ReservationQueryAppState,
};
use axum::{
    extract::FromRef,
    middleware as axum_middleware,
    routing::{get, post},
    Router,
};
use composable_rust_web::middleware::correlation_id_layer;
use std::sync::Arc;

use super::health::{health_check, readiness_check};
use super::metrics::metrics_routes;
use super::middleware::metrics_middleware;

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

/// Build the complete router with all next-generation handlers.
///
/// Each route group is bound to its specific state before merging.
pub fn build_router(
    auth_state: AuthAppState,
    next_state: NextAppState,
    query_state: QueryAppState,
    full_query_state: FullQueryAppState,
    analytics_state: AnalyticsAppState,
    reservation_query_state: ReservationQueryAppState,
    reservation_state: ReservationAppState,
) -> Router {
    // Auth routes with their own state
    let auth_routes = Router::new()
        .route("/magic-link/request", post(handlers::send_magic_link))
        .route("/magic-link/verify", post(handlers::verify_magic_link))
        .with_state(auth_state);

    // API routes - each bound to its specific state
    let api_routes = Router::new()
        // Event write routes (need projection data for authorization)
        .merge(events_v2_routes().with_state(full_query_state.clone()))
        // Event query routes
        .merge(query_routes().with_state(query_state))
        // Availability routes
        .merge(availability_routes().with_state(full_query_state.clone()))
        // Payment routes
        .merge(payment_routes().with_state(full_query_state))
        // Reservation saga routes
        .merge(reservation_routes().with_state(reservation_state))
        // Reservation query routes
        .merge(reservation_query_routes().with_state(reservation_query_state))
        // Analytics routes
        .merge(analytics_routes().with_state(analytics_state));

    Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        .merge(metrics_routes())
        .nest("/auth", auth_routes)
        .nest("/api/v2", api_routes)
        .layer(axum_middleware::from_fn(metrics_middleware))
        .layer(correlation_id_layer())
}
