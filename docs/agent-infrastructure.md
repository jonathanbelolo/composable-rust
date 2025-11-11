# Agent Infrastructure Guide

Complete guide to building production-ready AI agents with Composable Rust.

## Table of Contents

1. [Overview](#overview)
2. [Architecture Patterns](#architecture-patterns)
3. [Resilience Patterns](#resilience-patterns)
4. [Health Checks & Liveness](#health-checks--liveness)
5. [Observability](#observability)
6. [Security & Audit](#security--audit)
7. [Deployment](#deployment)
8. [Operations](#operations)
9. [Best Practices](#best-practices)

## Overview

The `composable-rust-agent-patterns` crate provides production-ready infrastructure for building AI agents. It implements proven patterns from Anthropic's agent design guidelines with a focus on reliability, observability, and security.

### Key Features

- **Resilience**: Circuit breakers, rate limiting, bulkhead isolation
- **Observability**: OpenTelemetry tracing, structured logging, Prometheus metrics
- **Health Checks**: Kubernetes-ready startup, liveness, and readiness probes
- **Security**: Audit logging, security monitoring, threat detection
- **Graceful Shutdown**: Coordinated shutdown with timeout handling

### When to Use

Use this infrastructure when building:

- **Production AI Agents**: Customer-facing agents requiring high reliability
- **Multi-Agent Systems**: Coordinated agents with cross-cutting concerns
- **Enterprise Applications**: Systems requiring audit trails and compliance
- **High-Scale Services**: Applications serving thousands of requests per second

## Architecture Patterns

### Agent State Machine

Agents are implemented as state machines using the Reducer pattern:

```rust
use composable_rust_agent_patterns::*;

pub struct AgentState {
    conversation_id: Option<String>,
    messages: Vec<Message>,
    user_id: Option<String>,
}

pub enum AgentAction {
    StartConversation { user_id: String },
    SendMessage { content: String },
    ProcessResponse { response: String },
    EndConversation,
}

pub struct AgentReducer {
    audit_logger: Arc<InMemoryAuditLogger>,
    security_monitor: Arc<SecurityMonitor>,
}

impl AgentReducer {
    pub fn reduce(
        &self,
        state: &mut AgentState,
        action: AgentAction,
        env: &impl AgentEnvironment,
    ) -> Effects {
        match action {
            AgentAction::SendMessage { content } => {
                // 1. Security: Check for prompt injection
                if self.detect_prompt_injection(&content) {
                    return self.handle_security_incident(state);
                }

                // 2. Update state
                state.messages.push(Message { content, .. });

                // 3. Return effects (audit log, LLM call)
                self.log_and_call_llm(state)
            }
            // Handle other actions...
        }
    }
}
```

### Environment Abstraction

The `AgentEnvironment` trait provides dependency injection:

```rust
pub trait AgentEnvironment: Send + Sync {
    /// Call LLM with messages
    fn call_llm(
        &self,
        messages: &[Message],
    ) -> impl Future<Output = Result<String, AgentError>> + Send;

    /// Execute tool with input
    fn execute_tool(
        &self,
        tool_name: &str,
        input: &str,
    ) -> impl Future<Output = Result<String, AgentError>> + Send;

    /// Log audit event
    fn log_audit(
        &self,
        event_type: &str,
        actor: &str,
        action: &str,
        success: bool,
    ) -> impl Future<Output = Result<(), AgentError>> + Send;
}
```

**Production Implementation**:

```rust
pub struct ProductionEnvironment {
    llm_circuit_breaker: Arc<CircuitBreaker>,
    rate_limiter: Arc<RateLimiter>,
    bulkhead: Arc<Bulkhead>,
    audit_logger: Arc<InMemoryAuditLogger>,
    security_monitor: Arc<SecurityMonitor>,
}

impl AgentEnvironment for ProductionEnvironment {
    async fn call_llm(&self, messages: &[Message]) -> Result<String, AgentError> {
        // 1. Rate limiting
        self.rate_limiter.try_acquire(1).await?;

        // 2. Circuit breaker
        self.llm_circuit_breaker.allow_request().await?;

        // 3. Call LLM (with timeout)
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            self.actual_llm_call(messages)
        ).await??;

        // 4. Record success/failure
        match &result {
            Ok(_) => self.llm_circuit_breaker.record_success().await,
            Err(_) => self.llm_circuit_breaker.record_failure().await,
        }

        result
    }
}
```

## Resilience Patterns

### Circuit Breaker

Prevents cascading failures by temporarily blocking requests to failing services.

**Configuration**:

```rust
use composable_rust_agent_patterns::resilience::*;

let config = CircuitBreakerConfig {
    failure_threshold: 5,    // Open after 5 failures
    success_threshold: 2,    // Close after 2 successes
    timeout: Duration::from_secs(60),  // Wait 60s before retry
};

let circuit_breaker = CircuitBreaker::new("llm".to_string(), config);
```

**State Machine**:

```
┌─────────┐  5 failures   ┌──────┐
│ Closed  │──────────────>│ Open │
└────┬────┘                └───┬──┘
     ↑                         │
     │                    60s timeout
     │                         │
     │                         ↓
     │                    ┌──────────┐
     └────2 successes─────│ HalfOpen │
                          └──────────┘
```

**Usage**:

```rust
// Check if request is allowed
circuit_breaker.allow_request().await?;

// Execute request
let result = dangerous_operation().await;

// Record result
match result {
    Ok(_) => circuit_breaker.record_success().await,
    Err(_) => circuit_breaker.record_failure().await,
}
```

**Metrics**:

```rust
let state = circuit_breaker.state();  // Closed, Open, or HalfOpen
let metrics = circuit_breaker.metrics();
println!("Total requests: {}", metrics.total_requests);
println!("Failure rate: {:.2}%", metrics.failure_rate * 100.0);
```

### Rate Limiter

Token bucket algorithm for request throttling.

**Configuration**:

```rust
let config = RateLimiterConfig {
    capacity: 100,          // Bucket capacity (burst)
    refill_rate: 10.0,      // Tokens per second
};

let rate_limiter = RateLimiter::new("api".to_string(), config);
```

**Usage**:

```rust
// Try to acquire N tokens
match rate_limiter.try_acquire(1).await {
    Ok(()) => {
        // Request allowed
        handle_request().await
    }
    Err(RateLimiterError::RateLimitExceeded) => {
        // Return 429 Too Many Requests
        Err(AgentError::RateLimited)
    }
}
```

**Dynamic Adjustment**:

```rust
// Adjust rate limit based on load
if high_load_detected {
    rate_limiter.set_rate(5.0);  // Reduce to 5 req/sec
} else {
    rate_limiter.set_rate(10.0);  // Normal rate
}
```

### Bulkhead Pattern

Isolates resources to prevent one failing component from exhausting all resources.

**Configuration**:

```rust
let config = BulkheadConfig {
    max_concurrent: 10,     // Max concurrent operations
    acquire_timeout: Duration::from_secs(5),  // Wait timeout
};

let bulkhead = Bulkhead::new("tool_execution".to_string(), config);
```

**Usage**:

```rust
// Execute with resource isolation
let result = bulkhead.execute(async {
    expensive_tool_call().await
}).await?;
```

**Benefits**:

- **Prevents resource exhaustion**: Limits concurrent operations
- **Isolates failures**: Tool failures don't affect LLM calls
- **Provides backpressure**: Queue fills → fast fail with error

## Health Checks & Liveness

Kubernetes-ready health check system with three probe types.

### Health Check Types

#### 1. Startup Probe (`/health`)

Checks if application has fully initialized.

```rust
use composable_rust_agent_patterns::health::*;

struct DatabaseHealthCheck {
    pool: PgPool,
}

#[async_trait]
impl HealthCheckable for DatabaseHealthCheck {
    async fn check_health(&self) -> ComponentHealth {
        match self.pool.acquire().await {
            Ok(_) => ComponentHealth::healthy("Database connected"),
            Err(e) => ComponentHealth::unhealthy(format!("DB error: {}", e)),
        }
    }

    fn component_name(&self) -> &str {
        "database"
    }
}
```

#### 2. Liveness Probe (`/health/live`)

Simple "am I alive?" check. Kubernetes restarts pod if this fails.

```rust
async fn liveness_handler() -> Response {
    // Process is alive
    (StatusCode::OK, "alive").into_response()
}
```

#### 3. Readiness Probe (`/health/ready`)

Checks if service can accept traffic. Kubernetes removes from load balancer if this fails.

```rust
async fn readiness_handler(state: ServerState) -> Response {
    let results = state.health_registry.check_all().await;
    let all_ready = results.values().all(|h| h.status == HealthStatus::Healthy);

    if all_ready {
        (StatusCode::OK, "ready").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response()
    }
}
```

### System Health Aggregation

```rust
let mut system_health = SystemHealthCheck::new();

// Register components
system_health.add_check(Arc::new(DatabaseHealthCheck::new(pool)));
system_health.add_check(Arc::new(LLMHealthCheck::new(client)));
system_health.add_check(Arc::new(RedisHealthCheck::new(redis)));

// Check all components (runs in parallel)
let results: HashMap<String, ComponentHealth> = system_health.check_all().await;

// Overall health status
let status = system_health.overall_health().await;
```

### Kubernetes Configuration

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: agent
spec:
  containers:
  - name: agent
    image: production-agent:latest
    ports:
    - containerPort: 8080
      name: http

    # Startup: Wait up to 5 minutes for initialization
    startupProbe:
      httpGet:
        path: /health
        port: 8080
      initialDelaySeconds: 0
      periodSeconds: 10
      failureThreshold: 30  # 5 minutes

    # Liveness: Restart if process is unresponsive
    livenessProbe:
      httpGet:
        path: /health/live
        port: 8080
      initialDelaySeconds: 10
      periodSeconds: 10
      timeoutSeconds: 5
      failureThreshold: 3

    # Readiness: Remove from LB if can't handle traffic
    readinessProbe:
      httpGet:
        path: /health/ready
        port: 8080
      initialDelaySeconds: 5
      periodSeconds: 5
      timeoutSeconds: 3
      failureThreshold: 2
```

### Graceful Shutdown

Coordinated shutdown ensures clean termination.

```rust
use composable_rust_agent_patterns::shutdown::*;

// Create coordinator with 30s timeout
let shutdown = Arc::new(ShutdownCoordinator::new(Duration::from_secs(30)));

// Register shutdown handlers
shutdown.register(Arc::new(ServerShutdownHandler::new(server)));
shutdown.register(Arc::new(DatabaseShutdownHandler::new(pool)));

// Subscribe to shutdown signal
let mut shutdown_rx = shutdown.subscribe();

// Use in axum server
axum::serve(listener, app)
    .with_graceful_shutdown(async move {
        let _ = shutdown_rx.recv().await;
    })
    .await?;

// Wait for Ctrl+C
signal::ctrl_c().await?;

// Initiate shutdown (broadcasts to all subscribers)
shutdown.shutdown().await?;
```

**Custom Shutdown Handler**:

```rust
struct CacheShutdownHandler {
    cache: Arc<RedisCache>,
}

#[async_trait]
impl ShutdownHandler for CacheShutdownHandler {
    fn name(&self) -> &str {
        "redis_cache"
    }

    async fn shutdown(&self) -> Result<(), String> {
        info!("Flushing cache...");
        self.cache.flush().await.map_err(|e| e.to_string())?;
        info!("Cache flushed");
        Ok(())
    }
}
```

## Observability

### Tracing with OpenTelemetry

**Setup**:

```rust
use composable_rust_agent_patterns::tracing_support::*;

// Initialize tracing (includes OpenTelemetry)
init_tracing("production-agent", "http://jaeger:4317").await?;
```

**Instrumentation**:

```rust
#[tracing::instrument(skip(self, state, env))]
fn send_message<E: AgentEnvironment>(
    &self,
    state: &mut AgentState,
    content: String,
    env: &E,
) -> Effects {
    info!("Processing message from user: {}", state.user_id);

    // Spans are automatically created and nested
    let result = self.process_with_llm(content);

    info!("Message processed successfully");
    result
}
```

**Context Propagation**:

```rust
use composable_rust_agent_patterns::context_propagation::*;

// Extract trace context from HTTP headers
let span_context = extract_trace_context(&headers);

// Create span with parent context
let span = tracing::info_span!(
    "handle_request",
    trace_id = %span_context.trace_id,
    span_id = %span_context.span_id,
);

// Execute with trace context
async move {
    // All nested spans inherit this context
    process_request().await
}.instrument(span).await
```

### Structured Logging

```rust
use tracing::{info, warn, error};

// Structured fields
info!(
    user_id = %user_id,
    session_id = %session_id,
    message_length = content.len(),
    "Processing user message"
);

// Conditional logging
if prompt_injection_detected {
    warn!(
        user_id = %user_id,
        pattern = "ignore_instructions",
        severity = "high",
        "Prompt injection attempt detected"
    );
}

// Error context
error!(
    error = %e,
    retry_count = attempt,
    "LLM call failed"
);
```

**Log Levels**:

- `ERROR`: Critical failures requiring immediate attention
- `WARN`: Recoverable issues (rate limits, security events)
- `INFO`: Normal operations (requests, state changes)
- `DEBUG`: Detailed execution flow
- `TRACE`: Very detailed debugging

**Configuration**:

```bash
# Production
RUST_LOG=production_agent=info,composable_rust_agent_patterns=info

# Development
RUST_LOG=production_agent=debug,composable_rust_agent_patterns=debug

# Debugging specific module
RUST_LOG=production_agent::reducer=trace
```

### Metrics with Prometheus

**Agent Metrics**:

```rust
use composable_rust_agent_patterns::AgentMetrics;

let metrics = AgentMetrics::new();

// Record tool calls
metrics.record_tool_call("search", true, Duration::from_millis(250));
metrics.record_tool_call("calculator", true, Duration::from_millis(50));

// Record errors
metrics.record_error("Rate limit exceeded".to_string());

// Get snapshot
let snapshot = metrics.snapshot();
println!("Success rate: {:.2}%", snapshot.success_rate() * 100.0);
```

**Prometheus Endpoint**:

```rust
use metrics_exporter_prometheus::PrometheusBuilder;

// Install Prometheus recorder
let recorder = PrometheusBuilder::new()
    .install_recorder()
    .expect("Failed to install Prometheus recorder");

// Serve metrics
let metrics_app = Router::new()
    .route("/metrics", get(|| async move {
        recorder.render()
    }));

axum::serve(listener, metrics_app).await?;
```

**Available Metrics**:

```
# Tool call metrics
agent_tool_calls_total{tool="search",status="success"} 1250
agent_tool_calls_total{tool="search",status="failure"} 15
agent_tool_latency_seconds{tool="search"} 0.25

# Circuit breaker metrics
circuit_breaker_state{name="llm",state="closed"} 1
circuit_breaker_requests_total{name="llm"} 5420
circuit_breaker_failures_total{name="llm"} 12

# Rate limiter metrics
rate_limiter_requests_total{name="api"} 10500
rate_limiter_rejected_total{name="api"} 42

# Bulkhead metrics
bulkhead_active_requests{name="tools"} 3
bulkhead_queue_size{name="tools"} 0
bulkhead_rejected_total{name="tools"} 0
```

**Grafana Dashboards**:

See `agent-patterns/GRAFANA_DASHBOARDS.md` for pre-built dashboards.

## Security & Audit

### Audit Logging

**Event Types**:

```rust
use composable_rust_agent_patterns::audit::*;

pub enum AuditEventType {
    Authentication,     // Login, logout
    Authorization,      // Permission checks
    DataAccess,        // Data read/write
    Configuration,     // Settings changes
    Security,          // Security incidents
    LlmInteraction,    // LLM calls
}
```

**Creating Audit Events**:

```rust
let audit_logger = Arc::new(InMemoryAuditLogger::new());

// Log authentication
let event = AuditEvent::new(
    AuditEventType::Authentication,
    user_id,
    "login",
    true,
)
.with_session_id(session_id)
.with_source_ip(request_ip)
.with_metadata("method", "password");

audit_logger.log(event).await?;

// Log LLM interaction
let event = AuditEvent::new(
    AuditEventType::LlmInteraction,
    user_id,
    "send_message",
    true,
)
.with_resource("conversation:123")
.with_metadata("message_length", "142")
.with_metadata("model", "claude-sonnet-4");

audit_logger.log(event).await?;
```

**Querying Audit Logs**:

```rust
// Get events for user
let events = audit_logger.get_events_for_actor(user_id).await?;

// Get recent events
let recent = audit_logger.get_recent_events(100).await?;

// Get events by type
let llm_events = audit_logger
    .get_events_by_type(AuditEventType::LlmInteraction)
    .await?;

// Count events
let total = audit_logger.count().await;
```

### Security Monitoring

**Incident Types**:

```rust
use composable_rust_agent_patterns::security::*;

pub enum IncidentType {
    BruteForceAttack,
    AnomalousAccess,
    PrivilegeEscalation,
    DataExfiltration,
    PromptInjection,
    RateLimitAbuse,
    UnauthorizedAccess,
    ConfigurationTampering,
    CredentialStuffing,
    SessionHijacking,
}
```

**Reporting Incidents**:

```rust
let security_monitor = Arc::new(SecurityMonitor::new());

// Report prompt injection
let incident = SecurityIncident::new(
    IncidentType::PromptInjection,
    ThreatLevel::High,
    user_id,
    "Pattern match: ignore previous instructions",
);

security_monitor.report_incident(incident).await?;

// Report brute force
let incident = SecurityIncident::new(
    IncidentType::BruteForceAttack,
    ThreatLevel::Critical,
    source_ip,
    "5 failed login attempts in 60 seconds",
);

security_monitor.report_incident(incident).await?;
```

**Security Dashboard**:

```rust
let dashboard = security_monitor.get_dashboard().await?;

println!("Total incidents: {}", dashboard.total_incidents);
println!("Active incidents: {}", dashboard.active_incidents);
println!("Incidents by severity:");
for (level, count) in &dashboard.incidents_by_severity {
    println!("  {:?}: {}", level, count);
}
```

**Prompt Injection Detection**:

```rust
impl AgentReducer {
    fn detect_prompt_injection(&self, content: &str) -> bool {
        let patterns = [
            "ignore previous instructions",
            "disregard all",
            "new instructions",
            "system:",
            "admin:",
            "sudo",
            "<!--",
            "<script>",
        ];

        let content_lower = content.to_lowercase();
        patterns.iter().any(|p| content_lower.contains(p))
    }
}
```

## Deployment

### Docker

**Dockerfile**:

```dockerfile
FROM rust:1.85 as builder

WORKDIR /app
COPY . .
RUN cargo build --release -p production-agent

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/production-agent /usr/local/bin/

EXPOSE 8080 9090

CMD ["production-agent"]
```

**Build & Run**:

```bash
docker build -t production-agent:latest .
docker run -p 8080:8080 -p 9090:9090 production-agent:latest
```

### Kubernetes

**Deployment**:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: production-agent
  labels:
    app: production-agent
spec:
  replicas: 3
  selector:
    matchLabels:
      app: production-agent
  template:
    metadata:
      labels:
        app: production-agent
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/port: "9090"
        prometheus.io/path: "/metrics"
    spec:
      containers:
      - name: agent
        image: production-agent:latest
        ports:
        - containerPort: 8080
          name: http
          protocol: TCP
        - containerPort: 9090
          name: metrics
          protocol: TCP

        env:
        - name: RUST_LOG
          value: "production_agent=info"
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

        startupProbe:
          httpGet:
            path: /health
            port: 8080
          initialDelaySeconds: 0
          periodSeconds: 10
          failureThreshold: 30

        livenessProbe:
          httpGet:
            path: /health/live
            port: 8080
          initialDelaySeconds: 10
          periodSeconds: 10
          failureThreshold: 3

        readinessProbe:
          httpGet:
            path: /health/ready
            port: 8080
          initialDelaySeconds: 5
          periodSeconds: 5
          failureThreshold: 2
```

**Service**:

```yaml
apiVersion: v1
kind: Service
metadata:
  name: production-agent
spec:
  selector:
    app: production-agent
  ports:
  - name: http
    port: 80
    targetPort: 8080
  - name: metrics
    port: 9090
    targetPort: 9090
  type: LoadBalancer
```

**Horizontal Pod Autoscaler**:

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: production-agent-hpa
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: production-agent
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
```

## Operations

### Monitoring

**Key Metrics to Monitor**:

1. **Request Rate**
   - Requests per second
   - Success rate
   - Error rate

2. **Latency**
   - P50, P95, P99 latencies
   - LLM call latency
   - Tool execution latency

3. **Resource Usage**
   - CPU utilization
   - Memory usage
   - Connection pool usage

4. **Circuit Breaker Status**
   - Current state (Closed/Open/HalfOpen)
   - Failure rate
   - Trip count

5. **Rate Limiter**
   - Request rate
   - Rejection rate
   - Token bucket level

**Alerting Rules**:

```yaml
# Prometheus alerts
groups:
- name: agent_alerts
  rules:
  # High error rate
  - alert: HighErrorRate
    expr: |
      rate(agent_requests_failed_total[5m]) /
      rate(agent_requests_total[5m]) > 0.05
    for: 5m
    labels:
      severity: warning
    annotations:
      summary: "High error rate detected"
      description: "Error rate is {{ $value | humanizePercentage }}"

  # Circuit breaker open
  - alert: CircuitBreakerOpen
    expr: circuit_breaker_state{state="open"} == 1
    for: 1m
    labels:
      severity: critical
    annotations:
      summary: "Circuit breaker {{ $labels.name }} is open"

  # High rate limit rejections
  - alert: HighRateLimitRejections
    expr: rate(rate_limiter_rejected_total[5m]) > 10
    for: 5m
    labels:
      severity: warning
    annotations:
      summary: "High rate of rate limit rejections"
```

### Troubleshooting

**Common Issues**:

#### 1. Circuit Breaker Stuck Open

**Symptoms**: All requests failing with `CircuitBreakerOpen` error.

**Diagnosis**:
```bash
# Check circuit breaker metrics
curl http://localhost:9090/metrics | grep circuit_breaker

# Check logs
kubectl logs -l app=production-agent | grep "circuit_breaker"
```

**Resolution**:
```rust
// Force reset (emergency only)
circuit_breaker.force_reset().await;

// Or adjust thresholds
circuit_breaker.update_config(CircuitBreakerConfig {
    failure_threshold: 10,  // Increased from 5
    success_threshold: 2,
    timeout: Duration::from_secs(30),  // Reduced from 60
});
```

#### 2. Rate Limit Exceeded

**Symptoms**: Many requests returning 429 errors.

**Diagnosis**:
```bash
# Check rate limiter metrics
curl http://localhost:9090/metrics | grep rate_limiter

# Check token bucket level
curl http://localhost:8080/debug/rate_limiter
```

**Resolution**:
```rust
// Increase capacity temporarily
rate_limiter.set_capacity(200);  // Doubled
rate_limiter.set_rate(20.0);     // Doubled

// Or add more replicas
kubectl scale deployment production-agent --replicas=6
```

#### 3. Memory Leak

**Symptoms**: Memory usage continuously increasing.

**Diagnosis**:
```bash
# Monitor memory over time
kubectl top pod -l app=production-agent --watch

# Check for conversation accumulation
curl http://localhost:8080/debug/state | jq '.message_count'
```

**Resolution**:
```rust
// Implement message pruning
fn prune_old_messages(&mut self, state: &mut AgentState) {
    const MAX_MESSAGES: usize = 100;
    if state.messages.len() > MAX_MESSAGES {
        state.messages.drain(0..state.messages.len() - MAX_MESSAGES);
    }
}
```

#### 4. Slow Response Times

**Symptoms**: P99 latency > 5 seconds.

**Diagnosis**:
```bash
# Check latency metrics
curl http://localhost:9090/metrics | grep latency

# Check trace for slow spans
# View in Jaeger UI
```

**Resolution**:
- Enable request timeout
- Increase parallelism
- Add caching
- Optimize database queries

### Performance Tuning

**Configuration Guidelines**:

```rust
// Development
CircuitBreakerConfig {
    failure_threshold: 10,        // More lenient
    success_threshold: 2,
    timeout: Duration::from_secs(30),
}

RateLimiterConfig {
    capacity: 50,                 // Lower capacity
    refill_rate: 5.0,            // Slower refill
}

BulkheadConfig {
    max_concurrent: 5,            // Fewer concurrent
    acquire_timeout: Duration::from_secs(10),
}

// Production
CircuitBreakerConfig {
    failure_threshold: 5,         // Fail fast
    success_threshold: 2,
    timeout: Duration::from_secs(60),  // Longer recovery
}

RateLimiterConfig {
    capacity: 200,                // Higher burst
    refill_rate: 20.0,           // Faster refill
}

BulkheadConfig {
    max_concurrent: 100,          // More concurrent
    acquire_timeout: Duration::from_secs(5),  // Shorter wait
}
```

## Best Practices

### 1. Always Use Circuit Breakers

```rust
// ❌ BAD: Direct LLM call
let response = llm_client.call(prompt).await?;

// ✅ GOOD: Circuit breaker + rate limit + timeout
async fn call_llm_with_resilience(&self, prompt: &str) -> Result<String> {
    // Rate limit
    self.rate_limiter.try_acquire(1).await?;

    // Circuit breaker
    self.circuit_breaker.allow_request().await?;

    // Call with timeout
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        llm_client.call(prompt)
    ).await??;

    // Record result
    match &result {
        Ok(_) => self.circuit_breaker.record_success().await,
        Err(_) => self.circuit_breaker.record_failure().await,
    }

    result
}
```

### 2. Instrument Everything

```rust
// ✅ GOOD: Instrumentation at every level
#[tracing::instrument(skip(self))]
pub async fn handle_request(&self, req: Request) -> Response {
    info!("Handling request");

    let user_id = self.authenticate(&req).await?;
    let response = self.process(user_id, &req).await?;

    info!("Request completed successfully");
    response
}

#[tracing::instrument(skip(self))]
async fn authenticate(&self, req: &Request) -> Result<String> {
    debug!("Authenticating request");
    // Auth logic...
}

#[tracing::instrument(skip(self))]
async fn process(&self, user_id: String, req: &Request) -> Result<Response> {
    info!(user_id = %user_id, "Processing request");
    // Processing logic...
}
```

### 3. Log Audit Events for Everything

```rust
// ✅ GOOD: Comprehensive audit trail
async fn execute_admin_action(&self, user_id: &str, action: &str) -> Result<()> {
    // Log attempt
    let event = AuditEvent::new(
        AuditEventType::Authorization,
        user_id,
        action,
        false,  // Not successful yet
    );
    self.audit_logger.log(event).await?;

    // Check permissions
    if !self.is_admin(user_id) {
        return Err(Error::Unauthorized);
    }

    // Execute action
    let result = self.perform_action(action).await;

    // Log result
    let event = AuditEvent::new(
        AuditEventType::Authorization,
        user_id,
        action,
        result.is_ok(),
    )
    .with_metadata("result", result.as_ref().map(|_| "success").unwrap_or("failure"));
    self.audit_logger.log(event).await?;

    result
}
```

### 4. Implement Health Checks Early

```rust
// ✅ GOOD: Health checks for all dependencies
#[async_trait]
impl HealthCheckable for LLMHealthCheck {
    async fn check_health(&self) -> ComponentHealth {
        // Try a minimal request
        match tokio::time::timeout(
            Duration::from_secs(5),
            self.client.ping()
        ).await {
            Ok(Ok(_)) => ComponentHealth::healthy("LLM service reachable"),
            Ok(Err(e)) => ComponentHealth::unhealthy(format!("LLM error: {}", e)),
            Err(_) => ComponentHealth::unhealthy("LLM timeout"),
        }
    }

    fn component_name(&self) -> &str {
        "llm_service"
    }
}
```

### 5. Use Structured Errors

```rust
// ✅ GOOD: Structured error types
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("LLM error: {0}")]
    Llm(String),

    #[error("Tool execution error: {tool}: {error}")]
    Tool { tool: String, error: String },

    #[error("Rate limited")]
    RateLimited,

    #[error("Circuit breaker open")]
    CircuitBreakerOpen,

    #[error("Timeout after {duration:?}")]
    Timeout { duration: Duration },
}

// Convert to HTTP status
impl AgentError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::CircuitBreakerOpen => StatusCode::SERVICE_UNAVAILABLE,
            Self::Timeout { .. } => StatusCode::GATEWAY_TIMEOUT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
```

### 6. Graceful Degradation

```rust
// ✅ GOOD: Fallback when services unavailable
async fn get_response(&self, prompt: &str) -> Result<String> {
    // Try primary LLM
    match self.primary_llm.call(prompt).await {
        Ok(response) => return Ok(response),
        Err(e) if e.is_temporary() => {
            warn!("Primary LLM failed, trying fallback");
        }
        Err(e) => return Err(e),
    }

    // Try fallback LLM
    match self.fallback_llm.call(prompt).await {
        Ok(response) => {
            info!("Used fallback LLM successfully");
            Ok(response)
        }
        Err(e) => {
            error!("Both LLMs failed");
            Err(e)
        }
    }
}
```

### 7. Test Failure Scenarios

```rust
#[tokio::test]
async fn test_circuit_breaker_opens_on_failures() {
    let env = TestEnvironment {
        llm: FailingLLM::new(5),  // Fail first 5 requests
    };

    let mut state = AgentState::new();

    // First 5 requests should fail and trip circuit breaker
    for _ in 0..5 {
        let result = reducer.reduce(&mut state, SendMessage { .. }, &env);
        assert!(result.is_err());
    }

    // Circuit breaker should now be open
    assert_eq!(env.circuit_breaker.state(), CircuitState::Open);

    // 6th request should fail fast (circuit open)
    let result = reducer.reduce(&mut state, SendMessage { .. }, &env);
    assert!(matches!(result, Err(AgentError::CircuitBreakerOpen)));
}
```

## Examples

See `examples/production-agent/` for a complete working example demonstrating all patterns.

## Next Steps

1. **Read**: `examples/production-agent/README.md` for detailed setup
2. **Explore**: Agent patterns documentation in `agent-patterns/`
3. **Deploy**: Follow Kubernetes deployment guide above
4. **Monitor**: Set up Grafana dashboards for your agents
5. **Iterate**: Add custom health checks, metrics, and resilience patterns

## Related Documentation

- [Observability Guide](./observability.md)
- [Production Database Operations](./production-database.md)
- [Event Bus Setup](./event-bus.md)
- [Saga Patterns](./saga-patterns.md)
