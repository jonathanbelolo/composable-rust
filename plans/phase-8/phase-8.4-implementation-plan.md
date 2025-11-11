# Phase 8.4: Agent Production Hardening - Implementation Plan

## Overview

**Goal**: Make the AI agent system production-ready with enterprise-grade observability, resilience, and operational excellence.

**Status**: Ready to begin

**Duration**: 8-9 days (~67 hours)

**Dependencies**: Phase 8.1 ✅, Phase 8.2 ✅, Phase 8.3 ✅

**Last Updated**: 2025-11-10

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
8. **Performance Monitoring** - Latency tracking, bottleneck detection

---

## Architecture Principles

### 1. Observable by Default

All agent operations emit:
- Distributed traces (OpenTelemetry)
- Structured logs (tracing)
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

## Implementation Plan

### Part 1: Distributed Tracing (12 hours)

#### 1.1 OpenTelemetry Integration (4 hours)

**Add tracing to agent reducer pattern:**

```rust
// agent-patterns/src/tracing.rs
use opentelemetry::{
    trace::{Span, Tracer, SpanKind, Status},
    Context as OtelContext,
    global,
};
use tracing::{instrument, info, warn, error};
use std::time::Instant;

/// Tracing wrapper for agent reducers
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
        )
    )]
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        let start = Instant::now();

        // Create span for this reduction
        let tracer = global::tracer(&self.service_name);
        let mut span = tracer
            .span_builder(format!("reduce.{:?}", action))
            .with_kind(SpanKind::Internal)
            .start(&tracer);

        // Execute reducer
        let effects = self.inner.reduce(state, action.clone(), env);

        let duration = start.elapsed();

        // Record metrics
        span.set_attribute(opentelemetry::KeyValue::new(
            "agent.effects.count",
            effects.len() as i64,
        ));
        span.set_attribute(opentelemetry::KeyValue::new(
            "agent.duration.ms",
            duration.as_millis() as i64,
        ));

        info!(
            action = ?action,
            effects_count = effects.len(),
            duration_ms = duration.as_millis(),
            "reducer executed"
        );

        effects
    }
}

/// Traced effect executor
pub struct TracedEffectExecutor;

impl TracedEffectExecutor {
    #[instrument(skip(effect), fields(effect.type = "future"))]
    pub async fn execute_with_trace<A>(
        effect: Effect<A>,
        parent_context: OtelContext,
    ) -> Option<A>
    where
        A: Clone + std::fmt::Debug + Send + 'static,
    {
        match effect {
            Effect::Future(fut) => {
                let _guard = parent_context.clone().attach();
                let result = fut.await;

                if result.is_some() {
                    info!("effect produced action");
                } else {
                    info!("effect completed without action");
                }

                result
            }
            Effect::None => {
                info!("no-op effect");
                None
            }
            _ => {
                warn!("unsupported effect type for tracing");
                None
            }
        }
    }
}
```

**Tasks**:
- [ ] Add `opentelemetry` and `opentelemetry-jaeger` dependencies
- [ ] Create `TracedReducer` wrapper
- [ ] Create `TracedEffectExecutor`
- [ ] Add span context propagation
- [ ] Test with Jaeger

**Files**:
- `agent-patterns/Cargo.toml` - Add dependencies
- `agent-patterns/src/tracing.rs` - New module
- `agent-patterns/src/lib.rs` - Export tracing utilities

---

#### 1.2 Trace Context Propagation (4 hours)

**Propagate trace context through effect chain:**

```rust
// core/src/trace_context.rs
use std::collections::HashMap;

/// Trace context for distributed tracing
#[derive(Debug, Clone)]
pub struct TraceContext {
    /// Trace ID (128-bit hex string)
    pub trace_id: String,
    /// Span ID (64-bit hex string)
    pub span_id: String,
    /// Parent span ID
    pub parent_span_id: Option<String>,
    /// Baggage items
    pub baggage: HashMap<String, String>,
}

impl TraceContext {
    /// Create root trace context
    pub fn root() -> Self {
        Self {
            trace_id: generate_trace_id(),
            span_id: generate_span_id(),
            parent_span_id: None,
            baggage: HashMap::new(),
        }
    }

    /// Create child span
    pub fn child(&self) -> Self {
        Self {
            trace_id: self.trace_id.clone(),
            span_id: generate_span_id(),
            parent_span_id: Some(self.span_id.clone()),
            baggage: self.baggage.clone(),
        }
    }

    /// Add baggage item
    pub fn with_baggage(mut self, key: String, value: String) -> Self {
        self.baggage.insert(key, value);
        self
    }
}

fn generate_trace_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    format!("{:032x}", rng.gen::<u128>())
}

fn generate_span_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    format!("{:016x}", rng.gen::<u64>())
}

/// Extend AgentAction with trace context
impl AgentAction {
    /// Attach trace context to action
    pub fn with_trace_context(self, context: TraceContext) -> TracedAction {
        TracedAction {
            action: self,
            trace_context: context,
        }
    }
}

/// Action with trace context
#[derive(Debug, Clone)]
pub struct TracedAction {
    pub action: AgentAction,
    pub trace_context: TraceContext,
}
```

**Tasks**:
- [ ] Add `TraceContext` type
- [ ] Propagate context through effect chains
- [ ] Inject context into tool calls
- [ ] Extract context from incoming requests
- [ ] Tests

**Files**:
- `core/src/trace_context.rs` - New module
- `core/src/agent.rs` - Extend AgentAction
- `tools/src/registry.rs` - Inject context into tool execution

---

#### 1.3 Agent-Specific Spans (4 hours)

**Add meaningful spans for agent operations:**

```rust
// agent-patterns/src/spans.rs
use opentelemetry::trace::{Span, Tracer, SpanKind};
use std::time::Instant;

/// Span builder for agent patterns
pub struct AgentSpanBuilder {
    tracer: Box<dyn Tracer + Send + Sync>,
}

impl AgentSpanBuilder {
    pub fn new(service_name: &str) -> Self {
        Self {
            tracer: Box::new(global::tracer(service_name)),
        }
    }

    /// Create span for pattern execution
    pub fn pattern_span(&self, pattern_name: &str, action: &str) -> impl Span {
        self.tracer
            .span_builder(format!("pattern.{}.{}", pattern_name, action))
            .with_kind(SpanKind::Internal)
            .with_attribute(opentelemetry::KeyValue::new("pattern.name", pattern_name.to_string()))
            .with_attribute(opentelemetry::KeyValue::new("pattern.action", action.to_string()))
            .start(&self.tracer)
    }

    /// Create span for tool execution
    pub fn tool_span(&self, tool_name: &str) -> impl Span {
        self.tracer
            .span_builder(format!("tool.{}", tool_name))
            .with_kind(SpanKind::Client)
            .with_attribute(opentelemetry::KeyValue::new("tool.name", tool_name.to_string()))
            .start(&self.tracer)
    }

    /// Create span for LLM call
    pub fn llm_span(&self, model: &str, tokens: usize) -> impl Span {
        self.tracer
            .span_builder("llm.generate")
            .with_kind(SpanKind::Client)
            .with_attribute(opentelemetry::KeyValue::new("llm.model", model.to_string()))
            .with_attribute(opentelemetry::KeyValue::new("llm.tokens", tokens as i64))
            .start(&self.tracer)
    }
}

/// Instrument specific patterns
pub mod instrumented {
    use super::*;
    use crate::prompt_chain::*;

    /// Instrumented prompt chain reducer
    pub struct InstrumentedChainReducer {
        inner: PromptChainReducer,
        span_builder: AgentSpanBuilder,
    }

    impl InstrumentedChainReducer {
        pub fn new(inner: PromptChainReducer) -> Self {
            Self {
                inner,
                span_builder: AgentSpanBuilder::new("prompt_chain"),
            }
        }
    }

    impl<E: AgentEnvironment> Reducer for InstrumentedChainReducer {
        type State = ChainState;
        type Action = ChainAction;
        type Environment = E;

        fn reduce(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            env: &Self::Environment,
        ) -> SmallVec<[Effect<Self::Action>; 4]> {
            let _span = match &action {
                ChainAction::Start { .. } => self.span_builder.pattern_span("prompt_chain", "start"),
                ChainAction::StepComplete { step, .. } => {
                    let mut span = self.span_builder.pattern_span("prompt_chain", "step_complete");
                    span.set_attribute(opentelemetry::KeyValue::new("step.index", *step as i64));
                    span
                }
                ChainAction::Complete { .. } => self.span_builder.pattern_span("prompt_chain", "complete"),
            };

            self.inner.reduce(state, action, env)
        }
    }
}
```

**Tasks**:
- [ ] Create `AgentSpanBuilder`
- [ ] Instrument all 7 patterns
- [ ] Add pattern-specific attributes
- [ ] Tool execution spans
- [ ] LLM call spans
- [ ] Tests

**Files**:
- `agent-patterns/src/spans.rs` - New module
- `agent-patterns/src/prompt_chain.rs` - Add instrumentation
- `agent-patterns/src/routing.rs` - Add instrumentation
- (Repeat for all patterns)

---

### Part 2: Health Checks & Lifecycle (10 hours)

#### 2.1 Health Check System (4 hours)

**Kubernetes-ready health checks:**

```rust
// agent-patterns/src/health.rs
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{Duration, Instant};
use serde::{Serialize, Deserialize};

/// Health status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    /// Service is healthy
    Healthy,
    /// Service is degraded but functional
    Degraded,
    /// Service is unhealthy
    Unhealthy,
}

/// Component health check
#[derive(Debug, Clone, Serialize)]
pub struct ComponentHealth {
    pub name: String,
    pub status: HealthStatus,
    pub message: Option<String>,
    pub last_check: Instant,
    pub latency_ms: u64,
}

/// Health check trait
#[async_trait::async_trait]
pub trait HealthCheck: Send + Sync {
    /// Check health of component
    async fn check(&self) -> ComponentHealth;
}

/// Agent health manager
pub struct AgentHealthManager {
    checks: Arc<RwLock<Vec<Box<dyn HealthCheck>>>>,
    overall_status: Arc<RwLock<HealthStatus>>,
}

impl AgentHealthManager {
    pub fn new() -> Self {
        Self {
            checks: Arc::new(RwLock::new(Vec::new())),
            overall_status: Arc::new(RwLock::new(HealthStatus::Healthy)),
        }
    }

    /// Register health check
    pub async fn register(&self, check: Box<dyn HealthCheck>) {
        self.checks.write().await.push(check);
    }

    /// Run all health checks
    pub async fn check_health(&self) -> OverallHealth {
        let checks = self.checks.read().await;
        let mut components = Vec::new();

        for check in checks.iter() {
            components.push(check.check().await);
        }

        // Determine overall status
        let overall = if components.iter().any(|c| c.status == HealthStatus::Unhealthy) {
            HealthStatus::Unhealthy
        } else if components.iter().any(|c| c.status == HealthStatus::Degraded) {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        };

        *self.overall_status.write().await = overall;

        OverallHealth {
            status: overall,
            components,
            timestamp: Instant::now(),
        }
    }

    /// Get liveness probe (always healthy unless critical failure)
    pub async fn liveness(&self) -> bool {
        *self.overall_status.read().await != HealthStatus::Unhealthy
    }

    /// Get readiness probe (healthy or degraded can serve traffic)
    pub async fn readiness(&self) -> bool {
        let status = *self.overall_status.read().await;
        status == HealthStatus::Healthy || status == HealthStatus::Degraded
    }
}

/// Overall health summary
#[derive(Debug, Clone, Serialize)]
pub struct OverallHealth {
    pub status: HealthStatus,
    pub components: Vec<ComponentHealth>,
    pub timestamp: Instant,
}

/// Tool registry health check
pub struct ToolRegistryHealthCheck {
    registry: Arc<ToolRegistry>,
}

impl ToolRegistryHealthCheck {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait::async_trait]
impl HealthCheck for ToolRegistryHealthCheck {
    async fn check(&self) -> ComponentHealth {
        let start = Instant::now();
        let tool_count = self.registry.list_tools().len();

        let status = if tool_count > 0 {
            HealthStatus::Healthy
        } else {
            HealthStatus::Degraded
        };

        ComponentHealth {
            name: "tool_registry".to_string(),
            status,
            message: Some(format!("{} tools registered", tool_count)),
            last_check: Instant::now(),
            latency_ms: start.elapsed().as_millis() as u64,
        }
    }
}

/// LLM availability health check
pub struct LLMHealthCheck {
    client: Arc<dyn LLMClient>,
    timeout: Duration,
}

#[async_trait::async_trait]
impl HealthCheck for LLMHealthCheck {
    async fn check(&self) -> ComponentHealth {
        let start = Instant::now();

        // Ping LLM with minimal request
        let result = tokio::time::timeout(
            self.timeout,
            self.client.ping(),
        ).await;

        let (status, message) = match result {
            Ok(Ok(())) => (HealthStatus::Healthy, "LLM available".to_string()),
            Ok(Err(e)) => (HealthStatus::Unhealthy, format!("LLM error: {}", e)),
            Err(_) => (HealthStatus::Unhealthy, "LLM timeout".to_string()),
        };

        ComponentHealth {
            name: "llm".to_string(),
            status,
            message: Some(message),
            last_check: Instant::now(),
            latency_ms: start.elapsed().as_millis() as u64,
        }
    }
}
```

**Tasks**:
- [ ] Create `HealthCheck` trait
- [ ] Implement `AgentHealthManager`
- [ ] Add liveness/readiness probes
- [ ] Tool registry health check
- [ ] LLM availability check
- [ ] Memory usage check
- [ ] Tests

**Files**:
- `agent-patterns/src/health.rs` - New module
- `web/src/health.rs` - HTTP endpoints

---

#### 2.2 Graceful Shutdown (3 hours)

**Clean resource cleanup:**

```rust
// runtime/src/graceful_shutdown.rs
use tokio::sync::{mpsc, broadcast};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn, error};

/// Shutdown coordinator
pub struct ShutdownCoordinator {
    /// Broadcast channel for shutdown signal
    shutdown_tx: broadcast::Sender<()>,
    /// Components to shutdown
    components: Arc<RwLock<Vec<Box<dyn ShutdownHandler>>>>,
    /// Graceful shutdown timeout
    timeout: Duration,
}

/// Shutdown handler trait
#[async_trait::async_trait]
pub trait ShutdownHandler: Send + Sync {
    /// Component name
    fn name(&self) -> &str;

    /// Shutdown component gracefully
    async fn shutdown(&self) -> Result<(), String>;
}

impl ShutdownCoordinator {
    pub fn new(timeout: Duration) -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            shutdown_tx,
            components: Arc::new(RwLock::new(Vec::new())),
            timeout,
        }
    }

    /// Register shutdown handler
    pub async fn register(&self, handler: Box<dyn ShutdownHandler>) {
        info!(component = handler.name(), "registered shutdown handler");
        self.components.write().await.push(handler);
    }

    /// Get shutdown signal receiver
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    /// Initiate graceful shutdown
    pub async fn shutdown(&self) -> Result<(), Vec<String>> {
        info!("initiating graceful shutdown");

        // Send shutdown signal to all subscribers
        if let Err(e) = self.shutdown_tx.send(()) {
            warn!("failed to send shutdown signal: {}", e);
        }

        // Shutdown components
        let components = self.components.read().await;
        let mut errors = Vec::new();

        for component in components.iter() {
            info!(component = component.name(), "shutting down");

            match tokio::time::timeout(self.timeout, component.shutdown()).await {
                Ok(Ok(())) => {
                    info!(component = component.name(), "shutdown complete");
                }
                Ok(Err(e)) => {
                    error!(component = component.name(), error = %e, "shutdown failed");
                    errors.push(format!("{}: {}", component.name(), e));
                }
                Err(_) => {
                    error!(component = component.name(), "shutdown timeout");
                    errors.push(format!("{}: timeout", component.name()));
                }
            }
        }

        if errors.is_empty() {
            info!("graceful shutdown complete");
            Ok(())
        } else {
            error!(errors = ?errors, "shutdown completed with errors");
            Err(errors)
        }
    }
}

/// Store shutdown handler
pub struct StoreShutdownHandler<S, A, E, R>
where
    S: Clone + Send + Sync,
    A: Clone + Send + Sync,
    E: Send + Sync,
    R: Reducer<State = S, Action = A, Environment = E>,
{
    store: Arc<Store<S, A, E, R>>,
}

#[async_trait::async_trait]
impl<S, A, E, R> ShutdownHandler for StoreShutdownHandler<S, A, E, R>
where
    S: Clone + Send + Sync + 'static,
    A: Clone + Send + Sync + 'static,
    E: Send + Sync + 'static,
    R: Reducer<State = S, Action = A, Environment = E> + Send + Sync + 'static,
{
    fn name(&self) -> &str {
        "store"
    }

    async fn shutdown(&self) -> Result<(), String> {
        // Flush pending effects
        info!("flushing pending effects");

        // Wait for in-flight effects to complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        Ok(())
    }
}

/// Tool registry shutdown handler
pub struct ToolRegistryShutdownHandler {
    registry: Arc<ToolRegistry>,
}

#[async_trait::async_trait]
impl ShutdownHandler for ToolRegistryShutdownHandler {
    fn name(&self) -> &str {
        "tool_registry"
    }

    async fn shutdown(&self) -> Result<(), String> {
        // Cancel in-flight tool executions
        info!("cancelling in-flight tool executions");

        // Cleanup resources
        Ok(())
    }
}
```

**Tasks**:
- [ ] Create `ShutdownCoordinator`
- [ ] Implement `ShutdownHandler` trait
- [ ] Store shutdown handler
- [ ] Tool registry shutdown
- [ ] Signal handling (SIGTERM, SIGINT)
- [ ] Tests

**Files**:
- `runtime/src/graceful_shutdown.rs` - New module
- `web/src/server.rs` - Integrate shutdown

---

#### 2.3 Startup Checks (3 hours)

**Validate dependencies before serving:**

```rust
// agent-patterns/src/startup.rs
use std::time::Duration;
use tracing::{info, error};

/// Startup check trait
#[async_trait::async_trait]
pub trait StartupCheck: Send + Sync {
    /// Check name
    fn name(&self) -> &str;

    /// Run startup check
    async fn check(&self) -> Result<(), String>;
}

/// Startup coordinator
pub struct StartupCoordinator {
    checks: Vec<Box<dyn StartupCheck>>,
    timeout: Duration,
}

impl StartupCoordinator {
    pub fn new(timeout: Duration) -> Self {
        Self {
            checks: Vec::new(),
            timeout,
        }
    }

    /// Register startup check
    pub fn register(&mut self, check: Box<dyn StartupCheck>) {
        self.checks.push(check);
    }

    /// Run all startup checks
    pub async fn run_checks(&self) -> Result<(), Vec<String>> {
        info!(checks = self.checks.len(), "running startup checks");

        let mut errors = Vec::new();

        for check in &self.checks {
            info!(check = check.name(), "running startup check");

            match tokio::time::timeout(self.timeout, check.check()).await {
                Ok(Ok(())) => {
                    info!(check = check.name(), "startup check passed");
                }
                Ok(Err(e)) => {
                    error!(check = check.name(), error = %e, "startup check failed");
                    errors.push(format!("{}: {}", check.name(), e));
                }
                Err(_) => {
                    error!(check = check.name(), "startup check timeout");
                    errors.push(format!("{}: timeout", check.name()));
                }
            }
        }

        if errors.is_empty() {
            info!("all startup checks passed");
            Ok(())
        } else {
            error!(errors = ?errors, "startup checks failed");
            Err(errors)
        }
    }
}

/// LLM connectivity check
pub struct LLMConnectivityCheck {
    client: Arc<dyn LLMClient>,
}

#[async_trait::async_trait]
impl StartupCheck for LLMConnectivityCheck {
    fn name(&self) -> &str {
        "llm_connectivity"
    }

    async fn check(&self) -> Result<(), String> {
        self.client.ping().await
            .map_err(|e| format!("LLM ping failed: {}", e))
    }
}

/// Required tools check
pub struct RequiredToolsCheck {
    registry: Arc<ToolRegistry>,
    required_tools: Vec<String>,
}

#[async_trait::async_trait]
impl StartupCheck for RequiredToolsCheck {
    fn name(&self) -> &str {
        "required_tools"
    }

    async fn check(&self) -> Result<(), String> {
        let available = self.registry.list_tools();

        for required in &self.required_tools {
            if !available.contains(required) {
                return Err(format!("required tool '{}' not registered", required));
            }
        }

        Ok(())
    }
}
```

**Tasks**:
- [ ] Create `StartupCheck` trait
- [ ] Implement `StartupCoordinator`
- [ ] LLM connectivity check
- [ ] Required tools check
- [ ] Configuration validation check
- [ ] Tests

**Files**:
- `agent-patterns/src/startup.rs` - New module

---

### Part 3: Advanced Resilience (15 hours)

#### 3.1 Circuit Breakers for Agents (5 hours)

**Agent-specific circuit breakers:**

```rust
// agent-patterns/src/circuit_breaker.rs
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{Duration, Instant};

/// Circuit breaker state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Circuit is closed, requests flow normally
    Closed,
    /// Circuit is open, requests fail fast
    Open,
    /// Circuit is half-open, testing if service recovered
    HalfOpen,
}

/// Circuit breaker configuration
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Failure threshold to open circuit
    pub failure_threshold: usize,
    /// Success threshold to close circuit from half-open
    pub success_threshold: usize,
    /// Time to wait before trying half-open
    pub timeout: Duration,
    /// Window for counting failures
    pub window: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 2,
            timeout: Duration::from_secs(30),
            window: Duration::from_secs(60),
        }
    }
}

/// Circuit breaker for agent operations
pub struct AgentCircuitBreaker {
    config: CircuitBreakerConfig,
    state: Arc<RwLock<CircuitBreakerState>>,
}

#[derive(Debug)]
struct CircuitBreakerState {
    current_state: CircuitState,
    failures: usize,
    successes: usize,
    last_failure: Option<Instant>,
    last_state_change: Instant,
}

impl AgentCircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(CircuitBreakerState {
                current_state: CircuitState::Closed,
                failures: 0,
                successes: 0,
                last_failure: None,
                last_state_change: Instant::now(),
            })),
        }
    }

    /// Execute operation with circuit breaker
    pub async fn call<F, T, E>(&self, f: F) -> Result<T, CircuitBreakerError<E>>
    where
        F: FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send>>,
        E: std::fmt::Display,
    {
        // Check if circuit is open
        {
            let state = self.state.read().await;
            if state.current_state == CircuitState::Open {
                // Check if timeout elapsed
                if state.last_state_change.elapsed() < self.config.timeout {
                    return Err(CircuitBreakerError::Open);
                }
            }
        }

        // Try to transition to half-open if timeout elapsed
        {
            let mut state = self.state.write().await;
            if state.current_state == CircuitState::Open
                && state.last_state_change.elapsed() >= self.config.timeout
            {
                info!("circuit breaker transitioning to half-open");
                state.current_state = CircuitState::HalfOpen;
                state.successes = 0;
                state.failures = 0;
                state.last_state_change = Instant::now();
            }
        }

        // Execute operation
        let result = f().await;

        // Update state based on result
        let mut state = self.state.write().await;
        match result {
            Ok(value) => {
                state.successes += 1;

                // In half-open, check if we can close
                if state.current_state == CircuitState::HalfOpen
                    && state.successes >= self.config.success_threshold
                {
                    info!("circuit breaker closing after successful recovery");
                    state.current_state = CircuitState::Closed;
                    state.failures = 0;
                    state.successes = 0;
                    state.last_state_change = Instant::now();
                }

                Ok(value)
            }
            Err(e) => {
                state.failures += 1;
                state.last_failure = Some(Instant::now());

                // Check if we should open circuit
                if state.current_state == CircuitState::Closed
                    && state.failures >= self.config.failure_threshold
                {
                    warn!(
                        failures = state.failures,
                        "circuit breaker opening due to failures"
                    );
                    state.current_state = CircuitState::Open;
                    state.last_state_change = Instant::now();
                }

                // In half-open, any failure reopens circuit
                if state.current_state == CircuitState::HalfOpen {
                    warn!("circuit breaker reopening after failure in half-open");
                    state.current_state = CircuitState::Open;
                    state.last_state_change = Instant::now();
                }

                Err(CircuitBreakerError::Failure(e.to_string()))
            }
        }
    }

    /// Get current state
    pub async fn state(&self) -> CircuitState {
        self.state.read().await.current_state
    }

    /// Reset circuit breaker
    pub async fn reset(&self) {
        let mut state = self.state.write().await;
        state.current_state = CircuitState::Closed;
        state.failures = 0;
        state.successes = 0;
        state.last_state_change = Instant::now();
        info!("circuit breaker manually reset");
    }
}

/// Circuit breaker error
#[derive(Debug, thiserror::Error)]
pub enum CircuitBreakerError<E>
where
    E: std::fmt::Display,
{
    /// Circuit is open
    #[error("circuit breaker is open")]
    Open,

    /// Operation failed
    #[error("operation failed: {0}")]
    Failure(String),
}
```

**Wrap tool execution with circuit breakers:**

```rust
// tools/src/resilient_registry.rs
use composable_rust_agent_patterns::circuit_breaker::*;
use std::collections::HashMap;

/// Tool registry with circuit breakers per tool
pub struct ResilientToolRegistry {
    inner: ToolRegistry,
    circuit_breakers: Arc<RwLock<HashMap<String, AgentCircuitBreaker>>>,
    config: CircuitBreakerConfig,
}

impl ResilientToolRegistry {
    pub fn new(inner: ToolRegistry, config: CircuitBreakerConfig) -> Self {
        Self {
            inner,
            circuit_breakers: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    /// Execute tool with circuit breaker protection
    pub async fn execute_protected(
        &self,
        name: &str,
        input: String,
    ) -> Result<String, ToolError> {
        // Get or create circuit breaker for this tool
        let breaker = {
            let mut breakers = self.circuit_breakers.write().await;
            breakers
                .entry(name.to_string())
                .or_insert_with(|| AgentCircuitBreaker::new(self.config.clone()))
                .clone()
        };

        // Execute with circuit breaker
        let tool_name = name.to_string();
        let registry = self.inner.clone();
        let input_clone = input.clone();

        breaker
            .call(|| {
                Box::pin(async move {
                    registry.execute(&tool_name, input_clone).await
                })
            })
            .await
            .map_err(|e| match e {
                CircuitBreakerError::Open => ToolError {
                    message: format!("Tool '{}' circuit breaker is open", tool_name),
                },
                CircuitBreakerError::Failure(msg) => ToolError { message: msg },
            })
    }
}
```

**Tasks**:
- [ ] Create `AgentCircuitBreaker`
- [ ] Implement state machine (Closed → Open → HalfOpen → Closed)
- [ ] Wrap tool execution
- [ ] Per-tool circuit breakers
- [ ] Metrics integration
- [ ] Tests (all state transitions)

**Files**:
- `agent-patterns/src/circuit_breaker.rs` - New module
- `tools/src/resilient_registry.rs` - New module

---

#### 3.2 Rate Limiting (5 hours)

**Multi-level rate limiting:**

```rust
// agent-patterns/src/rate_limit.rs
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{Duration, Instant};
use std::collections::HashMap;

/// Rate limiter using token bucket algorithm
pub struct TokenBucketRateLimiter {
    capacity: usize,
    refill_rate: usize,
    refill_interval: Duration,
    tokens: Arc<RwLock<TokenBucket>>,
}

struct TokenBucket {
    tokens: usize,
    last_refill: Instant,
}

impl TokenBucketRateLimiter {
    pub fn new(capacity: usize, refill_rate: usize, refill_interval: Duration) -> Self {
        Self {
            capacity,
            refill_rate,
            refill_interval,
            tokens: Arc::new(RwLock::new(TokenBucket {
                tokens: capacity,
                last_refill: Instant::now(),
            })),
        }
    }

    /// Try to acquire a token
    pub async fn try_acquire(&self) -> Result<(), RateLimitError> {
        let mut bucket = self.tokens.write().await;

        // Refill tokens based on time elapsed
        let elapsed = bucket.last_refill.elapsed();
        let intervals = elapsed.as_secs_f64() / self.refill_interval.as_secs_f64();
        let tokens_to_add = (intervals * self.refill_rate as f64) as usize;

        if tokens_to_add > 0 {
            bucket.tokens = (bucket.tokens + tokens_to_add).min(self.capacity);
            bucket.last_refill = Instant::now();
        }

        // Try to consume a token
        if bucket.tokens > 0 {
            bucket.tokens -= 1;
            Ok(())
        } else {
            Err(RateLimitError::Exceeded {
                retry_after: self.refill_interval,
            })
        }
    }

    /// Wait for token to become available
    pub async fn acquire(&self) -> Result<(), RateLimitError> {
        loop {
            match self.try_acquire().await {
                Ok(()) => return Ok(()),
                Err(RateLimitError::Exceeded { retry_after }) => {
                    tokio::time::sleep(retry_after).await;
                }
            }
        }
    }
}

/// Rate limit error
#[derive(Debug, thiserror::Error)]
pub enum RateLimitError {
    #[error("rate limit exceeded, retry after {retry_after:?}")]
    Exceeded { retry_after: Duration },
}

/// Multi-level rate limiter
pub struct AgentRateLimiter {
    /// Global rate limit
    global: TokenBucketRateLimiter,
    /// Per-user rate limits
    per_user: Arc<RwLock<HashMap<String, TokenBucketRateLimiter>>>,
    /// Per-tool rate limits
    per_tool: Arc<RwLock<HashMap<String, TokenBucketRateLimiter>>>,
    user_capacity: usize,
    tool_capacity: usize,
    refill_rate: usize,
    refill_interval: Duration,
}

impl AgentRateLimiter {
    pub fn new(
        global_capacity: usize,
        user_capacity: usize,
        tool_capacity: usize,
        refill_rate: usize,
        refill_interval: Duration,
    ) -> Self {
        Self {
            global: TokenBucketRateLimiter::new(global_capacity, refill_rate, refill_interval),
            per_user: Arc::new(RwLock::new(HashMap::new())),
            per_tool: Arc::new(RwLock::new(HashMap::new())),
            user_capacity,
            tool_capacity,
            refill_rate,
            refill_interval,
        }
    }

    /// Check rate limits for request
    pub async fn check_limits(
        &self,
        user_id: &str,
        tool_name: &str,
    ) -> Result<(), RateLimitError> {
        // Check global limit
        self.global.try_acquire().await?;

        // Check per-user limit
        {
            let mut users = self.per_user.write().await;
            let user_limiter = users
                .entry(user_id.to_string())
                .or_insert_with(|| {
                    TokenBucketRateLimiter::new(
                        self.user_capacity,
                        self.refill_rate,
                        self.refill_interval,
                    )
                });
            user_limiter.try_acquire().await?;
        }

        // Check per-tool limit
        {
            let mut tools = self.per_tool.write().await;
            let tool_limiter = tools
                .entry(tool_name.to_string())
                .or_insert_with(|| {
                    TokenBucketRateLimiter::new(
                        self.tool_capacity,
                        self.refill_rate,
                        self.refill_interval,
                    )
                });
            tool_limiter.try_acquire().await?;
        }

        Ok(())
    }
}
```

**Tasks**:
- [ ] Implement token bucket algorithm
- [ ] Global rate limiting
- [ ] Per-user rate limiting
- [ ] Per-tool rate limiting
- [ ] Configurable limits
- [ ] Metrics integration
- [ ] Tests

**Files**:
- `agent-patterns/src/rate_limit.rs` - New module
- `web/src/middleware/rate_limit.rs` - HTTP middleware

---

#### 3.3 Bulkheads & Isolation (5 hours)

**Resource isolation between agent types:**

```rust
// agent-patterns/src/bulkhead.rs
use tokio::sync::Semaphore;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;

/// Bulkhead configuration
#[derive(Debug, Clone)]
pub struct BulkheadConfig {
    /// Maximum concurrent operations
    pub max_concurrent: usize,
    /// Maximum queue size
    pub max_queue: usize,
}

/// Bulkhead for resource isolation
pub struct Bulkhead {
    config: BulkheadConfig,
    semaphore: Arc<Semaphore>,
    queued: Arc<RwLock<usize>>,
}

impl Bulkhead {
    pub fn new(config: BulkheadConfig) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(config.max_concurrent)),
            queued: Arc::new(RwLock::new(0)),
            config,
        }
    }

    /// Execute operation with bulkhead protection
    pub async fn execute<F, T>(&self, f: F) -> Result<T, BulkheadError>
    where
        F: FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>,
    {
        // Check queue size
        {
            let queued = self.queued.read().await;
            if *queued >= self.config.max_queue {
                return Err(BulkheadError::QueueFull);
            }
        }

        // Increment queue count
        {
            let mut queued = self.queued.write().await;
            *queued += 1;
        }

        // Acquire permit
        let _permit = self.semaphore.acquire().await
            .map_err(|_| BulkheadError::AcquireFailed)?;

        // Decrement queue count
        {
            let mut queued = self.queued.write().await;
            *queued -= 1;
        }

        // Execute operation
        Ok(f().await)
    }

    /// Get current stats
    pub async fn stats(&self) -> BulkheadStats {
        BulkheadStats {
            available: self.semaphore.available_permits(),
            max_concurrent: self.config.max_concurrent,
            queued: *self.queued.read().await,
            max_queue: self.config.max_queue,
        }
    }
}

/// Bulkhead error
#[derive(Debug, thiserror::Error)]
pub enum BulkheadError {
    #[error("bulkhead queue is full")]
    QueueFull,

    #[error("failed to acquire permit")]
    AcquireFailed,
}

/// Bulkhead statistics
#[derive(Debug, Clone, Serialize)]
pub struct BulkheadStats {
    pub available: usize,
    pub max_concurrent: usize,
    pub queued: usize,
    pub max_queue: usize,
}

/// Bulkhead registry for different agent types
pub struct BulkheadRegistry {
    bulkheads: Arc<RwLock<HashMap<String, Bulkhead>>>,
}

impl BulkheadRegistry {
    pub fn new() -> Self {
        Self {
            bulkheads: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register bulkhead for agent type
    pub async fn register(&self, agent_type: String, config: BulkheadConfig) {
        let bulkhead = Bulkhead::new(config);
        self.bulkheads.write().await.insert(agent_type, bulkhead);
    }

    /// Execute with bulkhead for agent type
    pub async fn execute<F, T>(
        &self,
        agent_type: &str,
        f: F,
    ) -> Result<T, BulkheadError>
    where
        F: FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>,
    {
        let bulkhead = {
            self.bulkheads.read().await
                .get(agent_type)
                .cloned()
                .ok_or(BulkheadError::AcquireFailed)?
        };

        bulkhead.execute(f).await
    }

    /// Get stats for all bulkheads
    pub async fn all_stats(&self) -> HashMap<String, BulkheadStats> {
        let bulkheads = self.bulkheads.read().await;
        let mut stats = HashMap::new();

        for (agent_type, bulkhead) in bulkheads.iter() {
            stats.insert(agent_type.clone(), bulkhead.stats().await);
        }

        stats
    }
}
```

**Tasks**:
- [ ] Implement `Bulkhead` with semaphore
- [ ] Queue management
- [ ] Per-agent-type bulkheads
- [ ] Bulkhead registry
- [ ] Stats collection
- [ ] Tests

**Files**:
- `agent-patterns/src/bulkhead.rs` - New module

---

### Part 4: Production Metrics (12 hours)

#### 4.1 Prometheus Integration (4 hours)

**Comprehensive metrics:**

```rust
// agent-patterns/src/metrics_exporter.rs
use prometheus::{
    Registry, Counter, Histogram, Gauge, HistogramOpts, Opts,
    IntCounter, IntGauge, IntCounterVec, HistogramVec,
};
use std::sync::Arc;
use lazy_static::lazy_static;

lazy_static! {
    /// Agent pattern executions
    pub static ref AGENT_PATTERN_EXECUTIONS: IntCounterVec = IntCounterVec::new(
        Opts::new("agent_pattern_executions_total", "Total agent pattern executions"),
        &["pattern", "action"]
    ).unwrap();

    /// Agent pattern duration
    pub static ref AGENT_PATTERN_DURATION: HistogramVec = HistogramVec::new(
        HistogramOpts::new("agent_pattern_duration_seconds", "Agent pattern execution duration")
            .buckets(vec![0.001, 0.01, 0.1, 0.5, 1.0, 2.5, 5.0, 10.0]),
        &["pattern", "action"]
    ).unwrap();

    /// Tool executions
    pub static ref TOOL_EXECUTIONS: IntCounterVec = IntCounterVec::new(
        Opts::new("tool_executions_total", "Total tool executions"),
        &["tool", "status"]
    ).unwrap();

    /// Tool duration
    pub static ref TOOL_DURATION: HistogramVec = HistogramVec::new(
        HistogramOpts::new("tool_duration_seconds", "Tool execution duration")
            .buckets(vec![0.01, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0]),
        &["tool"]
    ).unwrap();

    /// LLM calls
    pub static ref LLM_CALLS: IntCounterVec = IntCounterVec::new(
        Opts::new("llm_calls_total", "Total LLM API calls"),
        &["model", "status"]
    ).unwrap();

    /// LLM token usage
    pub static ref LLM_TOKENS: IntCounterVec = IntCounterVec::new(
        Opts::new("llm_tokens_total", "Total LLM tokens used"),
        &["model", "type"]  // type: input/output
    ).unwrap();

    /// Circuit breaker state
    pub static ref CIRCUIT_BREAKER_STATE: IntGaugeVec = IntGaugeVec::new(
        Opts::new("circuit_breaker_state", "Circuit breaker state (0=closed, 1=half-open, 2=open)"),
        &["resource"]
    ).unwrap();

    /// Rate limit rejections
    pub static ref RATE_LIMIT_REJECTIONS: IntCounterVec = IntCounterVec::new(
        Opts::new("rate_limit_rejections_total", "Rate limit rejections"),
        &["limiter"]  // global/user/tool
    ).unwrap();

    /// Active agent sessions
    pub static ref ACTIVE_SESSIONS: IntGauge = IntGauge::new(
        "active_agent_sessions",
        "Number of active agent sessions"
    ).unwrap();

    /// Bulkhead queue size
    pub static ref BULKHEAD_QUEUE_SIZE: IntGaugeVec = IntGaugeVec::new(
        Opts::new("bulkhead_queue_size", "Bulkhead queue size"),
        &["agent_type"]
    ).unwrap();
}

/// Initialize Prometheus registry
pub fn init_metrics(registry: &Registry) -> Result<(), prometheus::Error> {
    registry.register(Box::new(AGENT_PATTERN_EXECUTIONS.clone()))?;
    registry.register(Box::new(AGENT_PATTERN_DURATION.clone()))?;
    registry.register(Box::new(TOOL_EXECUTIONS.clone()))?;
    registry.register(Box::new(TOOL_DURATION.clone()))?;
    registry.register(Box::new(LLM_CALLS.clone()))?;
    registry.register(Box::new(LLM_TOKENS.clone()))?;
    registry.register(Box::new(CIRCUIT_BREAKER_STATE.clone()))?;
    registry.register(Box::new(RATE_LIMIT_REJECTIONS.clone()))?;
    registry.register(Box::new(ACTIVE_SESSIONS.clone()))?;
    registry.register(Box::new(BULKHEAD_QUEUE_SIZE.clone()))?;
    Ok(())
}

/// Record pattern execution
pub fn record_pattern_execution(pattern: &str, action: &str, duration_secs: f64) {
    AGENT_PATTERN_EXECUTIONS
        .with_label_values(&[pattern, action])
        .inc();

    AGENT_PATTERN_DURATION
        .with_label_values(&[pattern, action])
        .observe(duration_secs);
}

/// Record tool execution
pub fn record_tool_execution(tool: &str, success: bool, duration_secs: f64) {
    let status = if success { "success" } else { "error" };

    TOOL_EXECUTIONS
        .with_label_values(&[tool, status])
        .inc();

    TOOL_DURATION
        .with_label_values(&[tool])
        .observe(duration_secs);
}

/// Record LLM call
pub fn record_llm_call(model: &str, success: bool, input_tokens: usize, output_tokens: usize) {
    let status = if success { "success" } else { "error" };

    LLM_CALLS
        .with_label_values(&[model, status])
        .inc();

    LLM_TOKENS
        .with_label_values(&[model, "input"])
        .inc_by(input_tokens as u64);

    LLM_TOKENS
        .with_label_values(&[model, "output"])
        .inc_by(output_tokens as u64);
}
```

**Tasks**:
- [ ] Add prometheus dependency
- [ ] Define metrics (counters, histograms, gauges)
- [ ] Instrument patterns
- [ ] Instrument tools
- [ ] LLM token tracking
- [ ] Circuit breaker metrics
- [ ] Rate limiter metrics
- [ ] Tests

**Files**:
- `agent-patterns/Cargo.toml` - Add prometheus
- `agent-patterns/src/metrics_exporter.rs` - New module
- `web/src/routes/metrics.rs` - `/metrics` endpoint

---

#### 4.2 Grafana Dashboards (4 hours)

**Pre-built dashboards:**

Create JSON dashboard configurations:

```json
// docs/grafana/agent-overview.json
{
  "dashboard": {
    "title": "Agent System Overview",
    "panels": [
      {
        "title": "Pattern Executions Rate",
        "targets": [
          {
            "expr": "rate(agent_pattern_executions_total[5m])"
          }
        ],
        "type": "graph"
      },
      {
        "title": "Tool Success Rate",
        "targets": [
          {
            "expr": "sum(rate(tool_executions_total{status=\"success\"}[5m])) / sum(rate(tool_executions_total[5m]))"
          }
        ],
        "type": "graph"
      },
      {
        "title": "LLM Token Usage",
        "targets": [
          {
            "expr": "rate(llm_tokens_total[5m])"
          }
        ],
        "type": "graph"
      },
      {
        "title": "Circuit Breaker States",
        "targets": [
          {
            "expr": "circuit_breaker_state"
          }
        ],
        "type": "graph"
      },
      {
        "title": "Active Sessions",
        "targets": [
          {
            "expr": "active_agent_sessions"
          }
        ],
        "type": "stat"
      },
      {
        "title": "Pattern P95 Latency",
        "targets": [
          {
            "expr": "histogram_quantile(0.95, rate(agent_pattern_duration_seconds_bucket[5m]))"
          }
        ],
        "type": "graph"
      }
    ]
  }
}
```

**Tasks**:
- [ ] Create agent overview dashboard
- [ ] Create tool performance dashboard
- [ ] Create LLM usage dashboard
- [ ] Create resilience dashboard (circuit breakers, rate limits)
- [ ] Export dashboard JSON
- [ ] Documentation for importing

**Files**:
- `docs/grafana/agent-overview.json`
- `docs/grafana/tool-performance.json`
- `docs/grafana/llm-usage.json`
- `docs/grafana/resilience.json`
- `docs/grafana/README.md` - Import instructions

---

#### 4.3 Alert Rules (4 hours)

**Prometheus alert rules:**

```yaml
# docs/prometheus/alerts.yml
groups:
  - name: agent_alerts
    interval: 30s
    rules:
      # High error rate
      - alert: HighToolErrorRate
        expr: |
          (
            sum(rate(tool_executions_total{status="error"}[5m]))
            /
            sum(rate(tool_executions_total[5m]))
          ) > 0.1
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High tool error rate (> 10%)"
          description: "Tool error rate is {{ $value | humanizePercentage }}"

      # Circuit breaker opened
      - alert: CircuitBreakerOpen
        expr: circuit_breaker_state == 2
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "Circuit breaker {{ $labels.resource }} is open"
          description: "Service {{ $labels.resource }} is experiencing failures"

      # High latency
      - alert: HighPatternLatency
        expr: |
          histogram_quantile(0.95,
            rate(agent_pattern_duration_seconds_bucket[5m])
          ) > 10
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High pattern latency (P95 > 10s)"
          description: "Pattern {{ $labels.pattern }} P95 latency is {{ $value }}s"

      # Rate limit rejections
      - alert: HighRateLimitRejections
        expr: rate(rate_limit_rejections_total[5m]) > 10
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High rate of rate limit rejections"
          description: "{{ $labels.limiter }} limiter rejecting {{ $value }}/s"

      # LLM token budget exhaustion
      - alert: HighLLMTokenUsage
        expr: |
          sum(rate(llm_tokens_total[1h])) > 1000000
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "High LLM token usage"
          description: "Token usage is {{ $value }}/hour"

      # Bulkhead queue building up
      - alert: BulkheadQueueGrowing
        expr: bulkhead_queue_size > 10
        for: 2m
        labels:
          severity: warning
        annotations:
          summary: "Bulkhead queue for {{ $labels.agent_type }} is growing"
          description: "Queue size is {{ $value }}"

      # No active sessions (service might be down)
      - alert: NoActiveSessions
        expr: active_agent_sessions == 0
        for: 10m
        labels:
          severity: info
        annotations:
          summary: "No active agent sessions"
          description: "No sessions for 10+ minutes, service might be idle or down"
```

**Tasks**:
- [ ] Define alert rules
- [ ] Set appropriate thresholds
- [ ] Configure severity levels
- [ ] Add helpful annotations
- [ ] Test alerts
- [ ] Documentation

**Files**:
- `docs/prometheus/alerts.yml`
- `docs/monitoring.md` - Alert runbook

---

### Part 5: Deployment & Operations (18 hours)

#### 5.1 Docker Configuration (4 hours)

**Multi-stage Dockerfile:**

```dockerfile
# Dockerfile
FROM rust:1.85-alpine AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev openssl-dev

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY core ./core
COPY runtime ./runtime
COPY tools ./tools
COPY anthropic ./anthropic
COPY agent-patterns ./agent-patterns
COPY web ./web

# Build in release mode
RUN cargo build --release --bin agent-server

# Runtime stage
FROM alpine:latest

# Install runtime dependencies
RUN apk add --no-cache ca-certificates openssl

# Create non-root user
RUN addgroup -g 1000 agent && \
    adduser -D -u 1000 -G agent agent

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/agent-server /usr/local/bin/

# Copy configuration
COPY config/production.toml /app/config/

# Switch to non-root user
USER agent

# Expose ports
EXPOSE 8080 9090

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD wget --no-verbose --tries=1 --spider http://localhost:8080/health/live || exit 1

# Run server
CMD ["agent-server", "--config", "/app/config/production.toml"]
```

**docker-compose.yml:**

```yaml
version: '3.8'

services:
  agent-server:
    build: .
    ports:
      - "8080:8080"
      - "9090:9090"
    environment:
      - RUST_LOG=info
      - ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY}
      - DATABASE_URL=postgres://postgres:password@postgres:5432/agents
    depends_on:
      - postgres
      - jaeger
      - prometheus
    healthcheck:
      test: ["CMD", "wget", "--spider", "http://localhost:8080/health/live"]
      interval: 30s
      timeout: 3s
      retries: 3
    restart: unless-stopped

  postgres:
    image: postgres:16-alpine
    environment:
      - POSTGRES_DB=agents
      - POSTGRES_USER=postgres
      - POSTGRES_PASSWORD=password
    volumes:
      - postgres_data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 10s
      timeout: 5s
      retries: 5

  jaeger:
    image: jaegertracing/all-in-one:latest
    ports:
      - "16686:16686"  # Jaeger UI
      - "6831:6831/udp"  # Jaeger agent
    environment:
      - COLLECTOR_OTLP_ENABLED=true

  prometheus:
    image: prom/prometheus:latest
    ports:
      - "9091:9090"
    volumes:
      - ./docs/prometheus:/etc/prometheus
      - prometheus_data:/prometheus
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--storage.tsdb.path=/prometheus'

  grafana:
    image: grafana/grafana:latest
    ports:
      - "3000:3000"
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=admin
      - GF_INSTALL_PLUGINS=grafana-piechart-panel
    volumes:
      - ./docs/grafana:/etc/grafana/provisioning
      - grafana_data:/var/lib/grafana
    depends_on:
      - prometheus

volumes:
  postgres_data:
  prometheus_data:
  grafana_data:
```

**Tasks**:
- [ ] Create multi-stage Dockerfile
- [ ] Add docker-compose.yml
- [ ] Configure health checks
- [ ] Add Jaeger for tracing
- [ ] Add Prometheus + Grafana
- [ ] Documentation
- [ ] Test full stack

**Files**:
- `Dockerfile`
- `docker-compose.yml`
- `.dockerignore`
- `docs/docker-deploy.md`

---

#### 5.2 Kubernetes Manifests (6 hours)

**Kubernetes deployment:**

```yaml
# k8s/deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: agent-server
  namespace: agents
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
        - name: DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: agent-secrets
              key: database-url
        resources:
          requests:
            cpu: 100m
            memory: 256Mi
          limits:
            cpu: 1000m
            memory: 1Gi
        livenessProbe:
          httpGet:
            path: /health/live
            port: 8080
          initialDelaySeconds: 10
          periodSeconds: 30
          timeoutSeconds: 3
          failureThreshold: 3
        readinessProbe:
          httpGet:
            path: /health/ready
            port: 8080
          initialDelaySeconds: 5
          periodSeconds: 10
          timeoutSeconds: 3
          failureThreshold: 3
      serviceAccountName: agent-server

---
apiVersion: v1
kind: Service
metadata:
  name: agent-server
  namespace: agents
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
  type: ClusterIP

---
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: agent-server-hpa
  namespace: agents
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
  - type: Resource
    resource:
      name: memory
      target:
        type: Utilization
        averageUtilization: 80

---
apiVersion: v1
kind: Secret
metadata:
  name: agent-secrets
  namespace: agents
type: Opaque
stringData:
  anthropic-api-key: "YOUR_API_KEY_HERE"
  database-url: "postgres://user:pass@postgres:5432/agents"

---
apiVersion: v1
kind: ServiceAccount
metadata:
  name: agent-server
  namespace: agents

---
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: agent-server-pdb
  namespace: agents
spec:
  minAvailable: 2
  selector:
    matchLabels:
      app: agent-server
```

**Ingress:**

```yaml
# k8s/ingress.yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: agent-server-ingress
  namespace: agents
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
    nginx.ingress.kubernetes.io/rate-limit: "100"
spec:
  ingressClassName: nginx
  tls:
  - hosts:
    - agents.example.com
    secretName: agent-server-tls
  rules:
  - host: agents.example.com
    http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: agent-server
            port:
              number: 80
```

**Tasks**:
- [ ] Create deployment manifests
- [ ] Configure service
- [ ] Add HPA for autoscaling
- [ ] Configure ingress
- [ ] Add PodDisruptionBudget
- [ ] Secret management
- [ ] ServiceMonitor for Prometheus
- [ ] Tests with kind/minikube

**Files**:
- `k8s/namespace.yaml`
- `k8s/deployment.yaml`
- `k8s/service.yaml`
- `k8s/ingress.yaml`
- `k8s/hpa.yaml`
- `k8s/pdb.yaml`
- `k8s/servicemonitor.yaml`
- `docs/k8s-deploy.md`

---

#### 5.3 Configuration Management (4 hours)

**Environment-specific configs:**

```toml
# config/production.toml
[server]
host = "0.0.0.0"
port = 8080
metrics_port = 9090

[logging]
level = "info"
format = "json"

[tracing]
enabled = true
jaeger_endpoint = "http://jaeger:14268/api/traces"
service_name = "agent-server"

[llm]
provider = "anthropic"
model = "claude-3-5-sonnet-20241022"
max_tokens = 4096
timeout_secs = 60

[resilience]
[resilience.circuit_breaker]
failure_threshold = 5
success_threshold = 2
timeout_secs = 30

[resilience.rate_limit]
global_capacity = 1000
user_capacity = 100
tool_capacity = 500
refill_rate_per_sec = 10

[resilience.bulkhead]
default_max_concurrent = 10
default_max_queue = 100

[tools]
timeout_secs = 30
max_retries = 3

[health]
startup_timeout_secs = 30
check_interval_secs = 60

[shutdown]
graceful_timeout_secs = 30
```

**Configuration loader:**

```rust
// web/src/config.rs
use serde::Deserialize;
use std::path::Path;
use anyhow::Result;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub logging: LoggingConfig,
    pub tracing: TracingConfig,
    pub llm: LLMConfig,
    pub resilience: ResilienceConfig,
    pub tools: ToolsConfig,
    pub health: HealthConfig,
    pub shutdown: ShutdownConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub metrics_port: u16,
}

#[derive(Debug, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

#[derive(Debug, Deserialize)]
pub struct TracingConfig {
    pub enabled: bool,
    pub jaeger_endpoint: String,
    pub service_name: String,
}

#[derive(Debug, Deserialize)]
pub struct LLMConfig {
    pub provider: String,
    pub model: String,
    pub max_tokens: usize,
    pub timeout_secs: u64,
}

#[derive(Debug, Deserialize)]
pub struct ResilienceConfig {
    pub circuit_breaker: CircuitBreakerConfig,
    pub rate_limit: RateLimitConfig,
    pub bulkhead: BulkheadConfig,
}

// ... (other config structs)

impl Config {
    /// Load configuration from file
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Load from environment variables (override)
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(port) = std::env::var("PORT") {
            if let Ok(port) = port.parse() {
                self.server.port = port;
            }
        }

        if let Ok(log_level) = std::env::var("RUST_LOG") {
            self.logging.level = log_level;
        }

        if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
            // Store in LLM config
        }

        self
    }
}
```

**Tasks**:
- [ ] Create config structures
- [ ] Environment-specific configs (dev, staging, prod)
- [ ] Environment variable overrides
- [ ] Config validation
- [ ] Documentation
- [ ] Tests

**Files**:
- `config/development.toml`
- `config/staging.toml`
- `config/production.toml`
- `web/src/config.rs`
- `docs/configuration.md`

---

#### 5.4 Deployment Documentation (4 hours)

**Comprehensive deployment guides:**

```markdown
# docs/deployment-guide.md

# Agent System Deployment Guide

## Prerequisites

- Docker 24.0+
- Kubernetes 1.28+ (for K8s deployment)
- PostgreSQL 16+
- Anthropic API key

## Quick Start (Docker Compose)

1. Clone repository:
```bash
git clone https://github.com/yourorg/composable-rust.git
cd composable-rust
```

2. Set environment variables:
```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

3. Start services:
```bash
docker-compose up -d
```

4. Verify health:
```bash
curl http://localhost:8080/health/live
```

5. Access dashboards:
- Jaeger: http://localhost:16686
- Prometheus: http://localhost:9091
- Grafana: http://localhost:3000 (admin/admin)

## Production Deployment (Kubernetes)

### 1. Prepare Secrets

```bash
kubectl create namespace agents
kubectl create secret generic agent-secrets \
  --from-literal=anthropic-api-key="sk-ant-..." \
  --from-literal=database-url="postgres://..." \
  -n agents
```

### 2. Deploy Infrastructure

```bash
# PostgreSQL
kubectl apply -f k8s/postgres.yaml

# Observability stack
kubectl apply -f k8s/prometheus.yaml
kubectl apply -f k8s/jaeger.yaml
kubectl apply -f k8s/grafana.yaml
```

### 3. Deploy Agent Server

```bash
kubectl apply -f k8s/deployment.yaml
kubectl apply -f k8s/service.yaml
kubectl apply -f k8s/ingress.yaml
kubectl apply -f k8s/hpa.yaml
```

### 4. Verify Deployment

```bash
kubectl get pods -n agents
kubectl logs -f deployment/agent-server -n agents
```

### 5. Import Grafana Dashboards

1. Access Grafana: http://grafana.example.com
2. Go to Dashboards → Import
3. Upload JSON files from `docs/grafana/`

## Scaling

### Horizontal Scaling

HPA automatically scales based on CPU/memory:

```bash
kubectl get hpa -n agents
```

Manual scaling:

```bash
kubectl scale deployment agent-server --replicas=5 -n agents
```

### Vertical Scaling

Update resource limits in `k8s/deployment.yaml`:

```yaml
resources:
  requests:
    cpu: 500m
    memory: 1Gi
  limits:
    cpu: 2000m
    memory: 4Gi
```

## Monitoring

### Metrics

Prometheus metrics available at `/metrics`:

```bash
curl http://agent-server:9090/metrics
```

### Tracing

View distributed traces in Jaeger:

```bash
open http://jaeger.example.com
```

### Logs

Structured JSON logs:

```bash
kubectl logs -f deployment/agent-server -n agents | jq .
```

## Troubleshooting

### Health Check Failing

```bash
kubectl describe pod -l app=agent-server -n agents
kubectl logs -l app=agent-server -n agents --tail=100
```

### High Latency

1. Check Grafana dashboard: "Agent System Overview"
2. Look for slow tools in "Tool Performance"
3. Check circuit breaker states

### Circuit Breaker Open

1. Check Prometheus alert: `CircuitBreakerOpen`
2. Identify failing resource
3. Check resource logs
4. Manual reset: Restart pod

### Rate Limiting

Adjust limits in `config/production.toml`:

```toml
[resilience.rate_limit]
global_capacity = 2000  # Increase
user_capacity = 200
```

## Backup & Recovery

### Database Backup

```bash
kubectl exec -it postgres-0 -n agents -- \
  pg_dump -U postgres agents > backup.sql
```

### Restore

```bash
kubectl exec -i postgres-0 -n agents -- \
  psql -U postgres agents < backup.sql
```

## Rollback

```bash
kubectl rollout undo deployment/agent-server -n agents
kubectl rollout status deployment/agent-server -n agents
```

## Security

### Network Policies

Apply network policies to restrict traffic:

```bash
kubectl apply -f k8s/network-policy.yaml
```

### RBAC

Service account has minimal permissions:

```bash
kubectl get rolebinding -n agents
```

### TLS

Ingress uses Let's Encrypt:

```yaml
annotations:
  cert-manager.io/cluster-issuer: letsencrypt-prod
```

## Cost Optimization

### Resource Tuning

Monitor actual usage:

```bash
kubectl top pods -n agents
```

Adjust requests/limits accordingly.

### LLM Token Usage

Monitor in Grafana: "LLM Usage" dashboard

Set budget alerts in Prometheus.

## Disaster Recovery

### RTO/RPO

- RTO: 5 minutes (K8s auto-restart)
- RPO: 1 hour (database backups)

### Multi-Region

Deploy to multiple clusters with global load balancer.

## Support

- Documentation: https://docs.example.com
- Issues: https://github.com/yourorg/composable-rust/issues
- Slack: #agent-system
```

**Tasks**:
- [ ] Write deployment guide
- [ ] Create troubleshooting runbook
- [ ] Document scaling procedures
- [ ] Add monitoring guide
- [ ] Backup/recovery procedures
- [ ] Security best practices
- [ ] Cost optimization guide

**Files**:
- `docs/deployment-guide.md`
- `docs/troubleshooting.md`
- `docs/monitoring.md`
- `docs/security.md`

---

### Part 6: Audit Logging (6 hours)

#### 6.1 Audit Event System (4 hours)

**Compliance-ready audit logging:**

```rust
// agent-patterns/src/audit.rs
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Audit event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Event ID
    pub id: String,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Event type
    pub event_type: AuditEventType,
    /// User ID (if applicable)
    pub user_id: Option<String>,
    /// Session ID
    pub session_id: Option<String>,
    /// IP address
    pub ip_address: Option<String>,
    /// Event details
    pub details: serde_json::Value,
    /// Outcome
    pub outcome: AuditOutcome,
}

/// Audit event type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    /// Agent session started
    SessionStart,
    /// Agent session ended
    SessionEnd,
    /// Tool executed
    ToolExecution,
    /// LLM call made
    LLMCall,
    /// Rate limit exceeded
    RateLimitExceeded,
    /// Circuit breaker triggered
    CircuitBreakerTriggered,
    /// Authentication event
    Authentication,
    /// Authorization event
    Authorization,
    /// Configuration change
    ConfigChange,
    /// Error occurred
    Error,
}

/// Audit outcome
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Success,
    Failure { reason: String },
    Denied { reason: String },
}

/// Audit logger
pub struct AuditLogger {
    tx: mpsc::UnboundedSender<AuditEvent>,
}

impl AuditLogger {
    /// Create new audit logger
    pub fn new() -> (Self, AuditEventReceiver) {
        let (tx, rx) = mpsc::unbounded_channel();

        (
            Self { tx },
            AuditEventReceiver { rx },
        )
    }

    /// Log audit event
    pub fn log(&self, event: AuditEvent) {
        if let Err(e) = self.tx.send(event) {
            eprintln!("Failed to log audit event: {}", e);
        }
    }

    /// Log tool execution
    pub fn log_tool_execution(
        &self,
        user_id: Option<String>,
        session_id: Option<String>,
        tool_name: &str,
        input: &str,
        outcome: AuditOutcome,
    ) {
        self.log(AuditEvent {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            event_type: AuditEventType::ToolExecution,
            user_id,
            session_id,
            ip_address: None,
            details: serde_json::json!({
                "tool": tool_name,
                "input_length": input.len(),
            }),
            outcome,
        });
    }

    /// Log LLM call
    pub fn log_llm_call(
        &self,
        user_id: Option<String>,
        session_id: Option<String>,
        model: &str,
        input_tokens: usize,
        output_tokens: usize,
        outcome: AuditOutcome,
    ) {
        self.log(AuditEvent {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            event_type: AuditEventType::LLMCall,
            user_id,
            session_id,
            ip_address: None,
            details: serde_json::json!({
                "model": model,
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "total_tokens": input_tokens + output_tokens,
            }),
            outcome,
        });
    }

    /// Log session start
    pub fn log_session_start(
        &self,
        user_id: Option<String>,
        session_id: String,
        ip_address: Option<String>,
    ) {
        self.log(AuditEvent {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            event_type: AuditEventType::SessionStart,
            user_id,
            session_id: Some(session_id),
            ip_address,
            details: serde_json::json!({}),
            outcome: AuditOutcome::Success,
        });
    }
}

/// Audit event receiver
pub struct AuditEventReceiver {
    rx: mpsc::UnboundedReceiver<AuditEvent>,
}

impl AuditEventReceiver {
    /// Receive next audit event
    pub async fn recv(&mut self) -> Option<AuditEvent> {
        self.rx.recv().await
    }
}

/// Audit sink trait
#[async_trait::async_trait]
pub trait AuditSink: Send + Sync {
    /// Write audit event
    async fn write(&self, event: AuditEvent) -> Result<(), String>;
}

/// File-based audit sink
pub struct FileAuditSink {
    path: PathBuf,
}

#[async_trait::async_trait]
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
}

/// Postgres audit sink
pub struct PostgresAuditSink {
    pool: PgPool,
}

#[async_trait::async_trait]
impl AuditSink for PostgresAuditSink {
    async fn write(&self, event: AuditEvent) -> Result<(), String> {
        sqlx::query!(
            r#"
            INSERT INTO audit_log (
                id, timestamp, event_type, user_id, session_id,
                ip_address, details, outcome
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
            event.id,
            event.timestamp,
            serde_json::to_value(&event.event_type).unwrap(),
            event.user_id,
            event.session_id,
            event.ip_address,
            event.details,
            serde_json::to_value(&event.outcome).unwrap(),
        )
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(())
    }
}
```

**Tasks**:
- [ ] Create `AuditEvent` type
- [ ] Implement `AuditLogger`
- [ ] File-based sink
- [ ] PostgreSQL sink
- [ ] Audit event receiver
- [ ] Integration with patterns
- [ ] Tests

**Files**:
- `agent-patterns/src/audit.rs` - New module
- `postgres/migrations/` - Add audit_log table

---

#### 6.2 Compliance Reports (2 hours)

**Generate compliance reports:**

```rust
// agent-patterns/src/audit_reports.rs
use chrono::{DateTime, Utc, Duration};
use serde::Serialize;

/// Audit report generator
pub struct AuditReportGenerator {
    pool: PgPool,
}

impl AuditReportGenerator {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Generate usage report for time period
    pub async fn usage_report(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<UsageReport, String> {
        // Query audit log
        let events = sqlx::query_as!(
            AuditEvent,
            r#"
            SELECT * FROM audit_log
            WHERE timestamp >= $1 AND timestamp <= $2
            ORDER BY timestamp
            "#,
            start,
            end,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        // Aggregate statistics
        let total_sessions = events.iter()
            .filter(|e| matches!(e.event_type, AuditEventType::SessionStart))
            .count();

        let total_tool_calls = events.iter()
            .filter(|e| matches!(e.event_type, AuditEventType::ToolExecution))
            .count();

        let total_llm_calls = events.iter()
            .filter(|e| matches!(e.event_type, AuditEventType::LLMCall))
            .count();

        let total_tokens: usize = events.iter()
            .filter_map(|e| {
                if matches!(e.event_type, AuditEventType::LLMCall) {
                    e.details.get("total_tokens")?.as_u64()
                } else {
                    None
                }
            })
            .sum();

        // Group by user
        let mut user_stats = HashMap::new();
        for event in &events {
            if let Some(user_id) = &event.user_id {
                let stats = user_stats.entry(user_id.clone())
                    .or_insert(UserStats::default());

                match event.event_type {
                    AuditEventType::ToolExecution => stats.tool_calls += 1,
                    AuditEventType::LLMCall => stats.llm_calls += 1,
                    _ => {}
                }
            }
        }

        Ok(UsageReport {
            period_start: start,
            period_end: end,
            total_sessions,
            total_tool_calls,
            total_llm_calls,
            total_tokens,
            user_stats,
        })
    }

    /// Generate security report
    pub async fn security_report(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<SecurityReport, String> {
        let failed_auth = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) FROM audit_log
            WHERE timestamp >= $1 AND timestamp <= $2
            AND event_type = 'authentication'
            AND outcome->>'status' = 'failure'
            "#,
            start,
            end,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        let rate_limit_hits = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) FROM audit_log
            WHERE timestamp >= $1 AND timestamp <= $2
            AND event_type = 'rate_limit_exceeded'
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
            failed_authentications: failed_auth.unwrap_or(0) as usize,
            rate_limit_hits: rate_limit_hits.unwrap_or(0) as usize,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct UsageReport {
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub total_sessions: usize,
    pub total_tool_calls: usize,
    pub total_llm_calls: usize,
    pub total_tokens: usize,
    pub user_stats: HashMap<String, UserStats>,
}

#[derive(Debug, Default, Serialize)]
pub struct UserStats {
    pub tool_calls: usize,
    pub llm_calls: usize,
}

#[derive(Debug, Serialize)]
pub struct SecurityReport {
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub failed_authentications: usize,
    pub rate_limit_hits: usize,
}
```

**Tasks**:
- [ ] Create report generator
- [ ] Usage reports (per-user, per-tool, per-time)
- [ ] Security reports
- [ ] Cost reports (LLM token usage)
- [ ] Export to CSV/JSON
- [ ] Tests

**Files**:
- `agent-patterns/src/audit_reports.rs` - New module
- `web/src/routes/reports.rs` - Report endpoints

---

## Success Criteria

### Observability ✓
- [ ] OpenTelemetry distributed tracing integrated
- [ ] Trace context propagates through all effects
- [ ] Jaeger shows end-to-end agent workflows
- [ ] All 7 patterns instrumented with spans

### Health & Lifecycle ✓
- [ ] Liveness probe implemented
- [ ] Readiness probe checks LLM + tools
- [ ] Startup checks validate dependencies
- [ ] Graceful shutdown with 30s timeout
- [ ] All components cleanup properly

### Resilience ✓
- [ ] Circuit breakers protect tool calls
- [ ] Rate limiting (global/user/tool)
- [ ] Bulkheads isolate agent types
- [ ] All failures degrade gracefully

### Metrics & Monitoring ✓
- [ ] Prometheus metrics exported
- [ ] 4 Grafana dashboards created
- [ ] 7 alert rules defined
- [ ] SLOs documented

### Deployment ✓
- [ ] Docker multi-stage build
- [ ] docker-compose for local dev
- [ ] Kubernetes manifests (deployment, service, ingress, HPA)
- [ ] Configuration management (dev/staging/prod)
- [ ] Deployment documentation

### Audit & Compliance ✓
- [ ] Audit events for all sensitive operations
- [ ] PostgreSQL audit sink
- [ ] Usage reports
- [ ] Security reports
- [ ] Audit log retention policy

### Quality ✓
- [ ] All tests passing (target: 40+ new tests)
- [ ] Zero clippy warnings
- [ ] Documentation complete
- [ ] Example deployments tested

---

## Timeline Estimate

| Part | Task | Hours |
|------|------|-------|
| 1 | Distributed Tracing | 12 |
| | 1.1 OpenTelemetry integration | 4 |
| | 1.2 Trace context propagation | 4 |
| | 1.3 Agent-specific spans | 4 |
| 2 | Health Checks & Lifecycle | 10 |
| | 2.1 Health check system | 4 |
| | 2.2 Graceful shutdown | 3 |
| | 2.3 Startup checks | 3 |
| 3 | Advanced Resilience | 15 |
| | 3.1 Circuit breakers | 5 |
| | 3.2 Rate limiting | 5 |
| | 3.3 Bulkheads | 5 |
| 4 | Production Metrics | 12 |
| | 4.1 Prometheus integration | 4 |
| | 4.2 Grafana dashboards | 4 |
| | 4.3 Alert rules | 4 |
| 5 | Deployment & Operations | 18 |
| | 5.1 Docker configuration | 4 |
| | 5.2 Kubernetes manifests | 6 |
| | 5.3 Configuration management | 4 |
| | 5.4 Deployment documentation | 4 |
| 6 | Audit Logging | 6 |
| | 6.1 Audit event system | 4 |
| | 6.2 Compliance reports | 2 |
| **Total** | | **67 hours** |

**Schedule**: 8-9 days (67 hours ÷ 7-8 hours/day = 8.4-9.6 days)

---

## Design Decisions

### 1. OpenTelemetry Over Custom Tracing

**Decision**: Use OpenTelemetry standard

**Rationale**:
- Industry standard
- Works with Jaeger, Zipkin, DataDog, etc.
- Vendor-neutral
- Rich ecosystem

### 2. Token Bucket for Rate Limiting

**Decision**: Token bucket algorithm

**Rationale**:
- Handles bursts
- Simple to implement
- Well-understood
- Configurable refill rate

### 3. Per-Tool Circuit Breakers

**Decision**: Independent circuit breakers per tool

**Rationale**:
- Isolate failures
- One broken tool doesn't affect others
- Easier debugging
- Fine-grained control

### 4. PostgreSQL for Audit Log

**Decision**: Store audit events in PostgreSQL

**Rationale**:
- Queryable for reports
- ACID guarantees
- Existing infrastructure
- Easy backup/retention policies

### 5. Kubernetes-Native Health Checks

**Decision**: Separate liveness and readiness probes

**Rationale**:
- Liveness: Restart if critical failure
- Readiness: Remove from load balancer if degraded
- K8s best practice

### 6. Prometheus + Grafana

**Decision**: Standard monitoring stack

**Rationale**:
- Industry standard
- Rich query language (PromQL)
- Pre-built dashboards
- Active community

---

## Security Considerations

### Audit Log Protection

- Append-only table
- Separate retention policy
- Regular exports for compliance
- Access logging on audit table itself

### Secrets Management

- Never log secrets
- Use Kubernetes secrets
- Rotate API keys regularly
- Audit secret access

### Network Security

- Network policies in K8s
- TLS for all external traffic
- Mutual TLS for service-to-service
- Rate limiting at ingress

---

## Performance Considerations

### Tracing Overhead

- Sampling rate: 10% in production
- 100% for errors
- Minimal serialization overhead

### Metrics Collection

- Pre-aggregate where possible
- Use histograms for latencies
- Bounded cardinality (no user IDs in labels)

### Audit Logging

- Async channel for audit events
- Batch writes to database
- Separate thread pool
- No blocking on audit writes

---

## Testing Strategy

### Integration Tests

- Test health checks with testcontainers
- Verify shutdown cleanup
- Circuit breaker state transitions
- Rate limiter token refill

### Load Tests

- K6 scripts for load testing
- Target: 1000 req/sec
- Circuit breaker triggers under load
- HPA scales correctly

### Chaos Tests

- Kill random pods
- Network partitions
- Database failures
- LLM API failures

---

## Documentation Requirements

### Runbooks

- Deployment procedures
- Rollback procedures
- Incident response
- Common troubleshooting

### Architecture Diagrams

- Deployment architecture
- Observability flow
- Failure modes
- Data flow

### SLOs/SLAs

- Availability: 99.9%
- P95 latency: < 2s
- Error rate: < 1%

---

## Migration from Phase 8.3

### Backward Compatibility

All Phase 8.3 code works unchanged:
- ✅ Agent patterns work as-is
- ✅ Tool registry unchanged
- ✅ Examples work

### Upgrades

New features are opt-in:
- Tracing: Enable in config
- Health checks: Deploy with K8s
- Circuit breakers: Wrap tool registry
- Metrics: Prometheus scraping

---

## References

- **OpenTelemetry**: https://opentelemetry.io/docs/
- **Kubernetes Health Checks**: https://kubernetes.io/docs/tasks/configure-pod-container/configure-liveness-readiness-startup-probes/
- **Prometheus Best Practices**: https://prometheus.io/docs/practices/
- **Phase 8.3 Plan**: `plans/phase-8/phase-8.3-implementation-plan.md`
- **Modern Rust Expert**: `.claude/skills/modern-rust-expert/SKILL.md`

---

## Ready to Begin! 🚀

**Phase 8.4 will transform the agent system from feature-complete to production-ready.**

**Key Outcomes**:
1. **Observable**: Full distributed tracing + metrics
2. **Resilient**: Circuit breakers, rate limiting, bulkheads
3. **Operational**: Health checks, graceful shutdown, K8s-ready
4. **Compliant**: Comprehensive audit logging
5. **Scalable**: HPA, deployment automation, monitoring

**Estimated effort**: 67 hours across 8-9 focused days.
