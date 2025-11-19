//! Health check endpoints for the ticketing system.
//!
//! Provides endpoints for monitoring service health and readiness.

use crate::server::state::AppState;
use axum::{extract::State, http::StatusCode, Json};
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
    /// Individual component health checks
    pub components: ComponentHealth,
}

/// Health status of individual components.
#[derive(Serialize)]
#[allow(clippy::struct_excessive_bools)] // Health check response naturally has multiple boolean flags
pub struct ComponentHealth {
    /// Event store database connectivity
    pub event_store: bool,
    /// Projections database connectivity
    pub projections_db: bool,
    /// Redis connectivity (not yet implemented)
    pub redis: bool,
    /// Event bus connectivity (not yet implemented)
    pub event_bus: bool,
}

/// Readiness check endpoint.
///
/// Returns 200 OK if the service is ready to accept traffic.
/// Returns 503 Service Unavailable if any critical dependency is unhealthy.
///
/// Checks:
/// - Event store database (PostgreSQL)
/// - Projections database (PostgreSQL)
/// - Redis (TODO: not yet implemented)
/// - Event bus (TODO: not yet implemented)
///
/// This is used by Kubernetes readiness probes to determine if
/// the pod should receive traffic.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/ready
/// # {"ready":true,"components":{"event_store":true,"projections_db":true,"redis":true,"event_bus":true}}
/// ```
pub async fn readiness_check(State(state): State<AppState>) -> (StatusCode, Json<ReadinessResponse>) {
    // Check event store database connectivity
    let event_store_healthy = check_database_health(state.event_store.pool()).await;

    // Check projections database connectivity
    let projections_healthy = check_database_health(state.available_seats_projection.pool()).await;

    // TODO: Implement Redis health check (not critical for MVP)
    let redis_healthy = true;

    // TODO: Implement event bus health check (not critical for MVP)
    let event_bus_healthy = true;

    // Overall readiness: all components must be healthy
    let ready = event_store_healthy && projections_healthy && redis_healthy && event_bus_healthy;

    let status_code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status_code,
        Json(ReadinessResponse {
            ready,
            components: ComponentHealth {
                event_store: event_store_healthy,
                projections_db: projections_healthy,
                redis: redis_healthy,
                event_bus: event_bus_healthy,
            },
        }),
    )
}

/// Check database health with a simple query.
///
/// Executes `SELECT 1` to verify database connectivity.
/// Times out after 5 seconds to avoid hanging the health check.
///
/// # Returns
///
/// `true` if database is healthy, `false` otherwise.
async fn check_database_health(pool: &sqlx::PgPool) -> bool {
    use std::time::Duration;

    // Simple connectivity check with timeout
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        sqlx::query("SELECT 1").execute(pool),
    )
    .await;

    match result {
        Ok(Ok(_)) => true,
        Ok(Err(e)) => {
            tracing::warn!("Database health check failed: {e}");
            false
        }
        Err(_) => {
            tracing::warn!("Database health check timed out after 5 seconds");
            false
        }
    }
}
