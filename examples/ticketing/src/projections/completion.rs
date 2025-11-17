//! Projection completion tracking and notification system.
//!
//! This module provides infrastructure for tracking when projections complete processing
//! domain events, enabling read-after-write consistency guarantees when needed.
//!
//! # Architecture
//!
//! ```text
//! Projection Manager → Applies Event → Publishes to "projection.completed" topic
//!                                              ↓
//!                            ProjectionCompletionTracker (Singleton Consumer)
//!                                              ↓
//!                              In-Memory Map: CorrelationId → Waiters
//!                                              ↓
//!                              Notifies all waiting HTTP requests
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! // HTTP Handler
//! let correlation_id = CorrelationId::new();
//!
//! // Register interest (cheap - just creates oneshot channel)
//! let completion = tracker.register_interest(correlation_id, &["available_seats"]);
//!
//! // Dispatch command with correlation_id in metadata
//! reservation_store.send_with_metadata(command, correlation_id).await;
//!
//! // Optionally wait for projection completion (with timeout)
//! tokio::time::timeout(Duration::from_secs(5), completion).await??;
//! ```
//!
//! # Key Design Points
//!
//! - **ONE Consumer**: Created at startup, runs forever
//! - **Cheap Per-Request**: Only allocates oneshot channel
//! - **Automatic Cleanup**: Entries removed after notification
//! - **Multiple Waiters**: Many requests can wait for same correlation_id
//! - **Timeout Support**: Requests don't block forever

use composable_rust_core::event_bus::EventBus;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Unique identifier for tracking a request through its entire lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CorrelationId(Uuid);

impl CorrelationId {
    /// Create a new random correlation ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from existing UUID.
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

/// Event emitted by projections when they successfully process an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionCompleted {
    /// Correlation ID from the original domain event
    pub correlation_id: CorrelationId,
    /// Name of the projection (e.g., "available_seats", "sales_analytics")
    pub projection_name: String,
    /// Type of domain event that was processed
    pub event_type: String,
}

/// Event emitted by projections when they fail to process an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionFailed {
    /// Correlation ID from the original domain event
    pub correlation_id: CorrelationId,
    /// Name of the projection
    pub projection_name: String,
    /// Type of domain event that failed
    pub event_type: String,
    /// Error message
    pub error: String,
}

/// Unified projection completion event type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProjectionCompletionEvent {
    /// Projection successfully processed an event
    Completed(ProjectionCompleted),
    /// Projection failed to process an event
    Failed(ProjectionFailed),
}

impl ProjectionCompletionEvent {
    /// Get the correlation ID from this event.
    #[must_use]
    pub const fn correlation_id(&self) -> &CorrelationId {
        match self {
            Self::Completed(c) => &c.correlation_id,
            Self::Failed(f) => &f.correlation_id,
        }
    }

    /// Get the projection name from this event.
    #[must_use]
    pub fn projection_name(&self) -> &str {
        match self {
            Self::Completed(c) => &c.projection_name,
            Self::Failed(f) => &f.projection_name,
        }
    }
}

/// Result of waiting for projection completion.
#[derive(Debug, Clone)]
pub enum ProjectionResult {
    /// All expected projections completed successfully
    Completed(Vec<ProjectionCompleted>),
    /// One or more projections failed
    Failed(Vec<ProjectionFailed>),
}

/// Internal state for a waiter waiting for specific projections.
struct Waiter {
    /// Which projections we're waiting for
    expected_projections: HashSet<String>,
    /// Projections that have completed
    completed: Vec<ProjectionCompleted>,
    /// Projections that have failed
    failed: Vec<ProjectionFailed>,
    /// Channel to send result when all projections complete/fail
    sender: oneshot::Sender<ProjectionResult>,
}

impl Waiter {
    /// Record a projection completion.
    fn record_completion(&mut self, completion: ProjectionCompleted) {
        self.expected_projections.remove(&completion.projection_name);
        self.completed.push(completion);
    }

    /// Record a projection failure.
    fn record_failure(&mut self, failure: ProjectionFailed) {
        self.expected_projections.remove(&failure.projection_name);
        self.failed.push(failure);
    }

    /// Check if all projections have completed or failed.
    fn is_done(&self) -> bool {
        self.expected_projections.is_empty()
    }

    /// Send the result to the waiter.
    fn send_result(self) {
        let result = if self.failed.is_empty() {
            ProjectionResult::Completed(self.completed)
        } else {
            ProjectionResult::Failed(self.failed)
        };
        let _ = self.sender.send(result);
    }
}

/// Singleton service that tracks projection completions across all requests.
///
/// Uses ONE background consumer to monitor the "projection.completed" topic
/// and notifies individual HTTP requests when their projections complete.
pub struct ProjectionCompletionTracker {
    /// Pending requests waiting for projection completion
    /// Key: CorrelationId, Value: List of waiters
    pending: Arc<RwLock<HashMap<CorrelationId, Vec<Waiter>>>>,
    /// Background consumer task handle
    consumer_handle: Option<JoinHandle<()>>,
}

impl ProjectionCompletionTracker {
    /// Create a new tracker and start the background consumer.
    ///
    /// # Arguments
    ///
    /// - `event_bus`: Event bus to consume completion events from
    ///
    /// # Errors
    ///
    /// Returns error if subscription to "projection.completed" topic fails.
    pub async fn new(event_bus: Arc<dyn EventBus>) -> Result<Self, String> {
        let pending = Arc::new(RwLock::new(HashMap::new()));
        let pending_clone = pending.clone();

        // Spawn background consumer task (ONE consumer for entire app)
        let consumer_handle = tokio::spawn(async move {
            Self::consume_loop(event_bus, pending_clone).await;
        });

        Ok(Self {
            pending,
            consumer_handle: Some(consumer_handle),
        })
    }

    /// Background consumer loop (runs forever).
    ///
    /// Consumes from "projection.completed" topic and notifies waiting requests.
    async fn consume_loop(
        event_bus: Arc<dyn EventBus>,
        pending: Arc<RwLock<HashMap<CorrelationId, Vec<Waiter>>>>,
    ) {
        tracing::info!("Starting ProjectionCompletionTracker consumer");

        // Subscribe to completion topic
        let mut stream = match event_bus.subscribe(&["projection.completed"]).await {
            Ok(stream) => stream,
            Err(e) => {
                tracing::error!(error = ?e, "Failed to subscribe to projection.completed topic");
                return;
            }
        };

        // Consume completion events forever
        while let Some(result) = stream.next().await {
            match result {
                Ok(serialized_event) => {
                    // Deserialize completion event
                    match bincode::deserialize::<ProjectionCompletionEvent>(&serialized_event.data)
                    {
                        Ok(event) => {
                            Self::handle_completion_event(&pending, event);
                        }
                        Err(e) => {
                            tracing::error!(error = ?e, "Failed to deserialize projection completion event");
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = ?e, "Error receiving event from projection.completed topic");
                    // Backoff on transport errors
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }

        tracing::info!("ProjectionCompletionTracker consumer stopped");
    }

    /// Handle a projection completion/failure event.
    ///
    /// Updates all waiters for this correlation_id and notifies those that are done.
    fn handle_completion_event(
        pending: &Arc<RwLock<HashMap<CorrelationId, Vec<Waiter>>>>,
        event: ProjectionCompletionEvent,
    ) {
        let correlation_id = *event.correlation_id();

        let mut pending_lock = pending.write().expect("Pending lock poisoned");

        // Get all waiters for this correlation_id
        if let Some(waiters) = pending_lock.get_mut(&correlation_id) {
            let mut done_indices = Vec::new();

            // Update each waiter
            for (idx, waiter) in waiters.iter_mut().enumerate() {
                match &event {
                    ProjectionCompletionEvent::Completed(c) => {
                        if waiter.expected_projections.contains(&c.projection_name) {
                            waiter.record_completion(c.clone());
                        }
                    }
                    ProjectionCompletionEvent::Failed(f) => {
                        if waiter.expected_projections.contains(&f.projection_name) {
                            waiter.record_failure(f.clone());
                        }
                    }
                }

                // Check if this waiter is done
                if waiter.is_done() {
                    done_indices.push(idx);
                }
            }

            // Remove and notify completed waiters (in reverse to maintain indices)
            for idx in done_indices.into_iter().rev() {
                let waiter = waiters.swap_remove(idx);
                waiter.send_result();
            }

            // Clean up entry if no more waiters
            if waiters.is_empty() {
                pending_lock.remove(&correlation_id);
            }
        }
    }

    /// Register interest in waiting for specific projections to complete.
    ///
    /// Returns a future that resolves when all specified projections have
    /// processed the event with the given correlation_id.
    ///
    /// # Arguments
    ///
    /// - `correlation_id`: Correlation ID to track
    /// - `projection_names`: Names of projections to wait for (e.g., ["available_seats"])
    ///
    /// # Returns
    ///
    /// A receiver that will be notified when all projections complete or fail.
    #[must_use]
    pub fn register_interest(
        &self,
        correlation_id: CorrelationId,
        projection_names: &[&str],
    ) -> oneshot::Receiver<ProjectionResult> {
        let (sender, receiver) = oneshot::channel();

        let waiter = Waiter {
            expected_projections: projection_names.iter().map(|s| (*s).to_string()).collect(),
            completed: Vec::new(),
            failed: Vec::new(),
            sender,
        };

        let mut pending = self.pending.write().expect("Pending lock poisoned");
        pending
            .entry(correlation_id)
            .or_insert_with(Vec::new)
            .push(waiter);

        receiver
    }

    /// Get count of pending requests (for monitoring).
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.read().expect("Pending lock poisoned").len()
    }

    /// Shutdown the background consumer gracefully.
    pub async fn shutdown(mut self) {
        if let Some(handle) = self.consumer_handle.take() {
            handle.abort();
            let _ = handle.await;
        }
    }
}

impl Drop for ProjectionCompletionTracker {
    fn drop(&mut self) {
        if let Some(handle) = self.consumer_handle.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_testing::mocks::InMemoryEventBus;

    #[tokio::test]
    async fn test_single_waiter_single_projection() {
        let event_bus = Arc::new(InMemoryEventBus::new());
        let tracker = ProjectionCompletionTracker::new(event_bus.clone())
            .await
            .expect("Failed to create tracker");

        let correlation_id = CorrelationId::new();

        // Register interest
        let receiver = tracker.register_interest(correlation_id, &["available_seats"]);

        // Give consumer time to subscribe
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Publish completion event
        let completion = ProjectionCompleted {
            correlation_id,
            projection_name: "available_seats".to_string(),
            event_type: "SeatsReserved".to_string(),
        };
        let event = ProjectionCompletionEvent::Completed(completion);
        let data = bincode::serialize(&event).expect("Failed to serialize");
        event_bus
            .publish(
                "projection.completed",
                &composable_rust_core::event::SerializedEvent::new(
                    "ProjectionCompleted".to_string(),
                    data,
                    None,
                ),
            )
            .await
            .expect("Failed to publish");

        // Wait for notification (with timeout)
        let result = tokio::time::timeout(tokio::time::Duration::from_secs(1), receiver)
            .await
            .expect("Timeout waiting for completion")
            .expect("Channel closed");

        match result {
            ProjectionResult::Completed(completions) => {
                assert_eq!(completions.len(), 1);
                assert_eq!(completions[0].projection_name, "available_seats");
            }
            ProjectionResult::Failed(_) => panic!("Expected completion, got failure"),
        }

        // Verify cleanup
        assert_eq!(tracker.pending_count(), 0);
    }

    #[tokio::test]
    async fn test_multiple_projections() {
        let event_bus = Arc::new(InMemoryEventBus::new());
        let tracker = ProjectionCompletionTracker::new(event_bus.clone())
            .await
            .expect("Failed to create tracker");

        let correlation_id = CorrelationId::new();

        // Register interest in multiple projections
        let mut receiver = tracker.register_interest(
            correlation_id,
            &["available_seats", "sales_analytics"],
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Publish first completion
        let completion1 = ProjectionCompleted {
            correlation_id,
            projection_name: "available_seats".to_string(),
            event_type: "SeatsReserved".to_string(),
        };
        let event1 = ProjectionCompletionEvent::Completed(completion1);
        let data1 = bincode::serialize(&event1).expect("Failed to serialize");
        event_bus
            .publish(
                "projection.completed",
                &composable_rust_core::event::SerializedEvent::new(
                    "ProjectionCompleted".to_string(),
                    data1,
                    None,
                ),
            )
            .await
            .expect("Failed to publish");

        // Give time to process
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Should NOT be done yet (still waiting for sales_analytics)
        assert!(receiver.try_recv().is_err());

        // Publish second completion
        let completion2 = ProjectionCompleted {
            correlation_id,
            projection_name: "sales_analytics".to_string(),
            event_type: "SeatsReserved".to_string(),
        };
        let event2 = ProjectionCompletionEvent::Completed(completion2);
        let data2 = bincode::serialize(&event2).expect("Failed to serialize");
        event_bus
            .publish(
                "projection.completed",
                &composable_rust_core::event::SerializedEvent::new(
                    "ProjectionCompleted".to_string(),
                    data2,
                    None,
                ),
            )
            .await
            .expect("Failed to publish");

        // Now should be done
        let result = tokio::time::timeout(tokio::time::Duration::from_secs(1), receiver)
            .await
            .expect("Timeout waiting for completion")
            .expect("Channel closed");

        match result {
            ProjectionResult::Completed(completions) => {
                assert_eq!(completions.len(), 2);
            }
            ProjectionResult::Failed(_) => panic!("Expected completion, got failure"),
        }
    }

    #[tokio::test]
    async fn test_projection_failure() {
        let event_bus = Arc::new(InMemoryEventBus::new());
        let tracker = ProjectionCompletionTracker::new(event_bus.clone())
            .await
            .expect("Failed to create tracker");

        let correlation_id = CorrelationId::new();

        let receiver = tracker.register_interest(correlation_id, &["available_seats"]);

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Publish failure event
        let failure = ProjectionFailed {
            correlation_id,
            projection_name: "available_seats".to_string(),
            event_type: "SeatsReserved".to_string(),
            error: "Database connection failed".to_string(),
        };
        let event = ProjectionCompletionEvent::Failed(failure);
        let data = bincode::serialize(&event).expect("Failed to serialize");
        event_bus
            .publish(
                "projection.completed",
                &composable_rust_core::event::SerializedEvent::new(
                    "ProjectionFailed".to_string(),
                    data,
                    None,
                ),
            )
            .await
            .expect("Failed to publish");

        let result = tokio::time::timeout(tokio::time::Duration::from_secs(1), receiver)
            .await
            .expect("Timeout waiting for completion")
            .expect("Channel closed");

        match result {
            ProjectionResult::Failed(failures) => {
                assert_eq!(failures.len(), 1);
                assert_eq!(failures[0].projection_name, "available_seats");
                assert!(failures[0].error.contains("Database connection"));
            }
            ProjectionResult::Completed(_) => panic!("Expected failure, got completion"),
        }
    }
}
