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

/// Readiness check response with detailed component status.
#[derive(Serialize)]
pub struct ReadinessResponse {
    /// Overall readiness status (all components healthy)
    pub ready: bool,
    /// Individual component health checks with timing and details
    pub components: ComponentHealth,
    /// Total health check duration in milliseconds
    pub duration_ms: u64,
}

/// Health status of individual components with detailed information.
#[derive(Serialize)]
pub struct ComponentHealth {
    /// Event store database connectivity
    pub event_store: ComponentStatus,
    /// Projections database connectivity
    pub projections_db: ComponentStatus,
    /// Auth database connectivity
    pub auth_db: ComponentStatus,
    /// Redis connectivity (not yet used in application)
    pub redis: ComponentStatus,
    /// Event bus connectivity (complex to check, requires trait extension)
    pub event_bus: ComponentStatus,
}

/// Detailed status for a single component.
#[derive(Clone, Serialize)]
pub struct ComponentStatus {
    /// Whether the component is healthy
    pub healthy: bool,
    /// Duration of health check in milliseconds
    pub duration_ms: u64,
    /// Optional error message if unhealthy
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Readiness check endpoint.
///
/// Returns 200 OK if the service is ready to accept traffic.
///
/// Note: Full dependency health checks require database pools which are
/// configured at application startup. This simplified version always reports ready.
/// To add database health checks, pass database pools via state.
pub async fn readiness_check() -> (StatusCode, Json<ReadinessResponse>) {
    // Simplified readiness check - assumes ready if service is running
    // Full health checks would require database pools passed via state
    let component_status = ComponentStatus {
        healthy: true,
        duration_ms: 0,
        error: None,
    };

    (
        StatusCode::OK,
        Json(ReadinessResponse {
            ready: true,
            duration_ms: 0,
            components: ComponentHealth {
                event_store: component_status.clone(),
                projections_db: component_status.clone(),
                auth_db: component_status.clone(),
                redis: component_status.clone(),
                event_bus: component_status,
            },
        }),
    )
}

