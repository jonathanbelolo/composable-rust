//! # Composable Rust Next
//!
//! Next-generation business logic framework for Composable Rust.
//!
//! This crate provides a clean separation between business logic and infrastructure,
//! designed as a compilation target for higher-level YAML-based specifications.
//!
//! ## Core Concepts
//!
//! - **[`BusinessLogic`]**: Unified trait for aggregates and sagas
//! - **[`BusinessResult`]**: Return type indicating done or continue with calls
//! - **[`Handler`]**: Infrastructure orchestration (load, persist, broadcast)
//! - **[`CallExecutor`]**: Trait for saga call dispatch to aggregates
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │  Handler (infrastructure, nearly identical across all)      │
//! │  ┌─────────────────────────────────────────────────────────┐│
//! │  │ 1. Load current state from event store                  ││
//! │  │ 2. Delegate to business logic                           ││
//! │  │ 3. Persist resulting events                             ││
//! │  │ 4. Broadcast for projections/sagas                      ││
//! │  └─────────────────────────────────────────────────────────┘│
//! │                           │                                 │
//! │                           ▼                                 │
//! │  ┌─────────────────────────────────────────────────────────┐│
//! │  │  BusinessLogic (domain-specific, pure)                   ││
//! │  │                                                         ││
//! │  │  process(state, input) → Result<BusinessResult, Error>  ││
//! │  │  apply(state, event) → mutate state                     ││
//! │  └─────────────────────────────────────────────────────────┘│
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Aggregates vs Sagas
//!
//! Both use the same [`BusinessLogic`] trait. The difference:
//!
//! - **Aggregates**: Always return `Done(events)`, use `Infallible` for `Call`/`CallResult`
//! - **Sagas**: Return `Continue { events, calls }` when orchestrating, `Done` when finished
//!
//! ## Example: Aggregate
//!
//! ```rust,ignore
//! use composable_rust_next::{BusinessLogic, BusinessResult};
//! use std::convert::Infallible;
//!
//! struct EventBusinessLogic;
//!
//! impl BusinessLogic for EventBusinessLogic {
//!     type State = EventState;
//!     type Input = EventCommand;
//!     type Event = EventEvent;
//!     type Error = EventError;
//!     type Call = Infallible;        // Aggregates never call
//!     type CallResult = Infallible;  // Aggregates never receive
//!
//!     fn stream_id(input: &Self::Input) -> StreamId { /* ... */ }
//!
//!     fn process(&self, state: &Self::State, input: Self::Input, clock: &dyn Clock)
//!         -> Result<BusinessResult<Self::Event, Self::Call>, Self::Error>
//!     {
//!         // Pure business logic - validate, decide, return events
//!         Ok(BusinessResult::Done(vec![/* events */]))
//!     }
//!
//!     fn apply(&self, state: &mut Self::State, event: &Self::Event) {
//!         // Pure state mutation
//!     }
//!
//!     fn event_type_name(event: &Self::Event) -> &'static str { /* ... */ }
//! }
//! ```

#![doc = include_str!("../README.md")]

mod clock;
mod error;
mod executor;
mod handler;
mod logic;
mod result;
mod stream;
mod version;

// Re-export core types from modules
pub use clock::{Clock, FixedClock, SystemClock};
pub use error::{HandlerError, ProjectionError, SerializationError};
pub use executor::{CallExecutor, NoOpCallExecutor};
pub use handler::{HandleResult, Handler};
pub use logic::BusinessLogic;
pub use result::BusinessResult;
pub use stream::StreamId;
pub use version::Version;

// Note: The following traits and types are defined below in this file
// and are automatically public:
// - EventStore, EventBus, Projector (infrastructure traits)
// - HandlerEnvironment (environment trait)
// - SerializedEvent, EventMetadata (event types)
// - EventStoreError, EventBusError (error types)

// ═══════════════════════════════════════════════════════════════════
// Infrastructure Traits
// ═══════════════════════════════════════════════════════════════════

/// Event store trait for persistence
///
/// This trait provides loading and appending events to streams.
/// Implementations are provided by infrastructure crates (e.g., `composable-rust-postgres`).
///
/// # Async Methods
///
/// This trait uses native async fn in traits (Rust 2024).
/// Implementations must be `Send + Sync`.
pub trait EventStore: Send + Sync {
    /// Load events from a stream
    ///
    /// # Errors
    ///
    /// Returns an error if the event store is unavailable or the stream cannot be read.
    fn load(
        &self,
        stream_id: &StreamId,
        from_version: Option<Version>,
    ) -> impl std::future::Future<Output = Result<Vec<SerializedEvent>, EventStoreError>> + Send;

    /// Append events to a stream with optimistic concurrency
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The expected version doesn't match (concurrency conflict)
    /// - The event store is unavailable
    /// - Serialization fails
    fn append(
        &self,
        stream_id: &StreamId,
        expected_version: Option<Version>,
        events: Vec<SerializedEvent>,
    ) -> impl std::future::Future<Output = Result<Version, EventStoreError>> + Send;
}

/// Event bus trait for broadcasting events
///
/// This trait provides event publishing for cross-aggregate communication.
/// Implementations are provided by infrastructure crates (e.g., `composable-rust-redpanda`).
pub trait EventBus: Send + Sync {
    /// Publish an event to a topic
    ///
    /// # Errors
    ///
    /// Returns an error if the event bus is unavailable or publishing fails.
    fn publish(
        &self,
        topic: &str,
        event: SerializedEvent,
    ) -> impl std::future::Future<Output = Result<(), EventBusError>> + Send;
}

/// Projector trait for updating read models
///
/// This trait enables synchronous projection completion. When the [`Handler`] calls
/// `project()`, it waits for the projection to complete before returning. This ensures
/// that when `handle()` returns, the read model is updated.
///
/// # Coordination Guarantee
///
/// Unlike async event bus subscribers, projectors are called synchronously within
/// the handler loop. This provides strong consistency for the read model.
pub trait Projector: Send + Sync {
    /// Project events to the read model and wait for completion
    ///
    /// This method blocks until the projection is fully applied.
    /// When it returns `Ok(())`, the caller knows the read model is updated.
    ///
    /// # Errors
    ///
    /// Returns `ProjectionError` if the projection fails (database error, etc.)
    fn project(
        &self,
        events: &[SerializedEvent],
    ) -> impl std::future::Future<Output = Result<(), ProjectionError>> + Send;
}

// ═══════════════════════════════════════════════════════════════════
// Handler Environment
// ═══════════════════════════════════════════════════════════════════

/// Core infrastructure dependencies required by the [`Handler`]
///
/// Each aggregate/saga defines its own Environment struct with domain-specific
/// dependencies and implements this trait to expose the infrastructure the Handler needs.
///
/// # Design Pattern
///
/// This trait uses associated types for infrastructure dependencies, enabling
/// static dispatch and avoiding dyn compatibility issues with async traits.
/// Each environment specifies concrete types for `EventStore`, `Projector`, and `EventBus`.
///
/// # Example
///
/// ```rust,ignore
/// pub struct EventEnvironment<ES, P, EB> {
///     clock: SystemClock,
///     event_store: ES,
///     projector: P,
///     event_bus: Option<EB>,
///     broadcast_topic: String,
/// }
///
/// impl<ES, P, EB> HandlerEnvironment for EventEnvironment<ES, P, EB>
/// where
///     ES: EventStore,
///     P: Projector,
///     EB: EventBus,
/// {
///     type EventStore = ES;
///     type Projector = P;
///     type EventBus = EB;
///
///     fn clock(&self) -> &dyn Clock { &self.clock }
///     fn event_store(&self) -> &Self::EventStore { &self.event_store }
///     fn projector(&self) -> Option<&Self::Projector> { Some(&self.projector) }
///     fn event_bus(&self) -> Option<&Self::EventBus> { self.event_bus.as_ref() }
///     fn broadcast_topic(&self) -> &str { &self.broadcast_topic }
/// }
/// ```
pub trait HandlerEnvironment: Send + Sync {
    /// The event store type
    type EventStore: EventStore;

    /// The projector type (for updating read models)
    type Projector: Projector;

    /// The event bus type (for broadcasting events)
    type EventBus: EventBus;

    /// Clock for timestamps (used by business logic)
    fn clock(&self) -> &dyn Clock;

    /// Event store for loading and persisting events
    fn event_store(&self) -> &Self::EventStore;

    /// Projector for updating read models and waiting for completion
    ///
    /// Returns `None` if this aggregate doesn't use projections.
    fn projector(&self) -> Option<&Self::Projector>;

    /// Event bus for broadcasting events to other aggregates/sagas
    ///
    /// Returns `None` if this aggregate doesn't broadcast events.
    fn event_bus(&self) -> Option<&Self::EventBus>;

    /// Topic for broadcasting events (only used if `event_bus()` returns `Some`)
    fn broadcast_topic(&self) -> &str;
}

/// Serialized event for persistence and transport
#[derive(Clone, Debug)]
pub struct SerializedEvent {
    /// Event type discriminator (e.g., `EventCreated`, `OrderPlaced`)
    pub event_type: String,

    /// Bincode-serialized event payload
    pub payload: Vec<u8>,

    /// Optional metadata (correlation ID, causation ID, etc.)
    pub metadata: Option<EventMetadata>,

    /// Event version in the stream (set by event store on load)
    pub version: Option<Version>,
}

/// Event metadata for correlation and auditing
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct EventMetadata {
    /// Correlation ID for tracing across services
    pub correlation_id: Option<String>,

    /// Causation ID linking to the causing event
    pub causation_id: Option<String>,

    /// User who triggered the action
    pub user_id: Option<String>,

    /// Timestamp when the event was created
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Error from event store operations
#[derive(Debug, Clone, thiserror::Error)]
pub enum EventStoreError {
    /// Version conflict during append
    #[error("version conflict: expected {expected:?}, found {actual:?}")]
    VersionConflict {
        /// Expected version
        expected: Option<Version>,
        /// Actual version in store
        actual: Version,
    },

    /// Stream not found
    #[error("stream not found: {0}")]
    StreamNotFound(String),

    /// Connection error
    #[error("connection error: {0}")]
    Connection(String),

    /// Serialization error
    #[error("serialization error: {0}")]
    Serialization(String),
}

/// Error from event bus operations
#[derive(Debug, Clone, thiserror::Error)]
pub enum EventBusError {
    /// Connection error
    #[error("connection error: {0}")]
    Connection(String),

    /// Serialization error
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Topic not found
    #[error("topic not found: {0}")]
    TopicNotFound(String),
}
