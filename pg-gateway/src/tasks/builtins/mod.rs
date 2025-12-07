//! Built-in task executors.
//!
//! This module provides ready-to-use task executors for common
//! background task types:
//!
//! - **Email**: Send emails via SMTP with template support
//! - **SMS**: Send SMS messages via configurable providers
//! - **Webhook**: Send HTTP webhooks to external services
//!
//! # Feature Flags
//!
//! Each executor is gated behind a feature flag:
//!
//! - `tasks-email`: Email executor (requires `lettre` and `tera`)
//! - `tasks-sms`: SMS executor
//! - `tasks-webhook`: Webhook executor (requires `reqwest`)
//!
//! # Example
//!
//! ```rust,ignore
//! use composable_rust_pg_gateway::tasks::{
//!     OutboxWorker,
//!     builtins::{EmailExecutor, SmsExecutor, WebhookExecutor},
//! };
//!
//! // Register all executors with the outbox worker
//! let worker = OutboxWorker::builder(pool)
//!     .executor(Arc::new(email_executor))
//!     .executor(Arc::new(sms_executor))
//!     .executor(Arc::new(webhook_executor))
//!     .shutdown(shutdown_rx)
//!     .build();
//! ```

#[cfg(feature = "tasks-email")]
mod email;
#[cfg(feature = "tasks-sms")]
mod sms;
#[cfg(feature = "tasks-webhook")]
mod webhook;

#[cfg(feature = "tasks-email")]
pub use email::{EmailConfig, EmailExecutor, EmailTask};

#[cfg(feature = "tasks-sms")]
pub use sms::{
    ConsoleSmsProvider, DynSmsExecutor, SmsExecutor, SmsProvider, SmsTask,
};

#[cfg(feature = "tasks-webhook")]
pub use webhook::{WebhookConfig, WebhookExecutor, WebhookTask};
