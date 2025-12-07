//! Thin Server Example
//!
//! This example demonstrates the `pg-gateway` pattern where `PostgreSQL`
//! owns all business logic and Rust serves as a thin protocol adapter.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐     ┌─────────────────┐     ┌──────────────┐
//! │   Client    │────▶│   Rust Gateway  │────▶│  PostgreSQL  │
//! │   (HTTP)    │◀────│  (This Example) │◀────│   (Logic)    │
//! └─────────────┘     └─────────────────┘     └──────────────┘
//! ```
//!
//! # Running
//!
//! 1. Start `PostgreSQL` (see README.md for schema setup)
//! 2. Set `DATABASE_URL` environment variable
//! 3. Run: `cargo run -p thin-server-example`
//!
//! # Endpoints
//!
//! - `GET /health` - Health check
//! - `POST /api/items` - Create an item (requires auth)
//! - `GET /api/items/{id}` - Get an item (requires auth)

use anyhow::Result;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use composable_rust_pg_gateway::{
    create_pool, execute_with_identity, ApiError, DbConfig, Identity,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tracing::info;

// ═══════════════════════════════════════════════════════════════════════════
// Application State
// ═══════════════════════════════════════════════════════════════════════════

/// Shared application state.
#[derive(Clone)]
struct AppState {
    pool: PgPool,
}

impl AppState {
    fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Request/Response Types
// ═══════════════════════════════════════════════════════════════════════════

/// Request to create an item.
#[derive(Debug, Deserialize)]
struct CreateItemRequest {
    name: String,
    description: Option<String>,
}

/// Item response from `PostgreSQL`.
#[derive(Debug, Serialize, sqlx::FromRow)]
struct Item {
    id: i64,
    name: String,
    description: Option<String>,
    created_by: String,
    tenant_id: String,
}

// ═══════════════════════════════════════════════════════════════════════════
// Handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Create a new item.
///
/// In a real application, the `Identity` extractor would validate JWT/session.
/// Here we use a mock identity for demonstration.
///
/// # `PostgreSQL` Function Required
///
/// ```sql
/// CREATE FUNCTION items.create_item(p_name TEXT, p_description TEXT)
/// RETURNS TABLE(id BIGINT, name TEXT, description TEXT, created_by TEXT, tenant_id TEXT)
/// AS $$
/// DECLARE
///     v_user_id TEXT := current_setting('app.user_id', true);
///     v_tenant_id TEXT := current_setting('app.tenant_id', true);
/// BEGIN
///     -- Validation
///     IF p_name IS NULL OR length(p_name) = 0 THEN
///         RAISE EXCEPTION 'Name is required' USING ERRCODE = 'P0001';
///     END IF;
///
///     -- Insert (PostgreSQL owns the business logic)
///     RETURN QUERY
///     INSERT INTO items.items (name, description, created_by, tenant_id)
///     VALUES (p_name, p_description, v_user_id, v_tenant_id)
///     RETURNING id, name, description, created_by, tenant_id;
/// END;
/// $$ LANGUAGE plpgsql;
/// ```
async fn create_item(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateItemRequest>,
) -> Result<(StatusCode, Json<Item>), ApiError> {
    // In production, this would come from JWT/session via the Identity extractor
    let identity = mock_identity();

    let item: Item = execute_with_identity(&state.pool, &identity, |conn| {
        let name = req.name.clone();
        let description = req.description.clone();
        Box::pin(async move {
            sqlx::query_as(
                "SELECT id, name, description, created_by, tenant_id
                 FROM items.create_item($1, $2)",
            )
            .bind(&name)
            .bind(&description)
            .fetch_one(conn)
            .await
        })
    })
    .await?;

    info!(item_id = item.id, "Item created");

    Ok((StatusCode::CREATED, Json(item)))
}

/// Get an item by ID.
///
/// # `PostgreSQL` Function Required
///
/// ```sql
/// CREATE FUNCTION items.get_item(p_id BIGINT)
/// RETURNS TABLE(id BIGINT, name TEXT, description TEXT, created_by TEXT, tenant_id TEXT)
/// AS $$
/// DECLARE
///     v_tenant_id TEXT := current_setting('app.tenant_id', true);
/// BEGIN
///     RETURN QUERY
///     SELECT i.id, i.name, i.description, i.created_by, i.tenant_id
///     FROM items.items i
///     WHERE i.id = p_id AND i.tenant_id = v_tenant_id;
///
///     IF NOT FOUND THEN
///         RAISE EXCEPTION 'Item not found' USING ERRCODE = 'P0404';
///     END IF;
/// END;
/// $$ LANGUAGE plpgsql;
/// ```
async fn get_item(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<i64>,
) -> Result<Json<Item>, ApiError> {
    let identity = mock_identity();

    let item: Item = execute_with_identity(&state.pool, &identity, |conn| {
        Box::pin(async move {
            sqlx::query_as(
                "SELECT id, name, description, created_by, tenant_id
                 FROM items.get_item($1)",
            )
            .bind(item_id)
            .fetch_one(conn)
            .await
        })
    })
    .await?;

    Ok(Json(item))
}

/// Create a mock identity for demonstration.
///
/// In production, use the `Identity` extractor with JWT/session validation.
fn mock_identity() -> Identity {
    Identity::new(
        "demo-user-123",
        "demo-tenant-456",
        vec!["user".to_string()],
    )
}

// ═══════════════════════════════════════════════════════════════════════════
// Router
// ═══════════════════════════════════════════════════════════════════════════

/// Custom health check that extracts the pool from `AppState`.
async fn app_health_check(State(state): State<Arc<AppState>>) -> impl axum::response::IntoResponse {
    let healthy = composable_rust_pg_gateway::check_database(&state.pool).await;

    let status = if healthy {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };

    let response = composable_rust_pg_gateway::HealthResponse {
        status: if healthy {
            composable_rust_pg_gateway::HealthStatus::Healthy
        } else {
            composable_rust_pg_gateway::HealthStatus::Unhealthy
        },
        checks: composable_rust_pg_gateway::HealthChecks {
            database: healthy,
            outbox: None,
        },
    };

    (status, axum::Json(response))
}

fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        // Health check
        .route("/health", get(app_health_check))
        // API routes
        .route("/api/items", post(create_item))
        .route("/api/items/{id}", get(get_item))
        // State
        .with_state(state)
}

// ═══════════════════════════════════════════════════════════════════════════
// Main
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "thin_server_example=info,composable_rust_pg_gateway=info".into()),
        )
        .init();

    info!("Starting thin server example");

    // Load configuration
    let config = DbConfig::from_env().map_err(|e| anyhow::anyhow!("{e}"))?;

    // Create database pool
    let pool = create_pool(&config).await?;
    info!("Connected to database");

    // Build application state
    let state = AppState::new(pool);

    // Build router
    let app = create_router(state);

    // Start server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    info!("Listening on http://0.0.0.0:3000");

    axum::serve(listener, app).await?;

    Ok(())
}
