//! HTTP server with health checks and metrics

use crate::environment::ProductionEnvironment;
use crate::reducer::ProductionAgentReducer;
use crate::types::{AgentAction, AgentState};
use axum::{
    extract::State as AxumState,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use composable_rust_agent_patterns::health::{SystemHealthCheck, HealthStatus};
use composable_rust_agent_patterns::AgentMetrics;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;
use tracing::info;

/// Server state
#[derive(Clone)]
pub struct ServerState {
    /// Agent reducer
    pub reducer: Arc<ProductionAgentReducer>,
    /// Agent environment
    pub environment: Arc<ProductionEnvironment>,
    /// Agent state (per-session, in production use session store)
    pub agent_state: Arc<RwLock<AgentState>>,
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
pub async fn create_server(
    reducer: Arc<ProductionAgentReducer>,
    environment: Arc<ProductionEnvironment>,
    metrics: Arc<AgentMetrics>,
    health_registry: Arc<SystemHealthCheck>,
) -> Router {
    let agent_state = Arc::new(RwLock::new(AgentState::new()));

    let state = ServerState {
        reducer,
        environment,
        agent_state,
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
async fn chat_handler(
    AxumState(state): AxumState<ServerState>,
    Json(req): Json<ChatRequest>,
) -> Response {
    info!("Received chat request from user: {}", req.user_id);

    // Get agent state
    let mut agent_state = state.agent_state.write().await;

    // Start conversation if needed
    if agent_state.conversation_id.is_none() {
        let effects = state.reducer.reduce(
            &mut agent_state,
            AgentAction::StartConversation {
                user_id: req.user_id.clone(),
                session_id: req.session_id.clone(),
            },
            state.environment.as_ref(),
        );

        // Execute effects (simplified - in production use Store)
        for effect in effects {
            if let composable_rust_core::effect::Effect::Future(fut) = effect {
                let _ = fut.await;
            }
        }
    }

    // Send message
    let effects = state.reducer.reduce(
        &mut agent_state,
        AgentAction::SendMessage {
            content: req.message.clone(),
            source_ip: req.source_ip,
        },
        state.environment.as_ref(),
    );

    // Execute effects
    let mut response_text = String::new();
    for effect in effects {
        if let composable_rust_core::effect::Effect::Future(fut) = effect {
            if let Some(action) = fut.await {
                if let AgentAction::ProcessResponse { response } = action {
                    response_text = response;
                    break;
                }
            }
        }
    }

    // Process response
    if !response_text.is_empty() {
        state.reducer.reduce(
            &mut agent_state,
            AgentAction::ProcessResponse {
                response: response_text.clone(),
            },
            state.environment.as_ref(),
        );
    }

    let response = ChatResponse {
        message: response_text,
        conversation_id: agent_state.conversation_id.clone(),
    };

    Json(response).into_response()
}

/// Health check handler
async fn health_handler(AxumState(state): AxumState<ServerState>) -> Response {
    let results = state.health_registry.check_all().await;
    let all_healthy = results.values().all(|r| r.status == HealthStatus::Healthy);

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
async fn readiness_handler(AxumState(state): AxumState<ServerState>) -> Response {
    let results = state.health_registry.check_all().await;
    let all_ready = results.values().all(|r| r.status == HealthStatus::Healthy);

    if all_ready {
        (StatusCode::OK, "ready").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response()
    }
}

/// Metrics handler
async fn metrics_handler(AxumState(state): AxumState<ServerState>) -> Response {
    let snapshot = state.metrics.snapshot();
    (StatusCode::OK, format!("{:?}", snapshot)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_agent_patterns::audit::InMemoryAuditLogger;
    use composable_rust_agent_patterns::security::SecurityMonitor;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_endpoint() {
        let audit_logger = Arc::new(InMemoryAuditLogger::new());
        let security_monitor = Arc::new(SecurityMonitor::new());
        let environment = Arc::new(ProductionEnvironment::new(
            audit_logger.clone(),
            security_monitor.clone(),
        ));
        let reducer = Arc::new(ProductionAgentReducer::new(audit_logger, security_monitor));
        let metrics = Arc::new(AgentMetrics::new());
        let health_registry = Arc::new(SystemHealthCheck::new());

        let app = create_server(reducer, environment, metrics, health_registry).await;

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
        let environment = Arc::new(ProductionEnvironment::new(
            audit_logger.clone(),
            security_monitor.clone(),
        ));
        let reducer = Arc::new(ProductionAgentReducer::new(audit_logger, security_monitor));
        let metrics = Arc::new(AgentMetrics::new());
        let health_registry = Arc::new(SystemHealthCheck::new());

        let app = create_server(reducer, environment, metrics, health_registry).await;

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
