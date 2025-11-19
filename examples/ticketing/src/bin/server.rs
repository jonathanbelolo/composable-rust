//! Ticketing System Server
//!
//! Main server process that runs the ticketing application.
//!
//! This binary:
//! - Initializes `PostgreSQL` event store
//! - Connects to `RedPanda` event bus
//! - Starts aggregate services
//! - Subscribes projections to event streams
//! - Runs indefinitely processing events
//!
//! # Usage
//!
//! ```bash
//! # Start infrastructure
//! docker compose up -d
//!
//! # Run server
//! cargo run --bin server
//! ```

use ticketing::{Config, TicketingApp};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env file
    let _ = dotenvy::dotenv();

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,ticketing=debug,sqlx=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("🎫 Starting Ticketing System Server...");

    // Load configuration
    let config = Config::from_env();
    tracing::info!(
        postgres = %config.postgres.url,
        redpanda = %config.redpanda.brokers,
        "Configuration loaded"
    );

    // Initialize application
    tracing::info!("Initializing application components...");
    let app = TicketingApp::new(config).await?;
    tracing::info!("✓ Application initialized");

    // Start event processing
    tracing::info!("Starting event processing...");
    app.start().await?;
    tracing::info!("✓ Event processing started");

    tracing::info!("🎫 Ticketing System Server is running!");
    tracing::info!("  - Event store: PostgreSQL");
    tracing::info!("  - Event bus: RedPanda");
    tracing::info!("  - Projections: Real-time updates");
    tracing::info!("");
    tracing::info!("Press Ctrl+C to shutdown");

    // Run forever (until Ctrl+C)
    tokio::signal::ctrl_c().await?;

    tracing::info!("Shutting down gracefully...");
    Ok(())
}
