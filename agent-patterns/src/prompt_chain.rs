//! Prompt Chaining Pattern (Phase 8.3)
//!
//! Sequential execution where each step uses the previous step's result.
//! Useful for multi-stage workflows like: research → analyze → summarize.
//!
//! ## Pattern
//!
//! 1. Execute step 1 with initial input
//! 2. Use step 1 result as input to step 2
//! 3. Continue until all steps complete
//! 4. Return final accumulated result
//!
//! ## Example
//!
//! ```ignore
//! let reducer = PromptChainReducer::new(vec![
//!     ChainStep {
//!         name: "research".to_string(),
//!         prompt_template: "Research the topic: {}".to_string(),
//!     },
//!     ChainStep {
//!         name: "analyze".to_string(),
//!         prompt_template: "Analyze this research: {}".to_string(),
//!     },
//! ]);
//! ```

use composable_rust_core::agent::AgentEnvironment;
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use composable_rust_anthropic::{Message, MessagesRequest};
use smallvec::{smallvec, SmallVec};
use std::marker::PhantomData;

/// A step in the prompt chain
#[derive(Clone, Debug)]
pub struct ChainStep {
    /// Step name (for tracking)
    pub name: String,
    /// Prompt template (use {} for substitution)
    pub prompt_template: String,
}

/// State for prompt chain execution
#[derive(Clone, Debug)]
pub struct ChainState {
    /// Current step index
    current_step: usize,
    /// Accumulated result from previous steps
    accumulated_result: String,
    /// Whether chain is completed
    completed: bool,
}

impl ChainState {
    /// Create new chain state
    #[must_use]
    pub const fn new() -> Self {
        Self {
            current_step: 0,
            accumulated_result: String::new(),
            completed: false,
        }
    }

    /// Get current step index
    #[must_use]
    pub const fn current_step(&self) -> usize {
        self.current_step
    }

    /// Get accumulated result
    #[must_use]
    pub fn accumulated_result(&self) -> &str {
        &self.accumulated_result
    }

    /// Check if completed
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        self.completed
    }
}

impl Default for ChainState {
    fn default() -> Self {
        Self::new()
    }
}

/// Actions for prompt chain
#[derive(Clone, Debug)]
pub enum ChainAction {
    /// Start the chain with initial input
    Start {
        /// Initial input
        input: String,
    },
    /// LLM response received for current step
    StepComplete {
        /// Step index
        step: usize,
        /// Result from LLM
        result: String,
    },
    /// Chain completed
    Complete {
        /// Final result
        result: String,
    },
    /// Error occurred
    Error {
        /// Error message
        message: String,
    },
}

/// Prompt chain reducer
pub struct PromptChainReducer<E> {
    /// Steps to execute
    steps: Vec<ChainStep>,
    /// Phantom data for environment type
    _phantom: PhantomData<E>,
}

impl<E> PromptChainReducer<E> {
    /// Create new prompt chain reducer
    #[must_use]
    pub fn new(steps: Vec<ChainStep>) -> Self {
        Self {
            steps,
            _phantom: PhantomData,
        }
    }

    /// Get total number of steps
    #[must_use]
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }
}

impl<E: AgentEnvironment> Reducer for PromptChainReducer<E> {
    type State = ChainState;
    type Action = ChainAction;
    type Environment = E;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            ChainAction::Start { input } => {
                // Start with first step
                if self.steps.is_empty() {
                    return smallvec![Effect::None];
                }

                state.current_step = 0;
                state.accumulated_result = input.clone();
                state.completed = false;

                // Execute first step
                let step = &self.steps[0];
                let prompt = step.prompt_template.replace("{}", &input);

                let _request = MessagesRequest {
                    model: env.config().model.clone(),
                    max_tokens: env.config().max_tokens,
                    messages: vec![Message::user(&prompt)],
                    system: env.config().system_prompt.clone(),
                    tools: None,
                    stream: false,
                };

                // Call LLM and convert response to ChainAction
                smallvec![Effect::Future(Box::pin(async move {
                    // This will be filled in by environment implementation
                    // For now, return None as placeholder
                    None
                }))]
            }

            ChainAction::StepComplete { step, result } => {
                // Verify this is the expected step
                if step != state.current_step {
                    return smallvec![Effect::None];
                }

                // Accumulate result
                state.accumulated_result = result.clone();
                state.current_step += 1;

                // Check if we have more steps
                if state.current_step >= self.steps.len() {
                    // Chain complete
                    state.completed = true;
                    return smallvec![Effect::Future(Box::pin(async move {
                        Some(ChainAction::Complete { result })
                    }))];
                }

                // Execute next step
                let next_step = &self.steps[state.current_step];
                let prompt = next_step.prompt_template.replace("{}", &result);

                let _request = MessagesRequest {
                    model: env.config().model.clone(),
                    max_tokens: env.config().max_tokens,
                    messages: vec![Message::user(&prompt)],
                    system: env.config().system_prompt.clone(),
                    tools: None,
                    stream: false,
                };

                let _step_index = state.current_step;

                // Call LLM for next step
                smallvec![Effect::Future(Box::pin(async move {
                    // Placeholder - environment will implement actual LLM call
                    None
                }))]
            }

            ChainAction::Complete { .. } => {
                // Chain is complete, no more effects
                smallvec![Effect::None]
            }

            ChainAction::Error { .. } => {
                // Error occurred, stop chain
                state.completed = true;
                smallvec![Effect::None]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_core::agent::{AgentAction, AgentConfig};

    // Mock environment for testing
    struct MockEnvironment {
        config: AgentConfig,
    }

    impl AgentEnvironment for MockEnvironment {
        fn tools(&self) -> &[composable_rust_anthropic::Tool] {
            &[]
        }

        fn config(&self) -> &AgentConfig {
            &self.config
        }

        fn call_claude(&self, _request: MessagesRequest) -> Effect<AgentAction> {
            Effect::None
        }

        fn call_claude_streaming(&self, _request: MessagesRequest) -> Effect<AgentAction> {
            Effect::None
        }

        fn execute_tool(
            &self,
            _tool_use_id: String,
            _tool_name: String,
            _tool_input: String,
        ) -> Effect<AgentAction> {
            Effect::None
        }

        fn execute_tool_streaming(
            &self,
            _tool_use_id: String,
            _tool_name: String,
            _tool_input: String,
        ) -> Effect<AgentAction> {
            Effect::None
        }
    }

    #[test]
    fn test_chain_state() {
        let state = ChainState::new();
        assert_eq!(state.current_step(), 0);
        assert_eq!(state.accumulated_result(), "");
        assert!(!state.is_completed());
    }

    #[test]
    fn test_chain_reducer_creation() {
        let steps = vec![
            ChainStep {
                name: "step1".to_string(),
                prompt_template: "Do step 1: {}".to_string(),
            },
            ChainStep {
                name: "step2".to_string(),
                prompt_template: "Do step 2: {}".to_string(),
            },
        ];

        let reducer: PromptChainReducer<MockEnvironment> = PromptChainReducer::new(steps);
        assert_eq!(reducer.step_count(), 2);
    }

    #[test]
    fn test_chain_start() {
        let steps = vec![ChainStep {
            name: "step1".to_string(),
            prompt_template: "Process: {}".to_string(),
        }];

        let reducer: PromptChainReducer<MockEnvironment> = PromptChainReducer::new(steps);
        let mut state = ChainState::new();
        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            ChainAction::Start {
                input: "test input".to_string(),
            },
            &env,
        );

        assert_eq!(state.current_step(), 0);
        assert_eq!(state.accumulated_result(), "test input");
        assert!(!state.is_completed());
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_chain_step_complete() {
        let steps = vec![
            ChainStep {
                name: "step1".to_string(),
                prompt_template: "Step 1: {}".to_string(),
            },
            ChainStep {
                name: "step2".to_string(),
                prompt_template: "Step 2: {}".to_string(),
            },
        ];

        let reducer: PromptChainReducer<MockEnvironment> = PromptChainReducer::new(steps);
        let mut state = ChainState::new();
        state.current_step = 0;
        state.accumulated_result = "initial".to_string();

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            ChainAction::StepComplete {
                step: 0,
                result: "step1 result".to_string(),
            },
            &env,
        );

        assert_eq!(state.current_step(), 1);
        assert_eq!(state.accumulated_result(), "step1 result");
        assert!(!state.is_completed());
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_chain_completion() {
        let steps = vec![ChainStep {
            name: "step1".to_string(),
            prompt_template: "Step: {}".to_string(),
        }];

        let reducer: PromptChainReducer<MockEnvironment> = PromptChainReducer::new(steps);
        let mut state = ChainState::new();
        state.current_step = 0;

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        // Complete the only step
        let effects = reducer.reduce(
            &mut state,
            ChainAction::StepComplete {
                step: 0,
                result: "final result".to_string(),
            },
            &env,
        );

        assert_eq!(state.current_step(), 1);
        assert!(state.is_completed());
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_chain_error() {
        let steps = vec![ChainStep {
            name: "step1".to_string(),
            prompt_template: "Step: {}".to_string(),
        }];

        let reducer: PromptChainReducer<MockEnvironment> = PromptChainReducer::new(steps);
        let mut state = ChainState::new();

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            ChainAction::Error {
                message: "Something failed".to_string(),
            },
            &env,
        );

        assert!(state.is_completed());
        assert_eq!(effects.len(), 1);
    }
}
