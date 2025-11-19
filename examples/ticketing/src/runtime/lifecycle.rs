//! Application lifecycle management and graceful shutdown.
//!
//! This module provides the `Application` struct that manages the complete
//! lifecycle of an event-driven application:
//!
//! 1. **Startup**: Spawn all background consumers and projection managers
//! 2. **Runtime**: Run HTTP server and process events
//! 3. **Shutdown**: Coordinate graceful termination of all tasks
//!
//! # Graceful Shutdown
//!
//! When a shutdown signal is received (Ctrl+C or SIGTERM):
//! 1. HTTP server stops accepting new connections
//! 2. Shutdown signal broadcast to all consumers
//! 3. Wait for consumers to finish current work (10s timeout)
//! 4. Wait for projection managers to checkpoint (10s timeout)
//! 5. Clean exit
//!
//! # Example
//!
//! ```rust,ignore
//! let app = ApplicationBuilder::new()
//!     .with_config(config)
//!     .with_resources().await?
//!     .with_aggregates()
//!     .with_projections().await?
//!     .build().await?;
//!
//! app.run().await?;
//! ```

use crate::config::Config;
use crate::projections::ProjectionManagers;
use crate::runtime::EventConsumer;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{info, warn};

/// Running application with all background tasks.
///
/// This struct represents a fully configured and ready-to-run application.
/// It owns all the resources needed to run the application (HTTP server,
/// event consumers, projection managers) and coordinates their lifecycle.
///
/// # Lifecycle
///
/// 1. Created via `ApplicationBuilder::build()`
/// 2. Started via `Application::run()`
/// 3. Runs until shutdown signal received
/// 4. Coordinates graceful shutdown of all tasks
pub struct Application {
    /// TCP listener for HTTP server
    listener: tokio::net::TcpListener,

    /// Axum router with all HTTP routes
    app: axum::Router,

    /// Event consumers (aggregates + projections)
    consumers: Vec<EventConsumer>,

    /// PostgreSQL projection managers
    projection_managers: ProjectionManagers,

    /// Shutdown signal broadcaster
    shutdown_tx: broadcast::Sender<()>,

    /// Application configuration
    config: Arc<Config>,
}

impl Application {
    /// Create a new application instance.
    ///
    /// # Arguments
    ///
    /// * `listener` - TCP listener bound to server address
    /// * `app` - Axum router with all HTTP routes
    /// * `consumers` - Event consumers for aggregates and projections
    /// * `projection_managers` - PostgreSQL projection managers
    /// * `shutdown_tx` - Shutdown signal broadcaster
    /// * `config` - Application configuration
    ///
    /// # Returns
    ///
    /// A new `Application` ready to run.
    #[must_use]
    pub fn new(
        listener: tokio::net::TcpListener,
        app: axum::Router,
        consumers: Vec<EventConsumer>,
        projection_managers: ProjectionManagers,
        shutdown_tx: broadcast::Sender<()>,
        config: Arc<Config>,
    ) -> Self {
        Self {
            listener,
            app,
            consumers,
            projection_managers,
            shutdown_tx,
            config,
        }
    }

    /// Run the application until shutdown signal received.
    ///
    /// This method:
    /// 1. Spawns all event consumers
    /// 2. Starts all projection managers
    /// 3. Runs the HTTP server
    /// 4. Waits for shutdown signal (Ctrl+C or SIGTERM)
    /// 5. Coordinates graceful shutdown of all tasks
    ///
    /// # Returns
    ///
    /// `Ok(())` on clean shutdown, `Err(...)` on fatal error.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - HTTP server fails to start
    /// - TCP listener fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let app = build_application().await?;
    /// app.run().await?;
    /// ```
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        info!(
            address = %format!("{}:{}", self.config.server.host, self.config.server.port),
            "Starting HTTP server"
        );

        // Start all event consumers
        info!(consumer_count = self.consumers.len(), "Starting event consumers");
        let consumer_handles: Vec<_> = self
            .consumers
            .into_iter()
            .map(EventConsumer::spawn)
            .collect();

        // Start projection managers
        info!("Starting projection managers");
        let projection_handles = self.projection_managers.start_all();
        info!(
            projection_count = projection_handles.len(),
            "Projection managers started"
        );

        // Run HTTP server with graceful shutdown
        info!("HTTP server listening for requests");
        axum::serve(self.listener, self.app)
            .with_graceful_shutdown(shutdown_signal())
            .await?;

        info!("HTTP server stopped, initiating graceful shutdown...");

        // Send shutdown signal to all background tasks
        let _ = self.shutdown_tx.send(());

        // Wait for all tasks to complete
        Self::await_shutdown(consumer_handles, projection_handles).await;

        info!("Graceful shutdown complete");
        Ok(())
    }

    /// Wait for all background tasks to shut down gracefully.
    ///
    /// Gives each task 10 seconds to finish its current work before timing out.
    async fn await_shutdown(
        consumer_handles: Vec<tokio::task::JoinHandle<()>>,
        projection_handles: Vec<tokio::task::JoinHandle<()>>,
    ) {
        let timeout = Duration::from_secs(10);

        for (idx, handle) in consumer_handles.into_iter().enumerate() {
            match tokio::time::timeout(timeout, handle).await {
                Ok(Ok(())) => info!(consumer = idx, "Consumer stopped gracefully"),
                Ok(Err(e)) => warn!(consumer = idx, error = %e, "Consumer task failed"),
                Err(_) => warn!(consumer = idx, "Consumer shutdown timed out"),
            }
        }

        for (idx, handle) in projection_handles.into_iter().enumerate() {
            match tokio::time::timeout(timeout, handle).await {
                Ok(Ok(())) => info!(projection = idx, "Projection manager stopped gracefully"),
                Ok(Err(e)) => warn!(projection = idx, error = %e, "Projection manager task failed"),
                Err(_) => warn!(projection = idx, "Projection manager shutdown timed out"),
            }
        }
    }
}

/// Wait for shutdown signal (Ctrl+C or SIGTERM).
///
/// Returns when the process receives SIGINT (Ctrl+C) or SIGTERM.
async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {
            info!("Received Ctrl+C signal");
        }
        () = terminate => {
            info!("Received SIGTERM signal");
        }
    }
}
