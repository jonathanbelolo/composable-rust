//! Ticketing system HTTP server.
//!
//! Event-sourced ticketing platform with CQRS, sagas, and real-time updates.
//!
//! This binary demonstrates the **declarative bootstrap API** provided by the
//! `ApplicationBuilder`. The entire application is configured and started in
//! just a few lines of code.
//!
//! # Architecture
//!
//! The builder handles all the complex initialization:
//! - Database connections and migrations
//! - Event store setup
//! - HTTP server configuration
//! - Graceful shutdown coordination
//!
//! # Example Usage
//!
//! ```bash
//! # Start the server
//! cargo run --bin ticketing
//!
//! # The server will:
//! # 1. Load config from environment variables (.env file)
//! # 2. Setup tracing/logging
//! # 3. Connect to PostgreSQL databases (with migrations)
//! # 4. Initialize authentication
//! # 5. Build all handlers using next-gen architecture
//! # 6. Start HTTP server on configured port
//! # 7. Run until Ctrl+C or SIGTERM
//! ```

use ticketing::{bootstrap::ApplicationBuilder, config::Config};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env file (ignore errors if it doesn't exist - prod uses real env vars)
    let _ = dotenvy::dotenv();

    info!("Starting Ticketing System HTTP Server");

    // Load configuration
    let config = Config::from_env();
    info!(
        postgres_url = %config.postgres.url,
        server_address = %format!("{}:{}", config.server.host, config.server.port),
        "Configuration loaded"
    );

    // Build and run application using declarative builder API
    ApplicationBuilder::new()
        .with_config(config)
        .with_tracing()?
        .with_resources()
        .await?
        .with_auth()
        .await?
        .build()
        .await?
        .run()
        .await?;

    info!("Server shut down gracefully");
    Ok(())
}
