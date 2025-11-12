//! Span Context Propagation for Agents (Phase 8.4 Part 1.2)
//!
//! This module provides utilities for propagating trace context through
//! agent actions without modifying the `AgentAction` enum. It uses tracing's
//! span context which is automatically propagated via thread-local storage.
//!
//! ## Design
//!
//! Instead of wrapping `AgentAction` (which we can't do from an external module),
//! we use `tracing::Span` context which is the idiomatic Rust approach.
//!
//! ## Usage
//!
//! ```ignore
//! use agent_patterns::context_propagation::SpanContext;
//!
//! // Create span for an action
//! let span = SpanContext::for_action("user_message");
//! let _guard = span.enter();
//!
//! // Dispatch action - span context is automatically propagated
//! store.dispatch(AgentAction::UserMessage { content }).await;
//! ```

use tracing::{info_span, Span};
use uuid::Uuid;

/// Utilities for span context management
pub struct SpanContext;

impl SpanContext {
    /// Create a span for an agent action
    ///
    /// # Arguments
    ///
    /// * `action_type` - Type of action (e.g., `"user_message"`, `"claude_response"`)
    ///
    /// # Returns
    ///
    /// A `Span` that should be entered before dispatching the action
    ///
    /// # Example
    ///
    /// ```ignore
    /// let span = SpanContext::for_action("user_message");
    /// let _guard = span.enter();
    /// store.dispatch(action).await;
    /// ```
    #[must_use]
    pub fn for_action(action_type: &str) -> Span {
        info_span!(
            "agent_action",
            action_type = action_type,
            action_id = %Uuid::new_v4(),
        )
    }

    /// Create a span for a tool execution
    ///
    /// # Arguments
    ///
    /// * `tool_name` - Name of the tool being executed
    /// * `tool_use_id` - Unique ID for this tool execution
    #[must_use]
    pub fn for_tool(tool_name: &str, tool_use_id: &str) -> Span {
        info_span!(
            "tool_execution",
            tool_name = tool_name,
            tool_use_id = tool_use_id,
        )
    }

    /// Create a span for a pattern execution
    ///
    /// # Arguments
    ///
    /// * `pattern_type` - Type of pattern (e.g., "`prompt_chain`", "`routing`")
    /// * `pattern_id` - Unique ID for this pattern execution
    #[must_use]
    pub fn for_pattern(pattern_type: &str, pattern_id: &str) -> Span {
        info_span!(
            "agent_pattern",
            pattern_type = pattern_type,
            pattern_id = pattern_id,
        )
    }
}

/// Extension trait for Store to enable traced dispatch
///
/// This trait cannot be implemented in this crate since Store is in runtime,
/// but it documents the pattern for users to implement.
///
/// # Example
///
/// ```ignore
/// use async_trait::async_trait;
/// use composable_rust_runtime::Store;
/// use agent_patterns::context_propagation::SpanContext;
///
/// #[async_trait]
/// pub trait StoreTracing<A> {
///     async fn dispatch_traced(&self, action: A, action_type: &str);
/// }
///
/// #[async_trait]
/// impl<S, A, E, R> StoreTracing<A> for Store<S, A, E, R>
/// where
///     S: Clone + Send + Sync + 'static,
///     A: Clone + Send + Sync + 'static,
///     E: Send + Sync + 'static,
///     R: Reducer<State = S, Action = A, Environment = E> + Send + Sync + 'static,
/// {
///     async fn dispatch_traced(&self, action: A, action_type: &str) {
///         let span = SpanContext::for_action(action_type);
///         let _guard = span.enter();
///         self.dispatch(action).await;
///     }
/// }
/// ```
pub struct StoreTracingPattern;

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::Level;

    #[test]
    fn test_span_for_action_creates_span() {
        let subscriber = tracing_subscriber::fmt()
            .with_test_writer()
            .with_max_level(Level::INFO)
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let span = SpanContext::for_action("test_action");
            assert!(!span.is_none(), "Span should be created");

            // Enter span to test it works
            let _guard = span.enter();
        });
    }

    #[test]
    fn test_span_for_tool_creates_span() {
        let subscriber = tracing_subscriber::fmt()
            .with_test_writer()
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let span = SpanContext::for_tool("test_tool", "tool_123");
            assert!(!span.is_none());
        });
    }

    #[test]
    fn test_span_for_pattern_creates_span() {
        let subscriber = tracing_subscriber::fmt()
            .with_test_writer()
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let span = SpanContext::for_pattern("prompt_chain", "chain_456");
            assert!(!span.is_none());
        });
    }

    #[test]
    fn test_nested_spans() {
        let subscriber = tracing_subscriber::fmt()
            .with_test_writer()
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let outer_span = SpanContext::for_action("outer");
            let _outer_guard = outer_span.enter();

            let inner_span = SpanContext::for_tool("inner_tool", "tool_789");
            let _inner_guard = inner_span.enter();

            // Both spans are active, inner is current
            // This tests that span nesting works correctly
        });
    }
}
