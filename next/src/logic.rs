//! The unified `BusinessLogic` trait for aggregates and sagas

use crate::{BusinessResult, Clock, StreamId};
use serde::{de::DeserializeOwned, Serialize};

/// Unified trait for all business logic—aggregates and sagas alike
///
/// This trait abstracts over both aggregates and sagas:
///
/// - **Aggregates**: `Call = Infallible`, `CallResult = Infallible` (never used)
/// - **Sagas**: `Call = SagaAggregateCall`, `CallResult = SagaAggregateResult`
///
/// The framework doesn't know or care which one it's running—the types encode
/// the distinction.
///
/// # Associated Types
///
/// | Type | Description |
/// |------|-------------|
/// | `State` | Domain state, reconstructed by replaying events |
/// | `Input` | Input to process (command for aggregates, command-or-feedback for sagas) |
/// | `Event` | Domain events to persist |
/// | `Error` | Business errors (validation failures, invalid state transitions) |
/// | `Call` | Aggregate calls (saga only). Use `Infallible` for aggregates. |
/// | `CallResult` | Results from aggregate calls (saga only). Use `Infallible` for aggregates. |
///
/// # Required Methods
///
/// | Method | Purpose |
/// |--------|---------|
/// | `stream_id` | Extract stream ID from input for loading/persisting |
/// | `process` | Process input and return events (and optionally calls) |
/// | `apply` | Apply an event to state for replay/reconstruction |
/// | `event_type_name` | Return event type name for serialization tagging |
///
/// # Optional Methods
///
/// | Method | Default | Purpose |
/// |--------|---------|---------|
/// | `feedback_input` | `unreachable!()` | Convert call results to input (saga only) |
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
///     type Call = Infallible;        // Aggregates never make calls
///     type CallResult = Infallible;  // Aggregates never receive call results
///
///     fn stream_id(input: &EventCommand) -> StreamId {
///         match input {
///             EventCommand::Create { event_id, .. } |
///             EventCommand::Publish { event_id } |
///             EventCommand::Cancel { event_id, .. } => {
///                 StreamId::new(format!("event-{}", event_id))
///             }
///         }
///     }
///
///     fn process(
///         &self,
///         state: &EventState,
///         command: EventCommand,
///         clock: &dyn Clock,
///     ) -> Result<BusinessResult<EventEvent, Infallible>, EventError> {
///         match command {
///             EventCommand::Create { event_id, name, .. } => {
///                 if state.event_id.is_some() {
///                     return Err(EventError::AlreadyExists);
///                 }
///                 Ok(BusinessResult::Done(vec![EventEvent::Created {
///                     event_id,
///                     name,
///                     created_at: clock.now(),
///                 }]))
///             }
///             // ... other commands
///         }
///     }
///
///     fn apply(&self, state: &mut EventState, event: &EventEvent) {
///         match event {
///             EventEvent::Created { event_id, name, .. } => {
///                 state.event_id = Some(*event_id);
///                 state.name = name.clone();
///                 state.status = EventStatus::Draft;
///             }
///             // ... other events
///         }
///     }
///
///     fn event_type_name(event: &EventEvent) -> &'static str {
///         match event {
///             EventEvent::Created { .. } => "EventCreated",
///             EventEvent::Published { .. } => "EventPublished",
///             EventEvent::Cancelled { .. } => "EventCancelled",
///         }
///     }
///
///     // Note: feedback_input uses default impl (unreachable) since aggregates
///     // never return Continue and thus never receive feedback.
/// }
/// ```
///
/// # Example: Saga
///
/// ```rust,ignore
/// impl BusinessLogic for OrderSagaLogic {
///     type State = SagaState;
///     type Input = SagaInput;  // Command | Feedback
///     type Event = SagaEvent;
///     type Error = SagaError;
///     type Call = SagaCall;           // Calls to other aggregates
///     type CallResult = SagaCallResult;  // Results from those calls
///
///     fn process(&self, state: &SagaState, input: SagaInput, clock: &dyn Clock)
///         -> Result<BusinessResult<SagaEvent, SagaCall>, SagaError>
///     {
///         match (&state.phase, input) {
///             (Phase::Initial, SagaInput::Command(cmd)) => {
///                 Ok(BusinessResult::Continue {
///                     events: vec![SagaEvent::Initiated { .. }],
///                     calls: vec![SagaCall::CreateOrder { .. }],
///                 })
///             }
///             (Phase::WaitingForOrder, SagaInput::Feedback(results)) => {
///                 // Check results, possibly continue or complete
///                 Ok(BusinessResult::Done(vec![SagaEvent::Completed { .. }]))
///             }
///             // ...
///         }
///     }
///
///     fn feedback_input(results: Vec<SagaCallResult>) -> SagaInput {
///         SagaInput::Feedback(results)
///     }
///
///     // ... other methods
/// }
/// ```
pub trait BusinessLogic: Send + Sync + 'static {
    /// Domain state, reconstructed by replaying events
    ///
    /// Must implement `Default` for initial state when no events exist.
    type State: Default + Send + Sync;

    /// Input to process
    ///
    /// For aggregates, this is typically the command enum.
    /// For sagas, this is typically `Command | Feedback`.
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

    /// Extract stream ID from input
    ///
    /// This determines which event stream to load and append to.
    /// The stream ID should be deterministic for a given input.
    ///
    /// # Examples
    ///
    /// - Aggregate: `event-{event_id}`, `order-{order_id}`
    /// - Saga: `saga-checkout-{saga_id}`, `saga-order-{saga_id}`
    fn stream_id(input: &Self::Input) -> StreamId;

    /// Process input (command or feedback) and return what to do next
    ///
    /// This is the core business logic—pure, deterministic, no I/O.
    ///
    /// # Returns
    ///
    /// - `Ok(Done(events))` → Persist events and finish
    /// - `Ok(Continue { events, calls })` → Persist events, execute calls, continue
    /// - `Err(error)` → Fail without persisting anything
    ///
    /// # Errors
    ///
    /// Returns an error for business rule violations:
    /// - Validation failures (empty name, invalid format)
    /// - Invalid state transitions (can't publish a cancelled event)
    /// - Precondition failures (entity doesn't exist)
    fn process(
        &self,
        state: &Self::State,
        input: Self::Input,
        clock: &dyn Clock,
    ) -> Result<BusinessResult<Self::Event, Self::Call>, Self::Error>;

    /// Apply an event to state (for replay/reconstruction)
    ///
    /// This must be a **pure function**: given the same state and event,
    /// it must produce the same result. No side effects, no I/O.
    ///
    /// This is called:
    /// 1. During state reconstruction (replaying persisted events)
    /// 2. After persisting new events (keeping in-memory state consistent)
    fn apply(&self, state: &mut Self::State, event: &Self::Event);

    /// Convert aggregate call results into input for the next iteration (saga only)
    ///
    /// Aggregates use `Infallible` for `CallResult`, so this is never called.
    /// The default implementation panics—sagas **must** override this.
    ///
    /// # Panics
    ///
    /// Panics if called on an aggregate (which should never happen since
    /// aggregates always return `Done` and never receive feedback).
    #[must_use]
    #[allow(clippy::panic)] // Intentional: this should never be called for aggregates
    fn feedback_input(_results: Vec<Self::CallResult>) -> Self::Input {
        panic!("Aggregates don't use Continue, so feedback_input should never be called")
    }

    /// Event type name for serialization
    ///
    /// Returns a unique, stable string identifier for each event variant.
    /// This is used to tag serialized events for correct deserialization.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// fn event_type_name(event: &EventEvent) -> &'static str {
    ///     match event {
    ///         EventEvent::Created { .. } => "EventCreated",
    ///         EventEvent::Published { .. } => "EventPublished",
    ///         EventEvent::Cancelled { .. } => "EventCancelled",
    ///     }
    /// }
    /// ```
    ///
    /// # Stability
    ///
    /// These names are persisted in the event store. Once an event type name
    /// is used in production, it should never change. For schema evolution,
    /// create new event types (e.g., `EventCreatedV2`) rather than renaming.
    fn event_type_name(event: &Self::Event) -> &'static str;
}
