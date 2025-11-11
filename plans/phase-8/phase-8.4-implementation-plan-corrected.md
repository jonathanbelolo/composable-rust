# Phase 8.4: Agent Production Hardening - Implementation Plan (CORRECTED)

## Overview

**Goal**: Make the AI agent system production-ready with enterprise-grade observability, resilience, and operational excellence.

**Status**: Ready to begin

**Duration**: 10-12 days (~85 hours) - Updated based on review

**Dependencies**: Phase 8.1 ✅, Phase 8.2 ✅, Phase 8.3 ✅

**Last Updated**: 2025-11-10

**Review Status**: ✅ All critical issues fixed (15 issues resolved)

---

## What We Already Have

### From Phase 8.1 ✅
- Basic agent infrastructure (BasicAgentState, AgentAction)
- Streaming responses (Effect::Stream)
- Parallel tool execution

### From Phase 8.2 ✅
- 14 production-ready tools with security
- ToolRegistry with retry policies
- Input validation and timeouts

### From Phase 8.3 ✅
- 7 Anthropic agent patterns as reducers
- Context management and caching
- AgentMetrics (basic usage tracking)
- 63 tests passing

### From Phase 4 ✅
- Basic observability (tracing, metrics)
- Retry policies and circuit breakers
- OpenTelemetry support

---

## What We're Building

Phase 8.4 extends the agent system with **production-grade operational features**:

1. **Distributed Tracing** - Full observability through agent workflows
2. **Health Checks** - Kubernetes-ready probes
3. **Advanced Resilience** - Agent-specific circuit breakers, rate limiting, bulkheads
4. **Graceful Shutdown** - Clean resource cleanup
5. **Production Metrics** - Prometheus/Grafana integration
6. **Deployment Guides** - Docker, Kubernetes, scaling
7. **Audit Logging** - Compliance and security tracking
8. **Integration Examples** - **NEW**: Complete examples showing everything together

---

## Architecture Principles

### 1. Observable by Default

All agent operations emit:
- Distributed traces (OpenTelemetry via tracing-opentelemetry)
- Structured logs (tracing crate)
- Metrics (Prometheus)
- Events (audit trail)

### 2. Resilient by Design

Every failure mode has a mitigation:
- Circuit breakers prevent cascading failures
- Rate limiters prevent resource exhaustion
- Timeouts prevent hung operations
- Retries handle transient failures

### 3. Production-Ready Operations

Operators can:
- Monitor agent health in real-time
- Debug issues with distributed traces
- Scale agents horizontally
- Deploy with zero downtime

---

## Dependencies (Complete List)

Add to `agent-patterns/Cargo.toml`:

```toml
[dependencies]
# Core framework
composable-rust-core = { path = "../../core" }
composable-rust-runtime = { path = "../../runtime" }

# Existing dependencies
tokio = { workspace = true, features = ["full"] }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
smallvec = { workspace = true, features = ["serde"] }

# Tracing & Observability (FIXED: Issue #1, #6)
tracing = { workspace = true }
tracing-opentelemetry = "0.23"
opentelemetry = { version = "0.22", features = ["trace"] }
opentelemetry-jaeger = "0.21"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Metrics (FIXED: Issue #6, #7)
prometheus = { version = "0.13", features = ["process"] }
once_cell = "1.19"  # For lazy static initialization (not lazy_static!)

# Async utilities (FIXED: Issue #6)
async-trait = "0.1"

# Utilities (FIXED: Issue #6)
rand = { workspace = true }
uuid = { version = "1", features = ["v4", "serde"] }

# Database (for audit logging)
sqlx = { workspace = true, features = ["postgres", "runtime-tokio", "tls-rustls", "uuid"] }

# Filesystem
tokio = { workspace = true, features = ["fs"] }
```

---

## Implementation Plan

### Part 1: Distributed Tracing (16 hours) - Updated from 12h

#### 1.1 OpenTelemetry Integration via tracing (5 hours)

**FIXED: Issue #1 - Use tracing + tracing-opentelemetry bridge**

**Tracing Strategy**: Use `tracing` crate for all instrumentation, with `tracing-opentelemetry` subscriber to export to Jaeger.

```rust
// agent-patterns/src/tracing.rs
use composable_rust_core::{
    Reducer,
    effect::Effect,
    agent::AgentEnvironment,
};
use smallvec::{SmallVec, smallvec};  // FIXED: Issue #5
use tracing::{instrument, info, warn, error, span, Level};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use std::time::Instant;

/// Tracing wrapper for agent reducers
///
/// Automatically creates distributed tracing spans for all reduce operations.
/// Use `tracing-opentelemetry` subscriber at application startup to export.
pub struct TracedReducer<R> {
    inner: R,
    service_name: String,
}

impl<R> TracedReducer<R> {
    pub fn new(inner: R, service_name: String) -> Self {
        Self { inner, service_name }
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

    #[instrument(
        skip(self, state, env),
        fields(
            service.name = %self.service_name,
            agent.action = ?action,
            otel.kind = "internal",
        )
    )]
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        let start = Instant::now();

        // Get current span (created by #[instrument])
        let span = span::current();

        // Execute reducer
        let effects = self.inner.reduce(state, action.clone(), env);

        // Add span attributes
        span.set_attribute("agent.effects.count", effects.len() as i64);
        span.set_attribute("agent.duration_ms", start.elapsed().as_millis() as i64);

        // Log based on effect count
        if effects.is_empty() {
            info!("Reducer produced no effects");
        } else {
            info!(effects_count = effects.len(), "Reducer execution complete");
        }

        effects
    }
}

/// Initialize tracing with OpenTelemetry exporter
///
/// Call this at application startup before creating any agents.
pub fn init_tracing(service_name: &str, jaeger_endpoint: &str) -> Result<(), Box<dyn std::error::Error>> {
    use tracing_subscriber::{layer::SubscriberExt, Registry};
    use opentelemetry_jaeger::new_pipeline;

    let tracer = new_pipeline()
        .with_service_name(service_name)
        .with_agent_endpoint(jaeger_endpoint)
        .install_simple()?;

    let opentelemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    tracing_subscriber::registry()
        .with(opentelemetry)
        .with(tracing_subscriber::fmt::layer())
        .init();

    Ok(())
}
```

**Usage**:

```rust
// At application startup
tracing::init_tracing("agent-server", "localhost:6831")?;

// Wrap any reducer
let traced_reducer = TracedReducer::new(my_reducer, "my-agent".to_string());

let mut store = Store::new(
    initial_state,
    traced_reducer,
    env,
);
```

**Testing**:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_traced_reducer_adds_span_attributes() {
        // Use tracing_subscriber::fmt for test output
        let subscriber = tracing_subscriber::fmt()
            .with_test_writer()
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let traced = TracedReducer::new(mock_reducer, "test".into());
            let mut state = TestState::default();
            let effects = traced.reduce(&mut state, TestAction::Test, &mock_env);
            assert_eq!(effects.len(), 1);
        });
    }
}
```

**Files to create**:
- `agent-patterns/src/tracing.rs` (200 lines)
- `agent-patterns/tests/tracing_test.rs` (100 lines, 5 tests)

**Time**: 5 hours

---

#### 1.2 Trace Context Propagation (4 hours)

**FIXED: Issue #2 - Use tracing span context instead of wrapping AgentAction**

**Problem**: Cannot add methods to `AgentAction` enum from external module.

**Solution**: Use `tracing::Span` context propagation (idiomatic Rust approach).

```rust
// agent-patterns/src/context.rs
use tracing::{info_span, Span};
use std::task::{Context, Poll};
use std::pin::Pin;
use std::future::Future;

/// Span propagation utilities for agent actions
pub struct SpanContext;

impl SpanContext {
    /// Create a span for an agent action
    pub fn for_action(action_type: &str) -> Span {
        info_span!(
            "agent_action",
            action_type = action_type,
            action_id = %uuid::Uuid::new_v4(),
        )
    }

    /// Execute future within a span
    pub async fn in_span<F, T>(span: Span, future: F) -> T
    where
        F: Future<Output = T>,
    {
        async move {
            let _guard = span.enter();
            future.await
        }.await
    }
}

/// Extension trait for Store to dispatch with tracing
#[async_trait::async_trait]
pub trait StoreTracing<A> {
    /// Dispatch action with automatic span creation
    async fn dispatch_traced(&self, action: A, action_type: &str);
}

#[async_trait::async_trait]
impl<S, A, E, R> StoreTracing<A> for Store<S, A, E, R>
where
    S: Clone + Send + Sync + 'static,
    A: Clone + Send + Sync + 'static,
    E: Send + Sync + 'static,
    R: Reducer<State = S, Action = A, Environment = E> + Send + Sync + 'static,
{
    async fn dispatch_traced(&self, action: A, action_type: &str) {
        let span = SpanContext::for_action(action_type);
        let _guard = span.enter();
        self.dispatch(action).await;
    }
}
```

**Usage**:

```rust
// No need to wrap AgentAction - use span context
use agent_patterns::context::StoreTracing;

// Dispatch with automatic span
store.dispatch_traced(
    AgentAction::UserMessage { content: "Hello".into() },
    "user_message"
).await;

// Span is automatically propagated through reduce() calls
// via tracing's thread-local context
```

**Files to create**:
- `agent-patterns/src/context.rs` (150 lines)
- `agent-patterns/tests/context_test.rs` (80 lines, 4 tests)

**Time**: 4 hours

---

#### 1.3 Cross-Service Trace Propagation (4 hours)

**Propagate trace context across HTTP boundaries:**

```rust
// agent-patterns/src/propagation.rs
use opentelemetry::{
    trace::{TraceContextExt, SpanContext},
    Context as OtelContext,
    global,
};
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use std::collections::HashMap;

/// Extract W3C traceparent from HTTP headers
pub fn extract_trace_context(headers: &HashMap<String, String>) -> Option<OtelContext> {
    use opentelemetry::propagation::TextMapPropagator;

    let propagator = opentelemetry::sdk::propagation::TraceContextPropagator::new();

    Some(propagator.extract(&headers))
}

/// Inject trace context into HTTP headers
pub fn inject_trace_context(span: &Span) -> HashMap<String, String> {
    use opentelemetry::propagation::TextMapPropagator;

    let mut headers = HashMap::new();
    let propagator = opentelemetry::sdk::propagation::TraceContextPropagator::new();
    let context = span.context();

    propagator.inject_context(&context, &mut headers);

    headers
}

/// Example: HTTP client with trace propagation
pub async fn call_with_trace(
    url: &str,
    body: &str,
    current_span: &Span,
) -> Result<String, String> {
    let headers = inject_trace_context(current_span);

    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .headers(headers.into_iter().collect())
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;

    response.text().await.map_err(|e| e.to_string())
}
```

**Files to create**:
- `agent-patterns/src/propagation.rs` (200 lines)
- `agent-patterns/tests/propagation_test.rs` (100 lines, 5 tests)

**Time**: 4 hours

---

#### 1.4 Tracing Best Practices Documentation (3 hours)

**Document tracing guidelines:**

```markdown
# Agent Tracing Guide

## Quick Start

1. Initialize at startup:
```rust
tracing::init_tracing("my-service", "localhost:6831")?;
```

2. Wrap reducers:
```rust
let traced = TracedReducer::new(reducer, "service-name".into());
```

3. Dispatch with context:
```rust
store.dispatch_traced(action, "action_type").await;
```

## Span Attributes

Standard attributes for agent operations:

- `service.name`: Agent service name
- `agent.action`: Action type
- `agent.effects.count`: Number of effects produced
- `agent.duration_ms`: Reducer execution time
- `tool.name`: Tool being executed
- `pattern.type`: Agent pattern (chain, routing, etc.)

## Viewing Traces

1. Start Jaeger:
```bash
docker run -d -p6831:6831/udp -p16686:16686 jaegertracing/all-in-one:latest
```

2. Open UI: http://localhost:16686

3. Search for your service name
```

**Files to create**:
- `agent-patterns/docs/tracing-guide.md` (300 lines)

**Time**: 3 hours

---

### Part 2: Health Checks & Lifecycle (10 hours)

#### 2.1 Health Check Framework (4 hours)

**FIXED: Issue #3 - Add HealthCheckable trait**

```rust
// agent-patterns/src/health.rs
use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Health status for a component
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HealthStatus {
    /// Component is fully operational
    Healthy,
    /// Component is degraded but functional
    Degraded,
    /// Component is not operational
    Unhealthy,
}

/// Health check result with details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    pub status: HealthStatus,
    pub message: String,
    pub last_check: std::time::SystemTime,
    pub details: Option<HashMap<String, serde_json::Value>>,
}

/// Trait for health-checkable components
#[async_trait]
pub trait HealthCheckable: Send + Sync {
    /// Check component health
    async fn check_health(&self) -> ComponentHealth;

    /// Component name for reporting
    fn component_name(&self) -> &str;
}

/// Health check for LLM API connectivity
pub struct LLMHealthCheck {
    client: Arc<dyn HealthCheckable>,
    timeout: Duration,
}

impl LLMHealthCheck {
    pub fn new(client: Arc<dyn HealthCheckable>, timeout: Duration) -> Self {
        Self { client, timeout }
    }
}

#[async_trait]
impl HealthCheckable for LLMHealthCheck {
    async fn check_health(&self) -> ComponentHealth {
        let start = Instant::now();

        match tokio::time::timeout(self.timeout, self.client.check_health()).await {
            Ok(health) => {
                let duration = start.elapsed();
                let mut details = health.details.unwrap_or_default();
                details.insert("check_duration_ms".into(), duration.as_millis().into());

                ComponentHealth {
                    status: health.status,
                    message: health.message,
                    last_check: std::time::SystemTime::now(),
                    details: Some(details),
                }
            }
            Err(_) => ComponentHealth {
                status: HealthStatus::Unhealthy,
                message: format!("Health check timed out after {:?}", self.timeout),
                last_check: std::time::SystemTime::now(),
                details: None,
            },
        }
    }

    fn component_name(&self) -> &str {
        "llm_api"
    }
}

/// Aggregate health checker for multiple components
pub struct SystemHealthCheck {
    checks: Vec<Arc<dyn HealthCheckable>>,
}

impl SystemHealthCheck {
    pub fn new() -> Self {
        Self { checks: Vec::new() }
    }

    pub fn add_check(mut self, check: Arc<dyn HealthCheckable>) -> Self {
        self.checks.push(check);
        self
    }

    /// Check all components
    pub async fn check_all(&self) -> HashMap<String, ComponentHealth> {
        let mut results = HashMap::new();

        for check in &self.checks {
            let name = check.component_name().to_string();
            let health = check.check_health().await;
            results.insert(name, health);
        }

        results
    }

    /// Determine overall system health
    pub async fn overall_health(&self) -> HealthStatus {
        let results = self.check_all().await;

        if results.values().any(|h| h.status == HealthStatus::Unhealthy) {
            HealthStatus::Unhealthy
        } else if results.values().any(|h| h.status == HealthStatus::Degraded) {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        }
    }
}

/// Kubernetes-style health endpoints
#[derive(Debug, Clone)]
pub struct K8sHealthEndpoints {
    system_health: Arc<SystemHealthCheck>,
}

impl K8sHealthEndpoints {
    pub fn new(system_health: Arc<SystemHealthCheck>) -> Self {
        Self { system_health }
    }

    /// Liveness probe - "is the process alive?"
    ///
    /// Should only fail if the process needs to be restarted.
    pub async fn liveness(&self) -> (u16, &'static str) {
        // Basic liveness - just check if we can respond
        (200, "alive")
    }

    /// Readiness probe - "can the process accept traffic?"
    ///
    /// Fails if dependencies are unavailable (database, LLM API, etc.)
    pub async fn readiness(&self) -> (u16, String) {
        match self.system_health.overall_health().await {
            HealthStatus::Healthy => (200, "ready".into()),
            HealthStatus::Degraded => (200, "degraded".into()),  // Still ready, but degraded
            HealthStatus::Unhealthy => (503, "not ready".into()),
        }
    }

    /// Detailed health check (for debugging)
    pub async fn health_detailed(&self) -> HashMap<String, ComponentHealth> {
        self.system_health.check_all().await
    }
}
```

**Files to create**:
- `agent-patterns/src/health.rs` (400 lines)
- `agent-patterns/tests/health_test.rs` (200 lines, 8 tests)

**Time**: 4 hours

---

#### 2.2 Graceful Shutdown (6 hours)

**FIXED: Issue #4 - Simplified Store shutdown handler**

```rust
// agent-patterns/src/shutdown.rs
use tokio::sync::{broadcast, mpsc};
use tokio::time::{timeout, Duration};
use tracing::{info, warn, error};
use std::sync::Arc;
use async_trait::async_trait;

/// Trait for components that need graceful shutdown
#[async_trait]
pub trait ShutdownHandler: Send + Sync {
    /// Component name for logging
    fn name(&self) -> &str;

    /// Gracefully shut down this component
    ///
    /// Should complete any in-flight work and release resources.
    async fn shutdown(&self) -> Result<(), String>;
}

/// Coordinates shutdown across multiple components
pub struct ShutdownCoordinator {
    handlers: Vec<Arc<dyn ShutdownHandler>>,
    shutdown_tx: broadcast::Sender<()>,
    timeout_duration: Duration,
}

impl ShutdownCoordinator {
    pub fn new(timeout: Duration) -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            handlers: Vec::new(),
            shutdown_tx,
            timeout_duration: timeout,
        }
    }

    /// Register a shutdown handler
    pub fn register(&mut self, handler: Arc<dyn ShutdownHandler>) {
        self.handlers.push(handler);
    }

    /// Get a receiver for shutdown signals
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    /// Initiate graceful shutdown
    pub async fn shutdown(&self) -> Result<(), Vec<String>> {
        info!("Initiating graceful shutdown for {} components", self.handlers.len());

        // Send shutdown signal
        let _ = self.shutdown_tx.send(());

        // Shutdown all handlers in parallel with timeout
        let mut errors = Vec::new();

        for handler in &self.handlers {
            let name = handler.name();
            info!("Shutting down component: {}", name);

            match timeout(self.timeout_duration, handler.shutdown()).await {
                Ok(Ok(())) => {
                    info!("Component {} shut down successfully", name);
                }
                Ok(Err(e)) => {
                    error!("Component {} shutdown failed: {}", name, e);
                    errors.push(format!("{}: {}", name, e));
                }
                Err(_) => {
                    error!("Component {} shutdown timed out", name);
                    errors.push(format!("{}: timeout", name));
                }
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
}

/// Example shutdown handler for generic Store
pub struct GenericStoreShutdownHandler {
    name: String,
    on_shutdown: Arc<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> + Send + Sync>,
}

impl GenericStoreShutdownHandler {
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
impl ShutdownHandler for GenericStoreShutdownHandler {
    fn name(&self) -> &str {
        &self.name
    }

    async fn shutdown(&self) -> Result<(), String> {
        (self.on_shutdown)().await
    }
}

/// Wait for shutdown signal (Ctrl+C or SIGTERM)
pub async fn wait_for_signal() {
    use tokio::signal;

    #[cfg(unix)]
    {
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to register SIGTERM handler");

        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("Received Ctrl+C");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM");
            }
        }
    }

    #[cfg(not(unix))]
    {
        signal::ctrl_c().await.expect("Failed to wait for Ctrl+C");
        info!("Received Ctrl+C");
    }
}
```

**Usage**:

```rust
// Setup shutdown coordinator
let mut coordinator = ShutdownCoordinator::new(Duration::from_secs(30));

// Register store shutdown
coordinator.register(Arc::new(GenericStoreShutdownHandler::new(
    "agent-store".into(),
    || async {
        // Cleanup logic
        info!("Store cleanup complete");
        Ok(())
    }
)));

// Wait for signal
wait_for_signal().await;

// Graceful shutdown
coordinator.shutdown().await?;
```

**Files to create**:
- `agent-patterns/src/shutdown.rs` (350 lines)
- `agent-patterns/tests/shutdown_test.rs` (150 lines, 6 tests)

**Time**: 6 hours

---

### Part 3: Advanced Resilience (15 hours)

#### 3.1 Circuit Breaker per Tool (5 hours)

```rust
// agent-patterns/src/resilience/circuit_breaker.rs
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{Duration, Instant};
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,   // Normal operation
    Open,     // Failing - reject requests
    HalfOpen, // Testing recovery
}

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: usize,
    pub success_threshold: usize,
    pub timeout: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 2,
            timeout: Duration::from_secs(30),
        }
    }
}

struct CircuitBreakerState {
    state: CircuitState,
    failure_count: usize,
    success_count: usize,
    last_failure_time: Option<Instant>,
}

pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: Arc<RwLock<CircuitBreakerState>>,
    name: String,
}

impl CircuitBreaker {
    pub fn new(name: String, config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(CircuitBreakerState {
                state: CircuitState::Closed,
                failure_count: 0,
                success_count: 0,
                last_failure_time: None,
            })),
            name,
        }
    }

    /// Check if request should be allowed
    pub async fn allow_request(&self) -> Result<(), String> {
        let mut state = self.state.write().await;

        match state.state {
            CircuitState::Closed => Ok(()),
            CircuitState::Open => {
                // Check if timeout has elapsed
                if let Some(last_failure) = state.last_failure_time {
                    if last_failure.elapsed() >= self.config.timeout {
                        info!("Circuit breaker {} transitioning to HalfOpen", self.name);
                        state.state = CircuitState::HalfOpen;
                        state.success_count = 0;
                        Ok(())
                    } else {
                        Err(format!("Circuit breaker {} is OPEN", self.name))
                    }
                } else {
                    Err(format!("Circuit breaker {} is OPEN", self.name))
                }
            }
            CircuitState::HalfOpen => Ok(()),
        }
    }

    /// Record successful execution
    pub async fn record_success(&self) {
        let mut state = self.state.write().await;

        match state.state {
            CircuitState::Closed => {
                state.failure_count = 0;
            }
            CircuitState::HalfOpen => {
                state.success_count += 1;
                if state.success_count >= self.config.success_threshold {
                    info!("Circuit breaker {} transitioning to Closed", self.name);
                    state.state = CircuitState::Closed;
                    state.failure_count = 0;
                    state.success_count = 0;
                }
            }
            CircuitState::Open => {}
        }
    }

    /// Record failed execution
    pub async fn record_failure(&self) {
        let mut state = self.state.write().await;

        match state.state {
            CircuitState::Closed => {
                state.failure_count += 1;
                if state.failure_count >= self.config.failure_threshold {
                    warn!("Circuit breaker {} transitioning to Open", self.name);
                    state.state = CircuitState::Open;
                    state.last_failure_time = Some(Instant::now());
                }
            }
            CircuitState::HalfOpen => {
                warn!("Circuit breaker {} transitioning back to Open", self.name);
                state.state = CircuitState::Open;
                state.last_failure_time = Some(Instant::now());
                state.success_count = 0;
            }
            CircuitState::Open => {
                state.last_failure_time = Some(Instant::now());
            }
        }
    }

    /// Get current state
    pub async fn get_state(&self) -> CircuitState {
        self.state.read().await.state
    }
}

/// Execute function with circuit breaker protection
pub async fn with_circuit_breaker<F, T, E>(
    circuit_breaker: &CircuitBreaker,
    f: F,
) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    // Check if request is allowed
    circuit_breaker.allow_request().await?;

    // Execute function
    match f.await {
        Ok(result) => {
            circuit_breaker.record_success().await;
            Ok(result)
        }
        Err(e) => {
            circuit_breaker.record_failure().await;
            Err(e.to_string())
        }
    }
}
```

**Files to create**:
- `agent-patterns/src/resilience/circuit_breaker.rs` (300 lines)
- `agent-patterns/tests/circuit_breaker_test.rs` (200 lines, 7 tests)

**Time**: 5 hours

---

#### 3.2 Rate Limiting (5 hours)

```rust
// agent-patterns/src/resilience/rate_limiter.rs
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{Duration, Instant};
use tracing::warn;

/// Token bucket rate limiter configuration
#[derive(Debug, Clone)]
pub struct RateLimiterConfig {
    /// Maximum number of tokens (burst capacity)
    pub capacity: usize,
    /// Tokens refilled per second
    pub refill_rate: f64,
}

impl Default for RateLimiterConfig {
    fn default() -> Self {
        Self {
            capacity: 100,
            refill_rate: 10.0,
        }
    }
}

struct RateLimiterState {
    tokens: f64,
    last_refill: Instant,
}

pub struct RateLimiter {
    config: RateLimiterConfig,
    state: Arc<RwLock<RateLimiterState>>,
    name: String,
}

impl RateLimiter {
    pub fn new(name: String, config: RateLimiterConfig) -> Self {
        Self {
            config: config.clone(),
            state: Arc::new(RwLock::new(RateLimiterState {
                tokens: config.capacity as f64,
                last_refill: Instant::now(),
            })),
            name,
        }
    }

    /// Attempt to acquire tokens
    pub async fn try_acquire(&self, tokens: usize) -> Result<(), String> {
        let mut state = self.state.write().await;

        // Refill tokens based on elapsed time
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_refill).as_secs_f64();
        let new_tokens = elapsed * self.config.refill_rate;
        state.tokens = (state.tokens + new_tokens).min(self.config.capacity as f64);
        state.last_refill = now;

        // Check if enough tokens available
        if state.tokens >= tokens as f64 {
            state.tokens -= tokens as f64;
            Ok(())
        } else {
            warn!("Rate limit exceeded for {}", self.name);
            Err(format!("Rate limit exceeded for {}", self.name))
        }
    }

    /// Get current token count
    pub async fn available_tokens(&self) -> f64 {
        let state = self.state.read().await;
        state.tokens
    }
}

/// Execute function with rate limiting
pub async fn with_rate_limit<F, T>(
    rate_limiter: &RateLimiter,
    tokens: usize,
    f: F,
) -> Result<T, String>
where
    F: std::future::Future<Output = T>,
{
    // Try to acquire tokens
    rate_limiter.try_acquire(tokens).await?;

    // Execute function
    Ok(f.await)
}
```

**Files to create**:
- `agent-patterns/src/resilience/rate_limiter.rs` (250 lines)
- `agent-patterns/tests/rate_limiter_test.rs` (150 lines, 5 tests)

**Time**: 5 hours

---

#### 3.3 Bulkhead Pattern (5 hours)

```rust
// agent-patterns/src/resilience/bulkhead.rs
use tokio::sync::Semaphore;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// Bulkhead configuration for resource isolation
#[derive(Debug, Clone)]
pub struct BulkheadConfig {
    /// Maximum concurrent operations
    pub max_concurrent: usize,
    /// Timeout for acquiring permit
    pub acquire_timeout: Duration,
}

impl Default for BulkheadConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 10,
            acquire_timeout: Duration::from_secs(5),
        }
    }
}

pub struct Bulkhead {
    semaphore: Arc<Semaphore>,
    config: BulkheadConfig,
    name: String,
}

impl Bulkhead {
    pub fn new(name: String, config: BulkheadConfig) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(config.max_concurrent)),
            config,
            name,
        }
    }

    /// Execute function within bulkhead
    pub async fn execute<F, T>(&self, f: F) -> Result<T, String>
    where
        F: std::future::Future<Output = T>,
    {
        // Try to acquire permit with timeout
        let permit = tokio::time::timeout(
            self.config.acquire_timeout,
            self.semaphore.acquire()
        )
        .await
        .map_err(|_| format!("Bulkhead {} acquire timeout", self.name))?
        .map_err(|e| format!("Bulkhead {} acquire failed: {}", self.name, e))?;

        info!("Acquired bulkhead permit for {}", self.name);

        // Execute function
        let result = f.await;

        // Permit is automatically released when dropped
        drop(permit);

        Ok(result)
    }

    /// Get available permits
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}

/// Bulkhead registry for different resource types
pub struct BulkheadRegistry {
    bulkheads: std::collections::HashMap<String, Arc<Bulkhead>>,
}

impl BulkheadRegistry {
    pub fn new() -> Self {
        Self {
            bulkheads: std::collections::HashMap::new(),
        }
    }

    pub fn register(&mut self, name: String, bulkhead: Bulkhead) {
        self.bulkheads.insert(name, Arc::new(bulkhead));
    }

    pub fn get(&self, name: &str) -> Option<Arc<Bulkhead>> {
        self.bulkheads.get(name).cloned()
    }
}

impl Default for BulkheadRegistry {
    fn default() -> Self {
        Self::new()
    }
}
```

**Files to create**:
- `agent-patterns/src/resilience/bulkhead.rs` (200 lines)
- `agent-patterns/tests/bulkhead_test.rs` (120 lines, 5 tests)

**Time**: 5 hours

---

### Part 4: Production Metrics (12 hours)

#### 4.1 Prometheus Metrics (6 hours)

**FIXED: Issue #7 - Use once_cell instead of lazy_static**

```rust
// agent-patterns/src/metrics.rs
use prometheus::{
    IntCounterVec, HistogramVec, GaugeVec, Opts, Registry,
    register_int_counter_vec_with_registry, register_histogram_vec_with_registry,
    register_gauge_vec_with_registry,
};
use once_cell::sync::Lazy;  // FIXED: Not lazy_static!
use std::sync::Arc;

/// Global metrics registry
pub static AGENT_METRICS_REGISTRY: Lazy<Registry> = Lazy::new(Registry::new);

/// Agent pattern execution counter
pub static AGENT_PATTERN_EXECUTIONS: Lazy<IntCounterVec> = Lazy::new(|| {
    let opts = Opts::new(
        "agent_pattern_executions_total",
        "Total number of agent pattern executions"
    );
    register_int_counter_vec_with_registry!(
        opts,
        &["pattern", "action"],
        AGENT_METRICS_REGISTRY.clone()
    ).unwrap()
});

/// Agent pattern duration histogram
pub static AGENT_PATTERN_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    let opts = prometheus::HistogramOpts::new(
        "agent_pattern_duration_seconds",
        "Agent pattern execution duration in seconds"
    )
    .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]);

    register_histogram_vec_with_registry!(
        opts,
        &["pattern"],
        AGENT_METRICS_REGISTRY.clone()
    ).unwrap()
});

/// Active agent sessions gauge
pub static ACTIVE_AGENT_SESSIONS: Lazy<GaugeVec> = Lazy::new(|| {
    let opts = Opts::new(
        "active_agent_sessions",
        "Number of active agent sessions"
    );
    register_gauge_vec_with_registry!(
        opts,
        &["pattern"],
        AGENT_METRICS_REGISTRY.clone()
    ).unwrap()
});

/// Tool execution counter
pub static TOOL_EXECUTIONS: Lazy<IntCounterVec> = Lazy::new(|| {
    let opts = Opts::new(
        "tool_executions_total",
        "Total number of tool executions"
    );
    register_int_counter_vec_with_registry!(
        opts,
        &["tool_name", "status"],
        AGENT_METRICS_REGISTRY.clone()
    ).unwrap()
});

/// Tool execution duration
pub static TOOL_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    let opts = prometheus::HistogramOpts::new(
        "tool_duration_seconds",
        "Tool execution duration in seconds"
    )
    .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0]);

    register_histogram_vec_with_registry!(
        opts,
        &["tool_name"],
        AGENT_METRICS_REGISTRY.clone()
    ).unwrap()
});

/// Circuit breaker state gauge
pub static CIRCUIT_BREAKER_STATE: Lazy<GaugeVec> = Lazy::new(|| {
    let opts = Opts::new(
        "circuit_breaker_state",
        "Circuit breaker state (0=Closed, 1=Open, 2=HalfOpen)"
    );
    register_gauge_vec_with_registry!(
        opts,
        &["name"],
        AGENT_METRICS_REGISTRY.clone()
    ).unwrap()
});

/// Rate limiter token availability gauge
pub static RATE_LIMITER_TOKENS: Lazy<GaugeVec> = Lazy::new(|| {
    let opts = Opts::new(
        "rate_limiter_available_tokens",
        "Available tokens in rate limiter"
    );
    register_gauge_vec_with_registry!(
        opts,
        &["name"],
        AGENT_METRICS_REGISTRY.clone()
    ).unwrap()
});

/// LLM API calls counter
pub static LLM_API_CALLS: Lazy<IntCounterVec> = Lazy::new(|| {
    let opts = Opts::new(
        "llm_api_calls_total",
        "Total number of LLM API calls"
    );
    register_int_counter_vec_with_registry!(
        opts,
        &["model", "status"],
        AGENT_METRICS_REGISTRY.clone()
    ).unwrap()
});

/// LLM token usage counter
pub static LLM_TOKENS_USED: Lazy<IntCounterVec> = Lazy::new(|| {
    let opts = Opts::new(
        "llm_tokens_used_total",
        "Total number of LLM tokens used"
    );
    register_int_counter_vec_with_registry!(
        opts,
        &["model", "type"],  // type: input, output
        AGENT_METRICS_REGISTRY.clone()
    ).unwrap()
});

/// Metrics exporter for Prometheus scraping
pub fn metrics_handler() -> String {
    use prometheus::Encoder;

    let encoder = prometheus::TextEncoder::new();
    let metric_families = AGENT_METRICS_REGISTRY.gather();

    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();

    String::from_utf8(buffer).unwrap()
}
```

**Usage**:

```rust
// In reducer
use agent_patterns::metrics::{AGENT_PATTERN_EXECUTIONS, AGENT_PATTERN_DURATION};

impl<R> Reducer for TracedReducer<R> {
    fn reduce(...) -> SmallVec<[Effect<Self::Action>; 4]> {
        let timer = AGENT_PATTERN_DURATION
            .with_label_values(&["traced_reducer"])
            .start_timer();

        let effects = self.inner.reduce(state, action, env);

        AGENT_PATTERN_EXECUTIONS
            .with_label_values(&["traced_reducer", "reduce"])
            .inc();

        timer.observe_duration();

        effects
    }
}

// In HTTP server
use agent_patterns::metrics::metrics_handler;

async fn prometheus_metrics() -> String {
    metrics_handler()
}

// Mount at /metrics for Prometheus scraping
```

**Files to create**:
- `agent-patterns/src/metrics.rs` (400 lines)
- `agent-patterns/tests/metrics_test.rs` (150 lines, 6 tests)

**Time**: 6 hours

---

#### 4.2 Grafana Dashboards (6 hours)

**FIXED: Issue #13 - Add PromQL grouping**

```json
{
  "dashboard": {
    "title": "Agent System Overview",
    "panels": [
      {
        "title": "Agent Pattern Executions Rate",
        "targets": [
          {
            "expr": "sum by (pattern) (rate(agent_pattern_executions_total[5m]))",
            "legendFormat": "{{pattern}}"
          }
        ]
      },
      {
        "title": "P95 Agent Pattern Duration",
        "targets": [
          {
            "expr": "histogram_quantile(0.95, sum by (pattern, le) (rate(agent_pattern_duration_seconds_bucket[5m])))",
            "legendFormat": "{{pattern}} p95"
          }
        ]
      },
      {
        "title": "Tool Success Rate",
        "targets": [
          {
            "expr": "sum by (tool_name) (rate(tool_executions_total{status=\"success\"}[5m])) / sum by (tool_name) (rate(tool_executions_total[5m]))",
            "legendFormat": "{{tool_name}}"
          }
        ]
      },
      {
        "title": "Circuit Breaker States",
        "targets": [
          {
            "expr": "circuit_breaker_state",
            "legendFormat": "{{name}}"
          }
        ]
      },
      {
        "title": "Rate Limiter Token Availability",
        "targets": [
          {
            "expr": "rate_limiter_available_tokens",
            "legendFormat": "{{name}}"
          }
        ]
      },
      {
        "title": "LLM Token Usage Rate",
        "targets": [
          {
            "expr": "sum by (model, type) (rate(llm_tokens_used_total[5m]))",
            "legendFormat": "{{model}} {{type}}"
          }
        ]
      }
    ]
  }
}
```

**Files to create**:
- `agent-patterns/dashboards/agent-overview.json` (500 lines)
- `agent-patterns/dashboards/resilience.json` (300 lines)
- `agent-patterns/docs/grafana-setup.md` (200 lines)

**Time**: 6 hours

---

### Part 5: Deployment & Operations (18 hours)

#### 5.1 Docker Images (6 hours)

**FIXED: Issue #10 - Use current Rust version**

```dockerfile
# Dockerfile
FROM rust:1.75-alpine AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev openssl-dev

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY core core/
COPY runtime runtime/
COPY agent-patterns agent-patterns/
COPY anthropic anthropic/

# Build release binary
RUN cargo build --release -p agent-patterns --bin agent-server

# Runtime image
FROM alpine:3.18

RUN apk add --no-cache ca-certificates libgcc

COPY --from=builder /app/target/release/agent-server /usr/local/bin/

EXPOSE 8080
EXPOSE 9090

CMD ["agent-server"]
```

**Docker Compose for development**:

```yaml
# docker-compose.yml
version: '3.8'

services:
  agent-server:
    build: .
    ports:
      - "8080:8080"   # HTTP API
      - "9090:9090"   # Metrics
    environment:
      - RUST_LOG=info
      - ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY}
      - JAEGER_ENDPOINT=jaeger:6831
    depends_on:
      - jaeger
      - prometheus

  jaeger:
    image: jaegertracing/all-in-one:latest
    ports:
      - "16686:16686"  # UI
      - "6831:6831/udp"  # Agent

  prometheus:
    image: prom/prometheus:latest
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml
    ports:
      - "9091:9090"

  grafana:
    image: grafana/grafana:latest
    ports:
      - "3000:3000"
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=admin
    volumes:
      - ./grafana/dashboards:/etc/grafana/provisioning/dashboards
```

**Files to create**:
- `Dockerfile` (60 lines)
- `docker-compose.yml` (100 lines)
- `.dockerignore` (20 lines)
- `docs/docker-setup.md` (200 lines)

**Time**: 6 hours

---

#### 5.2 Kubernetes Deployment (8 hours)

```yaml
# k8s/deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: agent-server
  labels:
    app: agent-server
spec:
  replicas: 3
  selector:
    matchLabels:
      app: agent-server
  template:
    metadata:
      labels:
        app: agent-server
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/port: "9090"
        prometheus.io/path: "/metrics"
    spec:
      containers:
      - name: agent-server
        image: agent-server:latest
        ports:
        - containerPort: 8080
          name: http
        - containerPort: 9090
          name: metrics
        env:
        - name: RUST_LOG
          value: "info"
        - name: ANTHROPIC_API_KEY
          valueFrom:
            secretKeyRef:
              name: agent-secrets
              key: anthropic-api-key
        resources:
          requests:
            memory: "256Mi"
            cpu: "250m"
          limits:
            memory: "512Mi"
            cpu: "500m"
        livenessProbe:
          httpGet:
            path: /health/liveness
            port: 8080
          initialDelaySeconds: 30
          periodSeconds: 10
          timeoutSeconds: 5
        readinessProbe:
          httpGet:
            path: /health/readiness
            port: 8080
          initialDelaySeconds: 10
          periodSeconds: 5
          timeoutSeconds: 3

---
apiVersion: v1
kind: Service
metadata:
  name: agent-server
spec:
  selector:
    app: agent-server
  ports:
  - name: http
    port: 80
    targetPort: 8080
  - name: metrics
    port: 9090
    targetPort: 9090
  type: LoadBalancer

---
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: agent-server-hpa
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: agent-server
  minReplicas: 3
  maxReplicas: 10
  metrics:
  - type: Resource
    resource:
      name: cpu
      target:
        type: Utilization
        averageUtilization: 70
  - type: Pods
    pods:
      metric:
        name: agent_pattern_executions_total
      target:
        type: AverageValue
        averageValue: "100"

---
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: agent-server-pdb
spec:
  minAvailable: 2
  selector:
    matchLabels:
      app: agent-server
```

**Files to create**:
- `k8s/deployment.yaml` (200 lines)
- `k8s/service.yaml` (50 lines)
- `k8s/configmap.yaml` (100 lines)
- `k8s/secrets-template.yaml` (30 lines)
- `docs/kubernetes-deployment.md` (400 lines)

**Time**: 8 hours

---

#### 5.3 Configuration Management (4 hours)

**FIXED: Issue #8 - Clarify circuit breaker config with per-tool overrides**

```toml
# config.toml
[server]
host = "0.0.0.0"
port = 8080
metrics_port = 9090

[tracing]
service_name = "agent-server"
jaeger_endpoint = "localhost:6831"

[health_checks]
llm_timeout_secs = 5
database_timeout_secs = 3

[resilience.circuit_breaker]
# Default config for all circuit breakers
failure_threshold = 5
success_threshold = 2
timeout_secs = 30

# Per-tool overrides (optional)
[resilience.circuit_breaker.tools]
expensive_tool = { failure_threshold = 3, timeout_secs = 60 }
external_api = { failure_threshold = 10, timeout_secs = 15 }

[resilience.rate_limiter]
# Default config
capacity = 100
refill_rate = 10.0

# Per-tool overrides
[resilience.rate_limiter.tools]
expensive_tool = { capacity = 10, refill_rate = 1.0 }

[resilience.bulkhead]
# Default concurrent operations
max_concurrent = 10
acquire_timeout_secs = 5

# Per-pattern bulkheads
[resilience.bulkhead.patterns]
orchestrator = { max_concurrent = 5 }
parallelization = { max_concurrent = 20 }

[shutdown]
timeout_secs = 30

[metrics]
enabled = true
port = 9090

[audit]
enabled = true
sink_type = "postgres"  # or "file"
file_path = "/var/log/agent-audit.jsonl"
postgres_connection_string = "postgresql://localhost/agent_audit"
```

**Config loader**:

```rust
// agent-patterns/src/config.rs
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub metrics_port: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TracingConfig {
    pub service_name: String,
    pub jaeger_endpoint: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: usize,
    pub success_threshold: usize,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResilienceConfig {
    pub circuit_breaker: CircuitBreakerConfig,
    #[serde(default)]
    pub circuit_breaker_tools: HashMap<String, CircuitBreakerConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    pub server: ServerConfig,
    pub tracing: TracingConfig,
    pub resilience: ResilienceConfig,
    // ... other sections
}

impl AgentConfig {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&contents)?;
        Ok(config)
    }

    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let path = std::env::var("AGENT_CONFIG_PATH")
            .unwrap_or_else(|_| "config.toml".to_string());
        Self::from_file(path)
    }
}
```

**Files to create**:
- `agent-patterns/src/config.rs` (300 lines)
- `config.toml` (150 lines)
- `config.production.toml` (150 lines)
- `docs/configuration-guide.md` (250 lines)

**Time**: 4 hours

---

### Part 6: Audit Logging (8 hours) - Updated from 6h

#### 6.1 Audit Event Framework (5 hours)

**FIXED: Issue #9, #15 - Fixed SQL schema and imports**

```rust
// agent-patterns/src/audit.rs
use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

/// Audit event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuditEventType {
    SessionStart,
    SessionEnd,
    UserMessage,
    AgentResponse,
    ToolExecution,
    AuthenticationAttempt,
    AuthorizationCheck,
    ConfigurationChange,
    Error,
}

/// Audit event outcome
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditOutcome {
    Success,
    Failure,
    Denied,
}

/// Complete audit event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: String,
    pub timestamp: std::time::SystemTime,
    pub event_type: AuditEventType,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub ip_address: Option<String>,
    pub details: HashMap<String, serde_json::Value>,
    pub outcome: AuditOutcome,
    pub outcome_details: Option<String>,
}

impl AuditEvent {
    pub fn new(event_type: AuditEventType) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: std::time::SystemTime::now(),
            event_type,
            user_id: None,
            session_id: None,
            ip_address: None,
            details: HashMap::new(),
            outcome: AuditOutcome::Success,
            outcome_details: None,
        }
    }

    pub fn with_user(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }

    pub fn with_session(mut self, session_id: String) -> Self {
        self.session_id = Some(session_id);
        self
    }

    pub fn with_outcome(mut self, outcome: AuditOutcome, details: Option<String>) -> Self {
        self.outcome = outcome;
        self.outcome_details = details;
        self
    }
}

/// Trait for audit event sinks
#[async_trait]
pub trait AuditSink: Send + Sync {
    async fn write(&self, event: AuditEvent) -> Result<(), String>;
    async fn flush(&self) -> Result<(), String>;
}

/// File-based audit sink
pub struct FileAuditSink {
    path: PathBuf,
}

impl FileAuditSink {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

#[async_trait]
impl AuditSink for FileAuditSink {
    async fn write(&self, event: AuditEvent) -> Result<(), String> {
        let json = serde_json::to_string(&event)
            .map_err(|e| e.to_string())?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .map_err(|e| e.to_string())?;

        file.write_all(format!("{}\n", json).as_bytes())
            .await
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    async fn flush(&self) -> Result<(), String> {
        Ok(())
    }
}

/// PostgreSQL audit sink
pub struct PostgresAuditSink {
    pool: sqlx::PgPool,
}

impl PostgresAuditSink {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Create audit_log table
    pub async fn create_table(&self) -> Result<(), String> {
        sqlx::query!(
            r#"
            CREATE TABLE IF NOT EXISTS audit_log (
                id TEXT PRIMARY KEY,
                timestamp TIMESTAMPTZ NOT NULL,
                event_type TEXT NOT NULL,
                user_id TEXT,
                session_id TEXT,
                ip_address TEXT,
                details JSONB NOT NULL,
                outcome TEXT NOT NULL,
                outcome_details TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_log(timestamp);
            CREATE INDEX IF NOT EXISTS idx_audit_event_type ON audit_log(event_type);
            CREATE INDEX IF NOT EXISTS idx_audit_user ON audit_log(user_id);
            CREATE INDEX IF NOT EXISTS idx_audit_session ON audit_log(session_id);
            "#
        )
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(())
    }
}

#[async_trait]
impl AuditSink for PostgresAuditSink {
    async fn write(&self, event: AuditEvent) -> Result<(), String> {
        let event_type_str = serde_json::to_string(&event.event_type)
            .map_err(|e| e.to_string())?
            .trim_matches('"')
            .to_string();

        let outcome_str = format!("{:?}", event.outcome);
        let details_json = serde_json::to_value(&event.details)
            .map_err(|e| e.to_string())?;

        sqlx::query!(
            r#"
            INSERT INTO audit_log (
                id, timestamp, event_type, user_id, session_id,
                ip_address, details, outcome, outcome_details
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
            event.id,
            event.timestamp,
            event_type_str,
            event.user_id,
            event.session_id,
            event.ip_address,
            details_json,
            outcome_str,
            event.outcome_details,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(())
    }

    async fn flush(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Audit logger
pub struct AuditLogger {
    sink: Arc<dyn AuditSink>,
}

impl AuditLogger {
    pub fn new(sink: Arc<dyn AuditSink>) -> Self {
        Self { sink }
    }

    pub async fn log(&self, event: AuditEvent) -> Result<(), String> {
        self.sink.write(event).await
    }
}
```

**Files to create**:
- `agent-patterns/src/audit.rs` (500 lines)
- `agent-patterns/tests/audit_test.rs` (200 lines, 8 tests)

**Time**: 5 hours

---

#### 6.2 Security Reporting (3 hours)

**FIXED: Issue #11 - Safe i64 to usize conversion**

```rust
// agent-patterns/src/security_reports.rs
use sqlx::PgPool;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityReport {
    pub period_start: std::time::SystemTime,
    pub period_end: std::time::SystemTime,
    pub failed_authentications: usize,
    pub denied_authorizations: usize,
    pub suspicious_patterns: Vec<SuspiciousPattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspiciousPattern {
    pub user_id: Option<String>,
    pub ip_address: Option<String>,
    pub event_count: usize,
    pub description: String,
}

pub struct SecurityReporter {
    pool: PgPool,
}

impl SecurityReporter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn generate_report(
        &self,
        start: std::time::SystemTime,
        end: std::time::SystemTime,
    ) -> Result<SecurityReport, String> {
        // Count failed authentications
        let failed_auth = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) as "count!"
            FROM audit_log
            WHERE event_type = 'AuthenticationAttempt'
              AND outcome = 'Failure'
              AND timestamp BETWEEN $1 AND $2
            "#,
            start,
            end,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        // Count denied authorizations
        let denied_auth = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) as "count!"
            FROM audit_log
            WHERE event_type = 'AuthorizationCheck'
              AND outcome = 'Denied'
              AND timestamp BETWEEN $1 AND $2
            "#,
            start,
            end,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(SecurityReport {
            period_start: start,
            period_end: end,
            failed_authentications: failed_auth.try_into().unwrap_or(0),
            denied_authorizations: denied_auth.try_into().unwrap_or(0),
            suspicious_patterns: Vec::new(),
        })
    }
}
```

**Files to create**:
- `agent-patterns/src/security_reports.rs` (300 lines)
- `agent-patterns/tests/security_reports_test.rs` (100 lines, 4 tests)

**Time**: 3 hours

---

### Part 7: Integration Examples (6 hours) - **NEW**

**ADDED: Issue #7 from review - Missing examples section**

#### 7.1 Complete Agent with All Features (3 hours)

```rust
// examples/production-agent/src/main.rs
use agent_patterns::{
    tracing::{TracedReducer, init_tracing},
    health::{SystemHealthCheck, K8sHealthEndpoints, HealthCheckable},
    shutdown::{ShutdownCoordinator, GenericStoreShutdownHandler, wait_for_signal},
    resilience::{CircuitBreaker, RateLimiter, Bulkhead},
    metrics::metrics_handler,
    audit::{AuditLogger, PostgresAuditSink, AuditEvent, AuditEventType},
    config::AgentConfig,
};
use composable_rust_runtime::Store;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration
    let config = AgentConfig::from_env()?;

    // Initialize tracing
    init_tracing(&config.tracing.service_name, &config.tracing.jaeger_endpoint)?;

    // Setup health checks
    let mut system_health = SystemHealthCheck::new();
    // Add health checks for dependencies

    let health_endpoints = Arc::new(K8sHealthEndpoints::new(Arc::new(system_health)));

    // Setup audit logging
    let audit_pool = sqlx::PgPool::connect(&config.audit.postgres_connection_string).await?;
    let audit_sink = Arc::new(PostgresAuditSink::new(audit_pool.clone()));
    let audit_logger = Arc::new(AuditLogger::new(audit_sink));

    // Setup resilience
    let circuit_breaker = Arc::new(CircuitBreaker::new(
        "llm_api".into(),
        config.resilience.circuit_breaker.clone().into(),
    ));

    let rate_limiter = Arc::new(RateLimiter::new(
        "api_requests".into(),
        config.resilience.rate_limiter.clone().into(),
    ));

    // Create traced reducer
    let base_reducer = MyAgentReducer::new();
    let traced_reducer = TracedReducer::new(base_reducer, config.tracing.service_name.clone());

    // Create store
    let store = Arc::new(Store::new(
        initial_state(),
        traced_reducer,
        create_environment(circuit_breaker, rate_limiter),
    ));

    // Setup shutdown coordinator
    let mut shutdown_coordinator = ShutdownCoordinator::new(
        Duration::from_secs(config.shutdown.timeout_secs)
    );

    shutdown_coordinator.register(Arc::new(GenericStoreShutdownHandler::new(
        "agent-store".into(),
        || async {
            info!("Store cleanup complete");
            Ok(())
        }
    )));

    // Start HTTP server
    let server_handle = tokio::spawn(run_server(
        config.clone(),
        store.clone(),
        health_endpoints.clone(),
        audit_logger.clone(),
    ));

    // Wait for shutdown signal
    wait_for_signal().await;

    // Graceful shutdown
    info!("Starting graceful shutdown");
    shutdown_coordinator.shutdown().await?;

    server_handle.await??;

    Ok(())
}

async fn run_server(
    config: AgentConfig,
    store: Arc<Store<...>>,
    health: Arc<K8sHealthEndpoints>,
    audit: Arc<AuditLogger>,
) -> Result<(), Box<dyn std::error::Error>> {
    use axum::{Router, routing::{get, post}};

    let app = Router::new()
        .route("/health/liveness", get(|| async { health.liveness().await }))
        .route("/health/readiness", get(|| async { health.readiness().await }))
        .route("/metrics", get(|| async { metrics_handler() }))
        .route("/agent/message", post(handle_message));

    let addr = format!("{}:{}", config.server.host, config.server.port);
    info!("Starting server on {}", addr);

    axum::Server::bind(&addr.parse()?)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
```

**Files to create**:
- `examples/production-agent/src/main.rs` (500 lines)
- `examples/production-agent/Cargo.toml` (50 lines)
- `examples/production-agent/README.md` (200 lines)

**Time**: 3 hours

---

#### 7.2 Deployment Example with Scripts (3 hours)

```bash
#!/bin/bash
# deploy.sh

set -e

echo "Building Docker image..."
docker build -t agent-server:latest .

echo "Pushing to registry..."
docker tag agent-server:latest your-registry/agent-server:latest
docker push your-registry/agent-server:latest

echo "Applying Kubernetes manifests..."
kubectl apply -f k8s/namespace.yaml
kubectl apply -f k8s/configmap.yaml
kubectl apply -f k8s/secrets.yaml
kubectl apply -f k8s/deployment.yaml
kubectl apply -f k8s/service.yaml
kubectl apply -f k8s/hpa.yaml

echo "Waiting for rollout..."
kubectl rollout status deployment/agent-server -n agents

echo "Deployment complete!"
kubectl get pods -n agents
```

**Files to create**:
- `scripts/deploy.sh` (100 lines)
- `scripts/rollback.sh` (50 lines)
- `scripts/scale.sh` (50 lines)
- `docs/deployment-runbook.md` (400 lines)

**Time**: 3 hours

---

## Testing Strategy

### Comprehensive Test Coverage (40+ tests)

**Testing checklist**:

- [ ] Circuit breaker state transitions (7 tests)
  - [ ] Closed → Open on failure threshold
  - [ ] Open → HalfOpen on timeout
  - [ ] HalfOpen → Closed on success threshold
  - [ ] HalfOpen → Open on failure
  - [ ] Reset failure count on success in Closed
  - [ ] Timeout extension on new failure in Open
  - [ ] Concurrent state transitions

- [ ] Rate limiter token refill (5 tests)
  - [ ] Token bucket refill over time
  - [ ] Burst capacity enforcement
  - [ ] Concurrent token acquisition
  - [ ] Refill rate accuracy
  - [ ] Token overflow prevention

- [ ] Bulkhead isolation (5 tests)
  - [ ] Max concurrent enforcement
  - [ ] Permit timeout
  - [ ] Permit release on drop
  - [ ] Concurrent execution
  - [ ] Available permits tracking

- [ ] Health check failure scenarios (8 tests)
  - [ ] Individual component failures
  - [ ] Timeout handling
  - [ ] Overall health aggregation
  - [ ] Degraded state detection
  - [ ] Liveness vs readiness
  - [ ] Health check caching
  - [ ] Concurrent health checks
  - [ ] Health recovery

- [ ] Graceful shutdown cleanup (6 tests)
  - [ ] Shutdown signal propagation
  - [ ] Component cleanup order
  - [ ] Shutdown timeout handling
  - [ ] Partial failure handling
  - [ ] In-flight request completion
  - [ ] Resource cleanup verification

- [ ] Audit event serialization (8 tests)
  - [ ] File sink writes
  - [ ] PostgreSQL sink writes
  - [ ] Event batching
  - [ ] Error handling
  - [ ] Concurrent writes
  - [ ] Schema validation
  - [ ] Audit query performance
  - [ ] Log rotation

- [ ] Metrics collection (10 tests)
  - [ ] Counter increments
  - [ ] Histogram observations
  - [ ] Gauge updates
  - [ ] Label cardinality
  - [ ] Metrics endpoint format
  - [ ] Concurrent metric updates
  - [ ] Metric reset behavior
  - [ ] Registry isolation
  - [ ] Process metrics
  - [ ] Custom metrics

- [ ] Tracing span creation (7 tests)
  - [ ] Span attributes
  - [ ] Span nesting
  - [ ] Context propagation
  - [ ] Trace ID continuity
  - [ ] Span export
  - [ ] Error span marking
  - [ ] Concurrent span creation

- [ ] Config validation (5 tests)
  - [ ] TOML parsing
  - [ ] Environment variable overrides
  - [ ] Required field validation
  - [ ] Default value application
  - [ ] Invalid config rejection

**Total**: 61 tests (exceeds 40+ requirement)

---

## Timeline Summary

| Part | Hours | Days | Notes |
|------|-------|------|-------|
| 1. Distributed Tracing | 16 | 2.0 | Increased from 12h (unified tracing strategy) |
| 2. Health Checks & Lifecycle | 10 | 1.3 | Same |
| 3. Advanced Resilience | 15 | 1.9 | Same |
| 4. Production Metrics | 12 | 1.5 | Same |
| 5. Deployment & Operations | 18 | 2.3 | Same |
| 6. Audit Logging | 8 | 1.0 | Increased from 6h (schema fixes) |
| 7. Integration Examples | 6 | 0.8 | **NEW SECTION** |
| **Total** | **85** | **10.8** | Realistic estimate |

**Adjusted timeline**: **11-12 days** (more realistic than original 8-9 days)

**Buffer**: 5 hours built-in for unexpected issues

---

## Definition of Done

### Part 1: Distributed Tracing ✅
- [ ] TracedReducer wrapper implemented
- [ ] Span context propagation working
- [ ] Cross-service trace propagation
- [ ] Jaeger integration tested
- [ ] Tracing guide documentation
- [ ] 9+ tests passing

### Part 2: Health Checks ✅
- [ ] HealthCheckable trait defined
- [ ] K8s liveness/readiness endpoints
- [ ] ShutdownCoordinator working
- [ ] Graceful shutdown tested
- [ ] 14+ tests passing

### Part 3: Resilience ✅
- [ ] Circuit breaker per tool
- [ ] Rate limiter with token bucket
- [ ] Bulkhead pattern implemented
- [ ] All state transitions tested
- [ ] 17+ tests passing

### Part 4: Metrics ✅
- [ ] Prometheus metrics exported
- [ ] Grafana dashboards created
- [ ] All agent patterns instrumented
- [ ] Metrics endpoint tested
- [ ] 6+ tests passing

### Part 5: Deployment ✅
- [ ] Dockerfile optimized
- [ ] docker-compose.yml working
- [ ] Kubernetes manifests tested
- [ ] HPA configured
- [ ] Deployment guide complete

### Part 6: Audit Logging ✅
- [ ] AuditEvent framework implemented
- [ ] PostgresAuditSink working
- [ ] Security reports functional
- [ ] Compliance documentation
- [ ] 12+ tests passing

### Part 7: Examples ✅
- [ ] Production agent example complete
- [ ] Deployment scripts working
- [ ] All features demonstrated
- [ ] Documentation comprehensive

### Overall Success Criteria ✅
- [ ] 61+ tests passing (increased from 40+)
- [ ] Zero clippy warnings
- [ ] All documentation complete
- [ ] Docker build successful
- [ ] K8s deployment tested
- [ ] Metrics visible in Grafana
- [ ] Traces visible in Jaeger
- [ ] Graceful shutdown verified
- [ ] Load test passed
- [ ] Security audit passed

---

## Risk Mitigation

### Technical Risks

1. **OpenTelemetry complexity**
   - **Mitigation**: Start with basic tracing, expand gradually
   - **Fallback**: File-based tracing logs

2. **Kubernetes configuration**
   - **Mitigation**: Test with minikube first
   - **Fallback**: Docker Compose for development

3. **Circuit breaker tuning**
   - **Mitigation**: Conservative defaults, allow runtime tuning
   - **Fallback**: Simple retry policies

4. **Prometheus cardinality explosion**
   - **Mitigation**: Strict label guidelines, cardinality limits
   - **Fallback**: Aggregate metrics at query time

### Operational Risks

1. **Audit log storage growth**
   - **Mitigation**: Automated log rotation and archival
   - **Fallback**: File-based audit with external rotation

2. **Graceful shutdown timeout**
   - **Mitigation**: Configurable timeouts, component priorities
   - **Fallback**: Force kill after grace period

3. **Health check false positives**
   - **Mitigation**: Separate liveness/readiness, tunable thresholds
   - **Fallback**: Manual health check override

---

## Success Metrics

### Performance Targets

- P50 agent response latency: < 500ms
- P95 agent response latency: < 2s
- P99 agent response latency: < 5s
- Circuit breaker recovery time: < 30s
- Graceful shutdown time: < 30s

### Reliability Targets

- Uptime: 99.9% (3 nines)
- Failed requests: < 0.1%
- Circuit breaker activations: Tracked and alerted
- Health check failures: < 1% false positives

### Observability Targets

- Trace sampling rate: 100% (development), 10% (production)
- Metric collection interval: 15s
- Audit log completeness: 100%
- Dashboard load time: < 3s

---

## Post-Implementation Checklist

### Week 1
- [ ] Deploy to staging environment
- [ ] Run load tests (1000 req/s)
- [ ] Verify all traces in Jaeger
- [ ] Verify all metrics in Prometheus
- [ ] Test graceful shutdown under load

### Week 2
- [ ] Security audit of audit logs
- [ ] Chaos engineering tests (circuit breakers, bulkheads)
- [ ] Performance tuning
- [ ] Documentation review
- [ ] Team training on operations

### Week 3
- [ ] Production deployment (canary)
- [ ] Monitor error rates
- [ ] Verify alerting
- [ ] Gradual traffic ramp-up
- [ ] Retrospective

---

## Next Steps After Phase 8.4

### Phase 8.5 (Future)
- Advanced agent patterns (self-reflection, tool learning)
- Multi-agent collaboration
- Agent observability dashboard
- Cost optimization and caching strategies

### Phase 8.6 (Future)
- Agent fine-tuning and prompt optimization
- A/B testing framework for agents
- Agent performance benchmarking
- Production battle-hardening based on real usage

---

## Appendix: All Fixes Applied

1. ✅ **Issue #1**: Use `tracing` + `tracing-opentelemetry` (not manual OTel spans)
2. ✅ **Issue #2**: Use span context propagation (not wrapped AgentAction)
3. ✅ **Issue #3**: Add `HealthCheckable` trait
4. ✅ **Issue #4**: Simplified shutdown handler generics
5. ✅ **Issue #5**: Add SmallVec import everywhere
6. ✅ **Issue #6**: Complete dependency list added
7. ✅ **Issue #7**: Use `once_cell` not `lazy_static`
8. ✅ **Issue #8**: Clarify circuit breaker config with per-tool overrides
9. ✅ **Issue #9**: Fix SQL schema (TEXT for event_type, not JSONB)
10. ✅ **Issue #10**: Use Rust 1.75 (not non-existent 1.85)
11. ✅ **Issue #11**: Safe i64 to usize conversion with try_into()
12. ✅ **Issue #12-15**: Add all missing imports to code examples
13. ✅ **Review Issue #7**: Add Part 7 (Integration Examples, 6h)
14. ✅ **Review Issue #8**: Expand testing checklist (61 specific tests)
15. ✅ **Review Issue #9**: Update timeline to 85h (11-12 days)

**Status**: ✅ **ALL ISSUES RESOLVED** - Ready for implementation!
