//! Reducer for request lifecycle management.

use crate::request_lifecycle::{
    RequestLifecycleAction, RequestLifecycleEnvironment, RequestLifecycle, RequestLifecycleState,
    RequestStatus,
};
use composable_rust_core::{effect::Effect, reducer::Reducer};
use smallvec::{smallvec, SmallVec};

/// Reducer for managing request lifecycles.
///
/// Tracks HTTP requests from initiation through completion, coordinating:
/// - Domain event emission
/// - Projection updates
/// - External operations (emails, webhooks, etc.)
///
/// Emits `RequestCompleted` events when all operations finish.
pub struct RequestLifecycleReducer;

impl RequestLifecycleReducer {
    /// Create a new request lifecycle reducer.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for RequestLifecycleReducer {
    fn default() -> Self {
        Self::new()
    }
}

impl Reducer for RequestLifecycleReducer {
    type State = RequestLifecycleState;
    type Action = RequestLifecycleAction;
    type Environment = crate::request_lifecycle::environment::ProductionRequestLifecycleEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            RequestLifecycleAction::InitiateRequest {
                correlation_id,
                metadata,
                expected_domain_events,
                expected_projections,
                expected_external_ops,
            } => {
                // Create new lifecycle
                let lifecycle = RequestLifecycle::new(
                    correlation_id,
                    metadata,
                    env.clock().now(),
                    expected_domain_events,
                    expected_projections,
                    expected_external_ops,
                );

                state.insert(correlation_id, lifecycle);

                // Schedule timeout check (30 seconds)
                smallvec![Effect::Delay {
                    duration: std::time::Duration::from_secs(30),
                    action: Box::new(RequestLifecycleAction::TimeoutRequest { correlation_id }),
                }]
            }

            RequestLifecycleAction::DomainEventEmitted {
                correlation_id,
                event_type,
            } => {
                if let Some(lifecycle) = state.get_mut(&correlation_id) {
                    // Add to emitted events
                    lifecycle.emitted_domain_events.push(event_type);

                    // Update status if this was the first event
                    if lifecycle.status == RequestStatus::Pending {
                        lifecycle.status = RequestStatus::DomainEventEmitted;
                    }

                    // Check if all operations complete
                    if lifecycle.is_complete() {
                        lifecycle.status = RequestStatus::Completed;
                        lifecycle.completed_at = Some(env.clock().now());

                        // Emit completion event (would publish to EventBus in real implementation)
                        // For now, just return None - we'll add EventBus publishing later
                        smallvec![Effect::None]
                    } else {
                        smallvec![Effect::None]
                    }
                } else {
                    // Unknown correlation_id - this shouldn't happen but handle gracefully
                    smallvec![Effect::None]
                }
            }

            RequestLifecycleAction::ProjectionCompleted {
                correlation_id,
                projection_name,
            } => {
                if let Some(lifecycle) = state.get_mut(&correlation_id) {
                    // Mark projection as completed
                    lifecycle.completed_projections.insert(projection_name);

                    // Check if all operations complete
                    if lifecycle.is_complete() {
                        lifecycle.status = RequestStatus::Completed;
                        lifecycle.completed_at = Some(env.clock().now());

                        // Emit completion event
                        smallvec![Effect::None]
                    } else if lifecycle.projections_complete()
                        && !lifecycle.external_ops_complete()
                    {
                        // Projections done, waiting for external ops
                        lifecycle.status = RequestStatus::ProjectionsCompleted;
                        smallvec![Effect::None]
                    } else {
                        smallvec![Effect::None]
                    }
                } else {
                    smallvec![Effect::None]
                }
            }

            RequestLifecycleAction::ExternalOperationCompleted {
                correlation_id,
                operation_name,
            } => {
                if let Some(lifecycle) = state.get_mut(&correlation_id) {
                    // Mark operation as completed
                    lifecycle.completed_external_ops.insert(operation_name);

                    // Check if all operations complete
                    if lifecycle.is_complete() {
                        lifecycle.status = RequestStatus::Completed;
                        lifecycle.completed_at = Some(env.clock().now());

                        // Emit completion event
                        smallvec![Effect::None]
                    } else {
                        smallvec![Effect::None]
                    }
                } else {
                    smallvec![Effect::None]
                }
            }

            RequestLifecycleAction::RequestFailed {
                correlation_id,
                error,
            } => {
                if let Some(lifecycle) = state.get_mut(&correlation_id) {
                    lifecycle.status = RequestStatus::Failed;
                    lifecycle.error = Some(error);
                    lifecycle.completed_at = Some(env.clock().now());

                    // Emit failure event
                    smallvec![Effect::None]
                } else {
                    smallvec![Effect::None]
                }
            }

            RequestLifecycleAction::CancelRequest { correlation_id } => {
                if let Some(lifecycle) = state.get_mut(&correlation_id) {
                    lifecycle.status = RequestStatus::Cancelled;
                    lifecycle.completed_at = Some(env.clock().now());

                    // Emit cancellation event
                    smallvec![Effect::None]
                } else {
                    smallvec![Effect::None]
                }
            }

            RequestLifecycleAction::TimeoutRequest { correlation_id } => {
                if let Some(lifecycle) = state.get_mut(&correlation_id) {
                    // Only timeout if still in progress
                    if lifecycle.status != RequestStatus::Completed
                        && lifecycle.status != RequestStatus::Failed
                        && lifecycle.status != RequestStatus::Cancelled
                        && lifecycle.status != RequestStatus::TimedOut
                    {
                        lifecycle.status = RequestStatus::TimedOut;
                        lifecycle.completed_at = Some(env.clock().now());
                        lifecycle.error =
                            Some("Request timed out after 30 seconds".to_string());

                        // Emit timeout event
                        smallvec![Effect::None]
                    } else {
                        // Already completed/failed - ignore timeout
                        smallvec![Effect::None]
                    }
                } else {
                    smallvec![Effect::None]
                }
            }
        }
    }
}
