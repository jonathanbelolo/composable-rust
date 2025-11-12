//! # Basic Agent Example
//!
//! Demonstrates a simple conversational agent using the Anthropic Claude API.
//!
//! This example shows:
//! - Basic message flow (user → Claude → response)
//! - Tool use with parallel execution
//! - Collector pattern for gathering tool results
//! - Environment pattern (returns effects, not clients)
//!
//! ## Architecture
//!
//! - **State**: `BasicAgentState` (from core)
//! - **Actions**: `AgentAction` (from core)
//! - **Reducer**: `BasicAgentReducer` (generic over environment)
//! - **Environment**: `ProductionAgentEnvironment` or `MockAgentEnvironment`

pub mod environment;

use composable_rust_core::{
    agent::{
        AgentAction, AgentEnvironment, BasicAgentState, ContentBlock, Message, MessagesRequest,
        Role, StopReason,
    },
    effect::Effect,
    reducer::Reducer,
};
use smallvec::{smallvec, SmallVec};

/// Basic agent reducer
///
/// Handles the core agent loop:
/// 1. User sends message
/// 2. Call Claude
/// 3. Claude responds (possibly with tool use requests)
/// 4. Execute tools in parallel
/// 5. Collect all tool results
/// 6. Continue conversation with Claude
#[derive(Clone)]
pub struct BasicAgentReducer<E> {
    _phantom: std::marker::PhantomData<E>,
}

impl<E> BasicAgentReducer<E> {
    /// Create a new basic agent reducer
    #[must_use]
    pub const fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<E> Default for BasicAgentReducer<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> Reducer for BasicAgentReducer<E>
where
    E: AgentEnvironment,
{
    type State = BasicAgentState;
    type Action = AgentAction;
    type Environment = E;

    #[allow(clippy::too_many_lines)] // Complex agent logic with tool handling
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            AgentAction::UserMessage { content } => {
                // Add user message to history
                state.add_message(Message::user(content));

                // Create request to Claude
                let mut request = MessagesRequest::new(state.messages.clone())
                    .with_model(state.config.model.clone())
                    .with_max_tokens(state.config.max_tokens);

                // Add system prompt if configured
                if let Some(system) = state.config.system_prompt.clone() {
                    request = request.with_system(system);
                }

                // Add tools if available
                let tools = env.tools();
                if !tools.is_empty() {
                    request = request.with_tools(tools.to_vec());
                }

                // Call Claude (non-streaming for this example)
                smallvec![env.call_claude(request)]
            }

            AgentAction::ClaudeResponse {
                content,
                stop_reason,
                ..
            } => {
                // Add assistant message to history
                state.add_message(Message {
                    role: Role::Assistant,
                    content: content.clone(),
                });

                // Check if tool use is requested
                let tool_uses: Vec<_> = content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::ToolUse { id, name, input } => {
                            // Convert JSON Value to string for tool execution
                            let input_str = serde_json::to_string(input).ok()?;
                            Some((id.clone(), name.clone(), input_str))
                        }
                        _ => None,
                    })
                    .collect();

                if !tool_uses.is_empty() {
                    // Initialize pending tool results (collector pattern)
                    state.pending_tool_results = tool_uses
                        .iter()
                        .map(|(id, _, _)| (id.clone(), None))
                        .collect();

                    // Execute tools in parallel
                    let effects: SmallVec<_> = tool_uses
                        .into_iter()
                        .map(|(id, name, input)| env.execute_tool(id, name, input))
                        .collect();

                    effects
                } else if stop_reason == StopReason::EndTurn {
                    // Conversation complete, no tools
                    smallvec![Effect::None]
                } else {
                    // Other stop reasons (max tokens, etc.)
                    smallvec![Effect::None]
                }
            }

            AgentAction::ToolResult { tool_use_id, result } => {
                // Store result in collector
                state
                    .pending_tool_results
                    .insert(tool_use_id.clone(), Some(result.clone()));

                // Check if all results received
                if state.all_tool_results_received() {
                    // Collect tool result messages first (to avoid borrow issues)
                    let tool_messages: Vec<_> = state
                        .pending_tool_results
                        .iter()
                        .filter_map(|(tool_use_id, result_opt)| {
                            let (content, is_error) = match result_opt {
                                Some(Ok(output)) => (output.clone(), false),
                                Some(Err(err)) => (err.message.clone(), true),
                                None => return None,
                            };
                            Some(Message::tool_result(
                                tool_use_id.clone(),
                                content,
                                is_error,
                            ))
                        })
                        .collect();

                    // Add all tool results to message history
                    for message in tool_messages {
                        state.add_message(message);
                    }

                    // Clear pending results
                    state.pending_tool_results.clear();

                    // Continue conversation with Claude
                    let mut request = MessagesRequest::new(state.messages.clone())
                        .with_model(state.config.model.clone())
                        .with_max_tokens(state.config.max_tokens);

                    if let Some(system) = state.config.system_prompt.clone() {
                        request = request.with_system(system);
                    }

                    let tools = env.tools();
                    if !tools.is_empty() {
                        request = request.with_tools(tools.to_vec());
                    }

                    smallvec![env.call_claude(request)]
                } else {
                    // Still waiting for more results
                    smallvec![Effect::None]
                }
            }

            AgentAction::StreamChunk { .. } | AgentAction::StreamComplete { .. } => {
                // Streaming not used in this basic example
                // (See streaming-agent example for streaming implementation)
                smallvec![Effect::None]
            }

            AgentAction::ToolChunk { .. } | AgentAction::ToolComplete { .. } => {
                // Tool streaming not used in this basic example
                // (Tool streaming is for long-running tools with progress updates)
                smallvec![Effect::None]
            }

            AgentAction::Error { error } => {
                // Log error, but don't crash
                eprintln!("Agent error: {error}");
                smallvec![Effect::None]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_core::agent::{AgentConfig, MessagesResponse, Tool, ToolResult, Usage};

    // Mock environment for testing
    struct MockEnvironment {
        tools: Vec<Tool>,
        claude_response: Option<MessagesResponse>,
        tool_results: std::collections::HashMap<String, ToolResult>,
        config: AgentConfig,
    }

    impl MockEnvironment {
        fn new() -> Self {
            Self {
                tools: Vec::new(),
                claude_response: None,
                tool_results: std::collections::HashMap::new(),
                config: AgentConfig::default(),
            }
        }

        fn with_claude_response(mut self, response: MessagesResponse) -> Self {
            self.claude_response = Some(response);
            self
        }
    }

    impl AgentEnvironment for MockEnvironment {
        fn tools(&self) -> &[Tool] {
            &self.tools
        }

        fn config(&self) -> &AgentConfig {
            &self.config
        }

        fn call_claude(&self, _request: MessagesRequest) -> Effect<AgentAction> {
            if let Some(ref response) = self.claude_response {
                Effect::Future(Box::pin({
                    let response = response.clone();
                    async move {
                        Some(AgentAction::ClaudeResponse {
                            response_id: response.id,
                            content: response.content,
                            stop_reason: response.stop_reason,
                            usage: response.usage,
                        })
                    }
                }))
            } else {
                Effect::None
            }
        }

        fn call_claude_streaming(&self, _request: MessagesRequest) -> Effect<AgentAction> {
            Effect::None
        }

        fn execute_tool(
            &self,
            tool_use_id: String,
            _tool_name: String,
            _tool_input: String,
        ) -> Effect<AgentAction> {
            let result = self
                .tool_results
                .get(&tool_use_id)
                .cloned()
                .unwrap_or_else(|| Ok("mock result".to_string()));

            Effect::Future(Box::pin(async move {
                Some(AgentAction::ToolResult { tool_use_id, result })
            }))
        }

        fn execute_tool_streaming(
            &self,
            _tool_use_id: String,
            _tool_name: String,
            _tool_input: String,
        ) -> Effect<AgentAction> {
            // Not used in basic example
            Effect::None
        }
    }

    #[test]
    fn test_user_message_calls_claude() {
        let reducer = BasicAgentReducer::<MockEnvironment>::new();
        let env = MockEnvironment::new().with_claude_response(MessagesResponse {
            id: "msg_123".to_string(),
            model: "claude-sonnet-4-5-20250929".to_string(),
            role: Role::Assistant,
            content: vec![ContentBlock::Text {
                text: "Hello!".to_string(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 10,
                output_tokens: 5,
            },
        });

        let mut state = BasicAgentState::new(AgentConfig::default());

        let effects = reducer.reduce(
            &mut state,
            AgentAction::UserMessage {
                content: "Hi".to_string(),
            },
            &env,
        );

        assert_eq!(effects.len(), 1);
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].role, Role::User);
    }

    #[test]
    fn test_claude_response_adds_to_history() {
        let reducer = BasicAgentReducer::<MockEnvironment>::new();
        let env = MockEnvironment::new();
        let mut state = BasicAgentState::new(AgentConfig::default());

        let effects = reducer.reduce(
            &mut state,
            AgentAction::ClaudeResponse {
                response_id: "msg_123".to_string(),
                content: vec![ContentBlock::Text {
                    text: "Hello!".to_string(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                },
            },
            &env,
        );

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].role, Role::Assistant);
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::None));
    }

    #[test]
    fn test_tool_use_creates_parallel_effects() {
        let reducer = BasicAgentReducer::<MockEnvironment>::new();
        let env = MockEnvironment::new();
        let mut state = BasicAgentState::new(AgentConfig::default());

        let effects = reducer.reduce(
            &mut state,
            AgentAction::ClaudeResponse {
                response_id: "msg_123".to_string(),
                content: vec![
                    ContentBlock::ToolUse {
                        id: "tool_1".to_string(),
                        name: "get_weather".to_string(),
                        input: serde_json::json!({"location": "NYC"}),
                    },
                    ContentBlock::ToolUse {
                        id: "tool_2".to_string(),
                        name: "get_time".to_string(),
                        input: serde_json::json!({}),
                    },
                ],
                stop_reason: StopReason::ToolUse,
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                },
            },
            &env,
        );

        // Should create 2 tool execution effects
        assert_eq!(effects.len(), 2);
        // Should track 2 pending results
        assert_eq!(state.pending_tool_results.len(), 2);
        assert!(state.pending_tool_results.contains_key("tool_1"));
        assert!(state.pending_tool_results.contains_key("tool_2"));
    }
}
