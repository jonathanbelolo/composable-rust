//! Advanced Agent Patterns (Phase 8.3)
//!
//! This crate provides seven sophisticated agent patterns based on Anthropic's
//! guidance for building robust AI agent systems. All patterns are implemented
//! as reducers following composable-rust architecture principles.
//!
//! ## Seven Patterns
//!
//! 1. **Prompt Chaining**: Sequential execution where each step uses previous result
//! 2. **Routing**: Classify input and route to specialist agent/function
//! 3. **Parallelization**: Execute multiple tasks concurrently with coordination
//! 4. **Orchestrator-Workers**: Delegate subtasks to specialized worker agents
//! 5. **Evaluator-Optimizer**: Iterative improvement with evaluation loop
//! 6. **Aggregation**: Combine multiple sources/perspectives into unified output
//! 7. **Memory/RAG**: Retrieve relevant context from vector store before responding
//!
//! ## Architecture
//!
//! All patterns are implemented as:
//! - **Reducers**: Pure functions that return effect descriptions

#![allow(
    clippy::uninlined_format_args,
    clippy::format_push_string,
    clippy::unused_self,
    clippy::assigning_clones,
    clippy::no_effect_underscore_binding,
    clippy::missing_const_for_fn,
    clippy::manual_let_else,
    clippy::single_match_else
)]
//! - **LLM-Agnostic**: Generic over `AgentEnvironment` trait
//! - **Production-Ready**: Bounded memory growth, proper error handling
//! - **Testable**: No side effects in reducer logic
//!
//! ## Example
//!
//! ```ignore
//! use composable_rust_agent_patterns::prompt_chain::{ChainState, ChainAction, PromptChainReducer};
//! use composable_rust_core::reducer::Reducer;
//!
//! let reducer = PromptChainReducer::new(vec![
//!     ChainStep {
//!         name: "analyze".to_string(),
//!         prompt: "Analyze: {}".to_string(),
//!     },
//!     ChainStep {
//!         name: "summarize".to_string(),
//!         prompt: "Summarize: {}".to_string(),
//!     },
//! ]);
//!
//! let mut state = ChainState::new();
//! let effects = reducer.reduce(&mut state, action, &env);
//! ```

pub mod context;
pub mod caching;
pub mod analytics;
pub mod prompt_chain;
pub mod routing;
pub mod parallelization;
pub mod orchestrator;
pub mod evaluator;
pub mod aggregation;
pub mod memory;

// Phase 8.4: Production Hardening
pub mod tracing_support;
pub mod context_propagation;
pub mod http_propagation;
pub mod health;
pub mod shutdown;
pub mod resilience;
pub mod metrics;
pub mod config;
pub mod audit;
pub mod security;

// Re-export commonly used types
pub use context::{ContextManager, ContextWindow};
pub use caching::{ToolResultCache, CachedToolResult};
pub use analytics::{AgentMetrics, MetricsSnapshot};
pub use prompt_chain::{ChainAction, ChainState, ChainStep, PromptChainReducer};
pub use routing::{Route, RouterAction, RouterState, RoutingReducer, SpecialistFn};
pub use parallelization::{ParallelAction, ParallelReducer, ParallelState, Task, TaskResult};
pub use orchestrator::{OrchestratorAction, OrchestratorReducer, OrchestratorState, Subtask, WorkerFn, WorkerRegistry};
pub use evaluator::{Evaluation, EvaluatorAction, EvaluatorConfig, EvaluatorReducer, EvaluatorState};
pub use aggregation::{AggregationAction, AggregationReducer, AggregationState, Source};
pub use memory::{Memory, MemoryAction, MemoryConfig, MemoryReducer, MemoryState};
pub use tracing_support::TracedReducer;
pub use context_propagation::SpanContext;
