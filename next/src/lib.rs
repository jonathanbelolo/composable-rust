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
mod fetcher;
mod handler;
mod in_process_event_bus;
mod logic;
mod multi_projector;
mod projections;
mod result;
mod stream;
pub mod testing;
mod version;

// Re-export core types from modules
pub use clock::{Clock, FixedClock, SystemClock};
pub use error::{HandlerError, ProjectionError, SerializationError};
pub use executor::{CallExecutor, NoOpCallExecutor};
pub use fetcher::{FetchResult, NoOpQueryFetcher, QueryFetcher};
pub use handler::{
    HandleResult, Handler, HandlerBuilder, DEFAULT_MAX_RETRIES, DEFAULT_MAX_SAGA_ITERATIONS,
};
pub use in_process_event_bus::{InProcessEventBus, DEFAULT_CHANNEL_CAPACITY};
pub use logic::BusinessLogic;
pub use multi_projector::{DynProjector, MultiProjector};
pub use projections::{GetById, NoOpProjectionQueries, ProjectionQueries};
pub use result::BusinessResult;
pub use stream::StreamId;
pub use version::Version;

// Note: The following traits and types are defined below in this file
// and are automatically public:
// - EventStore, EventBus, Projector (infrastructure traits)
// - HandlerEnvironment (environment trait)
// - SerializedEvent, EventMetadata, MetadataContext (event types)
// - EventStoreError, EventBusError (error types)
// - SubscriptionHandle (event bus subscription management)

// ═══════════════════════════════════════════════════════════════════
// Infrastructure Traits
// ═══════════════════════════════════════════════════════════════════

/// Event store trait for persistence
///
/// This trait provides loading and appending events to streams.
/// Implementations are provided by infrastructure crates (e.g., `composable-rust-postgres`).
///
/// # CQRS Note
///
/// In normal operation, the event store is used for:
/// 1. **Appending events** (with version check for optimistic concurrency)
/// 2. **Rebuilding projections** (disaster recovery, new projections)
///
/// It is NOT used for reading validation state during command handling.
/// Validation data comes from projections via [`QueryFetcher`].
///
/// # Async Methods
///
/// This trait uses native async fn in traits (Rust 2024).
/// Implementations must be `Send + Sync`.
pub trait EventStore: Send + Sync {
    /// Load events from a stream (for projection rebuilding)
    ///
    /// This is typically used for:
    /// - Rebuilding projections from scratch
    /// - Debugging and auditing
    /// - Testing
    ///
    /// **NOT for command validation** - use projections instead.
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
///
/// # Batch Publishing
///
/// Use [`publish_batch`](Self::publish_batch) for efficiency when publishing multiple events.
/// The default implementation calls `publish` for each event, but implementations
/// should override this for better performance (single round-trip to Kafka/Redpanda).
pub trait EventBus: Send + Sync {
    /// Publish a single event to a topic
    ///
    /// # Errors
    ///
    /// Returns an error if the event bus is unavailable or publishing fails.
    fn publish(
        &self,
        topic: &str,
        event: SerializedEvent,
    ) -> impl std::future::Future<Output = Result<(), EventBusError>> + Send;

    /// Publish multiple events to a topic in a single batch
    ///
    /// This is more efficient than calling `publish` multiple times,
    /// as it can use a single network round-trip.
    ///
    /// The default implementation calls `publish` for each event sequentially.
    /// Implementations should override this for better performance.
    ///
    /// # Errors
    ///
    /// Returns an error if the event bus is unavailable or publishing fails.
    /// On failure, some events may have been published.
    fn publish_batch(
        &self,
        topic: &str,
        events: &[SerializedEvent],
    ) -> impl std::future::Future<Output = Result<(), EventBusError>> + Send {
        async move {
            for event in events {
                self.publish(topic, event.clone()).await?;
            }
            Ok(())
        }
    }

    /// Subscribe to events on a topic
    ///
    /// The handler is called for each event published to the topic.
    /// Returns a [`SubscriptionHandle`] that can be used to cancel the subscription.
    ///
    /// # Handler
    ///
    /// The handler receives each event and returns a future that resolves to
    /// `Ok(())` on success or an error. Errors are logged but do not stop
    /// the subscription.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use futures::future::BoxFuture;
    ///
    /// let handle = event_bus.subscribe("orders", |event| {
    ///     Box::pin(async move {
    ///         println!("Received event: {}", event.event_type);
    ///         Ok(())
    ///     })
    /// }).await?;
    ///
    /// // Subscription runs until handle is dropped or cancelled
    /// handle.cancel();
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the subscription cannot be created.
    fn subscribe<F>(
        &self,
        topic: &str,
        handler: F,
    ) -> impl std::future::Future<Output = Result<SubscriptionHandle, EventBusError>> + Send
    where
        F: Fn(SerializedEvent) -> futures::future::BoxFuture<'static, Result<(), EventBusError>>
            + Send
            + Sync
            + 'static;
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

/// Per-event projection trait for read model maintenance.
///
/// Implementations receive deserialized domain events and update
/// their read model (typically a `PostgreSQL` table). Each projection
/// has a unique name for checkpoint tracking.
///
/// This is a higher-level abstraction than `Projector` — it works
/// with typed domain events instead of raw `SerializedEvent`s.
pub trait Projection<E: Send + Sync>: Send + Sync {
    /// Unique name for this projection (used for checkpoint tracking).
    fn name(&self) -> &'static str;

    /// Apply a single domain event to the read model.
    ///
    /// # Errors
    ///
    /// Returns `ProjectionError` if the database update fails.
    fn apply_event(
        &mut self,
        event: &E,
    ) -> impl std::future::Future<Output = Result<(), ProjectionError>> + Send;
}

// ═══════════════════════════════════════════════════════════════════
// Handler Environment
// ═══════════════════════════════════════════════════════════════════

/// Metadata context for event correlation and auditing
///
/// This struct provides metadata that is attached to events during persistence.
/// It enables distributed tracing, audit logging, and debugging.
///
/// # Example
///
/// ```rust,ignore
/// let metadata = MetadataContext::new()
///     .with_correlation_id("req-12345")
///     .with_user_id("user-abc");
///
/// let env = MyEnvironment::new(...)
///     .with_metadata(metadata);
/// ```
#[derive(Debug, Clone, Default)]
pub struct MetadataContext {
    /// Correlation ID for tracing across services
    ///
    /// Typically propagated from HTTP headers (e.g., `X-Correlation-ID`).
    pub correlation_id: Option<String>,

    /// Causation ID linking to the event that caused this action
    ///
    /// For sagas, this links back to the triggering event.
    pub causation_id: Option<String>,

    /// User who triggered the action
    ///
    /// Extracted from authentication context (JWT, session, etc.).
    pub user_id: Option<String>,
}

impl MetadataContext {
    /// Create an empty metadata context
    #[must_use]
    pub const fn new() -> Self {
        Self {
            correlation_id: None,
            causation_id: None,
            user_id: None,
        }
    }

    /// Set the correlation ID
    #[must_use]
    pub fn with_correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    /// Set the causation ID
    #[must_use]
    pub fn with_causation_id(mut self, id: impl Into<String>) -> Self {
        self.causation_id = Some(id.into());
        self
    }

    /// Set the user ID
    #[must_use]
    pub fn with_user_id(mut self, id: impl Into<String>) -> Self {
        self.user_id = Some(id.into());
        self
    }

    /// Create [`EventMetadata`] with the current timestamp
    #[must_use]
    pub fn to_event_metadata(&self) -> EventMetadata {
        EventMetadata {
            correlation_id: self.correlation_id.clone(),
            causation_id: self.causation_id.clone(),
            user_id: self.user_id.clone(),
            timestamp: chrono::Utc::now(),
        }
    }
}

/// Core infrastructure dependencies required by the [`Handler`]
///
/// Each aggregate/saga defines its own Environment struct with domain-specific
/// dependencies and implements this trait to expose the infrastructure the Handler needs.
///
/// # Design Pattern
///
/// This trait uses associated types for ALL infrastructure dependencies, enabling
/// static dispatch throughout. This includes Clock, which was previously dynamic.
///
/// # Example
///
/// ```rust,ignore
/// pub struct EventEnvironment<C, ES, P, EB, PQ> {
///     clock: C,
///     event_store: ES,
///     projector: P,
///     event_bus: Option<EB>,
///     projections: PQ,
///     broadcast_topic: String,
///     metadata: MetadataContext,
/// }
///
/// impl<C, ES, P, EB, PQ> HandlerEnvironment for EventEnvironment<C, ES, P, EB, PQ>
/// where
///     C: Clock,
///     ES: EventStore,
///     P: Projector,
///     EB: EventBus,
///     PQ: ProjectionQueries,
/// {
///     type Clock = C;
///     type EventStore = ES;
///     type Projector = P;
///     type EventBus = EB;
///     type Projections = PQ;
///
///     fn clock(&self) -> &Self::Clock { &self.clock }
///     fn event_store(&self) -> &Self::EventStore { &self.event_store }
///     fn projector(&self) -> Option<&Self::Projector> { Some(&self.projector) }
///     fn event_bus(&self) -> Option<&Self::EventBus> { self.event_bus.as_ref() }
///     fn broadcast_topic(&self) -> &str { &self.broadcast_topic }
///     fn projections(&self) -> &Self::Projections { &self.projections }
///     fn metadata(&self) -> &MetadataContext { &self.metadata }
/// }
/// ```
pub trait HandlerEnvironment: Send + Sync {
    /// The clock type (for timestamps)
    ///
    /// Use [`SystemClock`] in production, [`FixedClock`] in tests.
    type Clock: Clock;

    /// The event store type
    type EventStore: EventStore;

    /// The projector type (for updating read models)
    type Projector: Projector;

    /// The event bus type (for broadcasting events)
    type EventBus: EventBus;

    /// The projection queries type (for reading from projections)
    ///
    /// Use [`NoOpProjectionQueries`] for aggregates that don't support queries.
    type Projections: ProjectionQueries;

    /// Clock for timestamps
    fn clock(&self) -> &Self::Clock;

    /// Event store for appending events
    ///
    /// In CQRS, this is NOT used for reading validation state.
    /// Use projections for that.
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

    /// Projection queries for reading from read models
    ///
    /// This is the primary source of validation data for commands.
    fn projections(&self) -> &Self::Projections;

    /// Metadata context for event correlation
    ///
    /// Provides correlation IDs, user IDs, etc. for event metadata.
    fn metadata(&self) -> &MetadataContext;
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

    /// Subscription error
    #[error("subscription error: {0}")]
    Subscription(String),
}

/// Handle to cancel a subscription
///
/// When dropped, the subscription is automatically cancelled.
/// You can also explicitly cancel by calling [`cancel()`](Self::cancel).
///
/// # Example
///
/// ```rust,ignore
/// let handle = event_bus.subscribe("orders", |event| {
///     Box::pin(async move {
///         println!("Received: {:?}", event);
///         Ok(())
///     })
/// }).await?;
///
/// // Later, cancel the subscription
/// handle.cancel();
/// ```
pub struct SubscriptionHandle {
    /// Sender to signal cancellation
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
}

impl SubscriptionHandle {
    /// Create a new subscription handle with the given cancel sender.
    #[must_use]
    pub const fn new(cancel: tokio::sync::oneshot::Sender<()>) -> Self {
        Self {
            cancel: Some(cancel),
        }
    }

    /// Explicitly cancel the subscription.
    ///
    /// This consumes the sender, signaling the subscription task to stop.
    /// If already cancelled or dropped, this is a no-op.
    pub fn cancel(mut self) {
        if let Some(sender) = self.cancel.take() {
            // Send cancellation signal; ignore error if receiver already dropped
            let _ = sender.send(());
        }
    }

    /// Check if the subscription is still active.
    ///
    /// Returns `false` if `cancel()` has been called or the handle was dropped.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.cancel.is_some()
    }
}

impl Drop for SubscriptionHandle {
    fn drop(&mut self) {
        if let Some(sender) = self.cancel.take() {
            // Automatic cancellation on drop
            let _ = sender.send(());
        }
    }
}

impl std::fmt::Debug for SubscriptionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionHandle")
            .field("active", &self.is_active())
            .finish()
    }
}
