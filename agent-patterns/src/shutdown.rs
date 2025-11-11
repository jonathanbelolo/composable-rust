//! Graceful Shutdown Coordination (Phase 8.4 Part 2.2)
//!
//! Coordinates shutdown across multiple components with timeout handling.
//!
//! ## Architecture
//!
//! - **ShutdownHandler trait**: Components implement this for cleanup
//! - **ShutdownCoordinator**: Manages all handlers, handles timeouts
//! - **wait_for_signal()**: Waits for SIGTERM or Ctrl+C
//!
//! ## Usage
//!
//! ```ignore
//! use agent_patterns::shutdown::*;
//! use std::time::Duration;
//!
//! // Create shutdown coordinator
//! let mut coordinator = ShutdownCoordinator::new(Duration::from_secs(30));
//!
//! // Register shutdown handlers
//! coordinator.register(Arc::new(StoreShutdownHandler::new(...)));
//! coordinator.register(Arc::new(DatabaseShutdownHandler::new(...)));
//!
//! // Wait for shutdown signal
//! wait_for_signal().await;
//!
//! // Graceful shutdown
//! coordinator.shutdown().await?;
//! ```

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

/// Trait for components that need graceful shutdown
///
/// Implement this for any component that needs cleanup:
/// - Database connections
/// - HTTP servers
/// - Background tasks
/// - File handles
#[async_trait]
pub trait ShutdownHandler: Send + Sync {
    /// Component name for logging
    fn name(&self) -> &str;

    /// Gracefully shut down this component
    ///
    /// Should complete any in-flight work and release resources.
    ///
    /// # Errors
    ///
    /// Returns error if shutdown fails (e.g., cannot flush buffers)
    async fn shutdown(&self) -> Result<(), String>;
}

/// Coordinates shutdown across multiple components
///
/// Manages multiple shutdown handlers with:
/// - Parallel shutdown (all components at once)
/// - Timeout handling (force shutdown after grace period)
/// - Error collection (report which components failed)
/// - Broadcast signal (notify all waiting tasks)
pub struct ShutdownCoordinator {
    handlers: Vec<Arc<dyn ShutdownHandler>>,
    shutdown_tx: broadcast::Sender<()>,
    timeout_duration: Duration,
}

impl ShutdownCoordinator {
    /// Create new shutdown coordinator
    ///
    /// # Arguments
    ///
    /// * `timeout` - Maximum time to wait for all components to shut down
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        let (shutdown_tx, _) = broadcast::channel(16);

        Self {
            handlers: Vec::new(),
            shutdown_tx,
            timeout_duration: timeout,
        }
    }

    /// Register a shutdown handler
    ///
    /// Handlers are shut down in parallel, not in registration order.
    pub fn register(&mut self, handler: Arc<dyn ShutdownHandler>) {
        info!("Registered shutdown handler: {}", handler.name());
        self.handlers.push(handler);
    }

    /// Get a receiver for shutdown signals
    ///
    /// Components can subscribe to this to be notified when shutdown starts.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    /// Initiate graceful shutdown
    ///
    /// 1. Send broadcast signal to all subscribers
    /// 2. Shutdown all handlers in parallel
    /// 3. Wait for completion or timeout
    /// 4. Return errors for any failed shutdowns
    ///
    /// # Errors
    ///
    /// Returns list of errors if any component fails to shut down
    pub async fn shutdown(&self) -> Result<(), Vec<String>> {
        info!(
            "Initiating graceful shutdown for {} components (timeout: {:?})",
            self.handlers.len(),
            self.timeout_duration
        );

        // Send shutdown signal
        if let Err(e) = self.shutdown_tx.send(()) {
            warn!("No active shutdown signal receivers: {}", e);
        }

        // Shutdown all handlers in parallel with timeout
        let mut errors = Vec::new();

        let shutdown_futures: Vec<_> = self
            .handlers
            .iter()
            .map(|handler| async move {
                let name = handler.name();
                info!("Shutting down component: {}", name);

                match tokio::time::timeout(self.timeout_duration, handler.shutdown()).await {
                    Ok(Ok(())) => {
                        info!("Component {} shut down successfully", name);
                        Ok(())
                    }
                    Ok(Err(e)) => {
                        error!("Component {} shutdown failed: {}", name, e);
                        Err(format!("{}: {}", name, e))
                    }
                    Err(_) => {
                        error!("Component {} shutdown timed out", name);
                        Err(format!("{}: timeout after {:?}", name, self.timeout_duration))
                    }
                }
            })
            .collect();

        // Wait for all shutdowns to complete
        let results = futures::future::join_all(shutdown_futures).await;

        for result in results {
            if let Err(e) = result {
                errors.push(e);
            }
        }

        if errors.is_empty() {
            info!("All components shut down successfully");
            Ok(())
        } else {
            error!("Shutdown completed with {} errors", errors.len());
            Err(errors)
        }
    }

    /// Get number of registered handlers
    #[must_use]
    pub fn handler_count(&self) -> usize {
        self.handlers.len()
    }
}

/// Generic shutdown handler using closures
///
/// Use this for simple shutdown logic without defining a custom type.
///
/// # Example
///
/// ```ignore
/// let handler = GenericShutdownHandler::new("database".into(), || async {
///     database_pool.close().await.map_err(|e| e.to_string())
/// });
/// ```
pub struct GenericShutdownHandler {
    name: String,
    on_shutdown: Arc<
        dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
            + Send
            + Sync,
    >,
}

impl GenericShutdownHandler {
    /// Create new generic shutdown handler
    ///
    /// # Arguments
    ///
    /// * `name` - Component name for logging
    /// * `on_shutdown` - Async function to call during shutdown
    pub fn new<F, Fut>(name: String, on_shutdown: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        Self {
            name,
            on_shutdown: Arc::new(move || Box::pin(on_shutdown())),
        }
    }
}

#[async_trait]
impl ShutdownHandler for GenericShutdownHandler {
    fn name(&self) -> &str {
        &self.name
    }

    async fn shutdown(&self) -> Result<(), String> {
        (self.on_shutdown)().await
    }
}

/// Wait for shutdown signal (Ctrl+C or SIGTERM)
///
/// Use this in `main()` to wait for shutdown:
///
/// ```ignore
/// #[tokio::main]
/// async fn main() {
///     // ... start services
///
///     wait_for_signal().await;
///
///     // ... graceful shutdown
/// }
/// ```
pub async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut sigterm = signal(SignalKind::terminate())
            .expect("Failed to register SIGTERM handler");

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Received Ctrl+C");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM");
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to wait for Ctrl+C");
        info!("Received Ctrl+C");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Mock shutdown handler for testing
    struct MockShutdownHandler {
        name: String,
        should_fail: bool,
        shutdown_called: Arc<AtomicBool>,
    }

    impl MockShutdownHandler {
        fn new(name: impl Into<String>, should_fail: bool) -> Self {
            Self {
                name: name.into(),
                should_fail,
                shutdown_called: Arc::new(AtomicBool::new(false)),
            }
        }

        fn was_called(&self) -> bool {
            self.shutdown_called.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ShutdownHandler for MockShutdownHandler {
        fn name(&self) -> &str {
            &self.name
        }

        async fn shutdown(&self) -> Result<(), String> {
            self.shutdown_called.store(true, Ordering::SeqCst);

            if self.should_fail {
                Err("Simulated failure".to_string())
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn test_coordinator_shutdown_all_success() {
        let handler1 = Arc::new(MockShutdownHandler::new("handler1", false));
        let handler2 = Arc::new(MockShutdownHandler::new("handler2", false));

        let handler1_clone = Arc::clone(&handler1);
        let handler2_clone = Arc::clone(&handler2);

        let mut coordinator = ShutdownCoordinator::new(Duration::from_secs(5));
        coordinator.register(handler1);
        coordinator.register(handler2);

        let result = coordinator.shutdown().await;

        assert!(result.is_ok());
        assert!(handler1_clone.was_called());
        assert!(handler2_clone.was_called());
    }

    #[tokio::test]
    async fn test_coordinator_shutdown_one_failure() {
        let handler1 = Arc::new(MockShutdownHandler::new("handler1", false));
        let handler2 = Arc::new(MockShutdownHandler::new("handler2", true));

        let mut coordinator = ShutdownCoordinator::new(Duration::from_secs(5));
        coordinator.register(handler1);
        coordinator.register(handler2);

        let result = coordinator.shutdown().await;

        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("handler2"));
    }

    #[tokio::test]
    async fn test_coordinator_broadcast_signal() {
        let coordinator = ShutdownCoordinator::new(Duration::from_secs(5));
        let mut rx = coordinator.subscribe();

        // Spawn task that waits for signal
        let task = tokio::spawn(async move {
            rx.recv().await.ok();
            "signal_received"
        });

        // Trigger shutdown
        let _ = coordinator.shutdown().await;

        // Task should complete
        let result = task.await.unwrap();
        assert_eq!(result, "signal_received");
    }

    #[tokio::test]
    async fn test_generic_shutdown_handler_success() {
        let called = Arc::new(AtomicBool::new(false));
        let called_clone = Arc::clone(&called);

        let handler = GenericShutdownHandler::new("test".to_string(), move || {
            let called = Arc::clone(&called_clone);
            async move {
                called.store(true, Ordering::SeqCst);
                Ok(())
            }
        });

        let result = handler.shutdown().await;

        assert!(result.is_ok());
        assert!(called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_generic_shutdown_handler_failure() {
        let handler = GenericShutdownHandler::new("test".to_string(), || async {
            Err("Test error".to_string())
        });

        let result = handler.shutdown().await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Test error");
    }

    #[tokio::test]
    async fn test_coordinator_timeout() {
        struct SlowShutdownHandler;

        #[async_trait]
        impl ShutdownHandler for SlowShutdownHandler {
            fn name(&self) -> &str {
                "slow"
            }

            async fn shutdown(&self) -> Result<(), String> {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok(())
            }
        }

        let mut coordinator = ShutdownCoordinator::new(Duration::from_millis(100));
        coordinator.register(Arc::new(SlowShutdownHandler));

        let result = coordinator.shutdown().await;

        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("timeout"));
    }

    #[test]
    fn test_coordinator_handler_count() {
        let mut coordinator = ShutdownCoordinator::new(Duration::from_secs(5));
        assert_eq!(coordinator.handler_count(), 0);

        coordinator.register(Arc::new(MockShutdownHandler::new("h1", false)));
        assert_eq!(coordinator.handler_count(), 1);

        coordinator.register(Arc::new(MockShutdownHandler::new("h2", false)));
        assert_eq!(coordinator.handler_count(), 2);
    }
}
