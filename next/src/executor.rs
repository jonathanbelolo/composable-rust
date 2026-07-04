//! Call executor trait for saga dispatch

use std::convert::Infallible;
use std::future::Future;

/// Trait for executing aggregate calls from sagas
///
/// This trait abstracts how a saga dispatches calls to aggregate handlers.
/// The [`Handler`](crate::Handler) uses this to execute calls returned by
/// [`BusinessResult::Continue`](crate::BusinessResult::Continue).
///
/// # Type Parameters
///
/// - `C`: Call type (e.g., `SagaCall` enum with variants for each aggregate)
/// - `R`: Result type (e.g., `SagaCallResult` enum with success/failure variants)
///
/// # Implementations
///
/// - [`NoOpCallExecutor`]: For aggregates (which never make calls)
/// - Custom: For sagas, dispatching to aggregate handlers
///
/// # Example: Saga Call Executor
///
/// ```rust,ignore
/// use composable_rust_next::CallExecutor;
///
/// pub struct OrderSagaCallExecutor {
///     order_handler: Arc<Handler<OrderBusinessLogic, NoOpCallExecutor>>,
///     payment_handler: Arc<Handler<PaymentBusinessLogic, NoOpCallExecutor>>,
/// }
///
/// impl CallExecutor<SagaCall, SagaCallResult> for OrderSagaCallExecutor {
///     async fn execute(&self, calls: Vec<SagaCall>) -> Vec<SagaCallResult> {
///         // Execute all calls in parallel
///         let futures: Vec<_> = calls.into_iter()
///             .map(|call| self.execute_single(call))
///             .collect();
///
///         futures::future::join_all(futures).await
///     }
/// }
///
/// impl OrderSagaCallExecutor {
///     async fn execute_single(&self, call: SagaCall) -> SagaCallResult {
///         match call {
///             SagaCall::CreateOrder { order_id, items } => {
///                 match self.order_handler.handle(OrderCommand::Create { order_id, items }).await {
///                     Ok(_) => SagaCallResult::OrderCreated { order_id },
///                     Err(e) => SagaCallResult::OrderFailed { error: e.to_string() },
///                 }
///             }
///             SagaCall::ProcessPayment { order_id, amount } => {
///                 match self.payment_handler.handle(PaymentCommand::Process { order_id, amount }).await {
///                     Ok(_) => SagaCallResult::PaymentProcessed { order_id },
///                     Err(e) => SagaCallResult::PaymentFailed { error: e.to_string() },
///                 }
///             }
///             // Compensation calls...
///             SagaCall::CancelOrder { order_id, reason } => {
///                 match self.order_handler.handle(OrderCommand::Cancel { order_id, reason }).await {
///                     Ok(_) => SagaCallResult::OrderCancelled { order_id },
///                     Err(e) => SagaCallResult::OrderCancelFailed { error: e.to_string() },
///                 }
///             }
///         }
///     }
/// }
/// ```
pub trait CallExecutor<C, R>: Send + Sync {
    /// Execute a batch of calls and return results
    ///
    /// Implementations may execute calls in parallel or sequentially depending
    /// on the use case. Parallel execution maximizes throughput; sequential
    /// execution may be needed for ordering guarantees.
    ///
    /// # Arguments
    ///
    /// - `calls`: The calls to execute (typically from `BusinessResult::Continue`)
    ///
    /// # Returns
    ///
    /// Results from each call, in the same order as the input calls.
    fn execute(&self, calls: Vec<C>) -> impl Future<Output = Vec<R>> + Send;
}

/// Trait for executing a single saga call to completion
///
/// The durable saga loop (`Handler::handle_durable`) dispatches each call
/// individually and consumes completions **as they land**, so it needs a
/// single-call execution primitive instead of [`CallExecutor`]'s
/// batch-in/batch-out shape. The handler owns the unordered-completion
/// machinery; implementations stay trivial.
///
/// # Infallibility
///
/// `execute_one` is infallible at the type level: a failed call is an `R`
/// variant the saga handles in `process` (retry/backoff policy is
/// application-side). Implementations should convert transport errors into
/// a failure variant of `R` rather than panicking.
///
/// # Relationship to [`CallExecutor`]
///
/// The durable handler entry points require both traits (the `Handler`
/// struct itself is bounded on [`CallExecutor`]). Implementing the batch
/// method on top of `execute_one` is a one-liner:
///
/// ```rust,ignore
/// impl CallExecutor<MyCall, MyResult> for MyExecutor {
///     async fn execute(&self, calls: Vec<MyCall>) -> Vec<MyResult> {
///         futures::future::join_all(calls.into_iter().map(|c| self.execute_one(c))).await
///     }
/// }
/// ```
pub trait UnitCallExecutor<C, R>: Send + Sync {
    /// Execute a single call to completion.
    ///
    /// The returned future is polled concurrently with all other in-flight
    /// calls of the saga run; it may be dropped before completion when the
    /// run is cancelled (the call is then re-dispatched on resume).
    fn execute_one(&self, call: C) -> impl Future<Output = R> + Send;
}

impl UnitCallExecutor<Infallible, Infallible> for NoOpCallExecutor {
    /// Never invoked: an `Infallible` call cannot be constructed.
    async fn execute_one(&self, call: Infallible) -> Infallible {
        match call {}
    }
}

/// No-op executor for aggregates (which never make calls)
///
/// This is the canonical executor for aggregates. Since aggregates use
/// `Infallible` for their `Call` and `CallResult` types, this executor
/// is never actually invoked—the type system guarantees it.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_next::{Handler, NoOpCallExecutor};
///
/// let handler = Handler::new(
///     EventBusinessLogic,
///     event_store,
///     event_bus,
///     clock,
///     NoOpCallExecutor,  // Aggregates use this
///     "event-events".to_string(),
/// );
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOpCallExecutor;

impl NoOpCallExecutor {
    /// Create a new no-op executor
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl CallExecutor<Infallible, Infallible> for NoOpCallExecutor {
    /// Execute calls (never actually called for aggregates)
    ///
    /// Since `Infallible` can never be constructed, this method is never
    /// invoked in practice. It exists only to satisfy the trait bound.
    async fn execute(&self, _calls: Vec<Infallible>) -> Vec<Infallible> {
        // Infallible means this is never called—aggregates always return Done,
        // never Continue, so the handler never reaches the call execution path.
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn noop_executor_returns_empty() {
        let executor = NoOpCallExecutor::new();
        let result: Vec<Infallible> = executor.execute(Vec::new()).await;
        assert!(result.is_empty());
    }
}
