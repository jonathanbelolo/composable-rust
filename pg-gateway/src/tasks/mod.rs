//! Background task processing using the outbox pattern.
//!
//! This module provides a robust background task processing system built
//! on `PostgreSQL`'s transactional guarantees. Tasks are inserted into an
//! outbox table within the same transaction as domain changes, ensuring
//! exactly-once semantics through the outbox pattern.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        Application                              │
//! │                                                                  │
//! │  ┌──────────────┐         ┌─────────────────────────────────┐  │
//! │  │  Domain      │  BEGIN  │      outbox.pending_tasks       │  │
//! │  │  Changes     │ ──────► │  (same transaction)             │  │
//! │  └──────────────┘ COMMIT  └─────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//!                                       │
//!                                       │ pg_notify
//!                                       ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      OutboxWorker                               │
//! │                                                                  │
//! │  ┌─────────────┐  ┌─────────────┐  ┌────────────────────────┐  │
//! │  │   LISTEN    │  │   Polling   │  │      Executors         │  │
//! │  │ outbox_tasks│  │  (backup)   │  │                        │  │
//! │  └──────┬──────┘  └──────┬──────┘  │  send_email → Email    │  │
//! │         │                │         │  send_sms   → SMS      │  │
//! │         └────────┬───────┘         │  webhook    → Webhook  │  │
//! │                  ▼                 └────────────────────────┘  │
//! │         ┌────────────────┐                     ▲                │
//! │         │  Claim Tasks   │─────────────────────┘                │
//! │         └────────────────┘                                      │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Components
//!
//! - [`TaskError`]: Error type distinguishing temporary (retryable) from
//!   permanent failures
//! - [`TaskExecutor`]: Trait for implementing custom task executors
//! - [`OutboxWorker`]: Background worker that claims and processes tasks
//! - [`OutboxConfig`]: Configuration for the outbox worker
//! - [`builtins`]: Ready-to-use executors for email, SMS, and webhooks
//!
//! # Example
//!
//! ```rust,ignore
//! use composable_rust_pg_gateway::tasks::{
//!     OutboxWorker, OutboxConfig, TaskExecutor, TaskError,
//! };
//! use std::sync::Arc;
//! use tokio::sync::broadcast;
//!
//! // Create a custom executor
//! struct MyExecutor;
//!
//! impl TaskExecutor for MyExecutor {
//!     fn task_type(&self) -> &'static str {
//!         "my_task"
//!     }
//!
//!     async fn execute(&self, payload: serde_json::Value) -> Result<(), TaskError> {
//!         // Process the task...
//!         Ok(())
//!     }
//! }
//!
//! // Set up the worker
//! let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
//!
//! let worker = OutboxWorker::builder(pool)
//!     .config(OutboxConfig::default())
//!     .executor(Arc::new(MyExecutor))
//!     .shutdown(shutdown_rx)
//!     .build();
//!
//! // Run the worker (blocks until shutdown)
//! tokio::spawn(async move {
//!     worker.run().await;
//! });
//!
//! // Signal shutdown when done
//! shutdown_tx.send(()).ok();
//! ```
//!
//! # Error Handling
//!
//! Tasks can fail in two ways:
//!
//! - **Temporary failures** ([`TaskError::Temporary`]): The task will be
//!   retried with exponential backoff. Use for transient issues like
//!   network timeouts or rate limiting.
//!
//! - **Permanent failures** ([`TaskError::Permanent`]): The task is moved
//!   to a dead-letter queue and won't be retried. Use for invalid payloads
//!   or unrecoverable errors.
//!
//! # Feature Flags
//!
//! - `tasks`: Core task infrastructure (required)
//! - `tasks-email`: Email executor with SMTP support
//! - `tasks-sms`: SMS executor with pluggable providers
//! - `tasks-webhook`: HTTP webhook executor

mod error;
mod executor;
mod outbox;

pub mod builtins;

pub use error::TaskError;
pub use executor::TaskExecutor;
pub use outbox::{OutboxConfig, OutboxTask, OutboxWorker, OutboxWorkerBuilder};
