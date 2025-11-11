# Composable Rust Agent Patterns

Production-ready implementation of seven sophisticated agent patterns based on Anthropic's guidance for building robust AI systems.

## Overview

This crate provides **7 agent patterns** + **3 infrastructure modules** that work with any LLM through the `AgentEnvironment` trait. All patterns are implemented as pure reducers following composable-rust architecture principles.

**Status**: ✅ Production-ready (63 tests passing, clippy clean)

## Quick Start

```toml
[dependencies]
composable-rust-agent-patterns = { path = "../agent-patterns" }
composable-rust-core = { path = "../core" }
```

```rust
use composable_rust_agent_patterns::prompt_chain::{PromptChainReducer, ChainStep, ChainAction, ChainState};
use composable_rust_core::reducer::Reducer;

// Define sequential steps
let steps = vec![
    ChainStep {
        name: "analyze".to_string(),
        prompt_template: "Analyze this: {}".to_string(),
    },
    ChainStep {
        name: "summarize".to_string(),
        prompt_template: "Summarize: {}".to_string(),
    },
];

let reducer = PromptChainReducer::new(steps);
let mut state = ChainState::new();

// Execute
let effects = reducer.reduce(&mut state, ChainAction::Start {
    input: "Research paper content...".to_string()
}, &env);
```

## The Seven Patterns

### 1. Prompt Chaining
**When to use**: Multi-step analysis workflows where each step refines the previous output

Sequential execution where output from one step feeds into the next.

**Example**: Research paper pipeline (extract → summarize → critique → synthesize)

```rust
use composable_rust_agent_patterns::prompt_chain::{PromptChainReducer, ChainStep};

let steps = vec![
    ChainStep { name: "extract".into(), prompt_template: "Extract key findings: {}".into() },
    ChainStep { name: "summarize".into(), prompt_template: "Summarize: {}".into() },
];
let reducer = PromptChainReducer::new(steps);
```

**Key Features**:
- Sequential refinement
- Context preservation
- Accumulated results
- Step-by-step progress tracking

**Pattern file**: `src/prompt_chain.rs` (6 tests)

---

### 2. Routing
**When to use**: Different inputs need different specialist handlers

Classify input and route to appropriate specialist agent or function.

**Example**: Customer support tickets (technical → billing → sales → general)

```rust
use composable_rust_agent_patterns::routing::{RoutingReducer, Route};

let routes = vec![
    Route {
        category: "technical".into(),
        description: "Technical support".into(),
        specialist: Arc::new(|input| Box::pin(async move { /* handle tech */ })),
    },
];
let reducer = RoutingReducer::new(routes);
```

**Key Features**:
- Automatic categorization
- Specialized handling
- Extensible routing table
- Category-specific expertise

**Pattern file**: `src/routing.rs` (8 tests)

---

### 3. Parallelization
**When to use**: Independent tasks that can run concurrently

Execute multiple tasks in parallel with configurable concurrency limits.

**Example**: Multi-language translation, batch document processing

```rust
use composable_rust_agent_patterns::parallelization::{ParallelReducer, Task};

let tasks = vec![
    Task { id: "task1".into(), input: "Translate to Spanish...".into() },
    Task { id: "task2".into(), input: "Translate to French...".into() },
];
let reducer = ParallelReducer::new(3); // Max 3 concurrent
```

**Key Features**:
- Concurrent execution
- Configurable concurrency limit
- Independent task failures
- Automatic aggregation

**Pattern file**: `src/parallelization.rs` (7 tests)

---

### 4. Orchestrator-Workers
**When to use**: Complex workflows with task dependencies

Coordinate multiple worker agents with dependency-aware execution.

**Example**: Build pipeline (lint + test → build → deploy)

```rust
use composable_rust_agent_patterns::orchestrator::{OrchestratorReducer, Subtask, WorkerRegistry};

let registry = WorkerRegistry::new()
    .register("lint", lint_worker)
    .register("build", build_worker);

let subtasks = vec![
    Subtask { id: "lint".into(), worker_type: "lint".into(), input: "src/".into(), dependencies: vec![] },
    Subtask { id: "build".into(), worker_type: "build".into(), input: "release".into(), dependencies: vec!["lint".into()] },
];

let reducer = OrchestratorReducer::new(registry);
```

**Key Features**:
- Dependency-aware execution
- Parallel where possible
- Specialized workers
- Complex workflow coordination

**Pattern file**: `src/orchestrator.rs` (9 tests)

---

### 5. Evaluator-Optimizer
**When to use**: Iterative quality improvement tasks

Generate → Evaluate → Refine → Repeat until quality threshold or max iterations.

**Example**: Code generation, writing improvement, design iteration

```rust
use composable_rust_agent_patterns::evaluator::{EvaluatorReducer, EvaluatorConfig};

let config = EvaluatorConfig {
    max_iterations: 5,
    quality_threshold: 0.8,
};
let reducer = EvaluatorReducer::new(config);
```

**Key Features**:
- Iterative improvement
- Quality threshold termination
- Best candidate tracking
- Feedback-driven refinement

**Pattern file**: `src/evaluator.rs` (6 tests)

---

### 6. Aggregation
**When to use**: Combining multiple data sources or perspectives

Query multiple sources in parallel and synthesize unified output.

**Example**: Multi-source news aggregation, market research, consensus building

```rust
use composable_rust_agent_patterns::aggregation::{AggregationReducer, Source};

let sources = vec![
    Source { id: "news".into(), query: "AI developments".into() },
    Source { id: "research".into(), query: "ML papers".into() },
];
let reducer = AggregationReducer::new();
```

**Key Features**:
- Multi-source gathering
- Parallel queries
- Graceful failure handling
- Unified synthesis

**Pattern file**: `src/aggregation.rs` (8 tests)

---

### 7. Memory/RAG
**When to use**: Knowledge-grounded responses with context retrieval

Retrieve relevant context from vector store before generating response.

**Example**: Customer support with history, document QA, conversational memory

```rust
use composable_rust_agent_patterns::memory::{MemoryReducer, MemoryConfig};

let config = MemoryConfig {
    top_k: 5,
    similarity_threshold: 0.7,
};
let reducer = MemoryReducer::new(config);
```

**Key Features**:
- Context-aware responses
- Similarity-based retrieval
- Top-k filtering
- Conversation learning

**Pattern file**: `src/memory.rs` (10 tests)

---

## Infrastructure Modules

### Context Management (`src/context.rs`)
Sliding window context management for long conversations.

```rust
use composable_rust_agent_patterns::context::{ContextWindow, ContextManager};

let mut window = ContextWindow::new(100); // Max 100 messages
window.add_message(message);
let tokens = window.estimate_tokens(); // ~4 chars per token
```

**Features**:
- Sliding window with capacity limit
- Token estimation
- Summary preservation
- Bounded memory growth

**Tests**: 2 passing

---

### Tool Result Caching (`src/caching.rs`)
LRU cache for expensive tool results with TTL.

```rust
use composable_rust_agent_patterns::caching::ToolResultCache;
use std::time::Duration;

let cache = ToolResultCache::new(1000, Duration::from_secs(3600));
cache.put("tool_name", "input", Ok("result".into()));

if let Some(cached) = cache.get("tool_name", "input") {
    // Use cached result
}
```

**Features**:
- LRU eviction policy
- Time-to-live (TTL)
- Cache statistics
- Bounded capacity

**Tests**: 4 passing

---

### Usage Analytics (`src/analytics.rs`)
Track agent usage metrics with bounded memory.

```rust
use composable_rust_agent_patterns::analytics::AgentMetrics;
use std::time::Duration;

let mut metrics = AgentMetrics::new();
metrics.record_tool_call("search", true);
metrics.record_latency(Duration::from_millis(150));

let success_rate = metrics.tool_success_rate("search");
let avg_latency = metrics.average_latency();
```

**Features**:
- Tool usage tracking
- Latency monitoring
- Error tracking
- Circular buffers (bounded growth)

**Tests**: 4 passing

---

## Architecture Principles

All patterns follow these core principles:

### 1. LLM-Agnostic Design
Generic over `AgentEnvironment` trait - works with Claude, GPT, Gemini, or local models:

```rust
pub trait AgentEnvironment: Send + Sync {
    fn call_claude(&self, request: MessagesRequest) -> Effect<AgentAction>;
    fn execute_tool(&self, tool_use_id: String, tool_name: String, tool_input: String) -> Effect<AgentAction>;
    // ...
}
```

### 2. Pure Reducers
Business logic as pure functions, side effects as values:

```rust
fn reduce(
    &self,
    state: &mut State,
    action: Action,
    env: &Environment,
) -> SmallVec<[Effect<Action>; 4]>
```

### 3. Effect-Based Side Effects
Side effects are values, not executions:

```rust
Effect::Future(Box::pin(async move {
    // This returns an effect description, doesn't execute
    Some(MyAction::Complete { result })
}))
```

### 4. Bounded Memory Growth
All collections use fixed-size buffers:
- Circular buffers for latencies/errors
- LRU caches with capacity limits
- Explicit MAX constants

### 5. Production-Ready
- 63 tests passing
- Clippy clean (`-D warnings`)
- Proper error handling
- Comprehensive documentation

---

## Pattern Selection Guide

| Your Use Case | Recommended Pattern |
|--------------|-------------------|
| Multi-step refinement workflow | **Prompt Chaining** |
| Route to specialists by category | **Routing** |
| Independent tasks at scale | **Parallelization** |
| Complex task dependencies | **Orchestrator-Workers** |
| Iterative quality improvement | **Evaluator-Optimizer** |
| Combine multiple sources | **Aggregation** |
| Knowledge-grounded responses | **Memory/RAG** |

## Testing

All patterns have comprehensive test coverage:

```bash
# Run all pattern tests
cargo test -p composable-rust-agent-patterns

# Run specific pattern tests
cargo test -p composable-rust-agent-patterns prompt_chain
cargo test -p composable-rust-agent-patterns routing
```

**Total**: 63 tests across 10 modules

## Examples

See `../examples/agent-patterns-demo/` for complete working examples of all patterns.

```bash
cd examples/agent-patterns-demo
cargo run --bin prompt_chain_demo
cargo run --bin routing_demo
# ... etc
```

## References

- [Anthropic's Building Effective Agents Guide](https://www.anthropic.com/research/building-effective-agents)
- Composable Rust Architecture: `../../docs/concepts.md`
- Agent Environment: `../../core/src/agent.rs`

---

## License

Part of the Composable Rust framework (see repository root for license).
