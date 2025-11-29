//! Business result type for aggregates and sagas

/// Result of processing business logic
///
/// This unified type works for both aggregates and sagas:
///
/// - **Aggregates** always return `Done(events)` - they process a command and finish
/// - **Sagas** return `Continue { events, calls }` when orchestrating other aggregates,
///   and `Done(events)` when the saga completes (success or failure)
///
/// # Type Parameters
///
/// - `E`: Event type to persist
/// - `C`: Call type for saga orchestration (`Infallible` for aggregates)
///
/// # Examples
///
/// ## Aggregate (always Done)
///
/// ```rust,ignore
/// fn process(&self, state: &State, cmd: Command, clock: &dyn Clock)
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
/// fn process(&self, state: &State, input: Input, clock: &dyn Clock)
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusinessResult<E, C> {
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
}

impl<E, C> BusinessResult<E, C> {
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

    /// Get events if this is a `Done` result
    #[must_use]
    pub fn events(&self) -> &[E] {
        match self {
            Self::Done(events) | Self::Continue { events, .. } => events,
        }
    }

    /// Get calls if this is a `Continue` result
    #[must_use]
    pub fn calls(&self) -> Option<&[C]> {
        match self {
            Self::Done(_) => None,
            Self::Continue { calls, .. } => Some(calls),
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

    #[test]
    fn done_single_creates_single_event() {
        let result: BusinessResult<TestEvent, TestCall> =
            BusinessResult::done_single(TestEvent("created".into()));

        assert!(result.is_done());
        assert!(!result.is_continue());
        assert_eq!(result.events().len(), 1);
        assert_eq!(result.events()[0], TestEvent("created".into()));
        assert!(result.calls().is_none());
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
        assert_eq!(result.events().len(), 1);
        assert_eq!(result.calls().unwrap().len(), 1);
    }
}
