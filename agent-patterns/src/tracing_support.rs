//! Distributed Tracing Support for Agent Patterns (Phase 8.4)
//!
//! This module provides OpenTelemetry integration for agent patterns using
//! the `tracing` crate with `tracing-opentelemetry` bridge. This is the
//! idiomatic Rust approach for distributed tracing.
//!
//! ## Architecture
//!
//! - Use `tracing` macros (`#[instrument]`, `info!`, etc.) for all instrumentation
//! - Use `tracing-opentelemetry` subscriber to export spans to Jaeger/OTLP
//! - Span context is automatically propagated via `tracing`'s thread-local storage
//!
//! ## Usage
//!
//! ```ignore
//! // At application startup
//! tracing_support::init_tracing("my-service", "localhost:6831")?;
//!
//! // Wrap any reducer
//! let traced_reducer = TracedReducer::new(my_reducer, "my-agent".to_string());
//!
//! // Spans are automatically created and exported
//! ```

use composable_rust_core::{
    effect::Effect,
    reducer::Reducer,
    agent::AgentEnvironment,
};
use smallvec::SmallVec;
use tracing::{info, span, Level, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use std::time::Instant;

/// Wrapper reducer that adds distributed tracing to any reducer
///
/// Automatically creates OpenTelemetry spans for all reduce operations,
/// recording execution time and effect counts as span attributes.
pub struct TracedReducer<R> {
    inner: R,
    service_name: String,
}

impl<R> TracedReducer<R> {
    /// Create a new traced reducer wrapper
    ///
    /// # Arguments
    ///
    /// * `inner` - The reducer to wrap
    /// * `service_name` - Service name for tracing (e.g., "agent-server")
    pub fn new(inner: R, service_name: String) -> Self {
        Self {
            inner,
            service_name,
        }
    }

    /// Get reference to inner reducer
    pub fn inner(&self) -> &R {
        &self.inner
    }

    /// Get service name
    pub fn service_name(&self) -> &str {
        &self.service_name
    }
}

impl<R, E> Reducer for TracedReducer<R>
where
    R: Reducer<Environment = E>,
    R::State: Clone,
    R::Action: std::fmt::Debug + Clone,
    E: AgentEnvironment,
{
    type State = R::State;
    type Action = R::Action;
    type Environment = E;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        // Create span for this reduction
        let span = span!(
            Level::INFO,
            "agent.reduce",
            service.name = %self.service_name,
            otel.kind = "internal",
            agent.action = ?action,
        );
        let _guard = span.enter();

        let start = Instant::now();

        // Execute inner reducer
        let effects = self.inner.reduce(state, action, env);

        // Record span attributes
        let duration_ms = start.elapsed().as_millis();
        span.record("agent.effects.count", effects.len());
        span.record("agent.duration_ms", duration_ms);

        // Log completion
        if effects.is_empty() {
            info!("Reducer produced no effects");
        } else {
            info!(
                effects_count = effects.len(),
                duration_ms = duration_ms,
                "Reducer execution complete"
            );
        }

        effects
    }
}

/// Initialize tracing with OpenTelemetry Jaeger exporter
///
/// Call this at application startup before creating any agents.
///
/// # Arguments
///
/// * `service_name` - Name of the service (e.g., "agent-server")
/// * `jaeger_endpoint` - Jaeger agent endpoint (e.g., "localhost:6831")
///
/// # Errors
///
/// Returns error if Jaeger pipeline initialization fails.
///
/// # Example
///
/// ```ignore
/// use agent_patterns::tracing_support;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     tracing_support::init_tracing("agent-server", "localhost:6831")?;
///
///     // ... rest of application
///     Ok(())
/// }
/// ```
pub fn init_tracing(
    service_name: &str,
    jaeger_endpoint: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use tracing_subscriber::{layer::SubscriberExt, Registry};
    use opentelemetry_jaeger::new_agent_pipeline;

    // Create Jaeger tracer
    let tracer = new_agent_pipeline()
        .with_service_name(service_name)
        .with_endpoint(jaeger_endpoint)
        .install_simple()?;

    // Create OpenTelemetry layer
    let opentelemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    // Create subscriber with OpenTelemetry and console logging
    let subscriber = Registry::default()
        .with(opentelemetry)
        .with(tracing_subscriber::fmt::layer());

    // Set as global default
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Tracing initialized for service: {}", service_name);

    Ok(())
}

/// Shutdown tracing and flush any pending spans
///
/// Call this during graceful shutdown to ensure all spans are exported.
pub fn shutdown_tracing() {
    opentelemetry::global::shutdown_tracer_provider();
}

/// Get current span context for manual propagation
///
/// Use this when you need to manually propagate trace context
/// (e.g., across non-standard boundaries).
pub fn current_span_context() -> Option<opentelemetry::Context> {
    let span = Span::current();
    if span.is_none() {
        return None;
    }

    Some(span.context())
}

#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_core::agent::{
        AgentAction, AgentConfig, BasicAgentState, MessagesRequest, Tool,
    };

    /// Simple mock environment for testing
    struct MockEnvironment {
        config: AgentConfig,
    }

    impl MockEnvironment {
        fn new() -> Self {
            Self {
                config: AgentConfig {
                    model: "test-model".to_string(),
                    max_tokens: 1024,
                    system_prompt: None,
                },
            }
        }
    }

    impl AgentEnvironment for MockEnvironment {
        fn tools(&self) -> &[Tool] {
            &[]
        }

        fn config(&self) -> &AgentConfig {
            &self.config
        }

        fn call_claude(&self, _request: MessagesRequest) -> Effect<AgentAction> {
            Effect::None
        }

        fn call_claude_streaming(&self, _request: MessagesRequest) -> Effect<AgentAction> {
            Effect::None
        }

        fn execute_tool(
            &self,
            _tool_use_id: String,
            _tool_name: String,
            _tool_input: String,
        ) -> Effect<AgentAction> {
            Effect::None
        }

        fn execute_tool_streaming(
            &self,
            _tool_use_id: String,
            _tool_name: String,
            _tool_input: String,
        ) -> Effect<AgentAction> {
            Effect::None
        }
    }

    /// Simple test reducer that returns a fixed number of effects
    struct TestReducer {
        effect_count: usize,
    }

    impl Reducer for TestReducer {
        type State = BasicAgentState;
        type Action = AgentAction;
        type Environment = MockEnvironment;

        fn reduce(
            &self,
            _state: &mut Self::State,
            _action: Self::Action,
            _env: &Self::Environment,
        ) -> SmallVec<[Effect<Self::Action>; 4]> {
            let mut effects = SmallVec::new();
            for _ in 0..self.effect_count {
                effects.push(Effect::None);
            }
            effects
        }
    }

    #[test]
    fn test_traced_reducer_wraps_inner() {
        let inner = TestReducer { effect_count: 2 };
        let traced = TracedReducer::new(inner, "test-service".to_string());

        assert_eq!(traced.service_name(), "test-service");
        assert_eq!(traced.inner().effect_count, 2);
    }

    #[test]
    fn test_traced_reducer_preserves_effects() {
        // Use tracing_subscriber::fmt for test output
        let subscriber = tracing_subscriber::fmt()
            .with_test_writer()
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let inner = TestReducer { effect_count: 3 };
            let traced = TracedReducer::new(inner, "test".to_string());

            let mut state = BasicAgentState::new(AgentConfig {
                model: "test-model".to_string(),
                max_tokens: 1024,
                system_prompt: None,
            });
            let action = AgentAction::UserMessage {
                content: "test".to_string(),
            };
            let env = MockEnvironment::new();

            let effects = traced.reduce(&mut state, action, &env);

            assert_eq!(effects.len(), 3);
        });
    }

    #[test]
    fn test_traced_reducer_with_zero_effects() {
        let subscriber = tracing_subscriber::fmt()
            .with_test_writer()
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let inner = TestReducer { effect_count: 0 };
            let traced = TracedReducer::new(inner, "test".to_string());

            let mut state = BasicAgentState::new(AgentConfig {
                model: "test-model".to_string(),
                max_tokens: 1024,
                system_prompt: None,
            });
            let action = AgentAction::UserMessage {
                content: "test".to_string(),
            };
            let env = MockEnvironment::new();

            let effects = traced.reduce(&mut state, action, &env);

            assert_eq!(effects.len(), 0);
        });
    }

    #[test]
    fn test_traced_reducer_multiple_calls() {
        let subscriber = tracing_subscriber::fmt()
            .with_test_writer()
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let inner = TestReducer { effect_count: 1 };
            let traced = TracedReducer::new(inner, "test".to_string());

            let mut state = BasicAgentState::new(AgentConfig {
                model: "test-model".to_string(),
                max_tokens: 1024,
                system_prompt: None,
            });
            let env = MockEnvironment::new();

            // First call
            let effects1 = traced.reduce(
                &mut state,
                AgentAction::UserMessage {
                    content: "msg1".to_string(),
                },
                &env,
            );

            // Second call
            let effects2 = traced.reduce(
                &mut state,
                AgentAction::UserMessage {
                    content: "msg2".to_string(),
                },
                &env,
            );

            assert_eq!(effects1.len(), 1);
            assert_eq!(effects2.len(), 1);
        });
    }

    #[test]
    fn test_current_span_context_without_span() {
        // Outside any span, should return None or empty context
        let context = current_span_context();
        // This test just verifies the function doesn't panic
        let _ctx = context;
    }
}
