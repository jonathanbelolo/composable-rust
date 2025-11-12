//! Ticketing system HTTP server.
//!
//! Event-sourced ticketing platform with CQRS, sagas, and real-time updates.

use ticketing::{
    auth::setup::build_auth_store,
    config::Config,
    projections::setup_projection_managers,
    server::{build_router, AppState},
};
use composable_rust_postgres::PostgresEventStore;
use composable_rust_redpanda::RedpandaEventBus;
use std::sync::Arc;
use tokio::signal;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ticketing=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Ticketing System HTTP Server");

    // Load configuration
    let config = Config::from_env();
    info!(
        postgres_url = %config.postgres.url,
        projections_url = %config.projections.url,
        redpanda_brokers = %config.redpanda.brokers,
        "Configuration loaded"
    );

    // Setup event store (write side)
    info!("Connecting to event store database...");
    let event_store = Arc::new(PostgresEventStore::new(&config.postgres.url).await?);
    info!("Event store connected");

    // Setup event bus
    info!("Connecting to Redpanda event bus...");
    let event_bus: Arc<dyn composable_rust_core::event_bus::EventBus> = Arc::new(
        RedpandaEventBus::builder()
            .brokers(&config.redpanda.brokers)
            .consumer_group(&config.redpanda.consumer_group)
            .build()?,
    );
    info!("Event bus connected");

    // Setup PostgreSQL pool for auth
    info!("Connecting to auth database...");
    let auth_pg_pool = sqlx::PgPool::connect(&config.postgres.url).await?;

    // Setup authentication store
    info!("Initializing authentication store...");
    let auth_store = build_auth_store(&config, auth_pg_pool).await?;
    info!("Authentication store initialized");

    // Setup projections (read side)
    info!("Setting up projection managers...");
    let projection_managers = setup_projection_managers(&config, event_bus.clone()).await?;
    info!("Projection managers configured");

    // Start projection managers in background
    info!("Starting projection ETL services...");
    let projection_handles = projection_managers.start_all();
    info!(
        projection_count = projection_handles.len(),
        "Projection managers started"
    );

    // Get available seats projection for queries
    let available_seats_projection = Arc::new(
        ticketing::projections::PostgresAvailableSeatsProjection::new(
            Arc::new(
                sqlx::PgPool::connect(&config.projections.url).await?
            )
        )
    );

    // Build application state
    let state = AppState::new(
        auth_store,
        event_store,
        event_bus,
        available_seats_projection,
    );

    // Build router
    let app = build_router(state);

    // Create server address
    let addr = format!("{}:{}", config.server.host, config.server.port);
    info!(address = %addr, "Starting HTTP server");

    // Create TCP listener
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Server listening on {}", addr);

    // Run server with graceful shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Server stopped");
    Ok(())
}

/// Graceful shutdown signal handler.
///
/// Waits for:
/// - Ctrl+C (SIGINT)
/// - SIGTERM (in production environments)
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {
            info!("Received Ctrl+C signal, shutting down gracefully...");
        },
        () = terminate => {
            info!("Received SIGTERM signal, shutting down gracefully...");
        },
    }
}
