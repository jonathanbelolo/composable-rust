//! Health check endpoints.
//!
//! These endpoints are used by load balancers and monitoring systems
//! to verify service health.

use axum::{extract::State, http::StatusCode, Json};
use composable_rust_core::reducer::Reducer;
use composable_rust_runtime::{HealthCheck, HealthStatus, Store};
use std::sync::Arc;

/// Simple health check endpoint (for basic liveness).
///
/// Returns 200 OK to indicate the service is running.
/// This endpoint does NOT check dependencies (database, etc.).
///
/// # Endpoint
///
/// ```text
/// GET /health
/// ```
///
/// # Response
///
/// ```json
/// {
///   "status": "ok"
/// }
/// ```
#[allow(clippy::unused_async)]
pub async fn health_check() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}

/// Health check with Store diagnostics (for readiness).
///
/// Returns health status based on Store health, including DLQ status
/// and any other configured health checks.
///
/// # Status Codes
///
/// - 200 OK: Healthy or Degraded
/// - 503 Service Unavailable: Unhealthy
///
/// # Endpoint
///
/// ```text
/// GET /health/ready
/// ```
///
/// # Response
///
/// ```json
/// {
///   "component": "store",
///   "status": "Healthy",
///   "message": "Store is healthy",
///   "metadata": null
/// }
/// ```
pub async fn health_check_with_store<S, A, E, R>(
    State(store): State<Arc<Store<S, A, E, R>>>,
) -> (StatusCode, Json<HealthCheck>)
where
    R: Reducer<State = S, Action = A, Environment = E> + Send + Sync + 'static,
    S: Send + Sync + 'static,
    A: Send + Clone + 'static,
    E: Send + Sync + 'static,
{
    let health = store.health();

    let status = match health.status {
        HealthStatus::Healthy | HealthStatus::Degraded => StatusCode::OK,
        HealthStatus::Unhealthy => StatusCode::SERVICE_UNAVAILABLE,
    };

    (status, Json(health))
}

#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_core::{effect::Effect, SmallVec};

    #[tokio::test]
    async fn test_simple_health_check() {
        let (status, body) = health_check().await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "ok");
    }

    #[tokio::test]
    async fn test_health_check_with_healthy_store() {
        // Create a simple test reducer
        #[derive(Clone)]
        struct TestReducer;

        #[derive(Clone, Default)]
        struct TestState;

        #[derive(Clone)]
        struct TestAction;

        #[derive(Clone)]
        struct TestEnv;

        impl Reducer for TestReducer {
            type State = TestState;
            type Action = TestAction;
            type Environment = TestEnv;

            fn reduce(
                &self,
                _state: &mut Self::State,
                _action: Self::Action,
                _env: &Self::Environment,
            ) -> SmallVec<[Effect<Self::Action>; 4]> {
                SmallVec::new()
            }
        }

        let store = Arc::new(Store::new(TestState, TestReducer, TestEnv));

        let (status, Json(health)) = health_check_with_store(State(store)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(health.status, HealthStatus::Healthy);
    }
}
