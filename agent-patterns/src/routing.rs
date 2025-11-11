//! Routing Pattern (Phase 8.3)
//!
//! Classify input and route to specialist agent/function based on category.
//! Useful for multi-domain systems: customer service, technical support, billing, etc.
//!
//! ## Pattern
//!
//! 1. Classify input using LLM or rule-based classifier
//! 2. Route to appropriate specialist based on classification
//! 3. Execute specialist logic
//! 4. Return specialist result
//!
//! ## Example
//!
//! ```ignore
//! let router = Router::new(vec![
//!     Route {
//!         category: "technical".to_string(),
//!         description: "Technical support questions".to_string(),
//!         specialist: technical_specialist_fn,
//!     },
//!     Route {
//!         category: "billing".to_string(),
//!         description: "Billing and payment questions".to_string(),
//!         specialist: billing_specialist_fn,
//!     },
//! ]);
//! ```

use composable_rust_core::agent::AgentEnvironment;
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use composable_rust_anthropic::{Message, MessagesRequest};
use smallvec::{smallvec, SmallVec};
use std::marker::PhantomData;
use std::sync::Arc;

#[cfg(test)]
use composable_rust_core::agent::AgentAction;

/// Specialist function type
pub type SpecialistFn = Arc<
    dyn Fn(
            String,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, String>> + Send>,
        > + Send
        + Sync,
>;

/// A route to a specialist
#[derive(Clone)]
pub struct Route {
    /// Category name (e.g., "technical", "billing")
    pub category: String,
    /// Description of what this specialist handles
    pub description: String,
    /// Specialist function
    pub specialist: SpecialistFn,
}

/// State for routing
#[derive(Clone, Debug)]
pub struct RouterState {
    /// Classified category (if classification complete)
    category: Option<String>,
    /// Final result from specialist
    result: Option<String>,
    /// Whether routing is complete
    completed: bool,
}

impl RouterState {
    /// Create new router state
    #[must_use]
    pub const fn new() -> Self {
        Self {
            category: None,
            result: None,
            completed: false,
        }
    }

    /// Get classified category
    #[must_use]
    pub fn category(&self) -> Option<&str> {
        self.category.as_deref()
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

impl Default for RouterState {
    fn default() -> Self {
        Self::new()
    }
}

/// Actions for routing
#[derive(Clone, Debug)]
pub enum RouterAction {
    /// Classify input to determine route
    Classify {
        /// Input to classify
        input: String,
    },
    /// Classification complete
    Classified {
        /// Detected category
        category: String,
        /// Original input
        input: String,
    },
    /// Specialist processing complete
    SpecialistComplete {
        /// Category that was routed to
        category: String,
        /// Result from specialist
        result: Result<String, String>,
    },
    /// Routing complete
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

/// Routing reducer
pub struct RoutingReducer<E> {
    /// Available routes
    routes: Vec<Route>,
    /// Phantom data for environment type
    _phantom: PhantomData<E>,
}

impl<E> RoutingReducer<E> {
    /// Create new routing reducer
    #[must_use]
    pub fn new(routes: Vec<Route>) -> Self {
        Self {
            routes,
            _phantom: PhantomData,
        }
    }

    /// Get number of routes
    #[must_use]
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    /// Find route by category
    fn find_route(&self, category: &str) -> Option<&Route> {
        self.routes.iter().find(|r| r.category == category)
    }

    /// Build classification prompt
    fn build_classification_prompt(&self, input: &str) -> String {
        let mut prompt = String::from("Classify the following input into one of these categories:\n\n");

        for route in &self.routes {
            prompt.push_str(&format!("- {}: {}\n", route.category, route.description));
        }

        prompt.push_str("\nInput: ");
        prompt.push_str(input);
        prompt.push_str("\n\nRespond with only the category name, nothing else.");

        prompt
    }
}

impl<E: AgentEnvironment> Reducer for RoutingReducer<E> {
    type State = RouterState;
    type Action = RouterAction;
    type Environment = E;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            RouterAction::Classify { input } => {
                // Build classification prompt
                let prompt = self.build_classification_prompt(&input);

                let _request = MessagesRequest {
                    model: env.config().model.clone(),
                    max_tokens: env.config().max_tokens,
                    messages: vec![Message::user(&prompt)],
                    system: env.config().system_prompt.clone(),
                    tools: None,
                    stream: false,
                };

                // In real implementation, this would call LLM
                // For now, return placeholder effect
                let input_clone = input.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    // Placeholder - would parse LLM response to extract category
                    // For now, return None
                    let _ = input_clone;
                    None
                }))]
            }

            RouterAction::Classified { category, input } => {
                // Find matching route
                let route = match self.find_route(&category) {
                    Some(r) => r,
                    None => {
                        state.completed = true;
                        return smallvec![Effect::Future(Box::pin(async move {
                            Some(RouterAction::Error {
                                message: format!("Unknown category: {}", category),
                            })
                        }))];
                    }
                };

                // Update state
                state.category = Some(category.clone());

                // Call specialist
                let specialist = route.specialist.clone();
                let category_clone = category.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    let result = specialist(input).await;
                    Some(RouterAction::SpecialistComplete {
                        category: category_clone,
                        result,
                    })
                }))]
            }

            RouterAction::SpecialistComplete { category: _, result } => {
                match result {
                    Ok(output) => {
                        state.result = Some(output.clone());
                        state.completed = true;
                        smallvec![Effect::Future(Box::pin(async move {
                            Some(RouterAction::Complete { result: output })
                        }))]
                    }
                    Err(error) => {
                        state.completed = true;
                        smallvec![Effect::Future(Box::pin(async move {
                            Some(RouterAction::Error { message: error })
                        }))]
                    }
                }
            }

            RouterAction::Complete { .. } => {
                // Already complete
                smallvec![Effect::None]
            }

            RouterAction::Error { .. } => {
                // Error occurred, stop routing
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

    fn create_test_routes() -> Vec<Route> {
        vec![
            Route {
                category: "technical".to_string(),
                description: "Technical support".to_string(),
                specialist: Arc::new(|input| {
                    Box::pin(async move { Ok(format!("Tech support for: {}", input)) })
                }),
            },
            Route {
                category: "billing".to_string(),
                description: "Billing questions".to_string(),
                specialist: Arc::new(|input| {
                    Box::pin(async move { Ok(format!("Billing help for: {}", input)) })
                }),
            },
        ]
    }

    #[test]
    fn test_router_state() {
        let state = RouterState::new();
        assert!(state.category().is_none());
        assert!(state.result().is_none());
        assert!(!state.is_completed());
    }

    #[test]
    fn test_routing_reducer_creation() {
        let routes = create_test_routes();
        let reducer: RoutingReducer<MockEnvironment> = RoutingReducer::new(routes);
        assert_eq!(reducer.route_count(), 2);
    }

    #[test]
    fn test_classify_action() {
        let routes = create_test_routes();
        let reducer: RoutingReducer<MockEnvironment> = RoutingReducer::new(routes);
        let mut state = RouterState::new();
        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            RouterAction::Classify {
                input: "How do I reset my password?".to_string(),
            },
            &env,
        );

        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_classified_action_valid_category() {
        let routes = create_test_routes();
        let reducer: RoutingReducer<MockEnvironment> = RoutingReducer::new(routes);
        let mut state = RouterState::new();
        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            RouterAction::Classified {
                category: "technical".to_string(),
                input: "Reset password".to_string(),
            },
            &env,
        );

        assert_eq!(state.category(), Some("technical"));
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_classified_action_invalid_category() {
        let routes = create_test_routes();
        let reducer: RoutingReducer<MockEnvironment> = RoutingReducer::new(routes);
        let mut state = RouterState::new();
        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            RouterAction::Classified {
                category: "unknown".to_string(),
                input: "Some input".to_string(),
            },
            &env,
        );

        assert!(state.is_completed());
        assert_eq!(effects.len(), 1);
    }

    #[tokio::test]
    async fn test_specialist_execution() {
        let routes = create_test_routes();
        let route = &routes[0];

        let result = (route.specialist)("test input".to_string()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Tech support for: test input");
    }

    #[test]
    fn test_specialist_complete_success() {
        let routes = create_test_routes();
        let reducer: RoutingReducer<MockEnvironment> = RoutingReducer::new(routes);
        let mut state = RouterState::new();
        state.category = Some("technical".to_string());

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            RouterAction::SpecialistComplete {
                category: "technical".to_string(),
                result: Ok("Success".to_string()),
            },
            &env,
        );

        assert_eq!(state.result(), Some("Success"));
        assert!(state.is_completed());
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn test_specialist_complete_error() {
        let routes = create_test_routes();
        let reducer: RoutingReducer<MockEnvironment> = RoutingReducer::new(routes);
        let mut state = RouterState::new();

        let env = MockEnvironment {
            config: AgentConfig::default(),
        };

        let effects = reducer.reduce(
            &mut state,
            RouterAction::SpecialistComplete {
                category: "technical".to_string(),
                result: Err("Failed".to_string()),
            },
            &env,
        );

        assert!(state.is_completed());
        assert_eq!(effects.len(), 1);
    }
}
