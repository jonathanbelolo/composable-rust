//! The unified handler for aggregates and sagas
//!
//! The [`Handler`] orchestrates the complete command/query flow:
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────────────────┐
//! │                           Handler Flow                                  │
//! ├────────────────────────────────────────────────────────────────────────┤
//! │                                                                        │
//! │  INPUT ──► QueryFetcher.fetch() ──► BusinessLogic.process()            │
//! │                │                            │                          │
//! │                ▼                            ▼                          │
//! │         (prepared_input,              BusinessResult                   │
//! │          expected_version)                  │                          │
//! │                                             │                          │
//! │         ┌───────────────────────────────────┼────────────────────────┐ │
//! │         │                                   │                        │ │
//! │         ▼                                   ▼                        ▼ │
//! │      Respond ──► return Query          Done/Continue ──► persist     │ │
//! │                                              │            with       │ │
//! │                                              │         version check │ │
//! │                                              │              │        │ │
//! │                                              │   ┌──────────┴──────┐ │ │
//! │                                              │   │                 │ │ │
//! │                                              │   ▼                 ▼ │ │
//! │                                              │  OK ──► project   CONFLICT │
//! │                                              │    │    & broadcast   │ │
//! │                                              │    ▼                  │ │
//! │                                              │ return              RETRY │
//! │                                              │                       │ │
//! │                                              └───────────────────────┘ │
//! └────────────────────────────────────────────────────────────────────────┘
//! ```

use crate::{
    BusinessLogic, BusinessResult, CallExecutor, EventBus, EventStore, EventStoreError,
    FetchResult, HandlerEnvironment, HandlerError, Projector, QueryFetcher, SerializationError,
    SerializedEvent, StreamId, Version,
};

/// Default maximum retry attempts for version conflicts
pub const DEFAULT_MAX_RETRIES: u32 = 3;

/// Default maximum saga iterations before aborting
///
/// This is a safety guard against infinite loops in saga logic.
/// A well-designed saga should complete in far fewer iterations.
/// This limit is intentionally high to allow complex multi-step sagas
/// while still providing protection against bugs.
pub const DEFAULT_MAX_SAGA_ITERATIONS: u32 = 100;

/// Result of handling a command or query
///
/// The handler returns different result types for commands vs queries:
///
/// - **Commands**: Persist events, return version
/// - **Queries**: Return data without persistence
#[derive(Debug, Clone)]
pub enum HandleResult<R = ()> {
    /// Command completed - events were persisted
    ///
    /// Returned when `BusinessResult::Done` or `BusinessResult::Continue` is processed.
    Command {
        /// New version after persisting events
        version: Version,
        /// Number of events persisted
        event_count: usize,
    },

    /// Query completed - data returned from projection
    ///
    /// Returned when `BusinessResult::Respond` is processed.
    /// No events are persisted, projected, or broadcast.
    Query(R),
}

impl<R> HandleResult<R> {
    /// Get the version if this is a command result
    #[must_use]
    pub const fn version(&self) -> Option<Version> {
        match self {
            Self::Command { version, .. } => Some(*version),
            Self::Query(_) => None,
        }
    }

    /// Get the query response if this is a query result
    #[must_use]
    pub const fn query_response(&self) -> Option<&R> {
        match self {
            Self::Query(data) => Some(data),
            Self::Command { .. } => None,
        }
    }

    /// Convert into query response, consuming self
    #[must_use]
    pub fn into_query_response(self) -> Option<R> {
        match self {
            Self::Query(data) => Some(data),
            Self::Command { .. } => None,
        }
    }

    /// Check if this is a command result
    #[must_use]
    pub const fn is_command(&self) -> bool {
        matches!(self, Self::Command { .. })
    }

    /// Check if this is a query result
    #[must_use]
    pub const fn is_query(&self) -> bool {
        matches!(self, Self::Query(_))
    }

    /// Get the number of events persisted (0 for queries)
    #[must_use]
    pub const fn event_count(&self) -> usize {
        match self {
            Self::Command { event_count, .. } => *event_count,
            Self::Query(_) => 0,
        }
    }
}

/// The unified handler for aggregates and sagas
///
/// # CQRS Architecture
///
/// This handler implements true CQRS:
///
/// - **Reads**: Fetch from projections via [`QueryFetcher`], never touch event store
/// - **Writes**: Validate against projection data, append to event store
///
/// The event store is only used for:
/// 1. Appending events (with version check for optimistic concurrency)
/// 2. Rebuilding projections (disaster recovery) - not in the Handler
///
/// # Flow
///
/// 1. **Fetch**: `QueryFetcher` fetches projection data and expected version
/// 2. **Process**: `BusinessLogic` validates and returns events
/// 3. **Persist**: Append events with version check
/// 4. **Retry**: On version conflict, retry from step 1 (up to `max_retries`)
/// 5. **Project**: Update read model (wait for completion)
/// 6. **Broadcast**: Publish to event bus
///
/// # Aggregates vs Sagas vs Queries
///
/// - **Aggregates**: Return `Done` on first iteration
/// - **Sagas**: May return `Continue` multiple times before `Done`
/// - **Queries**: Return `Respond` immediately (no persistence)
///
/// # Retry Logic
///
/// On version conflict, the handler automatically retries:
/// 1. Re-fetch from projections (get fresh data + version)
/// 2. Re-process business logic
/// 3. Re-attempt persist
///
/// This handles the race condition where another request updated the entity
/// between our fetch and persist.
///
/// # Examples
///
/// ## Aggregate Handler
///
/// ```rust,ignore
/// use composable_rust_next::{Handler, NoOpCallExecutor, NoOpQueryFetcher};
///
/// let handler = Handler::new(
///     EventBusinessLogic,
///     NoOpCallExecutor,
///     NoOpQueryFetcher,
///     env,
/// );
///
/// // Command
/// let result = handler.handle(EventCommand::Create { event_id, name, fetched: None }).await?;
/// println!("Created at version: {:?}", result.version());
///
/// // Query
/// let result = handler.handle(EventCommand::GetEvent { event_id, fetched: None }).await?;
/// let event = result.into_query_response().unwrap();
/// ```
pub struct Handler<T, E, QF, Env>
where
    T: BusinessLogic,
    E: CallExecutor<T::Call, T::CallResult>,
    QF: QueryFetcher<T::Input, Env::Projections>,
    Env: HandlerEnvironment,
{
    business: T,
    call_executor: E,
    query_fetcher: QF,
    env: Env,
    max_retries: u32,
    max_saga_iterations: u32,
}

impl<T, E, QF, Env> Handler<T, E, QF, Env>
where
    T: BusinessLogic,
    T::Input: Clone,
    E: CallExecutor<T::Call, T::CallResult>,
    QF: QueryFetcher<T::Input, Env::Projections>,
    Env: HandlerEnvironment,
{
    /// Create a new handler with the given environment
    ///
    /// Uses default retry count (3) for version conflicts and
    /// default saga iteration limit (100).
    #[must_use]
    pub const fn new(business: T, call_executor: E, query_fetcher: QF, env: Env) -> Self {
        Self {
            business,
            call_executor,
            query_fetcher,
            env,
            max_retries: DEFAULT_MAX_RETRIES,
            max_saga_iterations: DEFAULT_MAX_SAGA_ITERATIONS,
        }
    }

    /// Create a new handler with custom retry count
    #[must_use]
    pub const fn with_max_retries(
        business: T,
        call_executor: E,
        query_fetcher: QF,
        env: Env,
        max_retries: u32,
    ) -> Self {
        Self {
            business,
            call_executor,
            query_fetcher,
            env,
            max_retries,
            max_saga_iterations: DEFAULT_MAX_SAGA_ITERATIONS,
        }
    }

    /// Handle input end-to-end with full coordination guarantees
    ///
    /// # CQRS Flow
    ///
    /// 1. **Fetch** from projections (NOT event store)
    /// 2. **Process** business logic with fetched data
    /// 3. **Persist** to event store with version check
    /// 4. **Retry** on version conflict (up to `max_retries`)
    /// 5. **Project** to read model (synchronous)
    /// 6. **Broadcast** to event bus
    ///
    /// # Coordination Guarantees
    ///
    /// When this returns `Ok(HandleResult::Command { .. })`:
    /// 1. Events are persisted to event store
    /// 2. Projections are updated
    /// 3. Events are broadcast
    ///
    /// When this returns `Ok(HandleResult::Query(..))`:
    /// - Data was fetched from projections
    /// - No events persisted/projected/broadcast
    ///
    /// # Errors
    ///
    /// - `HandlerError::QueryFetch`: Projection database error
    /// - `HandlerError::Business`: Validation/state error
    /// - `HandlerError::Persist`: Event store error (including max retries exceeded)
    /// - `HandlerError::Projection`: Read model update failed
    /// - `HandlerError::Broadcast`: Event bus error
    /// - `HandlerError::SagaIterationsExceeded`: Saga loop exceeded max iterations
    pub async fn handle(
        &self,
        input: T::Input,
    ) -> Result<HandleResult<T::Response>, HandlerError<T::Error>> {
        let mut current_input = input;
        let mut attempts = 0;
        let mut saga_iterations: u32 = 0;

        loop {
            // Safety guard: prevent infinite saga loops
            if saga_iterations >= self.max_saga_iterations {
                return Err(HandlerError::SagaIterationsExceeded {
                    max_iterations: self.max_saga_iterations,
                });
            }
            // Step 1: Fetch from projections (includes expected version)
            let FetchResult {
                input: prepared_input,
                expected_version,
            } = self
                .query_fetcher
                .fetch(current_input.clone(), self.env.projections())
                .await
                .map_err(|e| HandlerError::QueryFetch(e.to_string()))?;

            // Step 2: Process business logic (pure, no I/O)
            let result = self
                .business
                .process(prepared_input.clone(), self.env.clock())
                .map_err(HandlerError::Business)?;

            match result {
                BusinessResult::Respond(data) => {
                    // Query: return immediately, no persistence
                    return Ok(HandleResult::Query(data));
                }

                BusinessResult::Done(events) => {
                    if events.is_empty() {
                        return Ok(HandleResult::Command {
                            version: expected_version.unwrap_or_else(Version::initial),
                            event_count: 0,
                        });
                    }

                    let stream_id = T::stream_id(&prepared_input);
                    let serialized = self.serialize_events(&events)?;

                    // Step 3: Persist with version check
                    match self.persist_events(&stream_id, &serialized, expected_version).await {
                        Ok(version) => {
                            // Step 4: Project and broadcast (after successful persist)
                            self.project_and_wait(&serialized).await?;
                            self.broadcast(&serialized).await?;

                            return Ok(HandleResult::Command {
                                version,
                                event_count: events.len(),
                            });
                        }
                        Err(HandlerError::Persist(EventStoreError::VersionConflict { .. }))
                            if attempts < self.max_retries =>
                        {
                            // Retry: re-fetch from projections and try again
                            attempts += 1;
                            current_input = prepared_input;
                            // Continue to next loop iteration
                        }
                        Err(e) => return Err(e),
                    }
                }

                BusinessResult::Continue { events, calls } => {
                    if !events.is_empty() {
                        let stream_id = T::stream_id(&prepared_input);
                        let serialized = self.serialize_events(&events)?;

                        // Persist with retry
                        match self.persist_events(&stream_id, &serialized, expected_version).await
                        {
                            Ok(_version) => {
                                self.project_and_wait(&serialized).await?;
                                self.broadcast(&serialized).await?;
                            }
                            Err(HandlerError::Persist(EventStoreError::VersionConflict { .. }))
                                if attempts < self.max_retries =>
                            {
                                // Retry: re-fetch from projections
                                attempts += 1;
                                current_input = prepared_input;
                                continue; // Must continue here to avoid falling through to call executor
                            }
                            Err(e) => return Err(e),
                        }
                    }

                    // Execute calls and feed results back
                    let feedback = self.call_executor.execute(calls).await;
                    current_input = T::feedback_input(feedback);

                    // Track saga iterations for loop safety
                    saga_iterations += 1;

                    // For saga continuation, the next iteration will re-fetch from
                    // projections to get updated state and version
                }
            }
        }
    }

    /// Serialize events with metadata from environment
    fn serialize_events(
        &self,
        events: &[T::Event],
    ) -> Result<Vec<SerializedEvent>, HandlerError<T::Error>> {
        let metadata = Some(self.env.metadata().to_event_metadata());

        events
            .iter()
            .map(|event| {
                let type_name = T::event_type_name(event);
                let payload = bincode::serialize(event).map_err(|e| {
                    HandlerError::Serialization(SerializationError::Encode(e.to_string()))
                })?;

                Ok(SerializedEvent {
                    event_type: type_name.to_string(),
                    payload,
                    metadata: metadata.clone(),
                    version: None,
                })
            })
            .collect()
    }

    /// Persist events to event store with optimistic concurrency
    async fn persist_events(
        &self,
        stream_id: &StreamId,
        events: &[SerializedEvent],
        expected_version: Option<Version>,
    ) -> Result<Version, HandlerError<T::Error>> {
        self.env
            .event_store()
            .append(stream_id, expected_version, events.to_vec())
            .await
            .map_err(HandlerError::Persist)
    }

    /// Project events to read model (synchronous completion)
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

    /// Broadcast events to event bus (batch)
    async fn broadcast(&self, events: &[SerializedEvent]) -> Result<(), HandlerError<T::Error>> {
        if let Some(event_bus) = self.env.event_bus() {
            event_bus
                .publish_batch(self.env.broadcast_topic(), events)
                .await
                .map_err(HandlerError::Broadcast)?;
        }
        Ok(())
    }
}

// Manual Debug implementation since we can't derive it with generic environment
impl<T, E, QF, Env> std::fmt::Debug for Handler<T, E, QF, Env>
where
    T: BusinessLogic + std::fmt::Debug,
    E: CallExecutor<T::Call, T::CallResult> + std::fmt::Debug,
    QF: QueryFetcher<T::Input, Env::Projections> + std::fmt::Debug,
    Env: HandlerEnvironment,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handler")
            .field("business", &self.business)
            .field("call_executor", &self.call_executor)
            .field("query_fetcher", &self.query_fetcher)
            .field("max_retries", &self.max_retries)
            .finish_non_exhaustive()
    }
}

/// Builder for creating handlers with custom configuration
///
/// # Example
///
/// ```rust,ignore
/// let handler = HandlerBuilder::new(EventBusinessLogic)
///     .call_executor(NoOpCallExecutor)
///     .query_fetcher(EventQueryFetcher)
///     .environment(env)
///     .max_retries(5)
///     .max_saga_iterations(50) // For sagas
///     .build();
/// ```
pub struct HandlerBuilder<T, E = (), QF = (), Env = ()> {
    business: T,
    call_executor: E,
    query_fetcher: QF,
    env: Env,
    max_retries: u32,
    max_saga_iterations: u32,
}

impl<T: BusinessLogic> HandlerBuilder<T, (), (), ()> {
    /// Create a new handler builder with the given business logic
    #[must_use]
    pub const fn new(business: T) -> Self {
        Self {
            business,
            call_executor: (),
            query_fetcher: (),
            env: (),
            max_retries: DEFAULT_MAX_RETRIES,
            max_saga_iterations: DEFAULT_MAX_SAGA_ITERATIONS,
        }
    }
}

impl<T: BusinessLogic, E, QF, Env> HandlerBuilder<T, E, QF, Env> {
    /// Set the call executor (for sagas)
    #[must_use]
    pub fn call_executor<E2>(self, executor: E2) -> HandlerBuilder<T, E2, QF, Env> {
        HandlerBuilder {
            business: self.business,
            call_executor: executor,
            query_fetcher: self.query_fetcher,
            env: self.env,
            max_retries: self.max_retries,
            max_saga_iterations: self.max_saga_iterations,
        }
    }

    /// Set the query fetcher
    #[must_use]
    pub fn query_fetcher<QF2>(self, fetcher: QF2) -> HandlerBuilder<T, E, QF2, Env> {
        HandlerBuilder {
            business: self.business,
            call_executor: self.call_executor,
            query_fetcher: fetcher,
            env: self.env,
            max_retries: self.max_retries,
            max_saga_iterations: self.max_saga_iterations,
        }
    }

    /// Set the environment
    #[must_use]
    pub fn environment<Env2>(self, env: Env2) -> HandlerBuilder<T, E, QF, Env2> {
        HandlerBuilder {
            business: self.business,
            call_executor: self.call_executor,
            query_fetcher: self.query_fetcher,
            env,
            max_retries: self.max_retries,
            max_saga_iterations: self.max_saga_iterations,
        }
    }

    /// Set the maximum retry count for version conflicts
    #[must_use]
    pub const fn max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Set the maximum saga iterations before aborting
    ///
    /// This is a safety guard against infinite loops in saga logic.
    /// Default is 100 iterations.
    #[must_use]
    pub const fn max_saga_iterations(mut self, max_iterations: u32) -> Self {
        self.max_saga_iterations = max_iterations;
        self
    }
}

impl<T, E, QF, Env> HandlerBuilder<T, E, QF, Env>
where
    T: BusinessLogic,
    T::Input: Clone,
    E: CallExecutor<T::Call, T::CallResult>,
    QF: QueryFetcher<T::Input, Env::Projections>,
    Env: HandlerEnvironment,
{
    /// Build the handler
    #[must_use]
    pub fn build(self) -> Handler<T, E, QF, Env> {
        Handler {
            business: self.business,
            call_executor: self.call_executor,
            query_fetcher: self.query_fetcher,
            env: self.env,
            max_retries: self.max_retries,
            max_saga_iterations: self.max_saga_iterations,
        }
    }
}
