//! Agent types for AI agent systems (Phase 8)
//!
//! This module provides core types and traits for building AI agents using the
//! composable-rust architecture with the Anthropic Claude API.
//!
//! ## Architecture
//!
//! Agents are implemented as reducers with:
//! - **State**: Conversation history, pending tool results, configuration
//! - **Actions**: User messages, Claude responses, tool results, errors
//! - **Environment**: Claude API client, tool executors, configuration
//! - **Effects**: API calls, tool executions (via environment methods)
//!
//! ## Example
//!
//! ```ignore
//! use composable_rust_core::agent::{BasicAgentState, AgentAction, AgentConfig};
//! use composable_rust_core::reducer::Reducer;
//!
//! struct MyAgentReducer;
//!
//! impl<E: AgentEnvironment> Reducer for MyAgentReducer {
//!     type State = BasicAgentState;
//!     type Action = AgentAction;
//!     type Environment = E;
//!
//!     fn reduce(&self, state: &mut Self::State, action: Self::Action, env: &Self::Environment)
//!         -> SmallVec<[Effect<Self::Action>; 4]>
//!     {
//!         // Handle user messages, Claude responses, tool results...
//!     }
//! }
//! ```

use crate::effect::Effect;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export anthropic types for convenience
pub use composable_rust_anthropic::{
    ContentBlock, Message, MessagesRequest, MessagesResponse, Role, StopReason, Tool, Usage,
};

/// Basic agent state for conversational agents
///
/// This state manages:
/// - Conversation message history
/// - Pending tool results (for parallel tool execution)
/// - Agent configuration
#[derive(Clone, Debug)]
pub struct BasicAgentState {
    /// Conversation message history
    pub messages: Vec<Message>,

    /// Pending tool results (for parallel tool execution)
    ///
    /// When Claude requests multiple tools in parallel, we track which results
    /// we're still waiting for. Once all results are received, we continue the
    /// conversation with Claude.
    pub pending_tool_results: HashMap<String, Option<ToolResult>>,

    /// Agent configuration
    pub config: AgentConfig,
}

impl BasicAgentState {
    /// Create new agent state with config
    #[must_use]
    pub fn new(config: AgentConfig) -> Self {
        Self {
            messages: Vec::new(),
            pending_tool_results: HashMap::new(),
            config,
        }
    }

    /// Add message to conversation history
    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
    }

    /// Check if all pending tool results are received
    #[must_use]
    pub fn all_tool_results_received(&self) -> bool {
        self.pending_tool_results.values().all(Option::is_some)
    }
}

/// Agent configuration
#[derive(Clone, Debug)]
pub struct AgentConfig {
    /// Model to use (e.g., "claude-sonnet-4-5-20250929")
    pub model: String,
    /// Maximum tokens to generate
    pub max_tokens: u32,
    /// System prompt (optional)
    pub system_prompt: Option<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-5-20250929".to_string(),
            max_tokens: 4096,
            system_prompt: None,
        }
    }
}

impl AgentConfig {
    /// Builder: Set model
    #[must_use]
    pub fn with_model(mut self, model: String) -> Self {
        self.model = model;
        self
    }

    /// Builder: Set max tokens
    #[must_use]
    pub const fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Builder: Set system prompt
    #[must_use]
    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = Some(prompt);
        self
    }
}

/// Result from tool execution
pub type ToolResult = Result<String, ToolError>;

/// Tool execution errors
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolError {
    /// Error message
    pub message: String,
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ToolError {}

/// Agent actions - unified input type for all agent events
#[derive(Clone, Debug)]
pub enum AgentAction {
    /// User sends a message
    UserMessage {
        /// Message content
        content: String,
    },

    /// Claude responds (non-streaming)
    ClaudeResponse {
        /// Response ID from Claude
        response_id: String,
        /// Content blocks in the response
        content: Vec<ContentBlock>,
        /// Why Claude stopped generating
        stop_reason: StopReason,
        /// Token usage statistics
        usage: Usage,
    },

    /// Streaming chunk received
    StreamChunk {
        /// Incremental content
        content: String,
    },

    /// Stream complete
    StreamComplete {
        /// Response ID
        response_id: String,
        /// Stop reason
        stop_reason: StopReason,
        /// Token usage
        usage: Usage,
    },

    /// Tool result received
    ToolResult {
        /// ID of the tool use this responds to
        tool_use_id: String,
        /// Result from tool execution
        result: ToolResult,
    },

    /// Error occurred
    Error {
        /// Error message
        error: String,
    },
}

/// Agent environment trait
///
/// Environments provide:
/// - Access to available tools
/// - Agent configuration
/// - Methods that return effects (not direct API access)
///
/// **Key pattern**: Environment methods return `Effect` values, not futures.
/// This solves Rust borrowing issues and keeps reducers pure.
pub trait AgentEnvironment: Send + Sync {
    /// Get available tools for this agent
    fn tools(&self) -> &[Tool];

    /// Get agent configuration
    fn config(&self) -> &AgentConfig;

    /// Create effect to call Claude (non-streaming)
    ///
    /// Returns an `Effect::Future` that will yield a `ClaudeResponse` action
    /// when the API call completes.
    fn call_claude(&self, request: MessagesRequest) -> Effect<AgentAction>;

    /// Create effect to call Claude (streaming)
    ///
    /// Returns an `Effect::Stream` that yields `StreamChunk` actions as tokens
    /// arrive, followed by a `StreamComplete` action.
    fn call_claude_streaming(&self, request: MessagesRequest) -> Effect<AgentAction>;

    /// Create effect to execute a tool
    ///
    /// Returns an `Effect::Future` that will yield a `ToolResult` action when
    /// the tool execution completes.
    fn execute_tool(
        &self,
        tool_use_id: String,
        tool_name: String,
        tool_input: String,
    ) -> Effect<AgentAction>;
}

/// Tool executor trait for implementing custom tools
///
/// **Edition 2024**: Uses RPITIT (Return Position Impl Trait In Traits)
pub trait ToolExecutor: Send + Sync {
    /// Execute tool with JSON input string, return result or error
    ///
    /// # Errors
    ///
    /// Returns `ToolError` if the tool execution fails
    fn execute(&self, input: &str) -> impl std::future::Future<Output = ToolResult> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_config_builder() {
        let config = AgentConfig::default()
            .with_model("claude-3-opus-20240229".to_string())
            .with_max_tokens(2000)
            .with_system_prompt("You are helpful".to_string());

        assert_eq!(config.model, "claude-3-opus-20240229");
        assert_eq!(config.max_tokens, 2000);
        assert_eq!(config.system_prompt, Some("You are helpful".to_string()));
    }

    #[test]
    fn test_agent_config_defaults() {
        let config = AgentConfig::default();

        assert_eq!(config.model, "claude-sonnet-4-5-20250929");
        assert_eq!(config.max_tokens, 4096);
        assert_eq!(config.system_prompt, None);
    }

    #[test]
    fn test_basic_agent_state() {
        let config = AgentConfig::default();
        let mut state = BasicAgentState::new(config);

        assert_eq!(state.messages.len(), 0);
        assert!(state.pending_tool_results.is_empty());

        state.add_message(Message::user("Hello"));
        assert_eq!(state.messages.len(), 1);
    }

    #[test]
    fn test_all_tool_results_received() {
        let config = AgentConfig::default();
        let mut state = BasicAgentState::new(config);

        // No pending results
        assert!(state.all_tool_results_received());

        // Add pending result
        state.pending_tool_results.insert("tool_1".to_string(), None);
        assert!(!state.all_tool_results_received());

        // Add result
        state.pending_tool_results.insert("tool_1".to_string(), Some(Ok("result".to_string())));
        assert!(state.all_tool_results_received());
    }

    #[test]
    fn test_tool_error_display() {
        let error = ToolError {
            message: "Tool failed".to_string(),
        };

        assert_eq!(error.to_string(), "Tool failed");
    }
}
