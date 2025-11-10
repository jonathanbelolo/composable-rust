//! Core types for Anthropic Claude API

use serde::{Deserialize, Serialize};

/// A message in the conversation
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Message {
    /// Role of the message sender
    pub role: Role,
    /// Content blocks in the message
    pub content: Vec<ContentBlock>,
}

impl Message {
    /// Create a user message with text content
    #[must_use]
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    /// Create an assistant message with text content
    #[must_use]
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    /// Create a tool result message
    #[must_use]
    pub fn tool_result(tool_use_id: String, content: String, is_error: bool) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            }],
        }
    }
}

/// Message role
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// User message
    User,
    /// Assistant message
    Assistant,
}

/// Content block types that can appear in messages
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Text content
    Text {
        /// The text content
        text: String,
    },
    /// Tool use request from Claude
    ToolUse {
        /// Unique identifier for this tool use
        id: String,
        /// Name of the tool to use
        name: String,
        /// Input parameters as JSON
        input: serde_json::Value,
    },
    /// Tool result from tool execution
    ToolResult {
        /// ID of the tool use this is responding to
        tool_use_id: String,
        /// Result content
        content: String,
        /// Whether this is an error result
        #[serde(default)]
        is_error: bool,
    },
}

/// Tool definition following Anthropic's schema
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Tool {
    /// Tool name (used to identify which tool to call)
    pub name: String,
    /// Human-readable description of what the tool does
    pub description: String,
    /// JSON schema for the tool's input parameters
    pub input_schema: serde_json::Value,
}

/// Stop reason for message completion
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Model naturally completed its turn
    EndTurn,
    /// Reached maximum token limit
    MaxTokens,
    /// Hit a stop sequence
    StopSequence,
    /// Model wants to use a tool
    ToolUse,
}

/// Token usage statistics
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    /// Number of input tokens
    pub input_tokens: u32,
    /// Number of output tokens
    pub output_tokens: u32,
}

impl Usage {
    /// Calculate approximate cost in USD based on model pricing
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_anthropic::types::{Usage, CLAUDE_SONNET_4_5_PRICING};
    ///
    /// let usage = Usage {
    ///     input_tokens: 1000,
    ///     output_tokens: 500,
    /// };
    ///
    /// let cost = usage.calculate_cost(&CLAUDE_SONNET_4_5_PRICING);
    /// assert!(cost > 0.0);
    /// ```
    #[must_use]
    pub fn calculate_cost(&self, pricing: &PricingModel) -> f64 {
        let input_cost = f64::from(self.input_tokens) / 1_000_000.0 * pricing.input_cost_per_1m;
        let output_cost = f64::from(self.output_tokens) / 1_000_000.0 * pricing.output_cost_per_1m;
        input_cost + output_cost
    }
}

/// Pricing model for cost calculation
#[derive(Clone, Debug, PartialEq)]
pub struct PricingModel {
    /// Cost per 1 million input tokens (USD)
    pub input_cost_per_1m: f64,
    /// Cost per 1 million output tokens (USD)
    pub output_cost_per_1m: f64,
}

/// Claude Sonnet 4.5 pricing (as of January 2025)
///
/// See: <https://www.anthropic.com/pricing>
pub const CLAUDE_SONNET_4_5_PRICING: PricingModel = PricingModel {
    input_cost_per_1m: 3.0,
    output_cost_per_1m: 15.0,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_user() {
        let msg = Message::user("Hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content.len(), 1);
        assert!(matches!(msg.content[0], ContentBlock::Text { .. }));
    }

    #[test]
    fn test_message_assistant() {
        let msg = Message::assistant("Hi there");
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content.len(), 1);
    }

    #[test]
    fn test_message_tool_result() {
        let msg = Message::tool_result("tool_123".to_string(), "result".to_string(), false);
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content.len(), 1);
        assert!(matches!(
            msg.content[0],
            ContentBlock::ToolResult { .. }
        ));
    }

    #[test]
    fn test_usage_cost_calculation() {
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
        };

        let cost = usage.calculate_cost(&CLAUDE_SONNET_4_5_PRICING);
        // 1M input @ $3 + 1M output @ $15 = $18
        assert!((cost - 18.0).abs() < 0.01);
    }

    #[test]
    #[allow(clippy::unwrap_used)] // Test code
    fn test_role_serialization() {
        let user_json = serde_json::to_string(&Role::User).unwrap();
        assert_eq!(user_json, r#""user""#);

        let assistant_json = serde_json::to_string(&Role::Assistant).unwrap();
        assert_eq!(assistant_json, r#""assistant""#);
    }

    #[test]
    #[allow(clippy::unwrap_used)] // Test code
    fn test_content_block_text_serialization() {
        let block = ContentBlock::Text {
            text: "Hello".to_string(),
        };

        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains(r#""type":"text""#));
        assert!(json.contains(r#""text":"Hello""#));
    }
}
