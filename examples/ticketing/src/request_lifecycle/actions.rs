//! Actions for request lifecycle management.

use crate::request_lifecycle::types::{CorrelationId, RequestMetadata};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::collections::HashSet;

/// Actions for managing request lifecycles.
///
/// These actions are processed by the `RequestLifecycleReducer` to track
/// the progress of HTTP requests through domain events, projections, and
/// external operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RequestLifecycleAction {
    /// Initiate a new request lifecycle.
    ///
    /// This is called when an HTTP request arrives, before dispatching to
    /// the business domain.
    InitiateRequest {
        /// Unique correlation ID for this request
        correlation_id: CorrelationId,

        /// HTTP request metadata
        metadata: RequestMetadata,

        /// Domain events that should be emitted (e.g., ["EventCreated", "InventoryInitialized"])
        expected_domain_events: SmallVec<[String; 3]>,

        /// Projections that need to be updated
        expected_projections: HashSet<String>,

        /// External operations that need to complete
        expected_external_ops: HashSet<String>,
    },

    /// Mark that the domain event has been emitted.
    ///
    /// This is called after the business domain reducer processes the action
    /// and emits an event to the event bus.
    DomainEventEmitted {
        /// Correlation ID to update
        correlation_id: CorrelationId,

        /// Type of event emitted (e.g., "EventCreated")
        event_type: String,
    },

    /// Mark a projection as completed.
    ///
    /// This is called by projection managers after they've successfully
    /// updated their projection.
    ProjectionCompleted {
        /// Correlation ID to update
        correlation_id: CorrelationId,

        /// Name of projection that completed (e.g., "events", "available_seats")
        projection_name: String,
    },

    /// Mark an external operation as completed.
    ///
    /// This is called after external operations like emails, webhooks, etc.
    /// have completed successfully.
    ExternalOperationCompleted {
        /// Correlation ID to update
        correlation_id: CorrelationId,

        /// Name of operation that completed (e.g., "confirmation_email")
        operation_name: String,
    },

    /// Mark a request as failed.
    ///
    /// This is called when any part of the request processing fails.
    RequestFailed {
        /// Correlation ID to update
        correlation_id: CorrelationId,

        /// Error message
        error: String,
    },

    /// Cancel a request.
    ///
    /// This is called when a user explicitly cancels their request
    /// (e.g., cancelling a reservation).
    CancelRequest {
        /// Correlation ID to update
        correlation_id: CorrelationId,
    },

    /// Timeout a request.
    ///
    /// This is called when a request takes too long to complete
    /// (automatically after a timeout period).
    TimeoutRequest {
        /// Correlation ID to update
        correlation_id: CorrelationId,
    },
}
