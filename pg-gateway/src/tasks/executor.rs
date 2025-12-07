//! Task executor trait.
//!
//! This module defines the [`TaskExecutor`] trait that all task executors must implement.
//! Executors are responsible for executing specific task types (email, SMS, webhook, etc.).

use super::error::TaskError;
use std::future::Future;
use std::pin::Pin;

/// Trait for executing background tasks.
///
/// Implement this trait to create custom task executors. Each executor handles
/// a specific task type and is responsible for:
///
/// 1. Deserializing the task payload
/// 2. Executing the task (sending email, making HTTP call, etc.)
/// 3. Returning success or an appropriate error type
///
/// # Task Types
///
/// The `task_type()` method returns a static string identifying what kind of
/// tasks this executor handles. This must match the `task_type` field in the
/// `outbox.pending_tasks` table.
///
/// # Error Handling
///
/// Return [`TaskError::Temporary`] for transient failures that should be retried,
/// and [`TaskError::Permanent`] for failures that won't be fixed by retrying.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::tasks::{TaskExecutor, TaskError};
/// use std::pin::Pin;
/// use std::future::Future;
///
/// struct MyExecutor;
///
/// impl TaskExecutor for MyExecutor {
///     fn task_type(&self) -> &'static str {
///         "my_task"
///     }
///
///     fn execute(
///         &self,
///         payload: serde_json::Value,
///     ) -> Pin<Box<dyn Future<Output = Result<(), TaskError>> + Send + '_>> {
///         Box::pin(async move {
///             let data: MyTaskPayload = serde_json::from_value(payload)
///                 .map_err(|e| TaskError::Permanent(format!("Invalid payload: {e}")))?;
///
///             // Do the work...
///             do_something(&data).await
///                 .map_err(|e| TaskError::Temporary(format!("Failed: {e}")))?;
///
///             Ok(())
///         })
///     }
/// }
/// ```
pub trait TaskExecutor: Send + Sync {
    /// Returns the task type this executor handles.
    ///
    /// This must be a static string that matches the `task_type` column
    /// in the `outbox.pending_tasks` table.
    ///
    /// Common task types:
    /// - `"send_email"` - Email delivery
    /// - `"send_sms"` - SMS delivery
    /// - `"webhook"` - HTTP webhook calls
    fn task_type(&self) -> &'static str;

    /// Execute the task with the given payload.
    ///
    /// # Arguments
    ///
    /// * `payload` - The task payload as a JSON value. The executor should
    ///   deserialize this into its expected format.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Task completed successfully
    /// * `Err(TaskError::Temporary(_))` - Task failed but should be retried
    /// * `Err(TaskError::Permanent(_))` - Task failed and should not be retried
    ///
    /// # Errors
    ///
    /// Returns [`TaskError::Temporary`] for transient failures (network issues,
    /// rate limiting, service unavailable) and [`TaskError::Permanent`] for
    /// permanent failures (invalid payload, authentication error, bad request).
    fn execute(
        &self,
        payload: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<(), TaskError>> + Send + '_>>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    struct TestExecutor {
        should_fail: bool,
        fail_permanently: bool,
    }

    impl TaskExecutor for TestExecutor {
        fn task_type(&self) -> &'static str {
            "test_task"
        }

        fn execute(
            &self,
            _payload: serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = Result<(), TaskError>> + Send + '_>> {
            let should_fail = self.should_fail;
            let fail_permanently = self.fail_permanently;
            Box::pin(async move {
                if should_fail {
                    if fail_permanently {
                        Err(TaskError::permanent("permanent failure"))
                    } else {
                        Err(TaskError::temporary("temporary failure"))
                    }
                } else {
                    Ok(())
                }
            })
        }
    }

    #[tokio::test]
    async fn executor_success() {
        let executor = TestExecutor {
            should_fail: false,
            fail_permanently: false,
        };

        assert_eq!(executor.task_type(), "test_task");
        let result = executor.execute(serde_json::json!({})).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn executor_temporary_failure() {
        let executor = TestExecutor {
            should_fail: true,
            fail_permanently: false,
        };

        let result = executor.execute(serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().is_temporary());
    }

    #[tokio::test]
    async fn executor_permanent_failure() {
        let executor = TestExecutor {
            should_fail: true,
            fail_permanently: true,
        };

        let result = executor.execute(serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().is_permanent());
    }
}
