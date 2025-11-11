//! Production environment for basic agent
//!
//! This module provides the production implementation of `AgentEnvironment`
//! that calls the real Anthropic Claude API.

use composable_rust_anthropic::AnthropicClient;
use composable_rust_core::{
    agent::{
        AgentAction, AgentConfig, AgentEnvironment, MessagesRequest, Tool, ToolExecutorFn,
    },
    effect::Effect,
};
use std::collections::HashMap;
use std::sync::Arc;

/// Production agent environment that calls the real Anthropic API
#[derive(Clone)]
pub struct ProductionAgentEnvironment {
    /// Claude API client
    client: Arc<AnthropicClient>,
    /// Available tools for this agent
    tools: Vec<Tool>,
    /// Tool executors keyed by tool name
    tool_executors: HashMap<String, ToolExecutorFn>,
    /// Agent configuration
    config: AgentConfig,
}

impl ProductionAgentEnvironment {
    /// Create a new production environment
    ///
    /// # Errors
    ///
    /// Returns an error if the Claude client cannot be created (e.g., missing API key)
    pub fn new(config: AgentConfig) -> Result<Self, composable_rust_anthropic::error::ClaudeError> {
        Ok(Self {
            client: Arc::new(AnthropicClient::from_env()?),
            tools: Vec::new(),
            tool_executors: HashMap::new(),
            config,
        })
    }

    /// Add a tool to this environment
    #[must_use]
    pub fn with_tool(mut self, tool: &Tool, executor: ToolExecutorFn) -> Self {
        self.tools.push(tool.clone());
        self.tool_executors.insert(tool.name.clone(), executor);
        self
    }
}

impl AgentEnvironment for ProductionAgentEnvironment {
    fn tools(&self) -> &[Tool] {
        &self.tools
    }

    fn config(&self) -> &AgentConfig {
        &self.config
    }

    fn call_claude(&self, request: MessagesRequest) -> Effect<AgentAction> {
        let client = self.client.clone();

        Effect::Future(Box::pin(async move {
            match client.messages(request).await {
                Ok(response) => Some(AgentAction::ClaudeResponse {
                    response_id: response.id,
                    content: response.content,
                    stop_reason: response.stop_reason,
                    usage: response.usage,
                }),
                Err(e) => Some(AgentAction::Error {
                    error: format!("Claude API error: {e}"),
                }),
            }
        }))
    }

    fn call_claude_streaming(&self, request: MessagesRequest) -> Effect<AgentAction> {
        let client = self.client.clone();

        // Create the stream of actions from Claude's SSE stream
        let action_stream = async_stream::stream! {
            use futures::StreamExt;
            use composable_rust_anthropic::{StreamEvent, ContentDelta};

            match client.messages_stream(request).await {
                Ok(mut event_stream) => {
                    let mut message_id = String::new();

                    while let Some(result) = event_stream.next().await {
                        match result {
                            Ok(event) => {
                                match event {
                                    StreamEvent::MessageStart { message } => {
                                        message_id = message.id;
                                    }
                                    StreamEvent::ContentBlockDelta {
                                        delta: ContentDelta::TextDelta { text },
                                        ..
                                    } => {
                                        yield AgentAction::StreamChunk { content: text };
                                    }
                                    StreamEvent::MessageDelta { delta } => {
                                        // Stream is completing, we have stop_reason
                                        if let Some(stop_reason) = delta.stop_reason {
                                            yield AgentAction::StreamComplete {
                                                response_id: message_id.clone(),
                                                stop_reason,
                                                usage: composable_rust_anthropic::Usage {
                                                    input_tokens: 0,  // Not available in streaming
                                                    output_tokens: 0,
                                                },
                                            };
                                        }
                                    }
                                    StreamEvent::MessageStop => {
                                        // Stream ended
                                        break;
                                    }
                                    _ => {
                                        // Ignore other events (ContentBlockStart, ContentBlockStop)
                                    }
                                }
                            }
                            Err(e) => {
                                yield AgentAction::Error {
                                    error: format!("Stream error: {e}"),
                                };
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    yield AgentAction::Error {
                        error: format!("Claude streaming error: {e}"),
                    };
                }
            }
        };

        Effect::Stream(Box::pin(action_stream))
    }

    fn execute_tool(
        &self,
        tool_use_id: String,
        tool_name: String,
        tool_input: String,
    ) -> Effect<AgentAction> {
        let executor = self.tool_executors.get(&tool_name).cloned();

        Effect::Future(Box::pin(async move {
            let result = match executor {
                Some(executor) => executor(tool_input).await,
                None => Err(composable_rust_core::agent::ToolError {
                    message: format!("Tool not found: {tool_name}"),
                }),
            };

            Some(AgentAction::ToolResult { tool_use_id, result })
        }))
    }

    fn execute_tool_streaming(
        &self,
        tool_use_id: String,
        tool_name: String,
        tool_input: String,
    ) -> Effect<AgentAction> {
        // For this basic example, tool streaming is not implemented
        // A full implementation would use Effect::Stream to yield ToolChunk
        // actions as the tool executes, followed by ToolComplete
        let executor = self.tool_executors.get(&tool_name).cloned();

        Effect::Future(Box::pin(async move {
            let result = match executor {
                Some(executor) => executor(tool_input).await,
                None => Err(composable_rust_core::agent::ToolError {
                    message: format!("Tool not found: {tool_name}"),
                }),
            };

            // Return complete result directly (not streaming)
            Some(AgentAction::ToolComplete { tool_use_id, result })
        }))
    }
}
