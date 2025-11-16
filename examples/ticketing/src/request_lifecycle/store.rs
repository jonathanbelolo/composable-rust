//! Store for request lifecycle management.

use crate::request_lifecycle::{
    RequestLifecycleAction, RequestLifecycleReducer, RequestLifecycleState,
};
use composable_rust_core::reducer::Reducer;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Store for managing request lifecycles.
///
/// This store tracks HTTP request lifecycles using a simple reducer pattern.
/// It does not integrate with EventBus - that's the responsibility of the
/// HTTP handlers when they publish RequestCompleted events.
pub struct RequestLifecycleStore {
    state: Arc<RwLock<RequestLifecycleState>>,
    reducer: RequestLifecycleReducer,
    env: crate::request_lifecycle::environment::ProductionRequestLifecycleEnvironment,
}

impl RequestLifecycleStore {
    /// Create a new request lifecycle store.
    #[must_use]
    pub fn new(environment: crate::request_lifecycle::environment::ProductionRequestLifecycleEnvironment) -> Self {
        Self {
            state: Arc::new(RwLock::new(RequestLifecycleState::new())),
            reducer: RequestLifecycleReducer::new(),
            env: environment,
        }
    }

    /// Dispatch an action to the store.
    ///
    /// This executes the reducer and updates the state.
    /// Effects are ignored for now (we'll add effect execution later).
    pub async fn dispatch(&self, action: RequestLifecycleAction) {
        let mut state = self.state.write().await;
        let _effects = self.reducer.reduce(&mut state, action, &self.env);
        // TODO: Execute effects (Delay for timeout)
    }

    /// Get a snapshot of the current state.
    pub async fn state(&self) -> RequestLifecycleState {
        self.state.read().await.clone()
    }

    /// Get a specific request lifecycle by correlation ID.
    pub async fn get(
        &self,
        correlation_id: &crate::request_lifecycle::CorrelationId,
    ) -> Option<crate::request_lifecycle::RequestLifecycle> {
        self.state.read().await.get(correlation_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request_lifecycle::{CorrelationId, RequestMetadata};
    use composable_rust_testing::FixedClock;
    use crate::request_lifecycle::environment::ProductionRequestLifecycleEnvironment;
    use smallvec::SmallVec;
    use std::collections::HashSet;

    #[tokio::test]
    #[allow(clippy::unwrap_used)] // Test code
    async fn test_store_creation() {
        let clock = Arc::new(FixedClock::new(chrono::Utc::now()));
        let env = ProductionRequestLifecycleEnvironment::new(clock);
        let store = RequestLifecycleStore::new(env);

        assert!(store.state().await.is_empty());
    }

    #[tokio::test]
    #[allow(clippy::unwrap_used)] // Test code
    async fn test_store_dispatch() {
        let clock = Arc::new(FixedClock::new(chrono::Utc::now()));
        let env = ProductionRequestLifecycleEnvironment::new(clock);
        let store = RequestLifecycleStore::new(env);

        let correlation_id = CorrelationId::new();
        let action = RequestLifecycleAction::InitiateRequest {
            correlation_id,
            metadata: RequestMetadata {
                method: "POST".to_string(),
                path: "/api/events".to_string(),
                user_id: None,
                ip_address: None,
            },
            expected_domain_events: SmallVec::new(),
            expected_projections: HashSet::new(),
            expected_external_ops: HashSet::new(),
        };

        store.dispatch(action).await;

        let state = store.state().await;
        assert_eq!(state.len(), 1);
        assert!(state.get(&correlation_id).is_some());
    }
}
