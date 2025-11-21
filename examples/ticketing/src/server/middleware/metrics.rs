//! HTTP request metrics middleware.
//!
//! Records HTTP request count and duration for all requests.

use axum::{extract::Request, middleware::Next, response::Response};
use std::time::Instant;

/// Middleware that records HTTP request metrics.
///
/// Records the following metrics for each request:
/// - `http_requests_total`: Counter incremented for each request
/// - `http_request_duration_seconds`: Histogram recording request duration
///
/// Both metrics are labeled with:
/// - `method`: HTTP method (GET, POST, etc.)
/// - `path`: Request path
/// - `status`: Response status code
pub async fn metrics_middleware(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();

    let start = Instant::now();

    let response = next.run(request).await;

    let duration = start.elapsed();
    let status = response.status().as_u16().to_string();

    // Record metrics
    crate::metrics::HTTP_REQUESTS_TOTAL
        .with_label_values(&[method.as_str(), &path, &status])
        .inc();

    crate::metrics::HTTP_REQUEST_DURATION
        .with_label_values(&[method.as_str(), &path, &status])
        .observe(duration.as_secs_f64());

    response
}
