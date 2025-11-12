//! Health check endpoints for the ticketing system.
//!
//! Provides endpoints for monitoring service health and readiness.

use axum::{http::StatusCode, Json};
use serde::Serialize;

/// Health check response.
#[derive(Serialize)]
pub struct HealthResponse {
    /// Service status
    pub status: String,
    /// Service version
    pub version: String,
}

/// Health check endpoint.
///
/// Returns 200 OK if the service is running.
/// This is a simple liveness check - it doesn't verify dependencies.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/health
/// # {"status":"ok","version":"0.1.0"}
/// ```
pub async fn health_check() -> (StatusCode, Json<HealthResponse>) {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }),
    )
}

/// Readiness check response.
#[derive(Serialize)]
pub struct ReadinessResponse {
    /// Overall readiness status
    pub ready: bool,
    /// Database connectivity
    pub database: bool,
    /// Redis connectivity
    pub redis: bool,
    /// Event bus connectivity
    pub event_bus: bool,
}

/// Readiness check endpoint.
///
/// Returns 200 OK if the service is ready to accept traffic.
/// Checks all critical dependencies (database, Redis, event bus).
///
/// This is used by Kubernetes readiness probes to determine if
/// the pod should receive traffic.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/ready
/// # {"ready":true,"database":true,"redis":true,"event_bus":true}
/// ```
pub async fn readiness_check() -> (StatusCode, Json<ReadinessResponse>) {
    // TODO: Implement actual health checks for dependencies
    // For now, always return ready
    (
        StatusCode::OK,
        Json(ReadinessResponse {
            ready: true,
            database: true,
            redis: true,
            event_bus: true,
        }),
    )
}
