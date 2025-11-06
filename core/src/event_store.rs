//! Event store trait and related types for event sourcing.
//!
//! This module defines the core abstraction for an event store - a specialized database
//! optimized for storing and retrieving event streams with optimistic concurrency control.
//!
//! # Design
//!
//! The `EventStore` trait is deliberately minimal and focused. It provides exactly what's
//! needed for event sourcing:
//!
//! - Append events to a stream with optimistic concurrency
//! - Load events from a stream for state reconstruction
//! - Save and load state snapshots for performance
//!
//! # Implementations
//!
//! - `PostgresEventStore` (in `composable-rust-postgres` crate): Production implementation
//! - `InMemoryEventStore` (in `composable-rust-testing` crate): Fast, deterministic testing
//!
//! # Example
//!
//! ```no_run
//! use composable_rust_core::event_store::{EventStore, EventStoreError};
//! use composable_rust_core::stream::{StreamId, Version};
//! use composable_rust_core::event::SerializedEvent;
//!
//! async fn example<E: EventStore>(store: &E) -> Result<(), EventStoreError> {
//!     let stream_id = StreamId::new("order-123");
//!
//!     // Append events with optimistic concurrency
//!     let events = vec![/* ... */];
//!     let new_version = store.append_events(
//!         stream_id.clone(),
//!         Some(Version::new(0)),  // Expected current version
//!         events,
//!     ).await?;
//!
//!     // Load events to reconstruct state
//!     let all_events = store.load_events(stream_id, None).await?;
//!
//!     Ok(())
//! }
//! ```

use crate::event::SerializedEvent;
use crate::stream::{StreamId, Version};
use std::future::Future;
use std::pin::Pin;
use thiserror::Error;

/// Type alias for snapshot data: `(Version, Vec<u8>)`
type SnapshotData = (Version, Vec<u8>);

/// Errors that can occur during event store operations.
#[derive(Error, Debug)]
pub enum EventStoreError {
    /// Optimistic concurrency conflict: expected version doesn't match current version.
    ///
    /// This error occurs when trying to append events with an expected version that
    /// doesn't match the stream's current version. This typically means another process
    /// has modified the stream concurrently.
    #[error("Concurrency conflict: expected version {expected}, found {actual}")]
    ConcurrencyConflict {
        /// The stream ID where the conflict occurred.
        stream_id: StreamId,
        /// The version we expected the stream to be at.
        expected: Version,
        /// The actual current version of the stream.
        actual: Version,
    },

    /// Stream not found in the event store.
    #[error("Stream not found: {0}")]
    StreamNotFound(StreamId),

    /// Database connection error.
    #[error("Database error: {0}")]
    DatabaseError(String),

    /// Serialization/deserialization error.
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// General I/O error.
    #[error("I/O error: {0}")]
    IoError(String),
}

/// Event store abstraction for storing and retrieving event streams.
///
/// An event store is a specialized database optimized for:
///
/// - Appending events to streams (immutable, append-only)
/// - Loading events for state reconstruction
/// - Optimistic concurrency control
/// - Snapshot support for performance
///
/// # Thread Safety
///
/// Implementations must be `Send + Sync` to be safely used in async contexts
/// and shared across threads.
///
/// # Implementations
///
/// Two main implementations are provided:
///
/// - **`PostgresEventStore`** (production): Uses `PostgreSQL` for durable storage
/// - **`InMemoryEventStore`** (testing): Uses `HashMap` for fast, deterministic tests
///
/// # Design Philosophy
///
/// The event store is deliberately simple and focused. It does NOT provide:
/// - Event projection management (that's the application's job)
/// - Subscription mechanisms (use event bus for that - Phase 3)
/// - Complex querying (events are accessed by stream ID only)
///
/// This keeps the event store focused on its core responsibility: reliable event persistence.
///
/// # Dyn Compatibility
///
/// This trait uses explicit `Pin<Box<dyn Future>>` returns instead of `async fn`
/// to enable trait object usage (`Arc<dyn EventStore>`). This is required for
/// the effect system where reducers create effects that capture the event store.
pub trait EventStore: Send + Sync {
    /// Append events to a stream with optimistic concurrency control.
    ///
    /// # Parameters
    ///
    /// - `stream_id`: The stream to append events to
    /// - `expected_version`: Optional version for optimistic concurrency control
    /// - `events`: Events to append (consumed/moved - they will be persisted)
    ///
    /// # Optimistic Concurrency
    ///
    /// The `expected_version` parameter implements optimistic concurrency control:
    ///
    /// - `Some(version)`: Assert the stream is currently at this version
    /// - `None`: Append to any stream (no version check, use with caution)
    ///
    /// If the stream's current version doesn't match `expected_version`, returns
    /// `EventStoreError::ConcurrencyConflict`.
    ///
    /// # Returns
    ///
    /// Returns the new version after appending events. For example, if the stream
    /// was at version 5 and you append 3 events, returns `Version(8)`.
    ///
    /// # Errors
    ///
    /// - `ConcurrencyConflict`: Version mismatch (concurrent modification detected)
    /// - `DatabaseError`: Database connection or query failed
    /// - `SerializationError`: Failed to serialize events
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use composable_rust_core::event_store::EventStore;
    /// use composable_rust_core::stream::{StreamId, Version};
    /// use composable_rust_core::event::SerializedEvent;
    ///
    /// async fn append_example<E: EventStore>(store: &E) -> Result<(), Box<dyn std::error::Error>> {
    ///     let stream_id = StreamId::new("order-123");
    ///     let events = vec![/* events */];
    ///
    ///     // First append to new stream
    ///     let v1 = store.append_events(stream_id.clone(), Some(Version::new(0)), events.clone()).await?;
    ///
    ///     // Subsequent append requires correct version
    ///     let v2 = store.append_events(stream_id, Some(v1), events).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    fn append_events(
        &self,
        stream_id: StreamId,
        expected_version: Option<Version>,
        events: Vec<SerializedEvent>,
    ) -> Pin<Box<dyn Future<Output = Result<Version, EventStoreError>> + Send + '_>>;

    /// Load events from a stream.
    ///
    /// # Parameters
    ///
    /// - `stream_id`: The stream to load events from
    /// - `from_version`: Optional starting version
    ///   - `Some(version)`: Load events from this version onwards (inclusive)
    ///   - `None`: Load all events from the beginning
    ///
    /// # Returns
    ///
    /// Returns events ordered by version (oldest first). If the stream doesn't exist,
    /// returns an empty vector (not an error - new streams start empty).
    ///
    /// # Errors
    ///
    /// - `DatabaseError`: Database connection or query failed
    /// - `SerializationError`: Failed to deserialize events
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use composable_rust_core::event_store::EventStore;
    /// use composable_rust_core::stream::{StreamId, Version};
    ///
    /// async fn load_example<E: EventStore>(store: &E) -> Result<(), Box<dyn std::error::Error>> {
    ///     let stream_id = StreamId::new("order-123");
    ///
    ///     // Load all events
    ///     let all_events = store.load_events(stream_id.clone(), None).await?;
    ///
    ///     // Load events from version 10 onwards (for snapshot + replay)
    ///     let recent_events = store.load_events(stream_id, Some(Version::new(10))).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    fn load_events(
        &self,
        stream_id: StreamId,
        from_version: Option<Version>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SerializedEvent>, EventStoreError>> + Send + '_>>;

    /// Save a snapshot of aggregate state.
    ///
    /// Snapshots allow rebuilding aggregate state without replaying all events.
    /// The snapshot captures the state at a specific version.
    ///
    /// # Strategy
    ///
    /// Typical snapshot strategy:
    /// - Save a snapshot every N events (e.g., every 100 events)
    /// - When loading state: load latest snapshot + replay events since snapshot
    /// - Snapshots are optional (can always replay from start)
    ///
    /// # Parameters
    ///
    /// - `stream_id`: The stream this snapshot belongs to
    /// - `version`: The version of the stream at the time of this snapshot
    /// - `state`: The bincode-serialized aggregate state
    ///
    /// # Errors
    ///
    /// - `DatabaseError`: Database connection or query failed
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use composable_rust_core::event_store::EventStore;
    /// use composable_rust_core::stream::{StreamId, Version};
    ///
    /// async fn snapshot_example<E: EventStore>(
    ///     store: &E,
    ///     state_bytes: Vec<u8>
    /// ) -> Result<(), Box<dyn std::error::Error>> {
    ///     let stream_id = StreamId::new("order-123");
    ///     let version = Version::new(100);
    ///
    ///     store.save_snapshot(stream_id, version, state_bytes).await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    fn save_snapshot(
        &self,
        stream_id: StreamId,
        version: Version,
        state: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<(), EventStoreError>> + Send + '_>>;

    /// Load the latest snapshot for a stream.
    ///
    /// # Returns
    ///
    /// - `Some((version, state))`: Latest snapshot found
    /// - `None`: No snapshot exists for this stream
    ///
    /// The returned version indicates which events have been included in the snapshot.
    /// To fully reconstruct state, load events from this version onwards.
    ///
    /// # Errors
    ///
    /// - `DatabaseError`: Database connection or query failed
    /// - `SerializationError`: Failed to deserialize snapshot
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use composable_rust_core::event_store::EventStore;
    /// use composable_rust_core::stream::{StreamId, Version};
    ///
    /// async fn load_with_snapshot<E: EventStore>(
    ///     store: &E
    /// ) -> Result<(), Box<dyn std::error::Error>> {
    ///     let stream_id = StreamId::new("order-123");
    ///
    ///     // Try to load snapshot
    ///     let state = if let Some((snapshot_version, snapshot_data)) =
    ///         store.load_snapshot(stream_id.clone()).await?
    ///     {
    ///         // Rebuild from snapshot
    ///         let mut state = deserialize_state(&snapshot_data)?;
    ///
    ///         // Replay events since snapshot
    ///         let events = store.load_events(stream_id, Some(snapshot_version.next())).await?;
    ///         for event in events {
    ///             // Apply events to state
    ///         }
    ///         state
    ///     } else {
    ///         // No snapshot, replay all events
    ///         let events = store.load_events(stream_id, None).await?;
    ///         let mut state = Default::default();
    ///         for event in events {
    ///             // Apply events to state
    ///         }
    ///         state
    ///     };
    ///
    ///     Ok(())
    /// }
    ///
    /// # fn deserialize_state(_: &[u8]) -> Result<(), Box<dyn std::error::Error>> { Ok(()) }
    /// ```
    fn load_snapshot(
        &self,
        stream_id: StreamId,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SnapshotData>, EventStoreError>> + Send + '_>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrency_conflict_error_display() {
        let error = EventStoreError::ConcurrencyConflict {
            stream_id: StreamId::new("test-stream"),
            expected: Version::new(5),
            actual: Version::new(7),
        };

        let display = format!("{error}");
        assert!(display.contains("expected version 5"));
        assert!(display.contains("found 7"));
    }

    #[test]
    fn stream_not_found_error_display() {
        let error = EventStoreError::StreamNotFound(StreamId::new("missing-stream"));
        let display = format!("{error}");
        assert!(display.contains("missing-stream"));
    }
}
