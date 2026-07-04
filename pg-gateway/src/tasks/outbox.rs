//! Outbox worker for processing background tasks.
//!
//! The [`OutboxWorker`] polls the `outbox.pending_tasks` table and executes
//! tasks using registered [`TaskExecutor`] implementations.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        OutboxWorker                              │
//! │                                                                  │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
//! │  │ pg_notify   │  │   Polling   │  │      Executors          │  │
//! │  │  listener   │  │   (backup)  │  │                         │  │
//! │  │             │  │             │  │  send_email → Email     │  │
//! │  │  LISTEN     │  │  every 5s   │  │  send_sms   → SMS       │  │
//! │  │  outbox_*   │  │             │  │  webhook    → Webhook   │  │
//! │  └──────┬──────┘  └──────┬──────┘  └─────────────────────────┘  │
//! │         │                │                      ▲                │
//! │         └────────┬───────┘                      │                │
//! │                  ▼                              │                │
//! │         ┌────────────────┐                      │                │
//! │         │  Claim Tasks   │──────────────────────┘                │
//! │         │  (batch of 10) │                                       │
//! │         └────────────────┘                                       │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

use super::error::TaskError;
use super::executor::TaskExecutor;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

/// A task claimed from the outbox.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OutboxTask {
    /// Task ID.
    pub id: i64,
    /// Task type (e.g., `send_email`, `webhook`).
    pub task_type: String,
    /// Task payload as JSON.
    pub payload: serde_json::Value,
    /// Correlation ID for tracing.
    pub correlation_id: Option<String>,
    /// Event ID that caused this task.
    pub causation_id: Option<i64>,
    /// Tenant ID for context.
    pub tenant_id: Option<String>,
    /// Number of execution attempts.
    pub attempts: i32,
    /// Maximum allowed attempts.
    pub max_attempts: i32,
}

/// Configuration for the outbox worker.
#[derive(Debug, Clone)]
pub struct OutboxConfig {
    /// Number of tasks to claim per batch.
    pub batch_size: i32,
    /// Duration to hold a task lock.
    pub lock_duration: Duration,
    /// Polling interval (backup for missed `pg_notify`).
    pub poll_interval: Duration,
    /// Whether to listen for `pg_notify` hints.
    pub use_notify: bool,
}

impl Default for OutboxConfig {
    fn default() -> Self {
        Self {
            batch_size: 10,
            lock_duration: Duration::from_secs(300), // 5 minutes
            poll_interval: Duration::from_secs(5),
            use_notify: true,
        }
    }
}

impl OutboxConfig {
    /// Create a new configuration with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the batch size.
    #[must_use]
    pub const fn with_batch_size(mut self, size: i32) -> Self {
        self.batch_size = size;
        self
    }

    /// Set the lock duration.
    #[must_use]
    pub const fn with_lock_duration(mut self, duration: Duration) -> Self {
        self.lock_duration = duration;
        self
    }

    /// Set the polling interval.
    #[must_use]
    pub const fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Enable or disable `pg_notify` listening.
    #[must_use]
    pub const fn with_notify(mut self, enabled: bool) -> Self {
        self.use_notify = enabled;
        self
    }
}

/// Background worker that processes outbox tasks.
///
/// The worker claims tasks from `outbox.pending_tasks`, executes them using
/// registered executors, and marks them as completed or failed.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::tasks::{OutboxWorker, OutboxConfig, EmailExecutor};
/// use tokio::sync::broadcast;
///
/// // Create shutdown channel
/// let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
///
/// // Create worker with executors
/// let worker = OutboxWorker::builder(pool)
///     .config(OutboxConfig::default())
///     .executor(Arc::new(email_executor))
///     .executor(Arc::new(webhook_executor))
///     .shutdown(shutdown_rx)
///     .build();
///
/// // Run worker (blocks until shutdown)
/// worker.run().await;
/// ```
pub struct OutboxWorker {
    pool: PgPool,
    worker_id: String,
    config: OutboxConfig,
    executors: HashMap<&'static str, Arc<dyn TaskExecutor>>,
    shutdown: broadcast::Receiver<()>,
}

impl OutboxWorker {
    /// Create a new builder for `OutboxWorker`.
    #[must_use]
    pub fn builder(pool: PgPool) -> OutboxWorkerBuilder {
        OutboxWorkerBuilder::new(pool)
    }

    /// Run the worker until shutdown is signaled.
    ///
    /// This method blocks until a shutdown signal is received on the
    /// broadcast channel.
    pub async fn run(mut self) {
        let mut poll_interval = tokio::time::interval(self.config.poll_interval);

        // Optionally subscribe to pg_notify
        let mut notify_rx = if self.config.use_notify {
            Some(self.subscribe_to_notifications())
        } else {
            None
        };

        tracing::info!(
            worker_id = %self.worker_id,
            executors = ?self.executors.keys().collect::<Vec<_>>(),
            "Outbox worker started"
        );

        loop {
            tokio::select! {
                // Shutdown signal
                _ = self.shutdown.recv() => {
                    tracing::info!(worker_id = %self.worker_id, "Outbox worker shutting down");
                    break;
                }

                // pg_notify hint (process immediately)
                Some(()) = async {
                    if let Some(ref mut rx) = notify_rx {
                        rx.recv().await
                    } else {
                        std::future::pending::<Option<()>>().await
                    }
                } => {
                    self.process_batch().await;
                }

                // Regular polling (catch any missed notifications)
                _ = poll_interval.tick() => {
                    self.process_batch().await;
                    self.release_stuck_tasks().await;
                }
            }
        }
    }

    /// Subscribe to `pg_notify` for `outbox_tasks` channel.
    fn subscribe_to_notifications(&self) -> mpsc::Receiver<()> {
        let (tx, rx) = mpsc::channel(100);
        let pool = self.pool.clone();
        let worker_id = self.worker_id.clone();

        tokio::spawn(async move {
            loop {
                let result = Self::run_listener(&pool, &tx).await;

                if let Err(e) = result {
                    tracing::warn!(
                        worker_id = %worker_id,
                        error = %e,
                        "Outbox LISTEN error, reconnecting in 5s..."
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        });

        rx
    }

    /// Run the LISTEN connection.
    async fn run_listener(pool: &PgPool, tx: &mpsc::Sender<()>) -> Result<(), sqlx::Error> {
        let mut conn = pool.acquire().await?;

        sqlx::query("LISTEN outbox_tasks")
            .execute(&mut *conn)
            .await?;

        tracing::debug!("LISTEN outbox_tasks established");

        // Use PgListener for notifications
        let mut listener = sqlx::postgres::PgListener::connect_with(pool).await?;
        listener.listen("outbox_tasks").await?;

        loop {
            match listener.recv().await {
                Ok(_notification) => {
                    // Signal the worker to check for tasks
                    if tx.send(()).await.is_err() {
                        // Receiver dropped, exit
                        return Ok(());
                    }
                },
                Err(e) => {
                    return Err(e);
                },
            }
        }
    }

    /// Process a batch of tasks.
    #[allow(clippy::cast_possible_truncation)]
    async fn process_batch(&self) {
        let lock_seconds = self.config.lock_duration.as_secs() as i32;

        // Claim tasks using the stored procedure
        let tasks: Vec<OutboxTask> = match sqlx::query_as(
            "SELECT * FROM outbox.claim_tasks($1, $2, ($3 || ' seconds')::interval)",
        )
        .bind(&self.worker_id)
        .bind(self.config.batch_size)
        .bind(lock_seconds)
        .fetch_all(&self.pool)
        .await
        {
            Ok(tasks) => tasks,
            Err(e) => {
                tracing::error!(error = %e, "Failed to claim tasks");
                return;
            },
        };

        if !tasks.is_empty() {
            tracing::debug!(
                worker_id = %self.worker_id,
                count = tasks.len(),
                "Claimed tasks"
            );
        }

        for task in tasks {
            self.process_task(task).await;
        }
    }

    /// Process a single task.
    #[allow(clippy::cast_possible_truncation, clippy::cognitive_complexity)]
    async fn process_task(&self, task: OutboxTask) {
        let Some(executor) = self.executors.get(task.task_type.as_str()) else {
            tracing::error!(
                task_type = %task.task_type,
                task_id = task.id,
                "No executor registered for task type"
            );
            self.fail_task(task.id, "No executor found").await;
            return;
        };

        let start = std::time::Instant::now();

        match executor.execute(task.payload.clone()).await {
            Ok(()) => {
                self.complete_task(task.id).await;
                tracing::info!(
                    task_type = %task.task_type,
                    task_id = task.id,
                    duration_ms = start.elapsed().as_millis() as u64,
                    "Task completed"
                );
            },
            Err(TaskError::Temporary(msg)) => {
                self.fail_task(task.id, &msg).await;
                tracing::warn!(
                    task_type = %task.task_type,
                    task_id = task.id,
                    attempt = task.attempts,
                    max_attempts = task.max_attempts,
                    error = %msg,
                    "Task failed (will retry)"
                );
            },
            Err(TaskError::Permanent(msg)) => {
                self.fail_task_permanent(task.id, &msg).await;
                tracing::error!(
                    task_type = %task.task_type,
                    task_id = task.id,
                    error = %msg,
                    "Task failed permanently"
                );
            },
        }
    }

    /// Mark a task as completed.
    async fn complete_task(&self, task_id: i64) {
        if let Err(e) = sqlx::query("SELECT outbox.complete_task($1, $2)")
            .bind(task_id)
            .bind(&self.worker_id)
            .execute(&self.pool)
            .await
        {
            tracing::error!(
                task_id = task_id,
                error = %e,
                "Failed to mark task as completed"
            );
        }
    }

    /// Mark a task as failed (will retry if attempts remain).
    async fn fail_task(&self, task_id: i64, error: &str) {
        if let Err(e) = sqlx::query("SELECT outbox.fail_task($1, $2, $3)")
            .bind(task_id)
            .bind(&self.worker_id)
            .bind(error)
            .execute(&self.pool)
            .await
        {
            tracing::error!(
                task_id = task_id,
                error = %e,
                "Failed to mark task as failed"
            );
        }
    }

    /// Mark a task as permanently failed (move to DLQ).
    async fn fail_task_permanent(&self, task_id: i64, error: &str) {
        // Set attempts to max to force DLQ
        let _ =
            sqlx::query("UPDATE outbox.pending_tasks SET attempts = max_attempts WHERE id = $1")
                .bind(task_id)
                .execute(&self.pool)
                .await;

        self.fail_task(task_id, error).await;
    }

    /// Release stuck tasks (where lock has expired).
    async fn release_stuck_tasks(&self) {
        match sqlx::query_scalar::<_, i32>("SELECT outbox.release_stuck_tasks()")
            .fetch_one(&self.pool)
            .await
        {
            Ok(released) if released > 0 => {
                tracing::warn!(
                    worker_id = %self.worker_id,
                    count = released,
                    "Released stuck tasks"
                );
            },
            Ok(_) => {},
            Err(e) => {
                tracing::error!(error = %e, "Failed to release stuck tasks");
            },
        }
    }
}

/// Builder for [`OutboxWorker`].
pub struct OutboxWorkerBuilder {
    pool: PgPool,
    config: OutboxConfig,
    executors: Vec<Arc<dyn TaskExecutor>>,
    shutdown: Option<broadcast::Receiver<()>>,
}

impl OutboxWorkerBuilder {
    /// Create a new builder.
    fn new(pool: PgPool) -> Self {
        Self {
            pool,
            config: OutboxConfig::default(),
            executors: Vec::new(),
            shutdown: None,
        }
    }

    /// Set the configuration.
    #[must_use]
    pub const fn config(mut self, config: OutboxConfig) -> Self {
        self.config = config;
        self
    }

    /// Register a task executor.
    #[must_use]
    pub fn executor(mut self, executor: Arc<dyn TaskExecutor>) -> Self {
        self.executors.push(executor);
        self
    }

    /// Register multiple task executors.
    #[must_use]
    pub fn executors(mut self, executors: Vec<Arc<dyn TaskExecutor>>) -> Self {
        self.executors.extend(executors);
        self
    }

    /// Set the shutdown receiver.
    #[must_use]
    pub fn shutdown(mut self, receiver: broadcast::Receiver<()>) -> Self {
        self.shutdown = Some(receiver);
        self
    }

    /// Build the worker.
    ///
    /// # Panics
    ///
    /// Panics if no shutdown receiver was provided.
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn build(self) -> OutboxWorker {
        let shutdown = self
            .shutdown
            .expect("shutdown receiver is required - call .shutdown() before .build()");

        let worker_id = format!("worker-{}", uuid::Uuid::new_v4());
        let executors = self
            .executors
            .into_iter()
            .map(|e| (e.task_type(), e))
            .collect();

        OutboxWorker {
            pool: self.pool,
            worker_id,
            config: self.config,
            executors,
            shutdown,
        }
    }

    /// Build the worker, returning an error if configuration is invalid.
    ///
    /// # Errors
    ///
    /// Returns an error if no shutdown receiver was provided.
    pub fn try_build(self) -> Result<OutboxWorker, &'static str> {
        let shutdown = self
            .shutdown
            .ok_or("shutdown receiver is required - call .shutdown() before .build()")?;

        let worker_id = format!("worker-{}", uuid::Uuid::new_v4());
        let executors = self
            .executors
            .into_iter()
            .map(|e| (e.task_type(), e))
            .collect();

        Ok(OutboxWorker {
            pool: self.pool,
            worker_id,
            config: self.config,
            executors,
            shutdown,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let config = OutboxConfig::default();
        assert_eq!(config.batch_size, 10);
        assert_eq!(config.lock_duration, Duration::from_secs(300));
        assert_eq!(config.poll_interval, Duration::from_secs(5));
        assert!(config.use_notify);
    }

    #[test]
    fn config_builder() {
        let config = OutboxConfig::new()
            .with_batch_size(20)
            .with_lock_duration(Duration::from_secs(600))
            .with_poll_interval(Duration::from_secs(10))
            .with_notify(false);

        assert_eq!(config.batch_size, 20);
        assert_eq!(config.lock_duration, Duration::from_secs(600));
        assert_eq!(config.poll_interval, Duration::from_secs(10));
        assert!(!config.use_notify);
    }

    #[test]
    fn outbox_task_fields() {
        // Just verify the struct can be constructed
        let task = OutboxTask {
            id: 1,
            task_type: "test".to_string(),
            payload: serde_json::json!({}),
            correlation_id: Some("corr-123".to_string()),
            causation_id: Some(100),
            tenant_id: Some("tenant-1".to_string()),
            attempts: 1,
            max_attempts: 3,
        };

        assert_eq!(task.id, 1);
        assert_eq!(task.task_type, "test");
        assert_eq!(task.attempts, 1);
    }
}
