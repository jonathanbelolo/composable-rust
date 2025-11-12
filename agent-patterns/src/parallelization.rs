//! Parallelization Pattern (Phase 8.3)
//!
//! Execute multiple tasks concurrently with coordination and aggregation.
//! Useful for independent operations: fetch multiple data sources, parallel analysis, etc.
//!
//! ## Pattern
//!
//! 1. Fan out: Launch N parallel tasks
//! 2. Coordinate: Track completion of each task
//! 3. Fan in: Aggregate results once all complete
//! 4. Return combined result
//!
//! ## Example
//!
//! ```ignore
//! let reducer = ParallelReducer::new(3); // max 3 concurrent
//!
//! // Execute parallel tasks
//! reducer.reduce(&mut state, ParallelAction::Execute {
//!     tasks: vec![
//!         Task { id: "1".to_string(), input: "data1".to_string() },
//!         Task { id: "2".to_string(), input: "data2".to_string() },
//!     ],
//! }, &env);
//! ```

use composable_rust_core::agent::AgentEnvironment;
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use smallvec::{smallvec, SmallVec};
use std::collections::HashMap;
use std::marker::PhantomData;

/// A task to execute in parallel
#[derive(Clone, Debug)]
pub struct Task {
    /// Task ID
    pub id: String,
    /// Task input
    pub input: String,
}

/// Task result
pub type TaskResult = Result<String, String>;

/// State for parallel execution
#[derive(Clone, Debug)]
pub struct ParallelState {
    /// Pending task IDs
    pending_tasks: Vec<String>,
    /// Completed task results
    completed_tasks: HashMap<String, TaskResult>,
    /// Combined result (once all complete)
    result: Option<String>,
    /// Whether execution is complete
    completed: bool,
}

impl ParallelState {
    /// Create new parallel state
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending_tasks: Vec::new(),
            completed_tasks: HashMap::new(),
            result: None,
            completed: false,
        }
    }

    /// Get number of pending tasks
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending_tasks.len()
    }

    /// Get number of completed tasks
    #[must_use]
    pub fn completed_count(&self) -> usize {
        self.completed_tasks.len()
    }

    /// Check if all tasks are complete
    #[must_use]
    pub fn all_tasks_complete(&self) -> bool {
        self.pending_tasks.is_empty() && !self.completed_tasks.is_empty()
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

impl Default for ParallelState {
    fn default() -> Self {
        Self::new()
    }
}

/// Actions for parallel execution
#[derive(Clone, Debug)]
pub enum ParallelAction {
    /// Execute tasks in parallel
    Execute {
        /// Tasks to execute
        tasks: Vec<Task>,
    },
    /// Task completed
    TaskComplete {
        /// Task ID
        task_id: String,
        /// Task result
        result: TaskResult,
    },
    /// All tasks complete, aggregate results
    Aggregate,
    /// Aggregation complete
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

/// Parallel execution reducer
pub struct ParallelReducer<E> {
    /// Maximum concurrent tasks
    max_concurrent: usize,
    /// Phantom data for environment type
    _phantom: PhantomData<E>,
}

impl<E> ParallelReducer<E> {
    /// Create new parallel reducer
    #[must_use]
    pub const fn new(max_concurrent: usize) -> Self {
        Self {
            max_concurrent,
            _phantom: PhantomData,
        }
    }

    /// Get max concurrent tasks
    #[must_use]
    pub const fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// Aggregate results from completed tasks
    fn aggregate_results(&self, results: &HashMap<String, TaskResult>) -> String {
        let mut output = String::from("Aggregated results:\n");

        for (task_id, result) in results {
            match result {
                Ok(data) => {
                    output.push_str(&format!("- Task {}: {}\n", task_id, data));
                }
                Err(error) => {
                    output.push_str(&format!("- Task {} (failed): {}\n", task_id, error));
                }
            }
        }

        output
    }
}

impl<E: AgentEnvironment> Reducer for ParallelReducer<E> {
    type State = ParallelState;
    type Action = ParallelAction;
    type Environment = E;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            ParallelAction::Execute { tasks } => {
                if tasks.is_empty() {
                    state.completed = true;
                    return smallvec![Effect::None];
                }

                // Initialize state
                state.pending_tasks = tasks.iter().map(|t| t.id.clone()).collect();
                state.completed_tasks.clear();
                state.result = None;
                state.completed = false;

                // Create effects for each task (respecting max_concurrent)
                let mut effects = SmallVec::new();
                let batch_size = self.max_concurrent.min(tasks.len());

                for task in tasks.iter().take(batch_size) {
                    let task_id = task.id.clone();
                    let _input = task.input.clone();

                    // In real implementation, this would execute the task
                    // For now, create placeholder effect
                    effects.push(Effect::Future(Box::pin(async move {
                        // Placeholder - would execute actual task
                        Some(ParallelAction::TaskComplete {
                            task_id,
                            result: Ok("completed".to_string()),
                        })
                    })));
                }

                effects
            }

            ParallelAction::TaskComplete { task_id, result } => {
                // Remove from pending
                state.pending_tasks.retain(|id| id != &task_id);

                // Add to completed
                state.completed_tasks.insert(task_id, result);

                // Check if all tasks are complete
                if state.all_tasks_complete() {
                    smallvec![Effect::Future(Box::pin(async {
                        Some(ParallelAction::Aggregate)
                    }))]
                } else {
                    smallvec![Effect::None]
                }
            }

            ParallelAction::Aggregate => {
                // Aggregate all results
                let aggregated = self.aggregate_results(&state.completed_tasks);
                state.result = Some(aggregated.clone());
                state.completed = true;

                smallvec![Effect::Future(Box::pin(async move {
                    Some(ParallelAction::Complete { result: aggregated })
                }))]
            }

            ParallelAction::Complete { .. } => {
                // Already complete
                smallvec![Effect::None]
            }

            ParallelAction::Error { .. } => {
                // Error occurred, stop execution
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

        fn call_claude(&self, _request: composable_rust_anthropic::MessagesRequest) -> Effect<AgentAction> {
            Effect::None
        }

        fn call_claude_streaming(&self, _request: composable_rust_anthropic::MessagesRequest) -> Effect<AgentAction> {
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
    fn test_parallel_state() {
        let state = ParallelState::new();
        assert_eq!(state.pending_count(), 0);
        assert_eq!(state.completed_count(), 0);
        assert!(!state.all_tasks_complete());
        assert!(!state.is_completed());
    }

    #[test]
    fn test_parallel_reducer_creation() {
        let reducer: ParallelReducer<MockEnvironment> = ParallelReducer::new(3);
        assert_eq!(reducer.max_concurrent(), 3);
    }

    #[test]
    fn test_execute_empty_tasks() {
        let reducer: ParallelReducer<MockEnvironment> = ParallelReducer::new(3);
        let mut state = ParallelState::new();
        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            ParallelAction::Execute { tasks: Vec::new() },
            &env,
        );

        assert!(state.is_completed());
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_execute_tasks() {
        let reducer: ParallelReducer<MockEnvironment> = ParallelReducer::new(3);
        let mut state = ParallelState::new();
        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let tasks = vec![
            Task {
                id: "1".to_string(),
                input: "data1".to_string(),
            },
            Task {
                id: "2".to_string(),
                input: "data2".to_string(),
            },
        ];

        let effects = reducer.reduce(&mut state, ParallelAction::Execute { tasks }, &env);

        assert_eq!(state.pending_count(), 2);
        assert_eq!(effects.len(), 2); // Respects max_concurrent
    }

    #[test]
    fn test_task_complete() {
        let reducer: ParallelReducer<MockEnvironment> = ParallelReducer::new(3);
        let mut state = ParallelState::new();
        state.pending_tasks = vec!["1".to_string(), "2".to_string()];

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            ParallelAction::TaskComplete {
                task_id: "1".to_string(),
                result: Ok("result1".to_string()),
            },
            &env,
        );

        assert_eq!(state.pending_count(), 1);
        assert_eq!(state.completed_count(), 1);
        assert!(!state.all_tasks_complete());
        assert_eq!(effects.len(), 1); // Effect::None
    }

    #[test]
    fn test_all_tasks_complete() {
        let reducer: ParallelReducer<MockEnvironment> = ParallelReducer::new(3);
        let mut state = ParallelState::new();
        state.pending_tasks = vec!["1".to_string()];
        state.completed_tasks.insert("2".to_string(), Ok("result2".to_string()));

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            ParallelAction::TaskComplete {
                task_id: "1".to_string(),
                result: Ok("result1".to_string()),
            },
            &env,
        );

        assert!(state.all_tasks_complete());
        assert_eq!(effects.len(), 1); // Aggregate effect
    }

    #[test]
    fn test_aggregate() {
        let reducer: ParallelReducer<MockEnvironment> = ParallelReducer::new(3);
        let mut state = ParallelState::new();
        state.completed_tasks.insert("1".to_string(), Ok("result1".to_string()));
        state.completed_tasks.insert("2".to_string(), Ok("result2".to_string()));

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(&mut state, ParallelAction::Aggregate, &env);

        assert!(state.result().is_some());
        assert!(state.is_completed());
        assert_eq!(effects.len(), 1);
    }
}
