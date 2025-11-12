//! Aggregation Pattern (Phase 8.3)
//!
//! Combine multiple sources or perspectives into unified output with synthesis.
//! Useful for: multi-source research, consensus building, perspective combination.
//!
//! ## Pattern
//!
//! 1. Gather data from multiple sources/perspectives
//! 2. Collect all responses in parallel
//! 3. Synthesize into coherent unified view
//! 4. Return aggregated result
//!
//! ## Example
//!
//! ```ignore
//! let reducer = AggregationReducer::new(vec![
//!     Source { id: "source1".to_string(), query: "What is X?" },
//!     Source { id: "source2".to_string(), query: "What is X?" },
//! ]);
//! ```

use composable_rust_core::agent::AgentEnvironment;
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use smallvec::{smallvec, SmallVec};
use std::collections::HashMap;
use std::marker::PhantomData;

/// A data source or perspective to query
#[derive(Clone, Debug)]
pub struct Source {
    /// Source ID
    pub id: String,
    /// Query or prompt for this source
    pub query: String,
}

/// State for aggregation
#[derive(Clone, Debug)]
pub struct AggregationState {
    /// All sources
    sources: Vec<Source>,
    /// Pending source IDs
    pending: Vec<String>,
    /// Collected responses
    responses: HashMap<String, Result<String, String>>,
    /// Synthesized result
    result: Option<String>,
    /// Whether aggregation is complete
    completed: bool,
}

impl AggregationState {
    /// Create new aggregation state
    #[must_use]
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            pending: Vec::new(),
            responses: HashMap::new(),
            result: None,
            completed: false,
        }
    }

    /// Get number of sources
    #[must_use]
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Get number of responses collected
    #[must_use]
    pub fn response_count(&self) -> usize {
        self.responses.len()
    }

    /// Check if all responses collected
    #[must_use]
    pub fn all_responses_collected(&self) -> bool {
        !self.sources.is_empty() && self.pending.is_empty() && self.responses.len() == self.sources.len()
    }

    /// Get result
    #[must_use]
    pub fn result(&self) -> Option<&str> {
        self.result.as_deref()
    }

    /// Check if completed
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        self.completed
    }
}

impl Default for AggregationState {
    fn default() -> Self {
        Self::new()
    }
}

/// Actions for aggregation
#[derive(Clone, Debug)]
pub enum AggregationAction {
    /// Start aggregation from sources
    Start {
        /// Sources to query
        sources: Vec<Source>,
    },
    /// Source response received
    SourceResponse {
        /// Source ID
        source_id: String,
        /// Response from source
        response: Result<String, String>,
    },
    /// All responses collected, synthesize
    Synthesize,
    /// Synthesis complete
    Complete {
        /// Final aggregated result
        result: String,
    },
    /// Error occurred
    Error {
        /// Error message
        message: String,
    },
}

/// Aggregation reducer
pub struct AggregationReducer<E> {
    /// Phantom data for environment type
    _phantom: PhantomData<E>,
}

impl<E> AggregationReducer<E> {
    /// Create new aggregation reducer
    #[must_use]
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    /// Synthesize responses into unified output
    fn synthesize_responses(&self, responses: &HashMap<String, Result<String, String>>) -> String {
        let mut output = String::from("Aggregated synthesis:\n\n");

        // Group successful and failed responses
        let mut successful = Vec::new();
        let mut failed = Vec::new();

        for (source_id, response) in responses {
            match response {
                Ok(data) => successful.push((source_id, data)),
                Err(error) => failed.push((source_id, error)),
            }
        }

        // Add successful responses
        if !successful.is_empty() {
            output.push_str("Sources:\n");
            for (source_id, data) in successful {
                output.push_str(&format!("- {}: {}\n", source_id, data));
            }
        }

        // Add failed responses
        if !failed.is_empty() {
            output.push_str("\nFailed sources:\n");
            for (source_id, error) in failed {
                output.push_str(&format!("- {}: {}\n", source_id, error));
            }
        }

        output.push_str("\n[In real implementation, would use LLM to synthesize unified perspective]");

        output
    }
}

impl<E> Default for AggregationReducer<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: AgentEnvironment> Reducer for AggregationReducer<E> {
    type State = AggregationState;
    type Action = AggregationAction;
    type Environment = E;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            AggregationAction::Start { sources } => {
                if sources.is_empty() {
                    state.completed = true;
                    return smallvec![Effect::None];
                }

                // Initialize state
                state.sources = sources.clone();
                state.pending = sources.iter().map(|s| s.id.clone()).collect();
                state.responses.clear();
                state.result = None;
                state.completed = false;

                // Query all sources in parallel
                let mut effects = SmallVec::new();

                for source in sources {
                    let source_id = source.id.clone();
                    let _query = source.query.clone();

                    // In real implementation, would query the source
                    effects.push(Effect::Future(Box::pin(async move {
                        // Placeholder - would query source
                        Some(AggregationAction::SourceResponse {
                            source_id,
                            response: Ok("response".to_string()),
                        })
                    })));
                }

                effects
            }

            AggregationAction::SourceResponse { source_id, response } => {
                // Remove from pending
                state.pending.retain(|id| id != &source_id);

                // Add to responses
                state.responses.insert(source_id, response);

                // Check if all collected
                if state.all_responses_collected() {
                    smallvec![Effect::Future(Box::pin(async {
                        Some(AggregationAction::Synthesize)
                    }))]
                } else {
                    smallvec![Effect::None]
                }
            }

            AggregationAction::Synthesize => {
                // Synthesize all responses
                let synthesized = self.synthesize_responses(&state.responses);
                state.result = Some(synthesized.clone());
                state.completed = true;

                smallvec![Effect::Future(Box::pin(async move {
                    Some(AggregationAction::Complete { result: synthesized })
                }))]
            }

            AggregationAction::Complete { .. } => {
                // Already complete
                smallvec![Effect::None]
            }

            AggregationAction::Error { .. } => {
                // Error occurred
                state.completed = true;
                smallvec![Effect::None]
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // Test code can use unwrap/expect
mod tests {
    use super::*;
    use composable_rust_core::agent::AgentConfig;

    #[cfg(test)]
    use composable_rust_core::agent::AgentAction;

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

        fn call_claude(&self, _request: composable_rust_anthropic::MessagesRequest) -> Effect<AgentAction> {
            Effect::None
        }

        fn call_claude_streaming(&self, _request: composable_rust_anthropic::MessagesRequest) -> Effect<AgentAction> {
            Effect::None
        }

        fn execute_tool(&self, _tool_use_id: String, _tool_name: String, _tool_input: String) -> Effect<AgentAction> {
            Effect::None
        }

        fn execute_tool_streaming(&self, _tool_use_id: String, _tool_name: String, _tool_input: String) -> Effect<AgentAction> {
            Effect::None
        }
    }

    #[test]
    fn test_aggregation_state() {
        let state = AggregationState::new();
        assert_eq!(state.source_count(), 0);
        assert_eq!(state.response_count(), 0);
        assert!(!state.all_responses_collected());
        assert!(!state.is_completed());
    }

    #[test]
    fn test_aggregation_reducer_creation() {
        let reducer: AggregationReducer<MockEnvironment> = AggregationReducer::new();
        let _ = reducer; // Use to avoid warning
    }

    #[test]
    fn test_start_empty_sources() {
        let reducer: AggregationReducer<MockEnvironment> = AggregationReducer::new();
        let mut state = AggregationState::new();
        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(&mut state, AggregationAction::Start { sources: Vec::new() }, &env);

        assert!(state.is_completed());
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_start_with_sources() {
        let reducer: AggregationReducer<MockEnvironment> = AggregationReducer::new();
        let mut state = AggregationState::new();
        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let sources = vec![
            Source {
                id: "source1".to_string(),
                query: "query1".to_string(),
            },
            Source {
                id: "source2".to_string(),
                query: "query2".to_string(),
            },
        ];

        let effects = reducer.reduce(&mut state, AggregationAction::Start { sources }, &env);

        assert_eq!(state.source_count(), 2);
        assert_eq!(effects.len(), 2); // Parallel queries
    }

    #[test]
    fn test_source_response() {
        let reducer: AggregationReducer<MockEnvironment> = AggregationReducer::new();
        let mut state = AggregationState::new();
        state.sources = vec![Source {
            id: "source1".to_string(),
            query: "query1".to_string(),
        }];
        state.pending = vec!["source1".to_string()];

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            AggregationAction::SourceResponse {
                source_id: "source1".to_string(),
                response: Ok("response1".to_string()),
            },
            &env,
        );

        assert_eq!(state.response_count(), 1);
        assert!(state.all_responses_collected());
        assert_eq!(effects.len(), 1); // Synthesize effect
    }

    #[test]
    fn test_source_response_partial() {
        let reducer: AggregationReducer<MockEnvironment> = AggregationReducer::new();
        let mut state = AggregationState::new();
        state.sources = vec![
            Source {
                id: "source1".to_string(),
                query: "query1".to_string(),
            },
            Source {
                id: "source2".to_string(),
                query: "query2".to_string(),
            },
        ];
        state.pending = vec!["source1".to_string(), "source2".to_string()];

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            AggregationAction::SourceResponse {
                source_id: "source1".to_string(),
                response: Ok("response1".to_string()),
            },
            &env,
        );

        assert_eq!(state.response_count(), 1);
        assert!(!state.all_responses_collected());
        assert_eq!(effects.len(), 1); // Effect::None
    }

    #[test]
    fn test_synthesize() {
        let reducer: AggregationReducer<MockEnvironment> = AggregationReducer::new();
        let mut state = AggregationState::new();
        state.responses.insert("source1".to_string(), Ok("data1".to_string()));
        state.responses.insert("source2".to_string(), Ok("data2".to_string()));

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(&mut state, AggregationAction::Synthesize, &env);

        assert!(state.result().is_some());
        assert!(state.is_completed());
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_synthesize_with_errors() {
        let reducer: AggregationReducer<MockEnvironment> = AggregationReducer::new();
        let mut state = AggregationState::new();
        state.responses.insert("source1".to_string(), Ok("data1".to_string()));
        state.responses.insert("source2".to_string(), Err("error".to_string()));

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(&mut state, AggregationAction::Synthesize, &env);

        assert!(state.result().is_some());
        let result = state.result().unwrap();
        assert!(result.contains("source1"));
        assert!(result.contains("Failed sources"));
        assert_eq!(effects.len(), 1);
    }
}
