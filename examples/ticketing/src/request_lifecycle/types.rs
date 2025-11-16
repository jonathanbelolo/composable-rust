//! Core types for request lifecycle management.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Unique identifier for correlating all operations in a request lifecycle.
///
/// This ID is generated when an HTTP request arrives and is propagated through:
/// - Domain events (in metadata)
/// - Projection completion events
/// - External operation events
///
/// Tests and clients use this to track when a request is fully processed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CorrelationId(Uuid);

impl CorrelationId {
    /// Generate a new correlation ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from an existing UUID.
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the inner UUID.
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for CorrelationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for CorrelationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Metadata about the HTTP request that initiated this lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMetadata {
    /// HTTP method (GET, POST, PUT, DELETE)
    pub method: String,

    /// Request path (/api/events, /api/reservations/:id, etc.)
    pub path: String,

    /// Authenticated user ID (if applicable)
    pub user_id: Option<Uuid>,

    /// Client IP address (if available)
    pub ip_address: Option<String>,
}

/// Status of a request lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestStatus {
    /// Request initiated, waiting for domain event
    Pending,

    /// Domain event emitted, waiting for projections
    DomainEventEmitted,

    /// All projections updated, waiting for external operations
    ProjectionsCompleted,

    /// Everything done successfully
    Completed,

    /// Request failed (error during processing)
    Failed,

    /// Request cancelled by user
    Cancelled,

    /// Request timed out (took too long to complete)
    TimedOut,
}

/// A single request lifecycle tracking all operations for one HTTP request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLifecycle {
    /// Unique correlation ID for this request
    pub correlation_id: CorrelationId,

    /// HTTP request metadata (method, path, user_id)
    pub request_metadata: RequestMetadata,

    /// When the request was initiated
    pub initiated_at: DateTime<Utc>,

    /// Domain events that should be emitted (e.g., ["EventCreated", "InventoryInitialized"])
    /// Most requests emit 1-3 events, so SmallVec[3] avoids heap allocation
    pub expected_domain_events: SmallVec<[String; 3]>,

    /// Domain events that have been emitted
    pub emitted_domain_events: SmallVec<[String; 3]>,

    /// Projections that need to be updated for this request
    pub expected_projections: HashSet<String>,

    /// Projections that have completed updating
    pub completed_projections: HashSet<String>,

    /// External operations that need to complete (emails, webhooks, etc.)
    pub expected_external_ops: HashSet<String>,

    /// External operations that have completed
    pub completed_external_ops: HashSet<String>,

    /// Overall request status
    pub status: RequestStatus,

    /// When the request completed (all operations done)
    pub completed_at: Option<DateTime<Utc>>,

    /// Error message if request failed
    pub error: Option<String>,
}

impl RequestLifecycle {
    /// Create a new request lifecycle.
    #[must_use]
    pub fn new(
        correlation_id: CorrelationId,
        request_metadata: RequestMetadata,
        initiated_at: DateTime<Utc>,
        expected_domain_events: SmallVec<[String; 3]>,
        expected_projections: HashSet<String>,
        expected_external_ops: HashSet<String>,
    ) -> Self {
        Self {
            correlation_id,
            request_metadata,
            initiated_at,
            expected_domain_events,
            emitted_domain_events: SmallVec::new(),
            expected_projections,
            completed_projections: HashSet::new(),
            expected_external_ops,
            completed_external_ops: HashSet::new(),
            status: RequestStatus::Pending,
            completed_at: None,
            error: None,
        }
    }

    /// Check if all domain events have been emitted.
    #[must_use]
    pub fn domain_events_complete(&self) -> bool {
        self.emitted_domain_events.len() == self.expected_domain_events.len()
    }

    /// Check if all operations are complete.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.domain_events_complete()
            && self.completed_projections == self.expected_projections
            && self.completed_external_ops == self.expected_external_ops
    }

    /// Check if all projections are complete.
    #[must_use]
    pub fn projections_complete(&self) -> bool {
        self.completed_projections == self.expected_projections
    }

    /// Check if all external operations are complete.
    #[must_use]
    pub fn external_ops_complete(&self) -> bool {
        self.completed_external_ops == self.expected_external_ops
    }

    /// Calculate duration in milliseconds (if completed).
    #[must_use]
    pub fn duration_ms(&self) -> Option<i64> {
        self.completed_at.map(|completed| {
            completed
                .signed_duration_since(self.initiated_at)
                .num_milliseconds()
        })
    }
}

/// State for the request lifecycle reducer.
///
/// Tracks all active and completed request lifecycles by correlation ID.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestLifecycleState {
    /// Active and completed lifecycles indexed by correlation_id
    lifecycles: HashMap<CorrelationId, RequestLifecycle>,
}

impl RequestLifecycleState {
    /// Create a new empty state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            lifecycles: HashMap::new(),
        }
    }

    /// Insert a new request lifecycle.
    pub fn insert(&mut self, correlation_id: CorrelationId, lifecycle: RequestLifecycle) {
        self.lifecycles.insert(correlation_id, lifecycle);
    }

    /// Get a request lifecycle by correlation ID.
    #[must_use]
    pub fn get(&self, correlation_id: &CorrelationId) -> Option<&RequestLifecycle> {
        self.lifecycles.get(correlation_id)
    }

    /// Get a mutable reference to a request lifecycle.
    pub fn get_mut(&mut self, correlation_id: &CorrelationId) -> Option<&mut RequestLifecycle> {
        self.lifecycles.get_mut(correlation_id)
    }

    /// Remove a request lifecycle (for cleanup).
    pub fn remove(&mut self, correlation_id: &CorrelationId) -> Option<RequestLifecycle> {
        self.lifecycles.remove(correlation_id)
    }

    /// Get all lifecycles.
    #[must_use]
    pub fn all(&self) -> &HashMap<CorrelationId, RequestLifecycle> {
        &self.lifecycles
    }

    /// Count of tracked lifecycles.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lifecycles.len()
    }

    /// Check if state is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lifecycles.is_empty()
    }
}
