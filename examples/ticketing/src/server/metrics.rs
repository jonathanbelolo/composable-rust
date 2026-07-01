//! Prometheus metrics endpoint.
//!
//! Exposes `/metrics` endpoint in Prometheus text format for scraping.

use axum::{Router, response::IntoResponse, routing::get};
use prometheus::{Encoder, TextEncoder};

/// Create metrics routes.
///
/// Returns a router with the `/metrics` endpoint.
/// Generic over state since handler doesn't require any state.
pub fn metrics_routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/metrics", get(metrics_handler))
}

/// Handler for the `/metrics` endpoint.
///
/// Returns all registered Prometheus metrics in text format.
///
/// # Errors
///
/// Returns 500 Internal Server Error if encoding fails.
async fn metrics_handler() -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = vec![];

    // Encode metrics to Prometheus text format
    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        tracing::error!(error = %e, "Failed to encode metrics");
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, vec![]);
    }

    (axum::http::StatusCode::OK, buffer)
}
