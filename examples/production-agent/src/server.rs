//! HTTP server with Store integration

use crate::environment::ProductionEnvironment;
use crate::reducer::ProductionAgentReducer;
use crate::types::{AgentAction, AgentEnvironment, AgentState};
use axum::{
    extract::State as AxumState,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use composable_rust_agent_patterns::audit::AuditLogger;
use composable_rust_agent_patterns::health::{HealthStatus, SystemHealthCheck};
use composable_rust_agent_patterns::AgentMetrics;
use composable_rust_runtime::Store;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use tracing::info;

/// Server state
#[derive(Clone)]
pub struct ServerState<A: AuditLogger + Send + Sync + Clone + 'static> {
    /// Agent store (manages state, reducer, environment)
    pub store: Arc<Store<AgentState, AgentAction, ProductionEnvironment<A>, ProductionAgentReducer<A>>>,
    /// Environment (for direct access to LLM calls)
    pub environment: Arc<ProductionEnvironment<A>>,
    /// Metrics
    pub metrics: Arc<AgentMetrics>,
    /// Health check registry
    pub health_registry: Arc<SystemHealthCheck>,
}

/// Chat request
#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    /// User ID
    pub user_id: String,
    /// Session ID
    pub session_id: String,
    /// Message content
    pub message: String,
    /// Source IP (optional)
    pub source_ip: Option<String>,
}

/// Chat response
#[derive(Debug, Serialize)]
pub struct ChatResponse {
    /// Response message
    pub message: String,
    /// Conversation ID
    pub conversation_id: Option<String>,
}

/// Error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Error message
    pub error: String,
}

/// Create HTTP server
pub async fn create_server<A: AuditLogger + Send + Sync + Clone + 'static>(
    store: Arc<Store<AgentState, AgentAction, ProductionEnvironment<A>, ProductionAgentReducer<A>>>,
    environment: Arc<ProductionEnvironment<A>>,
    metrics: Arc<AgentMetrics>,
    health_registry: Arc<SystemHealthCheck>,
) -> Router {
    let state = ServerState {
        store,
        environment,
        metrics,
        health_registry,
    };

    Router::new()
        .route("/chat", post(chat_handler))
        .route("/health", get(health_handler))
        .route("/health/live", get(liveness_handler))
        .route("/health/ready", get(readiness_handler))
        .route("/metrics", get(metrics_handler))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

/// Chat handler
#[tracing::instrument(skip(state))]
async fn chat_handler<A: AuditLogger + Send + Sync + Clone + 'static>(
    AxumState(state): AxumState<ServerState<A>>,
    Json(req): Json<ChatRequest>,
) -> Response {
    info!("Received chat request from user: {}", req.user_id);

    // Get current state
    let current_state = state.store.state(|s| s.clone()).await;

    // Start conversation if needed
    if current_state.conversation_id.is_none() {
        let _ = state
            .store
            .send(AgentAction::StartConversation {
                user_id: req.user_id.clone(),
                session_id: req.session_id.clone(),
            })
            .await;
    }

    // Send message (persists to EventStore)
    let _ = state
        .store
        .send(AgentAction::SendMessage {
            content: req.message.clone(),
            source_ip: req.source_ip,
        })
        .await;

    // Get updated state (with conversation_id)
    let current_state = state.store.state(|s| s.clone()).await;

    // Call LLM directly with current messages
    let response_text = match state.environment.call_llm(&current_state.messages).await {
        Ok(response) => response,
        Err(e) => {
            tracing::error!("LLM call failed: {:?}", e);
            format!("I apologize, but I encountered an error: {:?}", e)
        }
    };

    // Process response (persists to EventStore)
    if !response_text.is_empty() {
        let _ = state
            .store
            .send(AgentAction::ProcessResponse {
                response: response_text.clone(),
            })
            .await;
    }

    // Get final state
    let final_state = state.store.state(|s| s.clone()).await;

    let response = ChatResponse {
        message: response_text,
        conversation_id: final_state.conversation_id.clone(),
    };

    Json(response).into_response()
}

/// Health check handler
async fn health_handler<A: AuditLogger + Send + Sync + Clone + 'static>(
    AxumState(state): AxumState<ServerState<A>>,
) -> Response {
    let results = state.health_registry.check_all().await;
    let all_healthy = results
        .values()
        .all(|r| r.status == HealthStatus::Healthy);

    let status = if all_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (status, Json(results)).into_response()
}

/// Liveness handler
async fn liveness_handler() -> Response {
    // Simple liveness check (process is alive)
    (StatusCode::OK, "alive").into_response()
}

/// Readiness handler
async fn readiness_handler<A: AuditLogger + Send + Sync + Clone + 'static>(
    AxumState(state): AxumState<ServerState<A>>,
) -> Response {
    let results = state.health_registry.check_all().await;
    let all_ready = results
        .values()
        .all(|r| r.status == HealthStatus::Healthy);

    if all_ready {
        (StatusCode::OK, "ready").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response()
    }
}

/// Metrics handler
async fn metrics_handler<A: AuditLogger + Send + Sync + Clone + 'static>(
    AxumState(state): AxumState<ServerState<A>>,
) -> Response {
    let snapshot = state.metrics.snapshot();
    (StatusCode::OK, format!("{:?}", snapshot)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_agent_patterns::audit::InMemoryAuditLogger;
    use composable_rust_agent_patterns::security::SecurityMonitor;
    use composable_rust_core::environment::{Clock, SystemClock};
    use composable_rust_testing::mocks::InMemoryEventStore;
    use crate::environment::ProductionEnvironment;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_endpoint() {
        let audit_logger = Arc::new(InMemoryAuditLogger::new());
        let security_monitor = Arc::new(SecurityMonitor::new());
        let event_store: Arc<dyn composable_rust_core::event_store::EventStore> =
            Arc::new(InMemoryEventStore::new());
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let event_bus: Arc<dyn composable_rust_core::event_bus::EventBus> =
            Arc::new(composable_rust_testing::mocks::InMemoryEventBus::new());
        let pool = sqlx::PgPool::connect_lazy("postgres://test").expect("Test pool");
        let projection_store = Arc::new(composable_rust_projections::PostgresProjectionStore::new(
            pool,
            "test".to_string()
        ));

        let environment = Arc::new(ProductionEnvironment::new(
            audit_logger.clone(),
            security_monitor.clone(),
            event_store,
            clock,
            event_bus,
            projection_store,
        ));
        let reducer = ProductionAgentReducer::new(
            audit_logger,
            security_monitor,
        );
        let state = AgentState::new();
        let store = Arc::new(Store::new(state, reducer, (*environment).clone()));

        let metrics = Arc::new(AgentMetrics::new());
        let health_registry = Arc::new(SystemHealthCheck::new());

        let app = create_server(store, environment, metrics, health_registry).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_liveness_endpoint() {
        let audit_logger = Arc::new(InMemoryAuditLogger::new());
        let security_monitor = Arc::new(SecurityMonitor::new());
        let event_store: Arc<dyn composable_rust_core::event_store::EventStore> =
            Arc::new(InMemoryEventStore::new());
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let event_bus: Arc<dyn composable_rust_core::event_bus::EventBus> =
            Arc::new(composable_rust_testing::mocks::InMemoryEventBus::new());
        let pool = sqlx::PgPool::connect_lazy("postgres://test").expect("Test pool");
        let projection_store = Arc::new(composable_rust_projections::PostgresProjectionStore::new(
            pool,
            "test".to_string()
        ));

        let environment = Arc::new(ProductionEnvironment::new(
            audit_logger.clone(),
            security_monitor.clone(),
            event_store,
            clock,
            event_bus,
            projection_store,
        ));
        let reducer = ProductionAgentReducer::new(
            audit_logger,
            security_monitor,
        );
        let state = AgentState::new();
        let store = Arc::new(Store::new(state, reducer, (*environment).clone()));

        let metrics = Arc::new(AgentMetrics::new());
        let health_registry = Arc::new(SystemHealthCheck::new());

        let app = create_server(store, environment, metrics, health_registry).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health/live")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
