//! Comprehensive unit tests for RequestLifecycleReducer.
//!
//! These tests verify the complete lifecycle tracking logic including:
//! - Request initiation
//! - Domain event emission (multiple events)
//! - Projection completion tracking
//! - External operation completion
//! - Request failure handling
//! - Request cancellation
//! - Timeout handling

#![allow(clippy::unwrap_used, clippy::expect_used)] // Test code

use super::*;
use crate::request_lifecycle::environment::ProductionRequestLifecycleEnvironment;
use composable_rust_core::{effect::Effect, reducer::Reducer, SmallVec};
use composable_rust_testing::FixedClock;
use smallvec::smallvec;
use std::collections::HashSet;
use std::sync::Arc;

/// Helper to create a test environment with a fixed clock.
fn test_env() -> ProductionRequestLifecycleEnvironment {
    ProductionRequestLifecycleEnvironment::new(Arc::new(FixedClock::new(
        chrono::Utc::now(),
    )))
}

/// Helper to create test metadata.
fn test_metadata() -> RequestMetadata {
    RequestMetadata {
        method: "POST".to_string(),
        path: "/api/events".to_string(),
        user_id: None,
        ip_address: None,
    }
}

// ============================================================================
// Basic Lifecycle Tests
// ============================================================================

#[test]
fn test_initiate_request_creates_lifecycle() {
    let reducer = RequestLifecycleReducer::new();
    let mut state = RequestLifecycleState::new();
    let env = test_env();

    let correlation_id = CorrelationId::new();
    let action = RequestLifecycleAction::InitiateRequest {
        correlation_id,
        metadata: test_metadata(),
        expected_domain_events: smallvec!["EventCreated".to_string()],
        expected_projections: hashset!["events".to_string()],
        expected_external_ops: HashSet::new(),
    };

    let effects = reducer.reduce(&mut state, action, &env);

    // Should create lifecycle in state
    let lifecycle = state.get(&correlation_id).expect("Lifecycle should exist");
    assert_eq!(lifecycle.status, RequestStatus::Pending);
    assert_eq!(lifecycle.expected_domain_events.len(), 1);
    assert_eq!(lifecycle.emitted_domain_events.len(), 0);

    // Should schedule timeout effect
    assert_eq!(effects.len(), 1);
    assert!(matches!(effects[0], Effect::Delay { .. }));
}

#[test]
fn test_domain_event_emitted_single_event() {
    let reducer = RequestLifecycleReducer::new();
    let mut state = RequestLifecycleState::new();
    let env = test_env();

    let correlation_id = CorrelationId::new();

    // Initiate request expecting 1 event + 1 projection so it doesn't complete immediately
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::InitiateRequest {
            correlation_id,
            metadata: test_metadata(),
            expected_domain_events: smallvec!["EventCreated".to_string()],
            expected_projections: hashset!["events".to_string()],
            expected_external_ops: HashSet::new(),
        },
        &env,
    );

    // Emit the event
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::DomainEventEmitted {
            correlation_id,
            event_type: "EventCreated".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.status, RequestStatus::DomainEventEmitted);
    assert_eq!(lifecycle.emitted_domain_events.len(), 1);
    assert_eq!(lifecycle.emitted_domain_events[0], "EventCreated");
}

#[test]
fn test_domain_event_emitted_multiple_events() {
    let reducer = RequestLifecycleReducer::new();
    let mut state = RequestLifecycleState::new();
    let env = test_env();

    let correlation_id = CorrelationId::new();

    // Initiate request expecting 3 events
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::InitiateRequest {
            correlation_id,
            metadata: test_metadata(),
            expected_domain_events: smallvec![
                "EventCreated".to_string(),
                "InventoryInitialized".to_string(),
                "InventoryInitialized".to_string()
            ],
            expected_projections: HashSet::new(),
            expected_external_ops: HashSet::new(),
        },
        &env,
    );

    // Emit first event
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::DomainEventEmitted {
            correlation_id,
            event_type: "EventCreated".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.emitted_domain_events.len(), 1);
    assert!(!lifecycle.domain_events_complete());

    // Emit second event
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::DomainEventEmitted {
            correlation_id,
            event_type: "InventoryInitialized".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.emitted_domain_events.len(), 2);

    // Emit third event
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::DomainEventEmitted {
            correlation_id,
            event_type: "InventoryInitialized".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.emitted_domain_events.len(), 3);
    assert!(lifecycle.domain_events_complete());
}

#[test]
fn test_projection_completed_single_projection() {
    let reducer = RequestLifecycleReducer::new();
    let mut state = RequestLifecycleState::new();
    let env = test_env();

    let correlation_id = CorrelationId::new();

    // Initiate with 1 projection expected
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::InitiateRequest {
            correlation_id,
            metadata: test_metadata(),
            expected_domain_events: SmallVec::new(),
            expected_projections: hashset!["events".to_string()],
            expected_external_ops: HashSet::new(),
        },
        &env,
    );

    // Mark projection as completed
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::ProjectionCompleted {
            correlation_id,
            projection_name: "events".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert!(lifecycle.completed_projections.contains("events"));
    assert!(lifecycle.projections_complete());
}

#[test]
fn test_projection_completed_multiple_projections() {
    let reducer = RequestLifecycleReducer::new();
    let mut state = RequestLifecycleState::new();
    let env = test_env();

    let correlation_id = CorrelationId::new();

    // Initiate with 3 projections expected
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::InitiateRequest {
            correlation_id,
            metadata: test_metadata(),
            expected_domain_events: SmallVec::new(),
            expected_projections: hashset![
                "events".to_string(),
                "available_seats".to_string(),
                "sales_analytics".to_string()
            ],
            expected_external_ops: HashSet::new(),
        },
        &env,
    );

    // Complete first projection
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::ProjectionCompleted {
            correlation_id,
            projection_name: "events".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.completed_projections.len(), 1);
    assert!(!lifecycle.projections_complete());

    // Complete second projection
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::ProjectionCompleted {
            correlation_id,
            projection_name: "available_seats".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.completed_projections.len(), 2);
    assert!(!lifecycle.projections_complete());

    // Complete third projection
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::ProjectionCompleted {
            correlation_id,
            projection_name: "sales_analytics".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.completed_projections.len(), 3);
    assert!(lifecycle.projections_complete());
}

// ============================================================================
// Complete Lifecycle Tests
// ============================================================================

#[test]
fn test_complete_lifecycle_no_projections_or_external_ops() {
    let reducer = RequestLifecycleReducer::new();
    let mut state = RequestLifecycleState::new();
    let env = test_env();

    let correlation_id = CorrelationId::new();

    // Initiate with 1 domain event, no projections or external ops
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::InitiateRequest {
            correlation_id,
            metadata: test_metadata(),
            expected_domain_events: smallvec!["EventCreated".to_string()],
            expected_projections: HashSet::new(),
            expected_external_ops: HashSet::new(),
        },
        &env,
    );

    // Emit domain event - should complete immediately
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::DomainEventEmitted {
            correlation_id,
            event_type: "EventCreated".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.status, RequestStatus::Completed);
    assert!(lifecycle.is_complete());
    assert!(lifecycle.completed_at.is_some());
}

#[test]
fn test_complete_lifecycle_with_projections() {
    let reducer = RequestLifecycleReducer::new();
    let mut state = RequestLifecycleState::new();
    let env = test_env();

    let correlation_id = CorrelationId::new();

    // Initiate with 1 event and 2 projections
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::InitiateRequest {
            correlation_id,
            metadata: test_metadata(),
            expected_domain_events: smallvec!["EventCreated".to_string()],
            expected_projections: hashset!["events".to_string(), "available_seats".to_string()],
            expected_external_ops: HashSet::new(),
        },
        &env,
    );

    // Emit domain event
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::DomainEventEmitted {
            correlation_id,
            event_type: "EventCreated".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.status, RequestStatus::DomainEventEmitted);
    assert!(!lifecycle.is_complete());

    // Complete first projection
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::ProjectionCompleted {
            correlation_id,
            projection_name: "events".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert!(!lifecycle.is_complete());

    // Complete second projection - should mark as complete
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::ProjectionCompleted {
            correlation_id,
            projection_name: "available_seats".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.status, RequestStatus::Completed);
    assert!(lifecycle.is_complete());
}

#[test]
fn test_complete_lifecycle_with_external_ops() {
    let reducer = RequestLifecycleReducer::new();
    let mut state = RequestLifecycleState::new();
    let env = test_env();

    let correlation_id = CorrelationId::new();

    // Initiate with event, projection, and external op
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::InitiateRequest {
            correlation_id,
            metadata: test_metadata(),
            expected_domain_events: smallvec!["EventCreated".to_string()],
            expected_projections: hashset!["events".to_string()],
            expected_external_ops: hashset!["confirmation_email".to_string()],
        },
        &env,
    );

    // Emit event
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::DomainEventEmitted {
            correlation_id,
            event_type: "EventCreated".to_string(),
        },
        &env,
    );

    // Complete projection
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::ProjectionCompleted {
            correlation_id,
            projection_name: "events".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.status, RequestStatus::ProjectionsCompleted);
    assert!(!lifecycle.is_complete());

    // Complete external operation
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::ExternalOperationCompleted {
            correlation_id,
            operation_name: "confirmation_email".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.status, RequestStatus::Completed);
    assert!(lifecycle.is_complete());
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_request_failed() {
    let reducer = RequestLifecycleReducer::new();
    let mut state = RequestLifecycleState::new();
    let env = test_env();

    let correlation_id = CorrelationId::new();

    reducer.reduce(
        &mut state,
        RequestLifecycleAction::InitiateRequest {
            correlation_id,
            metadata: test_metadata(),
            expected_domain_events: smallvec!["EventCreated".to_string()],
            expected_projections: hashset!["events".to_string()],
            expected_external_ops: HashSet::new(),
        },
        &env,
    );

    // Mark as failed
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::RequestFailed {
            correlation_id,
            error: "Database connection failed".to_string(),
        },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.status, RequestStatus::Failed);
    assert_eq!(lifecycle.error.as_ref().unwrap(), "Database connection failed");
    assert!(lifecycle.completed_at.is_some());
}

#[test]
fn test_request_cancelled() {
    let reducer = RequestLifecycleReducer::new();
    let mut state = RequestLifecycleState::new();
    let env = test_env();

    let correlation_id = CorrelationId::new();

    reducer.reduce(
        &mut state,
        RequestLifecycleAction::InitiateRequest {
            correlation_id,
            metadata: test_metadata(),
            expected_domain_events: smallvec!["EventCreated".to_string()],
            expected_projections: hashset!["events".to_string()],
            expected_external_ops: HashSet::new(),
        },
        &env,
    );

    // Cancel request
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::CancelRequest { correlation_id },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.status, RequestStatus::Cancelled);
    assert!(lifecycle.completed_at.is_some());
}

#[test]
fn test_request_timeout() {
    let reducer = RequestLifecycleReducer::new();
    let mut state = RequestLifecycleState::new();
    let env = test_env();

    let correlation_id = CorrelationId::new();

    reducer.reduce(
        &mut state,
        RequestLifecycleAction::InitiateRequest {
            correlation_id,
            metadata: test_metadata(),
            expected_domain_events: smallvec!["EventCreated".to_string()],
            expected_projections: hashset!["events".to_string()],
            expected_external_ops: HashSet::new(),
        },
        &env,
    );

    // Timeout the request
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::TimeoutRequest { correlation_id },
        &env,
    );

    let lifecycle = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle.status, RequestStatus::TimedOut);
    assert!(lifecycle.error.is_some());
    assert!(lifecycle.completed_at.is_some());
}

#[test]
fn test_timeout_ignored_if_already_completed() {
    let reducer = RequestLifecycleReducer::new();
    let mut state = RequestLifecycleState::new();
    let env = test_env();

    let correlation_id = CorrelationId::new();

    // Initiate and immediately complete
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::InitiateRequest {
            correlation_id,
            metadata: test_metadata(),
            expected_domain_events: smallvec!["EventCreated".to_string()],
            expected_projections: HashSet::new(),
            expected_external_ops: HashSet::new(),
        },
        &env,
    );

    reducer.reduce(
        &mut state,
        RequestLifecycleAction::DomainEventEmitted {
            correlation_id,
            event_type: "EventCreated".to_string(),
        },
        &env,
    );

    let lifecycle_before = state.get(&correlation_id).unwrap().clone();
    assert_eq!(lifecycle_before.status, RequestStatus::Completed);

    // Try to timeout - should be ignored
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::TimeoutRequest { correlation_id },
        &env,
    );

    let lifecycle_after = state.get(&correlation_id).unwrap();
    assert_eq!(lifecycle_after.status, RequestStatus::Completed); // Still completed
    assert!(lifecycle_after.error.is_none()); // No error added
}

// ============================================================================
// Unknown Correlation ID Tests
// ============================================================================

#[test]
fn test_domain_event_unknown_correlation_id() {
    let reducer = RequestLifecycleReducer::new();
    let mut state = RequestLifecycleState::new();
    let env = test_env();

    let unknown_id = CorrelationId::new();

    // Try to emit event for unknown correlation_id - should be gracefully ignored
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::DomainEventEmitted {
            correlation_id: unknown_id,
            event_type: "EventCreated".to_string(),
        },
        &env,
    );

    // State should still be empty
    assert!(state.is_empty());
}

#[test]
fn test_projection_completed_unknown_correlation_id() {
    let reducer = RequestLifecycleReducer::new();
    let mut state = RequestLifecycleState::new();
    let env = test_env();

    let unknown_id = CorrelationId::new();

    // Try to complete projection for unknown correlation_id
    reducer.reduce(
        &mut state,
        RequestLifecycleAction::ProjectionCompleted {
            correlation_id: unknown_id,
            projection_name: "events".to_string(),
        },
        &env,
    );

    assert!(state.is_empty());
}

// Helper macro for creating hashsets
macro_rules! hashset {
    ($($item:expr),* $(,)?) => {{
        let mut set = HashSet::new();
        $(set.insert($item);)*
        set
    }};
}

// Re-export for use in tests
use hashset;
