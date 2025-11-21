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
#[derive(Serialize)]
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
/// Returns 503 Service Unavailable if any critical dependency is unhealthy.
///
/// Checks:
/// - Event store database (PostgreSQL)
/// - Projections database (PostgreSQL)
/// - Auth database (PostgreSQL)
/// - Redis (not yet used in application)
/// - Event bus (complex to check without trait extension)
///
/// This is used by Kubernetes readiness probes to determine if
/// the pod should receive traffic.
///
/// # Response Format
///
/// ```json
/// {
///   "ready": true,
///   "duration_ms": 45,
///   "components": {
///     "event_store": {"healthy": true, "duration_ms": 12},
///     "projections_db": {"healthy": true, "duration_ms": 10},
///     "auth_db": {"healthy": true, "duration_ms": 8},
///     "redis": {"healthy": true, "duration_ms": 0},
///     "event_bus": {"healthy": true, "duration_ms": 0}
///   }
/// }
/// ```
pub async fn readiness_check(State(state): State<AppState>) -> (StatusCode, Json<ReadinessResponse>) {
    use std::time::Instant;

    let start = Instant::now();

    // Check event store database connectivity
    let event_store_status = check_database_health_detailed(state.event_store.pool(), "event_store").await;

    // Check projections database connectivity
    let projections_status = check_database_health_detailed(state.available_seats_projection.pool(), "projections_db").await;

    // Check auth database connectivity
    let auth_db_status = check_database_health_detailed(&state.auth_pool, "auth_db").await;

    // Redis is not yet used in the application. When Redis is added for caching or
    // session storage, implement health check with: redis.ping().await
    let redis_status = ComponentStatus {
        healthy: true,
        duration_ms: 0,
        error: None,
    };

    // Event bus health check is complex (requires adding health check method to EventBus trait).
    // For now, event bus failures will surface through event publishing errors.
    // Future: Add health_check() method to EventBus trait and implement for RedpandaEventBus
    let event_bus_status = ComponentStatus {
        healthy: true,
        duration_ms: 0,
        error: None,
    };

    // Overall readiness: all components must be healthy
    let ready = event_store_status.healthy
        && projections_status.healthy
        && auth_db_status.healthy
        && redis_status.healthy
        && event_bus_status.healthy;

    let total_duration = start.elapsed();

    let status_code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status_code,
        Json(ReadinessResponse {
            ready,
            duration_ms: total_duration.as_millis() as u64,
            components: ComponentHealth {
                event_store: event_store_status,
                projections_db: projections_status,
                auth_db: auth_db_status,
                redis: redis_status,
                event_bus: event_bus_status,
            },
        }),
    )
}

/// Check database health with detailed status information.
///
/// Executes `SELECT 1` to verify database connectivity with timing information.
/// Times out after 5 seconds to avoid hanging the health check.
///
/// # Arguments
///
/// - `pool`: PostgreSQL connection pool to check
/// - `name`: Component name for logging (e.g., "event_store")
///
/// # Returns
///
/// [`ComponentStatus`] with health status, duration, and optional error message.
async fn check_database_health_detailed(pool: &sqlx::PgPool, name: &str) -> ComponentStatus {
    use std::time::{Duration, Instant};

    let start = Instant::now();

    // Simple connectivity check with timeout
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        sqlx::query("SELECT 1").execute(pool),
    )
    .await;

    let duration_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(Ok(_)) => ComponentStatus {
            healthy: true,
            duration_ms,
            error: None,
        },
        Ok(Err(e)) => {
            let error_msg = format!("Database query failed: {e}");
            tracing::warn!("{name} health check failed: {error_msg}");
            ComponentStatus {
                healthy: false,
                duration_ms,
                error: Some(error_msg),
            }
        }
        Err(_) => {
            let error_msg = "Health check timed out after 5 seconds".to_string();
            tracing::warn!("{name} health check timed out");
            ComponentStatus {
                healthy: false,
                duration_ms,
                error: Some(error_msg),
            }
        }
    }
}

