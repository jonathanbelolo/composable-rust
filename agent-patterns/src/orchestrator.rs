//! Orchestrator-Workers Pattern (Phase 8.3)
//!
//! Break down complex task into subtasks, delegate to specialized workers,
//! and coordinate results. Useful for multi-step workflows with different
//! specializations: research → analyze → summarize with different experts.
//!
//! ## Pattern
//!
//! 1. Orchestrator breaks down main task into subtasks
//! 2. Assign each subtask to appropriate worker
//! 3. Workers execute subtasks in parallel or sequentially
//! 4. Orchestrator aggregates worker results
//! 5. Return final coordinated result
//!
//! ## Example
//!
//! ```ignore
//! let orchestrator = OrchestratorReducer::new(
//!     WorkerRegistry::new()
//!         .register("researcher", research_worker_fn)
//!         .register("analyzer", analyze_worker_fn)
//! );
//! ```

use composable_rust_core::agent::AgentEnvironment;
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use smallvec::{smallvec, SmallVec};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

/// Worker function type
pub type WorkerFn = Arc<
    dyn Fn(
            String,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, String>> + Send>,
        > + Send
        + Sync,
>;

/// A subtask to be executed by a worker
#[derive(Clone, Debug)]
pub struct Subtask {
    /// Subtask ID
    pub id: String,
    /// Worker type to use
    pub worker_type: String,
    /// Input for worker
    pub input: String,
    /// Dependencies (subtask IDs that must complete first)
    pub dependencies: Vec<String>,
}

/// Worker registry
#[derive(Clone)]
pub struct WorkerRegistry {
    /// Registered workers by type
    workers: HashMap<String, WorkerFn>,
}

impl WorkerRegistry {
    /// Create new worker registry
    #[must_use]
    pub fn new() -> Self {
        Self {
            workers: HashMap::new(),
        }
    }

    /// Register a worker
    #[must_use]
    pub fn register(mut self, worker_type: impl Into<String>, worker: WorkerFn) -> Self {
        self.workers.insert(worker_type.into(), worker);
        self
    }

    /// Get worker by type
    #[must_use]
    pub fn get_worker(&self, worker_type: &str) -> Option<&WorkerFn> {
        self.workers.get(worker_type)
    }

    /// Get number of registered workers
    #[must_use]
    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }
}

impl Default for WorkerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// State for orchestrator
#[derive(Clone, Debug)]
pub struct OrchestratorState {
    /// All subtasks
    subtasks: Vec<Subtask>,
    /// Completed subtask results
    completed: HashMap<String, Result<String, String>>,
    /// Currently executing subtask IDs
    executing: Vec<String>,
    /// Final aggregated result
    result: Option<String>,
    /// Whether orchestration is complete
    completed_flag: bool,
}

impl OrchestratorState {
    /// Create new orchestrator state
    #[must_use]
    pub fn new() -> Self {
        Self {
            subtasks: Vec::new(),
            completed: HashMap::new(),
            executing: Vec::new(),
            result: None,
            completed_flag: false,
        }
    }

    /// Get number of subtasks
    #[must_use]
    pub fn subtask_count(&self) -> usize {
        self.subtasks.len()
    }

    /// Get number of completed subtasks
    #[must_use]
    pub fn completed_count(&self) -> usize {
        self.completed.len()
    }

    /// Check if all subtasks are complete
    #[must_use]
    pub fn all_subtasks_complete(&self) -> bool {
        !self.subtasks.is_empty()
            && self.completed.len() == self.subtasks.len()
            && self.executing.is_empty()
    }

    /// Get result
    #[must_use]
    pub fn result(&self) -> Option<&str> {
        self.result.as_deref()
    }

    /// Check if completed
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        self.completed_flag
    }
}

impl Default for OrchestratorState {
    fn default() -> Self {
        Self::new()
    }
}

/// Actions for orchestrator
#[derive(Clone, Debug)]
pub enum OrchestratorAction {
    /// Plan and decompose main task
    Plan {
        /// Main task description
        task: String,
    },
    /// Planning complete, subtasks identified
    Planned {
        /// Subtasks to execute
        subtasks: Vec<Subtask>,
    },
    /// Subtask completed
    SubtaskComplete {
        /// Subtask ID
        subtask_id: String,
        /// Result from worker
        result: Result<String, String>,
    },
    /// All subtasks complete, aggregate results
    Aggregate,
    /// Orchestration complete
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

/// Orchestrator reducer
pub struct OrchestratorReducer<E> {
    /// Worker registry
    registry: WorkerRegistry,
    /// Phantom data for environment type
    _phantom: PhantomData<E>,
}

impl<E> OrchestratorReducer<E> {
    /// Create new orchestrator reducer
    #[must_use]
    pub fn new(registry: WorkerRegistry) -> Self {
        Self {
            registry,
            _phantom: PhantomData,
        }
    }

    /// Get worker count
    #[must_use]
    pub fn worker_count(&self) -> usize {
        self.registry.worker_count()
    }

    /// Check if subtask dependencies are satisfied
    fn dependencies_satisfied(&self, subtask: &Subtask, completed: &HashMap<String, Result<String, String>>) -> bool {
        subtask.dependencies.iter().all(|dep| completed.contains_key(dep))
    }

    /// Get ready subtasks (dependencies satisfied, not yet executing or completed)
    fn get_ready_subtasks(&self, state: &OrchestratorState) -> Vec<Subtask> {
        state
            .subtasks
            .iter()
            .filter(|st| {
                !state.completed.contains_key(&st.id)
                    && !state.executing.contains(&st.id)
                    && self.dependencies_satisfied(st, &state.completed)
            })
            .cloned()
            .collect()
    }

    /// Aggregate results from all subtasks
    fn aggregate_results(&self, results: &HashMap<String, Result<String, String>>) -> String {
        let mut output = String::from("Orchestrated results:\n");

        for (subtask_id, result) in results {
            match result {
                Ok(data) => {
                    output.push_str(&format!("- Subtask {}: {}\n", subtask_id, data));
                }
                Err(error) => {
                    output.push_str(&format!("- Subtask {} (failed): {}\n", subtask_id, error));
                }
            }
        }

        output
    }
}

impl<E: AgentEnvironment> Reducer for OrchestratorReducer<E> {
    type State = OrchestratorState;
    type Action = OrchestratorAction;
    type Environment = E;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            OrchestratorAction::Plan { task: _ } => {
                // In real implementation, would use LLM to decompose task
                // For now, return placeholder effect
                smallvec![Effect::Future(Box::pin(async {
                    // Placeholder - would call LLM to create subtasks
                    None
                }))]
            }

            OrchestratorAction::Planned { subtasks } => {
                if subtasks.is_empty() {
                    state.completed_flag = true;
                    return smallvec![Effect::None];
                }

                // Initialize state
                state.subtasks = subtasks;
                state.completed.clear();
                state.executing.clear();
                state.result = None;
                state.completed_flag = false;

                // Get ready subtasks (those with satisfied dependencies)
                let ready = self.get_ready_subtasks(state);

                // Create effects for ready subtasks
                let mut effects = SmallVec::new();

                for subtask in ready {
                    let worker = match self.registry.get_worker(&subtask.worker_type) {
                        Some(w) => w.clone(),
                        None => {
                            continue;
                        }
                    };

                    state.executing.push(subtask.id.clone());

                    let subtask_id = subtask.id.clone();
                    let input = subtask.input.clone();

                    effects.push(Effect::Future(Box::pin(async move {
                        let result = worker(input).await;
                        Some(OrchestratorAction::SubtaskComplete { subtask_id, result })
                    })));
                }

                effects
            }

            OrchestratorAction::SubtaskComplete { subtask_id, result } => {
                // Remove from executing
                state.executing.retain(|id| id != &subtask_id);

                // Add to completed
                state.completed.insert(subtask_id, result);

                // Check if all done
                if state.all_subtasks_complete() {
                    return smallvec![Effect::Future(Box::pin(async {
                        Some(OrchestratorAction::Aggregate)
                    }))];
                }

                // Get newly ready subtasks
                let ready = self.get_ready_subtasks(state);

                if ready.is_empty() {
                    return smallvec![Effect::None];
                }

                // Launch newly ready subtasks
                let mut effects = SmallVec::new();

                for subtask in ready {
                    let worker = match self.registry.get_worker(&subtask.worker_type) {
                        Some(w) => w.clone(),
                        None => continue,
                    };

                    state.executing.push(subtask.id.clone());

                    let subtask_id = subtask.id.clone();
                    let input = subtask.input.clone();

                    effects.push(Effect::Future(Box::pin(async move {
                        let result = worker(input).await;
                        Some(OrchestratorAction::SubtaskComplete { subtask_id, result })
                    })));
                }

                effects
            }

            OrchestratorAction::Aggregate => {
                // Aggregate all results
                let aggregated = self.aggregate_results(&state.completed);
                state.result = Some(aggregated.clone());
                state.completed_flag = true;

                smallvec![Effect::Future(Box::pin(async move {
                    Some(OrchestratorAction::Complete { result: aggregated })
                }))]
            }

            OrchestratorAction::Complete { .. } => {
                // Already complete
                smallvec![Effect::None]
            }

            OrchestratorAction::Error { .. } => {
                // Error occurred
                state.completed_flag = true;
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

    fn create_test_registry() -> WorkerRegistry {
        WorkerRegistry::new()
            .register("worker1", Arc::new(|input| {
                Box::pin(async move { Ok(format!("Worker1: {}", input)) })
            }))
            .register("worker2", Arc::new(|input| {
                Box::pin(async move { Ok(format!("Worker2: {}", input)) })
            }))
    }

    #[test]
    fn test_orchestrator_state() {
        let state = OrchestratorState::new();
        assert_eq!(state.subtask_count(), 0);
        assert_eq!(state.completed_count(), 0);
        assert!(!state.all_subtasks_complete());
        assert!(!state.is_completed());
    }

    #[test]
    fn test_worker_registry() {
        let registry = create_test_registry();
        assert_eq!(registry.worker_count(), 2);
        assert!(registry.get_worker("worker1").is_some());
        assert!(registry.get_worker("nonexistent").is_none());
    }

    #[test]
    fn test_orchestrator_creation() {
        let registry = create_test_registry();
        let reducer: OrchestratorReducer<MockEnvironment> = OrchestratorReducer::new(registry);
        assert_eq!(reducer.worker_count(), 2);
    }

    #[test]
    fn test_planned_empty_subtasks() {
        let registry = create_test_registry();
        let reducer: OrchestratorReducer<MockEnvironment> = OrchestratorReducer::new(registry);
        let mut state = OrchestratorState::new();
        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            OrchestratorAction::Planned { subtasks: Vec::new() },
            &env,
        );

        assert!(state.is_completed());
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_planned_with_subtasks() {
        let registry = create_test_registry();
        let reducer: OrchestratorReducer<MockEnvironment> = OrchestratorReducer::new(registry);
        let mut state = OrchestratorState::new();
        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let subtasks = vec![
            Subtask {
                id: "1".to_string(),
                worker_type: "worker1".to_string(),
                input: "task1".to_string(),
                dependencies: Vec::new(),
            },
            Subtask {
                id: "2".to_string(),
                worker_type: "worker2".to_string(),
                input: "task2".to_string(),
                dependencies: Vec::new(),
            },
        ];

        let effects = reducer.reduce(&mut state, OrchestratorAction::Planned { subtasks }, &env);

        assert_eq!(state.subtask_count(), 2);
        assert_eq!(effects.len(), 2); // Both subtasks launched
    }

    #[test]
    fn test_subtask_with_dependencies() {
        let registry = create_test_registry();
        let reducer: OrchestratorReducer<MockEnvironment> = OrchestratorReducer::new(registry);
        let mut state = OrchestratorState::new();
        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let subtasks = vec![
            Subtask {
                id: "1".to_string(),
                worker_type: "worker1".to_string(),
                input: "task1".to_string(),
                dependencies: Vec::new(),
            },
            Subtask {
                id: "2".to_string(),
                worker_type: "worker2".to_string(),
                input: "task2".to_string(),
                dependencies: vec!["1".to_string()], // Depends on task 1
            },
        ];

        let effects = reducer.reduce(&mut state, OrchestratorAction::Planned { subtasks }, &env);

        // Only first subtask should launch (second has dependency)
        assert_eq!(effects.len(), 1);
    }

    #[tokio::test]
    async fn test_worker_execution() {
        let registry = create_test_registry();
        let worker = registry.get_worker("worker1").unwrap();

        let result = worker("test input".to_string()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Worker1: test input");
    }

    #[test]
    fn test_subtask_complete() {
        let registry = create_test_registry();
        let reducer: OrchestratorReducer<MockEnvironment> = OrchestratorReducer::new(registry);
        let mut state = OrchestratorState::new();
        state.subtasks = vec![Subtask {
            id: "1".to_string(),
            worker_type: "worker1".to_string(),
            input: "task1".to_string(),
            dependencies: Vec::new(),
        }];
        state.executing.push("1".to_string());

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            OrchestratorAction::SubtaskComplete {
                subtask_id: "1".to_string(),
                result: Ok("result1".to_string()),
            },
            &env,
        );

        assert_eq!(state.completed_count(), 1);
        assert!(state.all_subtasks_complete());
        assert_eq!(effects.len(), 1); // Aggregate effect
    }

    #[test]
    fn test_aggregate() {
        let registry = create_test_registry();
        let reducer: OrchestratorReducer<MockEnvironment> = OrchestratorReducer::new(registry);
        let mut state = OrchestratorState::new();
        state.completed.insert("1".to_string(), Ok("result1".to_string()));
        state.completed.insert("2".to_string(), Ok("result2".to_string()));

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(&mut state, OrchestratorAction::Aggregate, &env);

        assert!(state.result().is_some());
        assert!(state.is_completed());
        assert_eq!(effects.len(), 1);
    }
}
