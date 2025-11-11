//! Type definitions for production agent

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Agent state
#[derive(Debug, Clone)]
pub struct AgentState {
    /// Current conversation ID
    pub conversation_id: Option<String>,
    /// Message history
    pub messages: Vec<Message>,
    /// User context
    pub user_id: Option<String>,
    /// Session ID
    pub session_id: Option<String>,
}

impl Default for AgentState {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentState {
    /// Create new agent state
    #[must_use]
    pub const fn new() -> Self {
        Self {
            conversation_id: None,
            messages: Vec::new(),
            user_id: None,
            session_id: None,
        }
    }
}

/// Message in conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Message role
    pub role: Role,
    /// Message content
    pub content: String,
    /// Timestamp
    pub timestamp: String,
}

/// Message role
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// User message
    User,
    /// Assistant message
    Assistant,
    /// System message
    System,
}

/// Agent actions (commands and events)
#[derive(Debug, Clone)]
pub enum AgentAction {
    /// Start new conversation
    StartConversation {
        /// User ID
        user_id: String,
        /// Session ID
        session_id: String,
    },
    /// Send user message
    SendMessage {
        /// Message content
        content: String,
        /// Source IP
        source_ip: Option<String>,
    },
    /// Process LLM response
    ProcessResponse {
        /// Response text
        response: String,
    },
    /// Execute tool
    ExecuteTool {
        /// Tool name
        tool_name: String,
        /// Tool input
        input: String,
    },
    /// Tool result received
    ToolResult {
        /// Tool name
        tool_name: String,
        /// Tool result
        result: Result<String, String>,
    },
    /// End conversation
    EndConversation,
    /// Security event detected
    SecurityEvent {
        /// Event type
        event_type: String,
        /// Source
        source: String,
    },
}

/// Agent environment
pub trait AgentEnvironment: Send + Sync {
    /// Call LLM
    fn call_llm(
        &self,
        messages: &[Message],
    ) -> impl std::future::Future<Output = Result<String, AgentError>> + Send;

    /// Execute tool
    fn execute_tool(
        &self,
        tool_name: &str,
        input: &str,
    ) -> impl std::future::Future<Output = Result<String, AgentError>> + Send;

    /// Log audit event
    fn log_audit(
        &self,
        event_type: &str,
        actor: &str,
        action: &str,
        success: bool,
    ) -> impl std::future::Future<Output = Result<(), AgentError>> + Send;

    /// Report security incident
    fn report_security_incident(
        &self,
        incident_type: &str,
        source: &str,
        description: &str,
    ) -> impl std::future::Future<Output = Result<(), AgentError>> + Send;
}

/// Agent error
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// LLM error
    #[error("LLM error: {0}")]
    Llm(String),

    /// Tool execution error
    #[error("Tool execution error: {0}")]
    Tool(String),

    /// Invalid input
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// Rate limited
    #[error("Rate limited")]
    RateLimited,

    /// Circuit breaker open
    #[error("Circuit breaker open")]
    CircuitBreakerOpen,

    /// Timeout
    #[error("Timeout")]
    Timeout,

    /// Audit error
    #[error("Audit error: {0}")]
    Audit(String),

    /// Security error
    #[error("Security error: {0}")]
    Security(String),
}

/// Effect type alias
pub type Effect = composable_rust_core::effect::Effect<AgentAction>;

/// Effects alias for SmallVec
pub type Effects = SmallVec<[Effect; 4]>;
