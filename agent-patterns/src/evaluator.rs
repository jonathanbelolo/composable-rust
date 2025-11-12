//! Evaluator-Optimizer Pattern (Phase 8.3)
//!
//! Iterative improvement loop: generate candidate → evaluate → refine → repeat.
//! Useful for optimization tasks: code generation, writing improvement, design iteration.
//!
//! ## Pattern
//!
//! 1. Generate initial candidate solution
//! 2. Evaluate quality (score/feedback)
//! 3. If good enough: return; else: generate improved version
//! 4. Repeat until max iterations or quality threshold met
//!
//! ## Example
//!
//! ```ignore
//! let reducer = EvaluatorReducer::new(EvaluatorConfig {
//!     max_iterations: 3,
//!     quality_threshold: 0.8,
//! });
//! ```

use composable_rust_core::agent::AgentEnvironment;
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use smallvec::{smallvec, SmallVec};
use std::marker::PhantomData;

/// Evaluator configuration
#[derive(Clone, Debug)]
pub struct EvaluatorConfig {
    /// Maximum iterations
    pub max_iterations: usize,
    /// Quality threshold (0.0 to 1.0)
    pub quality_threshold: f64,
}

impl Default for EvaluatorConfig {
    fn default() -> Self {
        Self {
            max_iterations: 5,
            quality_threshold: 0.8,
        }
    }
}

/// Evaluation result
#[derive(Clone, Debug)]
pub struct Evaluation {
    /// Quality score (0.0 to 1.0)
    pub score: f64,
    /// Feedback for improvement
    pub feedback: String,
}

/// State for evaluator-optimizer
#[derive(Clone, Debug)]
pub struct EvaluatorState {
    /// Current iteration
    iteration: usize,
    /// Current candidate
    candidate: Option<String>,
    /// Latest evaluation
    evaluation: Option<Evaluation>,
    /// Best candidate so far
    best_candidate: Option<String>,
    /// Best score so far
    best_score: f64,
    /// Whether optimization is complete
    completed: bool,
}

impl EvaluatorState {
    /// Create new evaluator state
    #[must_use]
    pub const fn new() -> Self {
        Self {
            iteration: 0,
            candidate: None,
            evaluation: None,
            best_candidate: None,
            best_score: 0.0,
            completed: false,
        }
    }

    /// Get current iteration
    #[must_use]
    pub const fn iteration(&self) -> usize {
        self.iteration
    }

    /// Get best score
    #[must_use]
    pub const fn best_score(&self) -> f64 {
        self.best_score
    }

    /// Get best candidate
    #[must_use]
    pub fn best_candidate(&self) -> Option<&str> {
        self.best_candidate.as_deref()
    }

    /// Check if completed
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        self.completed
    }
}

impl Default for EvaluatorState {
    fn default() -> Self {
        Self::new()
    }
}

/// Actions for evaluator-optimizer
#[derive(Clone, Debug)]
pub enum EvaluatorAction {
    /// Start optimization
    Start {
        /// Initial prompt/task
        task: String,
    },
    /// Candidate generated
    CandidateGenerated {
        /// Generated candidate
        candidate: String,
    },
    /// Evaluation complete
    Evaluated {
        /// Evaluation result
        evaluation: Evaluation,
    },
    /// Optimization complete
    Complete {
        /// Final best candidate
        candidate: String,
        /// Final score
        score: f64,
    },
    /// Error occurred
    Error {
        /// Error message
        message: String,
    },
}

/// Evaluator-optimizer reducer
pub struct EvaluatorReducer<E> {
    /// Configuration
    config: EvaluatorConfig,
    /// Phantom data for environment type
    _phantom: PhantomData<E>,
}

impl<E> EvaluatorReducer<E> {
    /// Create new evaluator reducer
    #[must_use]
    pub const fn new(config: EvaluatorConfig) -> Self {
        Self {
            config,
            _phantom: PhantomData,
        }
    }

    /// Get configuration
    #[must_use]
    pub const fn config(&self) -> &EvaluatorConfig {
        &self.config
    }
}

impl<E: AgentEnvironment> Reducer for EvaluatorReducer<E> {
    type State = EvaluatorState;
    type Action = EvaluatorAction;
    type Environment = E;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            EvaluatorAction::Start { task: _ } => {
                // Initialize state
                state.iteration = 0;
                state.candidate = None;
                state.evaluation = None;
                state.best_candidate = None;
                state.best_score = 0.0;
                state.completed = false;

                // Generate initial candidate (placeholder)
                smallvec![Effect::Future(Box::pin(async {
                    // Placeholder - would call LLM to generate
                    None
                }))]
            }

            EvaluatorAction::CandidateGenerated { candidate } => {
                state.candidate = Some(candidate.clone());

                // Evaluate candidate (placeholder)
                smallvec![Effect::Future(Box::pin(async {
                    // Placeholder - would call evaluator
                    None
                }))]
            }

            EvaluatorAction::Evaluated { evaluation } => {
                state.evaluation = Some(evaluation.clone());

                // Update best if improved
                if evaluation.score > state.best_score {
                    state.best_score = evaluation.score;
                    state.best_candidate = state.candidate.clone();
                }

                // Check termination conditions
                if evaluation.score >= self.config.quality_threshold {
                    // Quality threshold met
                    state.completed = true;
                    let best = state.best_candidate.clone().unwrap_or_default();
                    let score = state.best_score;
                    return smallvec![Effect::Future(Box::pin(async move {
                        Some(EvaluatorAction::Complete {
                            candidate: best,
                            score,
                        })
                    }))];
                }

                state.iteration += 1;

                if state.iteration >= self.config.max_iterations {
                    // Max iterations reached
                    state.completed = true;
                    let best = state.best_candidate.clone().unwrap_or_default();
                    let score = state.best_score;
                    return smallvec![Effect::Future(Box::pin(async move {
                        Some(EvaluatorAction::Complete {
                            candidate: best,
                            score,
                        })
                    }))];
                }

                // Generate improved candidate (placeholder)
                let _feedback = evaluation.feedback.clone();
                smallvec![Effect::Future(Box::pin(async {
                    // Placeholder - would call LLM with feedback
                    None
                }))]
            }

            EvaluatorAction::Complete { .. } => {
                // Already complete
                smallvec![Effect::None]
            }

            EvaluatorAction::Error { .. } => {
                // Error occurred
                state.completed = true;
                smallvec![Effect::None]
            }
        }
    }
}

#[cfg(test)]
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
    #[allow(clippy::float_cmp)] // Test uses exact float comparison
    fn test_evaluator_state() {
        let state = EvaluatorState::new();
        assert_eq!(state.iteration(), 0);
        assert_eq!(state.best_score(), 0.0);
        assert!(state.best_candidate().is_none());
        assert!(!state.is_completed());
    }

    #[test]
    fn test_evaluator_config() {
        let config = EvaluatorConfig::default();
        assert_eq!(config.max_iterations, 5);
        assert!((config.quality_threshold - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_evaluator_creation() {
        let config = EvaluatorConfig::default();
        let reducer: EvaluatorReducer<MockEnvironment> = EvaluatorReducer::new(config);
        assert_eq!(reducer.config().max_iterations, 5);
    }

    #[test]
    #[allow(clippy::float_cmp)] // Test uses exact float comparison
    fn test_evaluated_quality_threshold_met() {
        let config = EvaluatorConfig {
            max_iterations: 5,
            quality_threshold: 0.8,
        };
        let reducer: EvaluatorReducer<MockEnvironment> = EvaluatorReducer::new(config);
        let mut state = EvaluatorState::new();
        state.candidate = Some("candidate1".to_string());

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let evaluation = Evaluation {
            score: 0.9,
            feedback: "Good".to_string(),
        };

        let effects = reducer.reduce(&mut state, EvaluatorAction::Evaluated { evaluation }, &env);

        assert!(state.is_completed());
        assert_eq!(state.best_score(), 0.9);
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_evaluated_max_iterations() {
        let config = EvaluatorConfig {
            max_iterations: 2,
            quality_threshold: 0.9,
        };
        let reducer: EvaluatorReducer<MockEnvironment> = EvaluatorReducer::new(config);
        let mut state = EvaluatorState::new();
        state.iteration = 1; // At max-1
        state.candidate = Some("candidate1".to_string());

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let evaluation = Evaluation {
            score: 0.5,
            feedback: "Needs improvement".to_string(),
        };

        let effects = reducer.reduce(&mut state, EvaluatorAction::Evaluated { evaluation }, &env);

        assert!(state.is_completed());
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_evaluated_continue_iterating() {
        let config = EvaluatorConfig {
            max_iterations: 5,
            quality_threshold: 0.9,
        };
        let reducer: EvaluatorReducer<MockEnvironment> = EvaluatorReducer::new(config);
        let mut state = EvaluatorState::new();
        state.candidate = Some("candidate1".to_string());

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let evaluation = Evaluation {
            score: 0.5,
            feedback: "Needs improvement".to_string(),
        };

        let effects = reducer.reduce(&mut state, EvaluatorAction::Evaluated { evaluation }, &env);

        assert!(!state.is_completed());
        assert_eq!(state.iteration(), 1);
        assert_eq!(effects.len(), 1); // Generate improved candidate
    }
}
