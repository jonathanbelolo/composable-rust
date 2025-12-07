//! Health check endpoints for the thin-server pattern.
//!
//! Provides health check handlers that verify database connectivity
//! and optional outbox queue health.
//!
//! # Example
//!
//! ```rust,ignore
//! use axum::{Router, routing::get};
//! use composable_rust_pg_gateway::pool::{health_check, health_check_with_outbox};
//!
//! let app = Router::new()
//!     .route("/health", get(health_check))
//!     .with_state(pool);
//! ```
//!
//! # Response Format
//!
//! ```json
//! {
//!     "status": "healthy",
//!     "checks": {
//!         "database": true,
//!         "outbox": true
//!     }
//! }
//! ```

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Serialize;
use sqlx::PgPool;

// ═══════════════════════════════════════════════════════════════════════════
// Response Types
// ═══════════════════════════════════════════════════════════════════════════

/// Health check response.
///
/// # Example
///
/// ```rust
/// use composable_rust_pg_gateway::pool::{HealthResponse, HealthStatus, HealthChecks};
///
/// let response = HealthResponse {
///     status: HealthStatus::Healthy,
///     checks: HealthChecks {
///         database: true,
///         outbox: None,
///     },
/// };
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct HealthResponse {
    /// Overall health status.
    pub status: HealthStatus,

    /// Individual check results.
    pub checks: HealthChecks,
}

/// Overall health status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// All checks passed.
    Healthy,
    /// One or more checks failed.
    Unhealthy,
}

/// Individual health check results.
#[derive(Debug, Clone, Serialize)]
pub struct HealthChecks {
    /// Database connectivity check.
    pub database: bool,

    /// Outbox queue health (if enabled).
    ///
    /// Only included when outbox monitoring is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outbox: Option<bool>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Configuration
// ═══════════════════════════════════════════════════════════════════════════

/// Configuration for health checks with outbox monitoring.
///
/// Used with [`health_check_with_outbox`] to configure stuck task detection.
///
/// # Example
///
/// ```rust
/// use composable_rust_pg_gateway::pool::HealthConfig;
///
/// let config = HealthConfig::new()
///     .with_stuck_task_threshold(100)
///     .with_stuck_duration_secs(300); // 5 minutes
/// ```
#[derive(Debug, Clone)]
pub struct HealthConfig {
    /// Maximum number of stuck tasks before unhealthy.
    pub stuck_task_threshold: i64,

    /// Duration in seconds after which a pending task is considered stuck.
    ///
    /// Default: 300 (5 minutes)
    pub stuck_duration_secs: i64,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            stuck_task_threshold: 100,
            stuck_duration_secs: 300, // 5 minutes
        }
    }
}

impl HealthConfig {
    /// Create a new configuration with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the threshold for stuck tasks.
    ///
    /// If the number of stuck tasks exceeds this threshold, the health check fails.
    #[must_use]
    pub const fn with_stuck_task_threshold(mut self, threshold: i64) -> Self {
        self.stuck_task_threshold = threshold;
        self
    }

    /// Set the duration after which tasks are considered stuck.
    ///
    /// Tasks pending longer than this duration count toward the threshold.
    #[must_use]
    pub const fn with_stuck_duration_secs(mut self, seconds: i64) -> Self {
        self.stuck_duration_secs = seconds;
        self
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Basic health check endpoint.
///
/// Verifies database connectivity with a simple `SELECT 1` query.
///
/// # Route
///
/// `GET /health`
///
/// # Response
///
/// - `200 OK` - Database is healthy
/// - `503 Service Unavailable` - Database is unhealthy
///
/// # Example
///
/// ```rust,ignore
/// use axum::{Router, routing::get};
/// use composable_rust_pg_gateway::pool::health_check;
///
/// let app = Router::new()
///     .route("/health", get(health_check))
///     .with_state(pool);
/// ```
pub async fn health_check(State(pool): State<PgPool>) -> impl IntoResponse {
    let db_healthy = check_database(&pool).await;

    let status = if db_healthy {
        HealthStatus::Healthy
    } else {
        HealthStatus::Unhealthy
    };

    let http_status = if db_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let response = HealthResponse {
        status,
        checks: HealthChecks {
            database: db_healthy,
            outbox: None,
        },
    };

    (http_status, Json(response))
}

/// Health check with outbox monitoring.
///
/// Verifies database connectivity and checks for stuck tasks in the outbox queue.
/// Tasks are considered stuck if they've been pending longer than the configured duration.
///
/// # Route
///
/// `GET /health`
///
/// # Response
///
/// - `200 OK` - All checks passed
/// - `503 Service Unavailable` - One or more checks failed
///
/// # Example
///
/// ```rust,ignore
/// use axum::{Router, routing::get};
/// use composable_rust_pg_gateway::pool::{health_check_with_outbox, HealthConfig};
///
/// let config = HealthConfig::new()
///     .with_stuck_task_threshold(50)
///     .with_stuck_duration_secs(300);
///
/// let app = Router::new()
///     .route("/health", get(health_check_with_outbox))
///     .with_state((pool, config));
/// ```
pub async fn health_check_with_outbox(
    State(pool): State<PgPool>,
    State(config): State<HealthConfig>,
) -> impl IntoResponse {
    let db_healthy = check_database(&pool).await;
    let outbox_healthy = check_outbox(&pool, config.stuck_task_threshold, config.stuck_duration_secs).await;

    let all_healthy = db_healthy && outbox_healthy;

    let status = if all_healthy {
        HealthStatus::Healthy
    } else {
        HealthStatus::Unhealthy
    };

    let http_status = if all_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let response = HealthResponse {
        status,
        checks: HealthChecks {
            database: db_healthy,
            outbox: Some(outbox_healthy),
        },
    };

    (http_status, Json(response))
}

// ═══════════════════════════════════════════════════════════════════════════
// Check Functions
// ═══════════════════════════════════════════════════════════════════════════

/// Check database connectivity.
///
/// Executes a simple `SELECT 1` query to verify the database is reachable.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::pool::check_database;
///
/// let healthy = check_database(&pool).await;
/// ```
pub async fn check_database(pool: &PgPool) -> bool {
    sqlx::query("SELECT 1")
        .fetch_one(pool)
        .await
        .is_ok()
}

/// Check outbox queue health.
///
/// Queries the outbox for tasks that have been pending longer than the specified
/// duration (stuck tasks). If the count exceeds the threshold, the outbox is unhealthy.
///
/// # Arguments
///
/// * `pool` - Database connection pool
/// * `threshold` - Maximum allowed stuck tasks before unhealthy
/// * `stuck_duration_secs` - Seconds after which a pending task is considered stuck
///
/// # `PostgreSQL` Schema Requirement
///
/// This function expects the following table to exist:
///
/// ```sql
/// CREATE TABLE outbox.pending_tasks (
///     id BIGSERIAL PRIMARY KEY,
///     status TEXT NOT NULL,
///     scheduled_for TIMESTAMPTZ NOT NULL,
///     -- other columns...
/// );
/// ```
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::pool::check_outbox;
///
/// // Check for tasks stuck longer than 5 minutes, threshold of 100
/// let healthy = check_outbox(&pool, 100, 300).await;
/// ```
pub async fn check_outbox(pool: &PgPool, threshold: i64, stuck_duration_secs: i64) -> bool {
    // PostgreSQL's make_interval expects a float for seconds.
    // Precision loss is negligible for realistic durations (< 1 year = 31M seconds).
    #[allow(clippy::cast_precision_loss)]
    let duration_float = stuck_duration_secs as f64;

    let stuck_count: Result<Option<i64>, _> = sqlx::query_scalar(
        "SELECT COUNT(*) FROM outbox.pending_tasks
         WHERE status = 'pending' AND scheduled_for < now() - make_interval(secs => $1)",
    )
    .bind(duration_float)
    .fetch_one(pool)
    .await;

    match stuck_count {
        Ok(Some(count)) => count < threshold,
        Ok(None) => true, // No tasks = healthy
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Failed to query outbox health"
            );
            false
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn health_status_serializes_lowercase() {
        let healthy = serde_json::to_string(&HealthStatus::Healthy).unwrap();
        assert_eq!(healthy, "\"healthy\"");

        let unhealthy = serde_json::to_string(&HealthStatus::Unhealthy).unwrap();
        assert_eq!(unhealthy, "\"unhealthy\"");
    }

    #[test]
    fn health_response_serializes() {
        let response = HealthResponse {
            status: HealthStatus::Healthy,
            checks: HealthChecks {
                database: true,
                outbox: None,
            },
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"healthy\""));
        assert!(json.contains("\"database\":true"));
        // outbox should be omitted when None
        assert!(!json.contains("outbox"));
    }

    #[test]
    fn health_response_with_outbox_serializes() {
        let response = HealthResponse {
            status: HealthStatus::Unhealthy,
            checks: HealthChecks {
                database: true,
                outbox: Some(false),
            },
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"unhealthy\""));
        assert!(json.contains("\"database\":true"));
        assert!(json.contains("\"outbox\":false"));
    }

    #[test]
    fn health_config_defaults() {
        let config = HealthConfig::default();

        assert_eq!(config.stuck_task_threshold, 100);
        assert_eq!(config.stuck_duration_secs, 300);
    }

    #[test]
    fn health_config_builder() {
        let config = HealthConfig::new()
            .with_stuck_task_threshold(50)
            .with_stuck_duration_secs(600);

        assert_eq!(config.stuck_task_threshold, 50);
        assert_eq!(config.stuck_duration_secs, 600);
    }
}
