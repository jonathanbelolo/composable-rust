//! Production Agent - Complete Phase 8.4 Example
//!
//! This example demonstrates all Phase 8.4 features:
//! - OpenTelemetry tracing with span propagation
//! - Health checks (startup, liveness, readiness)
//! - Graceful shutdown coordination
//! - Circuit breakers, rate limiting, bulkheads
//! - Prometheus metrics
//! - Configuration management
//! - Audit logging
//! - Security monitoring
//!
//! Run with: cargo run --bin production-agent
//! Health: http://localhost:8080/health
//! Metrics: http://localhost:9090/metrics
//! Chat: POST http://localhost:8080/chat

mod environment;
mod reducer;
mod server;
mod types;

use types::AgentState;

use composable_rust_agent_patterns::{
    audit::PostgresAuditLogger,
    health::{HealthCheckable, SystemHealthCheck, HealthStatus},
    AgentMetrics,
    security::SecurityMonitor,
    shutdown::ShutdownCoordinator,
};
use composable_rust_postgres::PostgresEventStore;
use composable_rust_redpanda::RedpandaEventBus;
use composable_rust_projections::PostgresProjectionStore;
use environment::ProductionEnvironment;
use metrics_exporter_prometheus::PrometheusBuilder;
use reducer::ProductionAgentReducer;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Simple health check
struct SimpleHealthCheck {
    name: String,
}

#[async_trait::async_trait]
impl HealthCheckable for SimpleHealthCheck {
    async fn check_health(&self) -> composable_rust_agent_patterns::health::ComponentHealth {
        // Simulate startup check
        tokio::time::sleep(Duration::from_millis(100)).await;
        composable_rust_agent_patterns::health::ComponentHealth::healthy("Service is healthy")
    }

    fn component_name(&self) -> &str {
        &self.name
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables from .env file (if present)
    let _ = dotenvy::dotenv();

    // Initialize tracing
    init_tracing()?;

    info!("🚀 Starting Production Agent with all Phase 8.4 features");

    // Initialize Prometheus metrics exporter
    let prometheus_handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("Failed to install Prometheus recorder");

    // Create shutdown coordinator
    let shutdown = Arc::new(ShutdownCoordinator::new(Duration::from_secs(30)));

    // Get database connection string
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:password@localhost:5432/composable_auth".to_string());

    info!("🔌 Connecting to PostgreSQL: {}", database_url.split('@').next_back().unwrap_or("unknown"));

    // Create database connection pool
    let db_pool = PgPoolOptions::new()
        .max_connections(20)
        .min_connections(5)
        .acquire_timeout(Duration::from_secs(30))
        .idle_timeout(Some(Duration::from_secs(600)))
        .max_lifetime(Some(Duration::from_secs(1800)))
        .connect(&database_url)
        .await?;

    info!("✅ PostgreSQL connected");

    // Run database migrations
    info!("🔄 Running database migrations...");
    sqlx::migrate!("../../migrations")
        .run(&db_pool)
        .await?;
    info!("✅ Migrations complete");

    // Initialize PostgreSQL audit logger
    let audit_logger = Arc::new(PostgresAuditLogger::new(db_pool.clone()));
    info!("✅ PostgreSQL audit logger initialized");

    // Initialize security monitor
    let security_monitor = Arc::new(SecurityMonitor::new());
    info!("✅ Security monitor initialized");

    // Initialize event store
    let event_store = Arc::new(PostgresEventStore::from_pool(db_pool.clone()));
    info!("✅ Event store initialized");

    // Initialize event bus (Redpanda)
    let redpanda_brokers = std::env::var("REDPANDA_BROKERS")
        .unwrap_or_else(|_| "localhost:9092".to_string());

    info!("🔌 Connecting to Redpanda: {}", redpanda_brokers);
    let event_bus: Arc<dyn composable_rust_core::event_bus::EventBus> = Arc::new(
        RedpandaEventBus::builder()
            .brokers(&redpanda_brokers)
            .producer_acks("all")  // Wait for all replicas
            .compression("lz4")
            .consumer_group("production-agent")
            .buffer_size(1000)
            .auto_offset_reset("latest")
            .build()
            .map_err(|e| format!("Failed to create Redpanda event bus: {e}"))?
    );
    info!("✅ Redpanda event bus initialized");

    // Initialize projection store
    let projection_store = Arc::new(PostgresProjectionStore::new(
        db_pool.clone(),
        "conversation_projections".to_string(),
    ));
    info!("✅ Projection store initialized");

    // Create clock for timestamps
    let clock: Arc<dyn composable_rust_core::environment::Clock> = Arc::new(composable_rust_core::environment::SystemClock);
    info!("✅ Clock initialized");

    // Create environment (loads Anthropic API key from ANTHROPIC_API_KEY env var)
    let environment = Arc::new(ProductionEnvironment::from_env(
        audit_logger.clone(),
        security_monitor.clone(),
        event_store.clone(),
        clock,
        event_bus,
        projection_store,
    ));
    info!("✅ Agent environment created with resilience features");

    // Create reducer
    let reducer = ProductionAgentReducer::new(
        audit_logger.clone(),
        security_monitor.clone(),
    );
    info!("✅ Agent reducer initialized");

    // Create initial state
    let initial_state = AgentState::new();

    // Create Store (manages state, reducer, environment)
    // Note: Store::new takes ownership of environment, so we clone it for the server
    let store = Arc::new(composable_rust_runtime::Store::new(
        initial_state,
        reducer,
        (*environment).clone(),
    ));
    info!("✅ Store initialized with event sourcing");

    // Create metrics
    let metrics = Arc::new(AgentMetrics::new());
    info!("✅ Metrics initialized");

    // Create health check registry
    let mut health_registry = SystemHealthCheck::new();

    // Register health checks
    health_registry.add_check(Arc::new(SimpleHealthCheck {
        name: "agent".to_string(),
    }));
    health_registry.add_check(Arc::new(SimpleHealthCheck {
        name: "llm_connection".to_string(),
    }));
    health_registry.add_check(Arc::new(SimpleHealthCheck {
        name: "audit_system".to_string(),
    }));
    info!("✅ Health checks registered");

    // Wrap in Arc after adding checks
    let health_registry = Arc::new(health_registry);

    // Run startup checks
    info!("🔍 Running startup health checks...");
    let startup_results = health_registry.check_all().await;
    let all_healthy = startup_results
        .values()
        .all(|r| r.status == HealthStatus::Healthy);

    if !all_healthy {
        error!("❌ Startup health checks failed:");
        for (name, health) in &startup_results {
            if health.status != HealthStatus::Healthy {
                error!("  - {} is unhealthy: {}", name, health.message);
            }
        }
        return Err("Startup health checks failed".into());
    }
    info!("✅ All startup checks passed");

    // Create HTTP server
    let app = server::create_server(
        store.clone(),
        environment.clone(),
        metrics.clone(),
        health_registry.clone(),
    )
    .await;

    // Start HTTP server
    let http_port = std::env::var("HTTP_PORT")
        .unwrap_or_else(|_| "8080".to_string());
    let bind_addr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0".to_string());
    let addr: SocketAddr = format!("{}:{}", bind_addr, http_port).parse()?;
    info!("🌐 Starting HTTP server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Subscribe to shutdown signal
    let mut shutdown_rx = shutdown.subscribe();

    // Spawn server task
    let server_handle = tokio::spawn(async move {
        info!("✅ HTTP server listening on {}", addr);
        info!("📊 Endpoints:");
        info!("  - Health: http://{}/health", addr);
        info!("  - Liveness: http://{}/health/live", addr);
        info!("  - Readiness: http://{}/health/ready", addr);
        info!("  - Metrics: http://{}/metrics", addr);
        info!("  - Chat: POST http://{}/chat", addr);

        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.recv().await;
            })
            .await
            .expect("Server error");
    });

    // Start metrics server
    let metrics_port = std::env::var("METRICS_PORT")
        .unwrap_or_else(|_| "9090".to_string());
    let metrics_addr: SocketAddr = format!("{}:{}", bind_addr, metrics_port).parse()?;
    info!("📊 Prometheus metrics available at http://{}/metrics", metrics_addr);

    let metrics_app = axum::Router::new().route(
        "/metrics",
        axum::routing::get(|| async move {
            prometheus_handle.render()
        }),
    );

    let metrics_listener = tokio::net::TcpListener::bind(metrics_addr).await?;
    let mut metrics_shutdown_rx = shutdown.subscribe();

    let metrics_handle = tokio::spawn(async move {
        axum::serve(metrics_listener, metrics_app)
            .with_graceful_shutdown(async move {
                let _ = metrics_shutdown_rx.recv().await;
            })
            .await
            .expect("Metrics server error");
    });

    info!("✨ Production Agent fully initialized and ready!");
    info!("Press Ctrl+C to shut down gracefully...");

    // Wait for shutdown signal
    match signal::ctrl_c().await {
        Ok(()) => {
            info!("🛑 Shutdown signal received, initiating graceful shutdown...");
        }
        Err(err) => {
            error!("Unable to listen for shutdown signal: {}", err);
        }
    }

    // Initiate shutdown (this broadcasts to all subscribers)
    if let Err(e) = shutdown.shutdown().await {
        error!("Shutdown errors: {:?}", e);
    }

    // Wait for tasks to complete
    info!("⏳ Waiting for HTTP server to shut down...");
    if let Err(e) = server_handle.await {
        warn!("Server task error during shutdown: {}", e);
    }

    info!("⏳ Waiting for metrics server to shut down...");
    if let Err(e) = metrics_handle.await {
        warn!("Metrics server task error during shutdown: {}", e);
    }

    // Display final metrics
    let final_snapshot = metrics.snapshot();
    info!("📈 Final metrics:");
    info!("  Total tool calls: {}", final_snapshot.total_tool_calls);
    info!("  Successful tool calls: {}", final_snapshot.total_successes);
    info!("  Failed tool calls: {}", final_snapshot.total_failures);

    // Display security summary
    let security_dashboard = security_monitor.get_dashboard().await?;
    info!("🔒 Security summary:");
    info!("  Total incidents: {}", security_dashboard.total_incidents);
    info!("  Active incidents: {}", security_dashboard.active_incidents);

    // Display audit summary
    match audit_logger.count().await {
        Ok(count) => {
            info!("📝 Audit summary:");
            info!("  Total events logged: {}", count);
        }
        Err(e) => {
            warn!("Failed to get audit count: {}", e);
        }
    }

    info!("✅ Shutdown complete. Goodbye!");
    Ok(())
}

/// Initialize tracing with OpenTelemetry support
fn init_tracing() -> Result<(), Box<dyn std::error::Error>> {
    // For production, you would configure OpenTelemetry exporter here
    // Example with Jaeger:
    // let tracer = opentelemetry_jaeger::new_pipeline()
    //     .with_service_name("production-agent")
    //     .install_batch(opentelemetry::runtime::Tokio)?;
    //
    // let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "production_agent=info,composable_rust_agent_patterns=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_target(false))
        // .with(telemetry)  // Add OpenTelemetry layer in production
        .init();

    Ok(())
}
