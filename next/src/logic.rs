//! The unified `BusinessLogic` trait for aggregates and sagas

use crate::{BusinessResult, Clock, HandlerError, StreamId};
use serde::{Serialize, de::DeserializeOwned};

/// Unified trait for all business logic—aggregates and sagas alike
///
/// This trait abstracts over both aggregates and sagas:
///
/// - **Aggregates**: `Call = Infallible`, `CallResult = Infallible` (never used)
/// - **Sagas**: `Call = SagaAggregateCall`, `CallResult = SagaAggregateResult`
///
/// # CQRS Architecture
///
/// In this architecture, **validation state comes from projections**, not the event store.
/// The `process()` method receives a prepared input that already contains any fetched
/// projection data (via [`QueryFetcher`](crate::QueryFetcher)).
///
/// The event store is only used for:
/// 1. Appending events (with version check for optimistic concurrency)
/// 2. Rebuilding projections (via `apply()`)
///
/// # Associated Types
///
/// | Type | Description |
/// |------|-------------|
/// | `State` | Domain state for projection rebuilding (via `apply()`) |
/// | `Input` | Input with fetched projection data embedded |
/// | `Event` | Domain events to persist |
/// | `Error` | Business errors (validation failures, invalid state transitions) |
/// | `Call` | Aggregate calls (saga only). Use `Infallible` for aggregates. |
/// | `CallResult` | Results from aggregate calls (saga only). Use `Infallible` for aggregates. |
/// | `Response` | Query response type. Use `()` if no queries. |
///
/// # Required Methods
///
/// | Method | Purpose |
/// |--------|---------|
/// | `stream_id` | Extract stream ID from input for event store operations |
/// | `process` | Process input and return events (pure business logic) |
/// | `apply` | Apply an event to state (for projection rebuilding) |
/// | `event_type_name` | Return event type name for serialization tagging |
///
/// # Example: Aggregate
///
/// ```rust,ignore
/// use composable_rust_next::{BusinessLogic, BusinessResult, Clock, StreamId};
/// use std::convert::Infallible;
///
/// pub struct EventBusinessLogic;
///
/// impl BusinessLogic for EventBusinessLogic {
///     type State = EventState;
///     type Input = EventCommand;
///     type Event = EventEvent;
///     type Error = EventError;
///     type Call = Infallible;
///     type CallResult = Infallible;
///     type Response = EventResponse;
///
///     fn stream_id(input: &EventCommand) -> StreamId {
///         match input {
///             EventCommand::Create { event_id, .. } |
///             EventCommand::Publish { event_id, .. } => {
///                 StreamId::new(format!("event-{event_id}"))
///             }
///             // Queries don't need stream_id (returns dummy)
///             EventCommand::GetEvent { .. } => StreamId::new("query"),
///         }
///     }
///
///     fn process(
///         &self,
///         input: EventCommand,
///         clock: &dyn Clock,
///     ) -> Result<BusinessResult<EventEvent, Infallible, EventResponse>, EventError> {
///         match input {
///             EventCommand::Create { event_id, name, fetched } => {
///                 // Validate: entity shouldn't exist
///                 if fetched.is_some() {
///                     return Err(EventError::AlreadyExists);
///                 }
///                 Ok(BusinessResult::Done(vec![EventEvent::Created {
///                     event_id,
///                     name,
///                     created_at: clock.now(),
///                 }]))
///             }
///             EventCommand::Publish { event_id, fetched } => {
///                 // Validate: entity must exist and be in correct state
///                 let data = fetched.ok_or(EventError::NotFound)?;
///                 if data.status != EventStatus::Draft {
///                     return Err(EventError::InvalidState);
///                 }
///                 Ok(BusinessResult::Done(vec![EventEvent::Published {
///                     event_id,
///                     published_at: clock.now(),
///                 }]))
///             }
///             EventCommand::GetEvent { fetched, .. } => {
///                 // Query: just return the fetched data
///                 let data = fetched.ok_or(EventError::NotFound)?;
///                 Ok(BusinessResult::Respond(EventResponse::Single(data)))
///             }
///         }
///     }
///
///     fn apply(&self, state: &mut EventState, event: &EventEvent) {
///         // Used for projection rebuilding
///         match event {
///             EventEvent::Created { event_id, name, .. } => {
///                 state.event_id = Some(*event_id);
///                 state.name = name.clone();
///                 state.status = EventStatus::Draft;
///             }
///             EventEvent::Published { .. } => {
///                 state.status = EventStatus::Published;
///             }
///         }
///     }
///
///     fn event_type_name(event: &EventEvent) -> &'static str {
///         match event {
///             EventEvent::Created { .. } => "EventCreated",
///             EventEvent::Published { .. } => "EventPublished",
///         }
///     }
/// }
/// ```
///
/// # Example: Saga
///
/// ```rust,ignore
/// impl BusinessLogic for ReservationSagaLogic {
///     type State = SagaState;
///     type Input = SagaInput;  // Contains saga state from projection
///     type Event = SagaEvent;
///     type Error = SagaError;
///     type Call = SagaCall;
///     type CallResult = SagaCallResult;
///     type Response = ();
///
///     fn process(&self, input: SagaInput, clock: &dyn Clock)
///         -> Result<BusinessResult<SagaEvent, SagaCall, ()>, SagaError>
///     {
///         match input {
///             SagaInput::Start { saga_id, command, fetched } => {
///                 // New saga - fetched should be None
///                 if fetched.is_some() {
///                     return Err(SagaError::AlreadyExists);
///                 }
///                 Ok(BusinessResult::Continue {
///                     events: vec![SagaEvent::Initiated { saga_id, .. }],
///                     calls: vec![SagaCall::ReserveInventory { .. }],
///                 })
///             }
///             SagaInput::Feedback { saga_id, results, fetched } => {
///                 // Continuing saga - use fetched state
///                 let saga_state = fetched.ok_or(SagaError::NotFound)?;
///                 match (saga_state.phase, results) {
///                     // ... handle based on current phase and call results
///                 }
///             }
///         }
///     }
///
///     fn feedback_input_from(
///         prior: &SagaInput,
///         results: Vec<SagaCallResult>,
///     ) -> Result<SagaInput, HandlerError<SagaError>> {
///         Ok(SagaInput::Feedback { results, .. })
///     }
/// }
/// ```
pub trait BusinessLogic: Send + Sync + 'static {
    /// Domain state for projection rebuilding
    ///
    /// This is NOT used in the normal command flow. Validation data comes from
    /// projections via the prepared input.
    ///
    /// Used for:
    /// - Rebuilding projections from event store
    /// - Testing (replaying events)
    type State: Default + Send + Sync;

    /// Input to process (command with fetched projection data)
    ///
    /// The input should contain any data needed for validation, fetched from
    /// projections by [`QueryFetcher`](crate::QueryFetcher).
    ///
    /// For aggregates, this is typically the command enum with optional `fetched` field.
    /// For sagas, this is typically `Command | Feedback` with saga state from projection.
    type Input: Send;

    /// Domain events to persist
    ///
    /// These are the only things stored in the event store—pure business events.
    type Event: Serialize + DeserializeOwned + Send + Sync + Clone;

    /// Business errors (validation failures, invalid state transitions)
    ///
    /// Returning `Err` from `process` means nothing is persisted.
    type Error: std::error::Error + Send + Sync;

    /// Aggregate calls (saga only)
    ///
    /// Use `std::convert::Infallible` for aggregates since they never make calls.
    type Call: Send;

    /// Results from aggregate calls (saga only)
    ///
    /// Use `std::convert::Infallible` for aggregates since they never receive results.
    type CallResult: Send;

    /// Response type for queries
    ///
    /// This is the data returned from `BusinessResult::Respond` for read operations.
    /// Use `()` for aggregates/sagas that don't support queries.
    type Response: Send;

    /// Extract stream ID from input
    ///
    /// This determines which event stream to append to.
    /// The stream ID should be deterministic for a given input.
    ///
    /// For queries (which return `Respond`), this may return a dummy value
    /// since no events are persisted.
    ///
    /// # Examples
    ///
    /// - Aggregate: `event-{event_id}`, `order-{order_id}`
    /// - Saga: `saga-reservation-{saga_id}`
    fn stream_id(input: &Self::Input) -> StreamId;

    /// Process input and return what to do next
    ///
    /// This is pure business logic—deterministic, no I/O.
    ///
    /// # Validation Data
    ///
    /// The input contains fetched projection data for validation.
    /// Do NOT load state from the event store—use the embedded data.
    ///
    /// # Returns
    ///
    /// - `Ok(Done(events))` → Persist events and finish
    /// - `Ok(Continue { events, calls })` → Persist events, execute calls, continue (saga)
    /// - `Ok(Respond(data))` → Return query response without persistence
    /// - `Err(error)` → Fail without persisting anything
    ///
    /// # Errors
    ///
    /// Returns an error for business rule violations:
    /// - Validation failures (empty name, invalid format)
    /// - Invalid state transitions (can't publish a cancelled event)
    /// - Authorization failures (user doesn't have access)
    /// - Precondition failures (entity doesn't exist)
    ///
    /// # Clock Parameter
    ///
    /// The clock is passed as `&dyn Clock` for flexibility. The Handler has
    /// the full environment with concrete types, but business logic only needs
    /// the clock for timestamps. Dynamic dispatch here keeps the trait simple
    /// while allowing different clock implementations (`SystemClock` in production,
    /// `FixedClock` in tests).
    #[allow(clippy::type_complexity)]
    fn process(
        &self,
        input: Self::Input,
        clock: &dyn Clock,
    ) -> Result<BusinessResult<Self::Event, Self::Call, Self::Response>, Self::Error>;

    /// Apply an event to state (for projection rebuilding)
    ///
    /// This is used to rebuild state by replaying events from the event store.
    /// It is NOT used in the normal command flow.
    ///
    /// # Use Cases
    ///
    /// - Rebuilding projections from scratch
    /// - Creating new projections
    /// - Testing (replaying events to verify state)
    fn apply(&self, state: &mut Self::State, event: &Self::Event);

    /// Build the next saga input from the prior input plus the call results.
    ///
    /// The `prior` input is the command/feedback that produced the `Continue`, so it
    /// already carries the saga's typed correlation key (e.g. an aggregate id). Reading
    /// the key from `prior` is exact and avoids reverse-engineering it from
    /// [`CallResult`](Self::CallResult)s — which is fragile when a call *fails* (an error
    /// result may carry no id).
    ///
    /// Only called for logic that returns [`BusinessResult::Continue`] (sagas). The
    /// default returns [`HandlerError::FeedbackNotImplemented`] so that a saga which
    /// forgets to implement it fails cleanly instead of panicking the worker task.
    /// Aggregates and queries (`CallResult = Infallible`) never return `Continue`, so
    /// they never reach this method and need not override it.
    ///
    /// # Errors
    ///
    /// Returns [`HandlerError::FeedbackNotImplemented`] if a `Continue`-returning logic
    /// does not override this method.
    fn feedback_input_from(
        _prior: &Self::Input,
        _results: Vec<Self::CallResult>,
    ) -> Result<Self::Input, HandlerError<Self::Error>> {
        Err(HandlerError::FeedbackNotImplemented)
    }

    /// Event type name for serialization
    ///
    /// Returns a unique, stable string identifier for each event variant.
    /// This is used to tag serialized events for correct deserialization.
    ///
    /// # Stability
    ///
    /// These names are persisted in the event store. Once an event type name
    /// is used in production, it should never change. For schema evolution,
    /// create new event types (e.g., `EventCreatedV2`) rather than renaming.
    fn event_type_name(event: &Self::Event) -> &'static str;
}
