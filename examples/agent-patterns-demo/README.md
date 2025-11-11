# Agent Patterns Demo Examples

This directory contains working examples demonstrating all 7 Anthropic agent patterns implemented in `composable-rust-agent-patterns`.

## Patterns Overview

### 1. Prompt Chaining (`src/prompt_chain_demo.rs`)

**Use Case**: Research paper analysis pipeline

Sequential execution where each step uses the result from the previous step.

```bash
cargo run --bin prompt_chain_demo
```

**Key Features**:
- Sequential refinement of output
- Each step builds on previous results
- Context preservation across steps
- Ideal for complex analysis workflows

### 2. Routing (`src/routing_demo.rs`)

**Use Case**: Customer support ticket routing

Classify input and route to specialist handlers.

```bash
cargo run --bin routing_demo
```

**Key Features**:
- Automatic categorization
- Specialized handling per category
- Scalable specialist system
- Improved response quality

### 3. Parallelization (`src/parallelization_demo.rs`)

**Use Case**: Multi-language document translation

Execute multiple tasks concurrently with coordination.

```bash
cargo run --bin parallelization_demo
```

**Key Features**:
- Concurrent execution for speed
- Configurable concurrency limit
- Independent task failures
- Automatic aggregation when complete

### 4. Orchestrator-Workers (`src/orchestrator_demo.rs`)

**Use Case**: Software build pipeline with dependencies

Delegate subtasks to specialized worker agents with dependency management.

```bash
cargo run --bin orchestrator_demo
```

**Key Features**:
- Dependency-aware execution
- Parallel execution where possible
- Specialized workers per task type
- Complex workflow coordination

### 5. Evaluator-Optimizer (`src/evaluator_demo.rs`)

**Use Case**: Iterative code generation with quality improvement

Generate → Evaluate → Refine → Repeat until quality threshold met.

```bash
cargo run --bin evaluator_demo
```

**Key Features**:
- Iterative quality improvement
- Automatic termination on threshold
- Best candidate tracking
- Feedback-driven refinement

### 6. Aggregation (`src/aggregation_demo.rs`)

**Use Case**: Multi-source news aggregation

Combine multiple sources/perspectives into unified output.

```bash
cargo run --bin aggregation_demo
```

**Key Features**:
- Multi-source data gathering
- Parallel queries for speed
- Graceful failure handling
- Unified synthesis of perspectives

### 7. Memory/RAG (`src/memory_demo.rs`)

**Use Case**: Context-aware customer support

Retrieve relevant context from knowledge base before responding.

```bash
cargo run --bin memory_demo
```

**Key Features**:
- Context-aware responses
- Relevant knowledge retrieval
- Similarity-based filtering
- Conversation history learning

## Pattern Selection Guide

Choose the right pattern for your use case:

| Use Case | Recommended Pattern |
|----------|-------------------|
| Multi-step analysis workflow | Prompt Chaining |
| Categorize and delegate | Routing |
| Independent tasks at scale | Parallelization |
| Complex workflows with dependencies | Orchestrator-Workers |
| Iterative quality improvement | Evaluator-Optimizer |
| Multiple data sources | Aggregation |
| Knowledge-grounded responses | Memory/RAG |

## Architecture

All patterns follow composable-rust principles:

- **Pure Reducers**: Business logic as pure functions
- **Effect-Based**: Side effects as values, not executions
- **LLM-Agnostic**: Generic over `AgentEnvironment` trait
- **Production-Ready**: Bounded memory growth, proper error handling
- **Testable**: Full test coverage (63 tests across all patterns)

## Implementation Status

✅ All 7 patterns fully implemented
✅ 63 tests passing
✅ Clippy clean
✅ Production-ready

See `../../agent-patterns/` for the pattern implementations.
