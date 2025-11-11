# Agent Tracing Guide

**Complete guide to distributed tracing for AI agent systems**

This guide covers how to instrument, configure, and debug agent systems using OpenTelemetry distributed tracing.

---

## Table of Contents

1. [Quick Start](#quick-start)
2. [Core Concepts](#core-concepts)
3. [Instrumenting Agents](#instrumenting-agents)
4. [Span Attributes Reference](#span-attributes-reference)
5. [HTTP Trace Propagation](#http-trace-propagation)
6. [Viewing Traces](#viewing-traces)
7. [Production Configuration](#production-configuration)
8. [Best Practices](#best-practices)
9. [Troubleshooting](#troubleshooting)

---

## Quick Start

### 1. Initialize Tracing at Application Startup

```rust
use composable_rust_agent_patterns::tracing_support;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing with Jaeger exporter
    tracing_support::init_tracing(
        "agent-server",        // Service name
        "localhost:6831"       // Jaeger agent endpoint
    )?;

    // ... rest of application
    Ok(())
}
```

### 2. Wrap Your Reducer with TracedReducer

```rust
use composable_rust_agent_patterns::TracedReducer;
use composable_rust_runtime::Store;

let base_reducer = MyAgentReducer::new();
let traced_reducer = TracedReducer::new(base_reducer, "my-agent".to_string());

let store = Store::new(
    initial_state,
    traced_reducer,
    env,
);
```

### 3. Start Jaeger (Local Development)

```bash
docker run -d \
  --name jaeger \
  -p 6831:6831/udp \
  -p 16686:16686 \
  jaegertracing/all-in-one:latest
```

### 4. View Traces

Open Jaeger UI: http://localhost:16686

Search for your service name ("agent-server") to see all traces.

---

## Core Concepts

### What is Distributed Tracing?

Distributed tracing tracks requests as they flow through multiple services, creating a complete picture of:
- **Where time is spent** (latency breakdown)
- **What operations occurred** (function calls, API requests)
- **How components interact** (service dependencies)

### Key Terminology

- **Trace**: Complete journey of a request through your system
- **Span**: Single operation within a trace (e.g., reducer execution, tool call)
- **Span Context**: Metadata that links spans together (trace ID, span ID, parent ID)
- **Attributes**: Key-value pairs attached to spans for filtering/debugging

### Architecture

```
User Request
    │
    ├─► Span: agent.reduce (TracedReducer)
    │       ├─► Span: tool_execution (execute_tool)
    │       │       └─► Span: http_request (external API)
    │       └─► Span: claude_api_call (call_claude)
    └─► Span: store_persistence (save state)
```

Each span captures:
- **Start/end timestamps** (for duration calculation)
- **Attributes** (metadata like action type, effect count)
- **Parent span ID** (for building the trace tree)

---

## Instrumenting Agents

### Automatic Instrumentation with TracedReducer

The easiest way to add tracing is using `TracedReducer`:

```rust
use composable_rust_agent_patterns::TracedReducer;

let traced_reducer = TracedReducer::new(
    my_reducer,
    "agent-service".to_string()  // Service name for Jaeger
);
```

**What it traces automatically**:
- Reducer execution start/end
- Action type (via Debug trait)
- Effect count produced
- Execution duration

**Generated span attributes**:
```
service.name = "agent-service"
otel.kind = "internal"
agent.action = UserMessage { content: "..." }
agent.effects.count = 3
agent.duration_ms = 142
```

### Manual Span Creation

For custom operations, create spans manually:

```rust
use composable_rust_agent_patterns::SpanContext;

// For agent actions
let span = SpanContext::for_action("user_message");
let _guard = span.enter();
store.dispatch(action).await;

// For tool executions
let span = SpanContext::for_tool("web_search", "tool_123");
let _guard = span.enter();
let result = execute_tool(&tool_input).await;

// For pattern executions
let span = SpanContext::for_pattern("prompt_chain", "chain_456");
let _guard = span.enter();
let chain_result = chain_reducer.reduce(&mut state, action, &env);
```

### Adding Custom Attributes

```rust
use tracing::{info_span, Span};

let span = info_span!(
    "custom_operation",
    user_id = %user.id,
    operation_type = "analysis",
    complexity = "high"
);
let _guard = span.enter();

// Later, add more attributes
Span::current().record("result_count", results.len());
```

### Nested Spans

Spans automatically nest when entered within another span:

```rust
let outer_span = SpanContext::for_action("process_request");
let _outer_guard = outer_span.enter();

// Inner span becomes child of outer span
let inner_span = SpanContext::for_tool("summarize", "tool_789");
let _inner_guard = inner_span.enter();

// Both spans are active, inner is current
```

---

## Span Attributes Reference

### Standard Attributes (TracedReducer)

| Attribute | Type | Description |
|-----------|------|-------------|
| `service.name` | string | Agent service name |
| `otel.kind` | string | Always "internal" for reducers |
| `agent.action` | string | Debug representation of action |
| `agent.effects.count` | i64 | Number of effects produced |
| `agent.duration_ms` | i64 | Execution time in milliseconds |

### Tool Execution Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `tool_name` | string | Name of tool being executed |
| `tool_use_id` | string | Unique ID for this tool execution |
| `tool.input_size` | i64 | Size of input in bytes |
| `tool.output_size` | i64 | Size of output in bytes |
| `tool.status` | string | "success", "failure", "timeout" |

### Pattern Execution Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `pattern_type` | string | "prompt_chain", "routing", "orchestrator", etc. |
| `pattern_id` | string | Unique ID for this pattern execution |
| `pattern.step` | string | Current step in multi-step patterns |
| `pattern.iteration` | i64 | Iteration number for loops |

### HTTP Request Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `http.method` | string | GET, POST, PUT, DELETE |
| `http.url` | string | Full URL (sanitize sensitive data!) |
| `http.status_code` | i64 | HTTP response status |
| `http.request_size` | i64 | Request body size |
| `http.response_size` | i64 | Response body size |

### Best Practices for Attributes

✅ **DO**:
- Use consistent naming (lowercase with dots)
- Include IDs for correlation (user_id, session_id)
- Add business context (subscription_tier, feature_flags)
- Use standard OpenTelemetry semantic conventions

❌ **DON'T**:
- Include PII without sanitization
- Use high-cardinality values (full URLs, timestamps)
- Add attributes inside tight loops
- Store large payloads (>1KB)

---

## HTTP Trace Propagation

### Outgoing Requests (Client)

Inject trace context into HTTP headers:

```rust
use composable_rust_agent_patterns::http_propagation::inject_trace_headers;
use tracing::Span;

let span = Span::current();
let headers = inject_trace_headers(&span);

let response = reqwest::Client::new()
    .get("https://api.example.com/data")
    .headers(headers.into_iter().collect())
    .send()
    .await?;
```

### Incoming Requests (Server)

Extract trace context from headers:

```rust
use composable_rust_agent_patterns::http_propagation::{extract_trace_context, create_child_span};
use axum::{http::HeaderMap, response::IntoResponse};

async fn handle_request(headers: HeaderMap) -> impl IntoResponse {
    // Convert HeaderMap to HashMap
    let header_map: HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str().ok().map(|v| (k.to_string(), v.to_string()))
        })
        .collect();

    // Extract trace context
    if let Some(context) = extract_trace_context(&header_map) {
        let span = create_child_span(context, "handle_request");
        let _guard = span.enter();

        // Process request - span context is now propagated
        process_request().await
    } else {
        // No trace context - create new trace
        let span = info_span!("handle_request");
        let _guard = span.enter();
        process_request().await
    }
}
```

### Helper Function for Traced HTTP Requests

```rust
use composable_rust_agent_patterns::http_propagation::http_request_with_trace;

let response = http_request_with_trace(
    "https://api.example.com/summarize",
    "POST",
    Some(&serde_json::to_string(&request_body)?),
    &Span::current()
).await?;
```

### W3C Trace Context Format

Standard `traceparent` header:
```
00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
│  │                                │                  │
│  │                                │                  └─ Flags (sampled)
│  │                                └──────────────────── Parent span ID
│  └───────────────────────────────────────────────────── Trace ID
└────────────────────────────────────────────────────────── Version
```

---

## Viewing Traces

### Jaeger UI (Local Development)

1. **Start Jaeger**:
   ```bash
   docker run -d --name jaeger \
     -p 6831:6831/udp \
     -p 16686:16686 \
     jaegertracing/all-in-one:latest
   ```

2. **Open UI**: http://localhost:16686

3. **Search for traces**:
   - Select service: "agent-server"
   - Set time range: Last 15 minutes
   - Add filters: operation, tags, duration
   - Click "Find Traces"

4. **Analyze trace**:
   - Click trace to see span timeline
   - Expand spans to see children
   - View span attributes (tags)
   - Check logs attached to spans

### Trace Analysis Workflow

#### Finding Slow Operations

1. Filter by minimum duration: `minDuration=1s`
2. Sort by duration (longest first)
3. Expand slowest trace
4. Identify longest span
5. Check span attributes for context

#### Debugging Errors

1. Filter by tag: `error=true`
2. Find recent failing traces
3. Check span attributes for error details
4. Trace back to root cause span
5. View logs/events attached to error span

#### Understanding Dependencies

1. View "System Architecture" tab
2. See service dependency graph
3. Identify bottleneck services
4. Check error rates per service

---

## Production Configuration

### Environment Variables

```bash
# Service identification
export SERVICE_NAME="agent-prod"
export SERVICE_VERSION="1.2.3"
export DEPLOYMENT_ENV="production"

# Jaeger configuration
export JAEGER_AGENT_HOST="jaeger-agent.monitoring.svc.cluster.local"
export JAEGER_AGENT_PORT="6831"

# Sampling (1% in production to reduce overhead)
export OTEL_TRACES_SAMPLER="parentbased_traceidratio"
export OTEL_TRACES_SAMPLER_ARG="0.01"
```

### Sampling Strategies

**Development**: 100% sampling
```rust
tracing_support::init_tracing("agent-dev", "localhost:6831")?;
```

**Production**: Adaptive sampling
- Sample 100% of errors
- Sample 1% of successful requests
- Sample 100% of slow requests (>1s)

### Resource Attributes

Add deployment context:

```rust
use opentelemetry::sdk::Resource;
use opentelemetry::KeyValue;

let resource = Resource::new(vec![
    KeyValue::new("service.name", "agent-prod"),
    KeyValue::new("service.version", "1.2.3"),
    KeyValue::new("deployment.environment", "production"),
    KeyValue::new("service.instance.id", std::env::var("HOSTNAME").unwrap()),
]);
```

### Performance Considerations

| Configuration | Development | Production |
|---------------|-------------|------------|
| Sampling rate | 100% | 1-10% |
| Attribute limit | 128 | 32 |
| Span buffer | 1000 | 5000 |
| Export interval | 1s | 5s |
| Timeout | 5s | 10s |

---

## Best Practices

### 1. Span Naming

✅ **Good span names**:
- `agent.reduce` (operation.subject)
- `tool.execute.web_search` (operation.action.target)
- `http.request.POST./api/chat` (protocol.operation.details)

❌ **Bad span names**:
- `process` (too vague)
- `user_123_message` (includes variable data)
- `handleRequestAndSaveToDatabase` (too long, do multiple)

### 2. Span Granularity

**Too coarse**: Miss important operations
```rust
// ❌ One span for entire request
let span = info_span!("handle_request");
// ... 50 operations hidden
```

**Too fine**: Performance overhead, noisy traces
```rust
// ❌ Span for every iteration
for item in items {
    let span = info_span!("process_item");  // Don't do this!
    // ...
}
```

**Just right**: Key operations with meaningful boundaries
```rust
// ✅ Span for high-level operations
let span = info_span!("process_batch", batch_size = items.len());
let _guard = span.enter();

// Process items without individual spans
for item in items {
    process_item(item);
}
```

### 3. Error Handling

Always mark error spans:

```rust
use tracing::{error, Span};

match risky_operation().await {
    Ok(result) => {
        Span::current().record("status", "success");
        Ok(result)
    }
    Err(e) => {
        Span::current().record("error", true);
        Span::current().record("error.message", e.to_string());
        error!("Operation failed: {}", e);
        Err(e)
    }
}
```

### 4. Context Propagation

Always enter spans before async operations:

```rust
// ✅ Correct - span entered before await
let span = info_span!("async_op");
let _guard = span.enter();
let result = async_function().await;

// ❌ Wrong - span not active during await
let span = info_span!("async_op");
let result = async_function().await;  // Span not active!
let _guard = span.enter();
```

### 5. Sensitive Data

Never log sensitive data in spans:

```rust
// ❌ Bad - includes password
let span = info_span!("auth", username = %username, password = %password);

// ✅ Good - sanitized
let span = info_span!("auth", username = %username, has_password = !password.is_empty());

// ✅ Good - hashed
let span = info_span!("auth", user_id = %hash(username));
```

---

## Troubleshooting

### No traces appearing in Jaeger

**Check 1**: Jaeger is running
```bash
docker ps | grep jaeger
curl http://localhost:16686/api/services
```

**Check 2**: Tracing initialized
```rust
// Should see log: "Tracing initialized for service: agent-server"
tracing_support::init_tracing("agent-server", "localhost:6831")?;
```

**Check 3**: Network connectivity
```bash
telnet localhost 6831  # Should connect
```

**Check 4**: Spans are being created
```rust
// Add debug logging
let span = info_span!("test_span");
let _guard = span.enter();
tracing::info!("Inside span");  // Should see in logs
```

### Traces incomplete (missing spans)

**Cause**: Span not entered before async operations

**Fix**: Always use `_guard = span.enter()` before `.await`:
```rust
let span = info_span!("operation");
let _guard = span.enter();  // Critical!
let result = async_op().await;
```

### High CPU usage from tracing

**Cause**: 100% sampling in production

**Fix**: Reduce sampling rate:
```bash
export OTEL_TRACES_SAMPLER_ARG="0.01"  # 1% sampling
```

### Spans not connected (no parent-child)

**Cause**: Span context not propagated

**Fix**: Ensure spans are nested:
```rust
let parent = info_span!("parent");
let _parent_guard = parent.enter();

// Child is created inside parent
let child = info_span!("child");
let _child_guard = child.enter();
```

### Trace IDs not propagating across services

**Cause**: HTTP headers not injected/extracted

**Fix**: Use trace propagation helpers:
```rust
// Client
let headers = inject_trace_headers(&Span::current());

// Server
if let Some(context) = extract_trace_context(&headers) {
    let span = create_child_span(context, "handler");
    let _guard = span.enter();
}
```

---

## Examples

### Complete Agent with Tracing

```rust
use composable_rust_agent_patterns::{TracedReducer, SpanContext, tracing_support};
use composable_rust_runtime::Store;
use composable_rust_core::agent::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Initialize tracing
    tracing_support::init_tracing("agent-app", "localhost:6831")?;

    // 2. Create traced reducer
    let base_reducer = AgentReducer::new();
    let traced_reducer = TracedReducer::new(base_reducer, "agent-app".into());

    // 3. Create store
    let store = Store::new(
        BasicAgentState::default(),
        traced_reducer,
        create_environment(),
    );

    // 4. Dispatch actions with trace context
    let span = SpanContext::for_action("user_message");
    let _guard = span.enter();

    store.dispatch(AgentAction::UserMessage {
        content: "Hello, Claude!".to_string(),
    }).await;

    // 5. Shutdown tracing on exit
    tracing_support::shutdown_tracing();

    Ok(())
}
```

### Pattern-Specific Tracing

```rust
// Prompt chain with span per step
let span = SpanContext::for_pattern("prompt_chain", "research_pipeline");
let _guard = span.enter();

for (i, step) in steps.iter().enumerate() {
    let step_span = info_span!("chain_step", step_number = i, step_name = %step.name);
    let _step_guard = step_span.enter();

    let result = execute_step(step).await?;
    step_span.record("result_length", result.len());
}
```

---

## Reference

### Useful Commands

```bash
# Start Jaeger (all-in-one)
docker run -d --name jaeger -p 6831:6831/udp -p 16686:16686 jaegertracing/all-in-one:latest

# View Jaeger logs
docker logs jaeger

# Stop Jaeger
docker stop jaeger && docker rm jaeger

# Query Jaeger API
curl "http://localhost:16686/api/traces?service=agent-server&limit=10"
```

### Additional Resources

- [OpenTelemetry Rust Docs](https://docs.rs/opentelemetry/latest/opentelemetry/)
- [Tracing Crate Guide](https://docs.rs/tracing/latest/tracing/)
- [Jaeger Documentation](https://www.jaegertracing.io/docs/)
- [W3C Trace Context Spec](https://www.w3.org/TR/trace-context/)

---

**Last Updated**: 2025-11-10
**Version**: Phase 8.4
**Contact**: See repository for support
