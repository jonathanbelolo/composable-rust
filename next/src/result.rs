//! Business result type for aggregates, sagas, and queries

/// Result of processing business logic
///
/// This unified type works for aggregates, sagas, and queries:
///
/// - **Aggregates** return `Done(events)` - they process a command and finish
/// - **Sagas** return `Continue { events, calls }` when orchestrating other aggregates,
///   and `Done(events)` when the saga completes (success or failure)
/// - **Queries** return `Respond(data)` - they return data without persistence
///
/// # Type Parameters
///
/// - `E`: Event type to persist
/// - `C`: Call type for saga orchestration (`Infallible` for aggregates)
/// - `R`: Response type for queries (defaults to `()` for backwards compatibility)
///
/// # Examples
///
/// ## Aggregate (always Done)
///
/// ```rust,ignore
/// async fn process(&self, state: &State, cmd: Command, env: &Env)
///     -> Result<BusinessResult<Event, Infallible>, Error>
/// {
///     // Validate and produce events
///     Ok(BusinessResult::Done(vec![Event::Created { /* ... */ }]))
/// }
/// ```
///
/// ## Saga (Continue then Done)
///
/// ```rust,ignore
/// async fn process(&self, state: &State, input: Input, env: &Env)
///     -> Result<BusinessResult<Event, Call>, Error>
/// {
///     match (&state.phase, input) {
///         (Phase::Initial, Input::Command(cmd)) => {
///             // Start orchestration
///             Ok(BusinessResult::Continue {
///                 events: vec![Event::Initiated { /* ... */ }],
///                 calls: vec![Call::CreateOrder { /* ... */ }],
///             })
///         }
///         (Phase::WaitingForOrder, Input::Feedback(results)) => {
///             // Complete the saga
///             Ok(BusinessResult::Done(vec![Event::Completed { /* ... */ }]))
///         }
///         // ...
///     }
/// }
/// ```
///
/// ## Query (Respond with data)
///
/// ```rust,ignore
/// async fn process(&self, state: &State, cmd: Command, env: &Env)
///     -> Result<BusinessResult<Event, Infallible, EventResponse>, Error>
/// {
///     match cmd {
///         Command::GetEvent { event_id } => {
///             // Query from projection (NOT event store)
///             let event = env.projections().get_event(event_id).await?;
///             match event {
///                 Some(e) => Ok(BusinessResult::Respond(EventResponse::Single(e))),
///                 None => Err(EventError::NotFound),
///             }
///         }
///         // ...
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BusinessResult<E, C, R = ()> {
    /// Emit events and finish processing
    ///
    /// Aggregates always use this variant. Sagas use this for their final step
    /// (either successful completion or terminal failure after compensation).
    ///
    /// The handler will:
    /// 1. Persist all events to the event store
    /// 2. Apply events to state (for consistency)
    /// 3. Broadcast events to the event bus
    /// 4. Return success to the caller
    Done(Vec<E>),

    /// Emit events, call other aggregates, then continue processing their feedback
    ///
    /// Only sagas use this variant—aggregates never call other aggregates.
    ///
    /// The handler will:
    /// 1. Persist events to the event store
    /// 2. Apply events to state
    /// 3. Broadcast events to the event bus
    /// 4. Execute all calls via the [`CallExecutor`](crate::CallExecutor)
    /// 5. Feed results back via `BusinessLogic::feedback_input()`
    /// 6. Call `process()` again with the feedback (loop continues)
    Continue {
        /// Events to persist before making calls
        events: Vec<E>,
        /// Calls to execute (dispatched to aggregate handlers)
        calls: Vec<C>,
    },

    /// Return data without persistence (query response)
    ///
    /// Used for read operations that query projections (read models).
    /// No events are persisted, projected, or broadcast.
    ///
    /// The handler will:
    /// 1. Return the response data directly
    /// 2. Skip all persistence, projection, and broadcast steps
    ///
    /// # Authorization
    ///
    /// Authorization should be performed in `process()` before returning `Respond`.
    /// The user ID can be passed in the command for access control checks.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// Command::GetReservation { reservation_id, requesting_user } => {
    ///     let reservation = env.projections()
    ///         .get_reservation(reservation_id)
    ///         .await?
    ///         .ok_or(ReservationError::NotFound)?;
    ///
    ///     // Authorization check
    ///     if reservation.customer_id != requesting_user {
    ///         return Err(ReservationError::Forbidden);
    ///     }
    ///
    ///     Ok(BusinessResult::Respond(reservation))
    /// }
    /// ```
    Respond(R),

    /// Park the saga mid-flight, waiting for a correlated external domain event
    /// (or a deadline) to wake it. **Durable-mode only** — rejected on the batch
    /// path, and rejected when the saga has outstanding calls (a saga is either
    /// call-driven or awaiting, never both).
    ///
    /// This is the legal form of "wait for something external" that the
    /// `SagaStuck` guard (empty `Continue` with no outstanding calls) otherwise
    /// rejects. The handler will:
    /// 1. Persist `events` **atomically** with a `$saga.awaiting` framework
    ///    marker carrying `resume_on` (+ `timeout`), in one append.
    /// 2. The marker's projector registers the correlation in the index in that
    ///    same transaction (never a separate `register()` call — the two-step
    ///    would park the saga unfindably on a crash).
    /// 3. Return control. The instance is resumed later by
    ///    [`Handler::resume_awaiting`](crate::Handler::resume_awaiting) — when an
    ///    awaited event arrives or the deadline fires (the caller resolves the
    ///    stream via a `CorrelationIndex`).
    Await {
        /// Events to persist atomically with the awaiting marker.
        events: Vec<E>,
        /// How a future event finds and wakes THIS instance (a
        /// `(key, value)` the awaited event must carry).
        resume_on: Correlation,
        /// Optional deadline; on expiry the saga is woken via the timeout path
        /// so the timeout and the event collapse into one resume.
        timeout: Option<AwaitDeadline>,
    },
}

/// Identifies which future event wakes a parked saga, and the value that routes
/// that event to THIS instance.
///
/// Scoped by the saga's `LOGIC_TAG` at the index layer, so two saga types may
/// reuse a `key` name (e.g. `"order_id"`) without collision.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Correlation {
    /// The logical correlation dimension (e.g. `"order_id"`).
    pub key: String,
    /// The instance value the awaited event must carry (e.g. a specific id).
    pub value: String,
}

impl Correlation {
    /// Create a correlation from a key and value.
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// An absolute deadline for a parked saga's await, plus an opaque tag the saga
/// chooses.
///
/// The tag lets the timeout wake and the event wake collapse into a single
/// resume path — the saga branches on which arrived (the timeout wake's input,
/// built by the caller and passed to `Handler::resume_awaiting`, carries the
/// tag back).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitDeadline {
    /// The absolute instant at which the await times out.
    pub at: chrono::DateTime<chrono::Utc>,
    /// Opaque tag identifying which await timed out (saga-chosen).
    pub tag: String,
}

impl AwaitDeadline {
    /// Create a deadline from an absolute instant and a saga-chosen tag.
    pub fn new(at: chrono::DateTime<chrono::Utc>, tag: impl Into<String>) -> Self {
        Self {
            at,
            tag: tag.into(),
        }
    }
}

impl<E, C, R> BusinessResult<E, C, R> {
    /// Create a `Done` result with a single event
    #[must_use]
    pub fn done_single(event: E) -> Self {
        Self::Done(vec![event])
    }

    /// Create a `Done` result with no events
    #[must_use]
    pub const fn done_empty() -> Self {
        Self::Done(Vec::new())
    }

    /// Create a `Continue` result with events and calls
    #[must_use]
    pub const fn continue_with(events: Vec<E>, calls: Vec<C>) -> Self {
        Self::Continue { events, calls }
    }

    /// Create a `Respond` result with query data
    #[must_use]
    pub const fn respond(data: R) -> Self {
        Self::Respond(data)
    }

    /// Create an `Await` result — park on a correlation, with an optional deadline
    #[must_use]
    pub const fn await_on(
        events: Vec<E>,
        resume_on: Correlation,
        timeout: Option<AwaitDeadline>,
    ) -> Self {
        Self::Await {
            events,
            resume_on,
            timeout,
        }
    }

    /// Check if this is a `Done` result
    #[must_use]
    pub const fn is_done(&self) -> bool {
        matches!(self, Self::Done(_))
    }

    /// Check if this is a `Continue` result
    #[must_use]
    pub const fn is_continue(&self) -> bool {
        matches!(self, Self::Continue { .. })
    }

    /// Check if this is a `Respond` result (query response)
    #[must_use]
    pub const fn is_respond(&self) -> bool {
        matches!(self, Self::Respond(_))
    }

    /// Check if this is an `Await` result (a parked saga waiting on a correlation)
    #[must_use]
    pub const fn is_await(&self) -> bool {
        matches!(self, Self::Await { .. })
    }

    /// Get events from `Done` or `Continue` variants
    ///
    /// Returns an empty slice for `Respond` variants.
    #[must_use]
    pub fn events(&self) -> &[E] {
        match self {
            Self::Done(events) | Self::Continue { events, .. } | Self::Await { events, .. } => {
                events
            }
            Self::Respond(_) => &[],
        }
    }

    /// Get calls if this is a `Continue` result
    #[must_use]
    pub fn calls(&self) -> Option<&[C]> {
        match self {
            Self::Done(_) | Self::Respond(_) | Self::Await { .. } => None,
            Self::Continue { calls, .. } => Some(calls),
        }
    }

    /// Get response data if this is a `Respond` result
    #[must_use]
    pub const fn response(&self) -> Option<&R> {
        match self {
            Self::Respond(data) => Some(data),
            Self::Done(_) | Self::Continue { .. } | Self::Await { .. } => None,
        }
    }

    /// Convert `Respond` into its inner data
    ///
    /// Returns `None` if this is not a `Respond` variant.
    #[must_use]
    pub fn into_response(self) -> Option<R> {
        match self {
            Self::Respond(data) => Some(data),
            Self::Done(_) | Self::Continue { .. } | Self::Await { .. } => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct TestEvent(String);

    #[derive(Debug, Clone, PartialEq)]
    struct TestCall(String);

    #[derive(Debug, Clone, PartialEq)]
    struct TestResponse {
        id: i32,
        name: String,
    }

    #[test]
    fn done_single_creates_single_event() {
        let result: BusinessResult<TestEvent, TestCall> =
            BusinessResult::done_single(TestEvent("created".into()));

        assert!(result.is_done());
        assert!(!result.is_continue());
        assert!(!result.is_respond());
        assert_eq!(result.events().len(), 1);
        assert_eq!(result.events()[0], TestEvent("created".into()));
        assert!(result.calls().is_none());
        assert!(result.response().is_none());
    }

    #[test]
    fn done_empty_creates_no_events() {
        let result: BusinessResult<TestEvent, TestCall> = BusinessResult::done_empty();

        assert!(result.is_done());
        assert!(result.events().is_empty());
    }

    #[test]
    fn continue_with_creates_events_and_calls() {
        let result: BusinessResult<TestEvent, TestCall> = BusinessResult::continue_with(
            vec![TestEvent("initiated".into())],
            vec![TestCall("create_order".into())],
        );

        assert!(result.is_continue());
        assert!(!result.is_done());
        assert!(!result.is_respond());
        assert_eq!(result.events().len(), 1);
        assert_eq!(result.calls().unwrap().len(), 1);
        assert!(result.response().is_none());
    }

    #[test]
    fn respond_creates_query_response() {
        let response = TestResponse {
            id: 42,
            name: "Test Event".into(),
        };
        let result: BusinessResult<TestEvent, TestCall, TestResponse> =
            BusinessResult::respond(response.clone());

        assert!(result.is_respond());
        assert!(!result.is_done());
        assert!(!result.is_continue());
        assert!(result.events().is_empty());
        assert!(result.calls().is_none());
        assert_eq!(result.response(), Some(&response));
    }

    #[test]
    fn into_response_extracts_data() {
        let response = TestResponse {
            id: 42,
            name: "Test Event".into(),
        };
        let result: BusinessResult<TestEvent, TestCall, TestResponse> =
            BusinessResult::respond(response.clone());

        assert_eq!(result.into_response(), Some(response));
    }

    #[test]
    fn into_response_returns_none_for_done() {
        let result: BusinessResult<TestEvent, TestCall, TestResponse> =
            BusinessResult::done_single(TestEvent("created".into()));

        assert!(result.into_response().is_none());
    }

    #[test]
    fn default_response_type_is_unit() {
        // Verify backwards compatibility: R defaults to ()
        let result: BusinessResult<TestEvent, TestCall> =
            BusinessResult::done_single(TestEvent("created".into()));

        // This compiles because R defaults to ()
        assert!(result.is_done());
    }
}
