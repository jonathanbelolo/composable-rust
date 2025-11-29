//! The unified handler for aggregates and sagas

use crate::{
    BusinessLogic, BusinessResult, CallExecutor, EventBus, EventStore, HandlerEnvironment,
    HandlerError, Projector, SerializationError, SerializedEvent, StreamId, Version,
};

/// Result of handling a command
#[derive(Debug, Clone)]
pub struct HandleResult {
    /// New version after persisting events
    pub version: Version,
}

/// The unified handler for aggregates and sagas
///
/// This handler orchestrates the entire command-handling flow:
///
/// 1. Load current state from event store
/// 2. Delegate to business logic
/// 3. Persist resulting events
/// 4. Project to read model (wait for completion)
/// 5. Broadcast for sagas/other subscribers
///
/// The same handler works for both aggregates and sagas:
///
/// - **Aggregates**: Loop exits on first iteration (always return `Done`)
/// - **Sagas**: Loop continues until saga returns `Done`
///
/// # Type Parameters
///
/// - `T`: Business logic implementation
/// - `E`: Call executor (saga dispatch)
/// - `Env`: Environment with all dependencies
///
/// # Coordination Guarantees
///
/// When `handle()` returns `Ok(HandleResult)`, the caller **knows**:
/// 1. All events are persisted to the event store
/// 2. All projections (read models) are updated
/// 3. All events are broadcast to the event bus
///
/// This eliminates the need for `respond_to` channels and callback chains.
///
/// # Examples
///
/// ## Aggregate Handler
///
/// ```rust,ignore
/// use composable_rust_next::{Handler, NoOpCallExecutor};
///
/// let env = EventEnvironment::new(
///     clock,
///     event_store,
///     projector,
///     Some(event_bus),
///     "event-events".to_string(),
/// );
///
/// let handler = Handler::new(
///     EventBusinessLogic,
///     NoOpCallExecutor,
///     env,
/// );
///
/// let result = handler.handle(EventCommand::Create { /* ... */ }).await?;
/// println!("Created at version: {}", result.version);
/// ```
///
/// ## Saga Handler
///
/// ```rust,ignore
/// let saga_executor = SagaCallExecutor {
///     event_handler: event_handler.clone(),
///     inventory_handler: inventory_handler.clone(),
/// };
///
/// let env = SagaEnvironment::new(clock, event_store, projector, event_bus, topic);
/// let handler = Handler::new(EventInventorySagaLogic, saga_executor, env);
///
/// let result = handler.handle(SagaInput::Command(cmd)).await?;
/// ```
pub struct Handler<T, E, Env>
where
    T: BusinessLogic,
    E: CallExecutor<T::Call, T::CallResult>,
    Env: HandlerEnvironment,
{
    business: T,
    call_executor: E,
    env: Env,
}

impl<T, E, Env> Handler<T, E, Env>
where
    T: BusinessLogic,
    E: CallExecutor<T::Call, T::CallResult>,
    Env: HandlerEnvironment,
{
    /// Create a new handler with the given environment
    ///
    /// # Arguments
    ///
    /// - `business`: The business logic implementation
    /// - `call_executor`: Executor for saga calls (use `NoOpCallExecutor` for aggregates)
    /// - `env`: Environment with all dependencies (event store, projector, event bus, clock)
    #[must_use]
    pub const fn new(business: T, call_executor: E, env: Env) -> Self {
        Self {
            business,
            call_executor,
            env,
        }
    }

    /// Handle input end-to-end with full coordination guarantees
    ///
    /// This unified loop works for both aggregates and sagas:
    ///
    /// - Aggregates always return `Done` on first iteration
    /// - Sagas may return `Continue` multiple times before `Done`
    ///
    /// # Coordination Guarantees
    ///
    /// When this method returns `Ok(HandleResult)`, the caller **knows**:
    /// 1. All events are persisted to the event store
    /// 2. All projections (read models) are updated
    /// 3. All events are broadcast to the event bus
    ///
    /// This eliminates the need for `respond_to` channels and callback chains.
    /// The HTTP handler can simply `await` this method and return the response.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Business logic returns an error (validation, invalid state)
    /// - Event store operations fail (load, persist)
    /// - Projection fails (database error)
    /// - Event bus broadcast fails
    /// - Serialization fails
    ///
    /// # Concurrency
    ///
    /// Uses optimistic concurrency control. If a version conflict occurs
    /// during persist, returns `HandlerError::Persist` with version conflict.
    /// The caller should retry with fresh state.
    pub async fn handle(&self, input: T::Input) -> Result<HandleResult, HandlerError<T::Error>> {
        let stream_id = T::stream_id(&input);
        let (mut state, mut version) = self.load_state(&stream_id).await?;
        let mut current_input = input;

        loop {
            // Business logic only receives Clock - it's pure
            let result = self
                .business
                .process(&state, current_input, self.env.clock())
                .map_err(HandlerError::Business)?;

            match result {
                BusinessResult::Done(events) => {
                    if !events.is_empty() {
                        // Step 1: Persist to event store (source of truth)
                        let serialized = Self::serialize_events(&events)?;
                        version = self.persist_events(&stream_id, &serialized, version).await?;

                        // Step 2: Apply to local state (for consistency within this handler)
                        self.apply_all(&mut state, &events);

                        // Step 3: Project to read model and WAIT for completion
                        self.project_and_wait(&serialized).await?;

                        // Step 4: Broadcast to event bus (for sagas, other aggregates)
                        self.broadcast(&serialized).await?;
                    }

                    // Only return when ALL steps are complete
                    return Ok(HandleResult { version });
                }

                BusinessResult::Continue { events, calls } => {
                    if !events.is_empty() {
                        let serialized = Self::serialize_events(&events)?;
                        version = self.persist_events(&stream_id, &serialized, version).await?;
                        self.apply_all(&mut state, &events);
                        self.project_and_wait(&serialized).await?;
                        self.broadcast(&serialized).await?;
                    }

                    // Execute aggregate calls and feed results back
                    let feedback = self.call_executor.execute(calls).await;
                    current_input = T::feedback_input(feedback);
                    // Loop continues with feedback...
                }
            }
        }
    }

    /// Load state by replaying events from the event store
    async fn load_state(
        &self,
        stream_id: &StreamId,
    ) -> Result<(T::State, Version), HandlerError<T::Error>> {
        let raw_events = self
            .env
            .event_store()
            .load(stream_id, None)
            .await
            .map_err(HandlerError::Load)?;

        let mut state = T::State::default();
        let mut version = Version::initial();

        for raw in raw_events {
            let event: T::Event = Self::deserialize_event(&raw)?;
            self.business.apply(&mut state, &event);
            if let Some(v) = raw.version {
                version = v;
            }
        }

        Ok((state, version))
    }

    /// Serialize events for persistence and projection
    fn serialize_events(
        events: &[T::Event],
    ) -> Result<Vec<SerializedEvent>, HandlerError<T::Error>> {
        events.iter().map(Self::serialize_event).collect()
    }

    /// Persist events to the event store with optimistic concurrency
    async fn persist_events(
        &self,
        stream_id: &StreamId,
        events: &[SerializedEvent],
        expected_version: Version,
    ) -> Result<Version, HandlerError<T::Error>> {
        self.env
            .event_store()
            .append(stream_id, Some(expected_version), events.to_vec())
            .await
            .map_err(HandlerError::Persist)
    }

    /// Project events to read model and wait for completion
    ///
    /// If no projector is configured, this is a no-op.
    async fn project_and_wait(
        &self,
        events: &[SerializedEvent],
    ) -> Result<(), HandlerError<T::Error>> {
        if let Some(projector) = self.env.projector() {
            projector
                .project(events)
                .await
                .map_err(HandlerError::Projection)?;
        }
        Ok(())
    }

    /// Broadcast events to the event bus for other subscribers
    ///
    /// If no event bus is configured, this is a no-op.
    async fn broadcast(&self, events: &[SerializedEvent]) -> Result<(), HandlerError<T::Error>> {
        if let Some(event_bus) = self.env.event_bus() {
            let topic = self.env.broadcast_topic();
            for event in events {
                event_bus
                    .publish(topic, event.clone())
                    .await
                    .map_err(HandlerError::Broadcast)?;
            }
        }
        Ok(())
    }

    /// Apply events to state (for state reconstruction)
    fn apply_all(&self, state: &mut T::State, events: &[T::Event]) {
        for event in events {
            self.business.apply(state, event);
        }
    }

    /// Serialize an event for persistence/projection/broadcast
    fn serialize_event(event: &T::Event) -> Result<SerializedEvent, HandlerError<T::Error>> {
        let type_name = T::event_type_name(event);
        let payload = bincode::serialize(event)
            .map_err(|e| HandlerError::Serialization(SerializationError::Encode(e.to_string())))?;

        Ok(SerializedEvent {
            event_type: type_name.to_string(),
            payload,
            metadata: None,
            version: None,
        })
    }

    /// Deserialize an event from storage
    fn deserialize_event(raw: &SerializedEvent) -> Result<T::Event, HandlerError<T::Error>> {
        bincode::deserialize(&raw.payload)
            .map_err(|e| HandlerError::Serialization(SerializationError::Decode(e.to_string())))
    }
}

// Manual Debug implementation since we can't derive it with generic environment
impl<T, E, Env> std::fmt::Debug for Handler<T, E, Env>
where
    T: BusinessLogic + std::fmt::Debug,
    E: CallExecutor<T::Call, T::CallResult> + std::fmt::Debug,
    Env: HandlerEnvironment,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handler")
            .field("business", &self.business)
            .field("call_executor", &self.call_executor)
            .finish_non_exhaustive()
    }
}
