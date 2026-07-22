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
    AtomicError, BusinessLogic, BusinessResult, CallExecutor, EventBus, EventStore,
    EventStoreError, FetchResult, HandlerEnvironment, HandlerError, InvocationContext,
    MetadataContext, Projector, QueryFetcher, SerializationError, SerializedEvent, StreamId,
    Subject, Version,
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

/// Default maximum total dispatched calls for durable saga runs
///
/// Durable mode's runaway guard counts **dispatched calls over the saga
/// instance's lifetime** instead of loop iterations (completion cycles are
/// bounded by dispatches; dispatches are the variable that can run away).
/// Resume seeds the counter from the journal, so the cap survives
/// crash/resume cycles.
pub const DEFAULT_MAX_TOTAL_CALLS: u32 = 1_000;

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
    pub(crate) business: T,
    pub(crate) call_executor: E,
    pub(crate) query_fetcher: QF,
    pub(crate) env: Env,
    pub(crate) max_retries: u32,
    max_saga_iterations: u32,
    pub(crate) max_total_calls: u32,
    pub(crate) max_call_duration: Option<std::time::Duration>,
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
            max_total_calls: DEFAULT_MAX_TOTAL_CALLS,
            max_call_duration: None,
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
            max_total_calls: DEFAULT_MAX_TOTAL_CALLS,
            max_call_duration: None,
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
            // Resolve the subject per attempt (a session may expire mid-retry)
            // and build the per-invocation context. Environments that don't
            // know about identity return None → Subject::System. The metadata
            // context is captured ONCE per attempt and reused for event
            // stamping, so stamped correlation/causation can never diverge
            // from what fetch/process saw.
            let subject = self.env.current_subject().unwrap_or(Subject::System);
            let metadata_context = self.env.metadata();
            let ctx = InvocationContext {
                clock: self.env.clock(),
                subject: &subject,
                correlation_id: metadata_context.correlation_id.as_deref(),
                causation_id: metadata_context.causation_id.as_deref(),
                origin_subject_id: None, // saga feedback threading is a later work package
            };

            // Step 1: Fetch from projections (includes expected version)
            let FetchResult {
                input: prepared_input,
                expected_version,
            } = self
                .query_fetcher
                .fetch_with_context(current_input.clone(), self.env.projections(), &ctx)
                .await
                .map_err(|e| HandlerError::QueryFetch(e.to_string()))?;

            // Step 2: Process business logic (pure, no I/O)
            let result = self
                .business
                .process_with_context(prepared_input.clone(), &ctx)
                .map_err(HandlerError::Business)?;

            match result {
                BusinessResult::Respond(data) => {
                    // Query: return immediately, no persistence
                    return Ok(HandleResult::Query(data));
                },

                BusinessResult::Done(events) => {
                    if events.is_empty() {
                        return Ok(HandleResult::Command {
                            version: expected_version.unwrap_or_else(Version::initial),
                            event_count: 0,
                        });
                    }

                    let stream_id = T::stream_id(&prepared_input);
                    let serialized = Self::serialize_events(&events, metadata_context, &ctx)?;

                    // Step 3+4: Persist and project (atomically if the environment
                    // provides it, otherwise append-then-project).
                    match self
                        .persist_and_project(&stream_id, serialized, expected_version)
                        .await
                    {
                        Ok((final_version, versioned)) => {
                            self.broadcast(&versioned).await?;

                            return Ok(HandleResult::Command {
                                version: final_version,
                                event_count: events.len(),
                            });
                        },
                        Err(HandlerError::Persist(EventStoreError::VersionConflict { .. }))
                            if attempts < self.max_retries =>
                        {
                            // Retry: re-fetch from projections and try again
                            attempts += 1;
                            current_input = prepared_input;
                            // Continue to next loop iteration
                        },
                        Err(e) => return Err(e),
                    }
                },

                BusinessResult::Continue { events, calls } => {
                    if !events.is_empty() {
                        let stream_id = T::stream_id(&prepared_input);
                        let serialized = Self::serialize_events(&events, metadata_context, &ctx)?;

                        // Persist and project (atomically if available) with retry
                        match self
                            .persist_and_project(&stream_id, serialized, expected_version)
                            .await
                        {
                            Ok((_final_version, versioned)) => {
                                self.broadcast(&versioned).await?;
                            },
                            Err(HandlerError::Persist(EventStoreError::VersionConflict {
                                ..
                            })) if attempts < self.max_retries => {
                                // Retry: re-fetch from projections
                                attempts += 1;
                                current_input = prepared_input;
                                continue; // Must continue here to avoid falling through to call executor
                            },
                            Err(e) => return Err(e),
                        }
                    }

                    // Execute calls and feed results back
                    let feedback = self.call_executor.execute(calls).await;
                    current_input = T::feedback_input_from(&prepared_input, feedback);

                    // Track saga iterations for loop safety
                    saga_iterations += 1;

                    // For saga continuation, the next iteration will re-fetch from
                    // projections to get updated state and version
                },

                BusinessResult::Await { .. } => {
                    // `Await` is a durable-mode-only primitive: parking on a
                    // correlation requires the atomic `$saga.awaiting` marker
                    // append + the correlation index, neither of which the batch
                    // path provides. Persisting the marker here without the
                    // atomic guarantee would risk parking the saga unfindably.
                    // Reject loudly rather than silently drop the wait.
                    return Err(HandlerError::AwaitRequiresDurable);
                },
            }
        }
    }

    /// Serialize events with the per-attempt metadata snapshot (the same one
    /// the [`InvocationContext`] was built from, so stamped
    /// correlation/causation can never diverge from what fetch/process saw),
    /// stamped with the invocation's subject (`author_id` from `ctx.subject`,
    /// `origin_subject_id` from `ctx.origin_subject_id`)
    fn serialize_events(
        events: &[T::Event],
        metadata_context: &MetadataContext,
        ctx: &InvocationContext<'_>,
    ) -> Result<Vec<SerializedEvent>, HandlerError<T::Error>> {
        let metadata = Self::stamped_metadata(metadata_context, ctx);
        Self::serialize_events_with(events, &metadata)
    }

    /// Build the per-cycle [`EventMetadata`] snapshot from the invocation's
    /// metadata context and subject, timestamped by the invocation's
    /// **injected clock** (deterministic under `FixedClock`). Extracted so
    /// the durable path can stamp domain events and journal markers with the
    /// **identical** snapshot.
    pub(crate) fn stamped_metadata(
        metadata_context: &MetadataContext,
        ctx: &InvocationContext<'_>,
    ) -> crate::EventMetadata {
        metadata_context.to_event_metadata_with_subject_at(
            ctx.subject,
            ctx.origin_subject_id,
            ctx.clock.now(),
        )
    }

    /// Serialize events with a pre-computed metadata snapshot.
    pub(crate) fn serialize_events_with(
        events: &[T::Event],
        metadata: &crate::EventMetadata,
    ) -> Result<Vec<SerializedEvent>, HandlerError<T::Error>> {
        let metadata = Some(metadata.clone());

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
                    stream_id: None,
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

    /// Persist events and update the read model, returning the final version and
    /// the version-stamped events (for broadcasting).
    ///
    /// If the environment provides [`atomic_persist`](HandlerEnvironment::atomic_persist),
    /// the append and projection commit in a single transaction and `project_and_wait`
    /// is NOT called. Otherwise it falls back to the separate append-then-project flow.
    pub(crate) async fn persist_and_project(
        &self,
        stream_id: &StreamId,
        serialized: Vec<SerializedEvent>,
        expected_version: Option<Version>,
    ) -> Result<(Version, Vec<SerializedEvent>), HandlerError<T::Error>> {
        let event_count = serialized.len();

        if let Some(atomic) = self.env.atomic_persist() {
            // Atomic path: projection happens inside the append transaction.
            let final_version = atomic
                .append_and_project(stream_id, expected_version, serialized.clone())
                .await
                .map_err(|e| match e {
                    AtomicError::Append(es) => HandlerError::Persist(es),
                    AtomicError::Projection(pe) => HandlerError::Projection(pe),
                })?;
            let versioned = Self::assign_versions(serialized, final_version, event_count, stream_id);
            Ok((final_version, versioned))
        } else {
            let final_version = self
                .persist_events(stream_id, &serialized, expected_version)
                .await?;
            let versioned = Self::assign_versions(serialized, final_version, event_count, stream_id);
            self.project_and_wait(&versioned).await?;
            Ok((final_version, versioned))
        }
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
    pub(crate) async fn broadcast(
        &self,
        events: &[SerializedEvent],
    ) -> Result<(), HandlerError<T::Error>> {
        if let Some(event_bus) = self.env.event_bus() {
            event_bus
                .publish_batch(self.env.broadcast_topic(), events)
                .await
                .map_err(HandlerError::Broadcast)?;
        }
        Ok(())
    }

    /// Assign versions to events after persistence
    ///
    /// The event store returns the final version after appending. Since versions
    /// are sequential starting from the previous version + 1, we can calculate
    /// each event's version:
    ///
    /// - If `final_version` is V and we persisted N events
    /// - The events have versions: V-N+1, V-N+2, ..., V
    ///
    /// The source `stream_id` is stamped alongside the version: projectors and
    /// bus subscribers receive [`SerializedEvent`]s without any other stream
    /// context, and per-stream read-model guards depend on it.
    fn assign_versions(
        mut events: Vec<SerializedEvent>,
        final_version: Version,
        event_count: usize,
        stream_id: &StreamId,
    ) -> Vec<SerializedEvent> {
        let start_version = final_version.as_u64() - event_count as u64 + 1;
        for (i, event) in events.iter_mut().enumerate() {
            event.version = Some(Version::new(start_version + i as u64));
            event.stream_id = Some(stream_id.as_str().to_string());
        }
        events
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
    max_total_calls: u32,
    max_call_duration: Option<std::time::Duration>,
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
            max_total_calls: DEFAULT_MAX_TOTAL_CALLS,
            max_call_duration: None,
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
            max_total_calls: self.max_total_calls,
            max_call_duration: self.max_call_duration,
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
            max_total_calls: self.max_total_calls,
            max_call_duration: self.max_call_duration,
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
            max_total_calls: self.max_total_calls,
            max_call_duration: self.max_call_duration,
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

    /// Set the maximum total dispatched calls for durable saga runs
    ///
    /// Durable mode's runaway guard: the cap on calls dispatched over the
    /// saga instance's **lifetime** (resume seeds the counter from the
    /// journal). Default is [`DEFAULT_MAX_TOTAL_CALLS`].
    #[must_use]
    pub const fn max_total_calls(mut self, max_total_calls: u32) -> Self {
        self.max_total_calls = max_total_calls;
        self
    }

    /// Set the stuck-call watchdog for durable saga runs (default: off)
    ///
    /// If the **oldest** in-flight call exceeds this duration, the run ends
    /// with [`HandlerError::CallStuck`] — crash-equivalently: nothing is
    /// persisted at timeout, all in-flight calls stay outstanding in the
    /// journal, and `resume` re-dispatches them. Note the trade-off: a
    /// timeout aborts the *sibling* in-flight calls too; their work is
    /// re-paid on resume. The watchdog also applies while **draining** — a
    /// hung call trips it rather than stalling the drain forever.
    #[must_use]
    pub const fn max_call_duration(mut self, max_call_duration: std::time::Duration) -> Self {
        self.max_call_duration = Some(max_call_duration);
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
            max_total_calls: self.max_total_calls,
            max_call_duration: self.max_call_duration,
        }
    }
}
