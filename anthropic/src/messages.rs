//! Messages API request and response types

use crate::types::{ContentBlock, Message, Role, StopReason, Tool, Usage};
use serde::{Deserialize, Serialize};

/// Request to create a message
#[derive(Clone, Debug, Serialize)]
pub struct MessagesRequest {
    /// Model to use (e.g., "claude-sonnet-4-5-20250929")
    pub model: String,
    /// Conversation history
    pub messages: Vec<Message>,
    /// Maximum tokens to generate
    pub max_tokens: u32,
    /// System prompt (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// Available tools (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    /// Whether to stream the response
    #[serde(default)]
    pub stream: bool,
}

impl MessagesRequest {
    /// Create a basic request with sensible defaults
    #[must_use]
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            model: "claude-sonnet-4-5-20250929".to_string(),
            messages,
            max_tokens: 4096,
            system: None,
            tools: None,
            stream: false,
        }
    }

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
    pub fn with_system(mut self, system: String) -> Self {
        self.system = Some(system);
        self
    }

    /// Builder: Set tools
    #[must_use]
    pub fn with_tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Builder: Enable streaming
    #[must_use]
    pub const fn with_streaming(mut self) -> Self {
        self.stream = true;
        self
    }
}

/// Response from creating a message
#[derive(Clone, Debug, Deserialize)]
pub struct MessagesResponse {
    /// Unique identifier for this message
    pub id: String,
    /// Model that generated the response
    pub model: String,
    /// Role (always "assistant" for responses)
    pub role: Role,
    /// Content blocks in the response
    pub content: Vec<ContentBlock>,
    /// Why the model stopped generating
    pub stop_reason: StopReason,
    /// Token usage statistics
    pub usage: Usage,
}

/// Streaming event types from the Messages API
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Message started
    MessageStart {
        /// Message metadata
        message: MessageStart,
    },
    /// Content block started
    ContentBlockStart {
        /// Index of this content block
        index: usize,
        /// The content block
        content_block: ContentBlock,
    },
    /// Content block delta (incremental update)
    ContentBlockDelta {
        /// Index of the content block
        index: usize,
        /// The delta
        delta: ContentDelta,
    },
    /// Content block stopped
    ContentBlockStop {
        /// Index of the content block
        index: usize,
    },
    /// Message delta (metadata update)
    MessageDelta {
        /// The delta
        delta: MessageDelta,
    },
    /// Message stopped
    MessageStop,
}

/// Message start metadata
#[derive(Clone, Debug, Deserialize)]
pub struct MessageStart {
    /// Message ID
    pub id: String,
    /// Model used
    pub model: String,
    /// Role (always "assistant")
    pub role: Role,
}

/// Content delta types
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentDelta {
    /// Text delta
    TextDelta {
        /// Incremental text
        text: String,
    },
    /// Input JSON delta (for tool use)
    InputJsonDelta {
        /// Partial JSON string
        partial_json: String,
    },
}

/// Message delta (stop reason update)
#[derive(Clone, Debug, Deserialize)]
pub struct MessageDelta {
    /// Stop reason (when complete)
    pub stop_reason: Option<StopReason>,
    /// Stop sequence that triggered stop (if applicable)
    pub stop_sequence: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_messages_request_builder() {
        let req = MessagesRequest::new(vec![Message::user("Hello")])
            .with_model("claude-3-opus-20240229".to_string())
            .with_max_tokens(1000)
            .with_system("You are helpful".to_string())
            .with_streaming();

        assert_eq!(req.model, "claude-3-opus-20240229");
        assert_eq!(req.max_tokens, 1000);
        assert_eq!(req.system, Some("You are helpful".to_string()));
        assert!(req.stream);
    }

    #[test]
    fn test_messages_request_defaults() {
        let req = MessagesRequest::new(vec![Message::user("Test")]);

        assert_eq!(req.model, "claude-sonnet-4-5-20250929");
        assert_eq!(req.max_tokens, 4096);
        assert_eq!(req.system, None);
        assert_eq!(req.tools, None);
        assert!(!req.stream);
    }
}
