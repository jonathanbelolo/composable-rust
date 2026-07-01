//! Call executors for saga orchestration.
//!
//! This module provides [`CallExecutor`] implementations that dispatch saga calls
//! to the appropriate aggregate handlers.
//!
//! # Architecture
//!
//! ```text
//! Saga → BusinessResult::Continue { events, calls }
//!                                          │
//!                                          ▼
//!                               ┌──────────────────┐
//!                               │  CallExecutor    │
//!                               │  execute(calls)  │
//!                               └────────┬─────────┘
//!                                        │
//!              ┌─────────────────────────┼─────────────────────────┐
//!              ▼                         ▼                         ▼
//!    ┌─────────────────┐      ┌─────────────────┐      ┌─────────────────┐
//!    │ EventHandler    │      │ InventoryHandler│      │ PaymentHandler  │
//!    └─────────────────┘      └─────────────────┘      └─────────────────┘
//!              │                         │                         │
//!              └─────────────────────────┼─────────────────────────┘
//!                                        ▼
//!                                  Vec<CallResult>
//! ```
//!
//! # Type Erasure
//!
//! The call executor uses trait objects (`Arc<dyn InventoryHandler>`) to avoid
//! exposing query fetcher types to the saga. This keeps the saga decoupled from
//! the implementation details of how children fetch their validation data.
//!
//! # Event Retrieval
//!
//! After a handler successfully processes a command, the executor loads the events
//! from the event store to include in the result. This allows sagas to inspect
//! the events for data extraction (e.g., seat IDs from `SeatsReserved`).

use std::sync::Arc;

use composable_rust_next::{
    BusinessLogic, CallExecutor, EventStore, HandleResult, Handler, HandlerEnvironment,
    HandlerError, QueryFetcher, Version,
};
use futures::future::BoxFuture;

use super::event_inventory_saga::{SagaCall, SagaCallResult};
use super::reservation_saga::{ReservationSagaCall, ReservationSagaCallResult};
use super::{
    EventBusinessLogic, EventCommand, EventError, EventEvent, InventoryBusinessLogic,
    InventoryCommand, InventoryError, InventoryEvent, InventoryResponse, PaymentBusinessLogic,
    PaymentCommand, PaymentError, PaymentEvent, PaymentResponse,
};

// ═══════════════════════════════════════════════════════════════════════════
// Type-Erased Handler Traits
// ═══════════════════════════════════════════════════════════════════════════

/// Type-erased handler trait for Inventory aggregate.
///
/// This trait allows the saga call executor to hold handlers without knowing
/// their query fetcher types. The concrete `Handler<InventoryBusinessLogic, ...>`
/// implements this trait.
///
/// Uses `BoxFuture` for dyn-compatibility (async fn in traits can't be used with
/// trait objects because the compiler can't build a vtable for unsized futures).
pub trait InventoryHandler: Send + Sync {
    /// Handle an inventory command.
    fn handle(
        &self,
        command: InventoryCommand,
    ) -> BoxFuture<'_, Result<HandleResult<InventoryResponse>, HandlerError<InventoryError>>>;
}

/// Type-erased handler trait for Payment aggregate.
///
/// This trait allows the saga call executor to hold handlers without knowing
/// their query fetcher types.
pub trait PaymentHandler: Send + Sync {
    /// Handle a payment command.
    fn handle(
        &self,
        command: PaymentCommand,
    ) -> BoxFuture<'_, Result<HandleResult<PaymentResponse>, HandlerError<PaymentError>>>;
}

/// Type-erased handler trait for Event aggregate.
pub trait EventHandler: Send + Sync {
    /// Handle an event command.
    fn handle(
        &self,
        command: EventCommand,
    ) -> BoxFuture<'_, Result<HandleResult<super::event::EventResponse>, HandlerError<EventError>>>;
}

// ═══════════════════════════════════════════════════════════════════════════
// Handler Trait Implementations
// ═══════════════════════════════════════════════════════════════════════════

impl<E, QF, Env> InventoryHandler for Handler<InventoryBusinessLogic, E, QF, Env>
where
    E: CallExecutor<std::convert::Infallible, std::convert::Infallible> + Send + Sync,
    QF: QueryFetcher<InventoryCommand, Env::Projections> + Send + Sync,
    Env: HandlerEnvironment + Send + Sync,
{
    fn handle(
        &self,
        command: InventoryCommand,
    ) -> BoxFuture<'_, Result<HandleResult<InventoryResponse>, HandlerError<InventoryError>>> {
        Box::pin(Handler::handle(self, command))
    }
}

impl<E, QF, Env> PaymentHandler for Handler<PaymentBusinessLogic, E, QF, Env>
where
    E: CallExecutor<std::convert::Infallible, std::convert::Infallible> + Send + Sync,
    QF: QueryFetcher<PaymentCommand, Env::Projections> + Send + Sync,
    Env: HandlerEnvironment + Send + Sync,
{
    fn handle(
        &self,
        command: PaymentCommand,
    ) -> BoxFuture<'_, Result<HandleResult<PaymentResponse>, HandlerError<PaymentError>>> {
        Box::pin(Handler::handle(self, command))
    }
}

impl<E, QF, Env> EventHandler for Handler<EventBusinessLogic, E, QF, Env>
where
    E: CallExecutor<std::convert::Infallible, std::convert::Infallible> + Send + Sync,
    QF: QueryFetcher<EventCommand, Env::Projections> + Send + Sync,
    Env: HandlerEnvironment + Send + Sync,
{
    fn handle(
        &self,
        command: EventCommand,
    ) -> BoxFuture<'_, Result<HandleResult<super::event::EventResponse>, HandlerError<EventError>>>
    {
        Box::pin(Handler::handle(self, command))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Event-Inventory Saga Call Executor
// ═══════════════════════════════════════════════════════════════════════════

/// Executes calls for the Event-Inventory saga.
///
/// Dispatches `SagaCall::Event` to the Event handler and `SagaCall::Inventory`
/// to the Inventory handler. Calls are executed in parallel when possible.
///
/// After each successful handler call, events are loaded from the event store
/// and deserialized to include in the result.
pub struct EventInventorySagaCallExecutor<ES>
where
    ES: EventStore,
{
    event_handler: Arc<dyn EventHandler>,
    inventory_handler: Arc<dyn InventoryHandler>,
    /// Shared event store for loading events after handling
    event_store: ES,
}

impl<ES> EventInventorySagaCallExecutor<ES>
where
    ES: EventStore,
{
    /// Create a new executor with the given handlers and shared event store.
    ///
    /// The event store is used to load events after handler calls complete,
    /// so the saga can inspect the events for data extraction.
    #[must_use]
    pub fn new(
        event_handler: Arc<dyn EventHandler>,
        inventory_handler: Arc<dyn InventoryHandler>,
        event_store: ES,
    ) -> Self {
        Self {
            event_handler,
            inventory_handler,
            event_store,
        }
    }

    /// Execute a single Event call and load resulting events.
    async fn execute_event_call(
        &self,
        command: EventCommand,
        before_version: Version,
    ) -> SagaCallResult {
        let stream_id = EventBusinessLogic::stream_id(&command);

        match self.event_handler.handle(command).await {
            Ok(_result) => {
                // Load new events from the stream
                let events = load_events_since::<EventEvent, _>(
                    &self.event_store,
                    &stream_id,
                    before_version,
                )
                .await;
                SagaCallResult::Event(Ok(events))
            },
            Err(e) => SagaCallResult::Event(Err(map_event_error(e))),
        }
    }

    /// Execute a single Inventory call and load resulting events.
    async fn execute_inventory_call(
        &self,
        section: String,
        command: InventoryCommand,
        before_version: Version,
    ) -> SagaCallResult {
        let stream_id = InventoryBusinessLogic::stream_id(&command);

        match self.inventory_handler.handle(command).await {
            Ok(_result) => {
                let events = load_events_since::<InventoryEvent, _>(
                    &self.event_store,
                    &stream_id,
                    before_version,
                )
                .await;
                SagaCallResult::Inventory {
                    section,
                    result: Ok(events),
                }
            },
            Err(e) => SagaCallResult::Inventory {
                section,
                result: Err(map_inventory_error(e)),
            },
        }
    }

    /// Get current version for a stream (for loading events after).
    async fn get_current_version(&self, stream_id: &composable_rust_next::StreamId) -> Version {
        match self.event_store.load(stream_id, None).await {
            Ok(events) => {
                if events.is_empty() {
                    Version::initial()
                } else {
                    events
                        .last()
                        .and_then(|e| e.version)
                        .unwrap_or_else(|| Version::new(events.len() as u64))
                }
            },
            Err(_) => Version::initial(),
        }
    }
}

impl<ES> CallExecutor<SagaCall, SagaCallResult> for EventInventorySagaCallExecutor<ES>
where
    ES: EventStore + Clone + Send + Sync,
{
    async fn execute(&self, calls: Vec<SagaCall>) -> Vec<SagaCallResult> {
        // Execute all calls in parallel using join_all
        let futures: Vec<_> = calls
            .into_iter()
            .map(|call| async move {
                match call {
                    SagaCall::Event(cmd) => {
                        let stream_id = EventBusinessLogic::stream_id(&cmd);
                        let before_version = self.get_current_version(&stream_id).await;
                        self.execute_event_call(cmd, before_version).await
                    },
                    SagaCall::Inventory { section, command } => {
                        let stream_id = InventoryBusinessLogic::stream_id(&command);
                        let before_version = self.get_current_version(&stream_id).await;
                        self.execute_inventory_call(section, command, before_version)
                            .await
                    },
                }
            })
            .collect();

        futures::future::join_all(futures).await
    }
}

// Manual Debug since Handler doesn't derive Debug with all generics
impl<ES> std::fmt::Debug for EventInventorySagaCallExecutor<ES>
where
    ES: EventStore,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventInventorySagaCallExecutor")
            .finish_non_exhaustive()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Reservation Saga Call Executor
// ═══════════════════════════════════════════════════════════════════════════

/// Executes calls for the Reservation saga.
///
/// Dispatches `ReservationSagaCall::Inventory` to the Inventory handler and
/// `ReservationSagaCall::Payment` to the Payment handler.
///
/// After each successful handler call, events are loaded from the event store
/// to include in the result.
///
/// # Type Erasure
///
/// This executor uses `Arc<dyn InventoryHandler>` and `Arc<dyn PaymentHandler>`
/// instead of concrete types. This keeps the saga decoupled from the query
/// fetcher types used by child handlers - those are implementation details
/// that the saga should not know about.
pub struct ReservationSagaCallExecutor<ES>
where
    ES: EventStore,
{
    inventory_handler: Arc<dyn InventoryHandler>,
    payment_handler: Arc<dyn PaymentHandler>,
    /// Shared event store for loading events after handling
    event_store: ES,
}

impl<ES> ReservationSagaCallExecutor<ES>
where
    ES: EventStore,
{
    /// Create a new executor with the given handlers and shared event store.
    #[must_use]
    pub fn new(
        inventory_handler: Arc<dyn InventoryHandler>,
        payment_handler: Arc<dyn PaymentHandler>,
        event_store: ES,
    ) -> Self {
        Self {
            inventory_handler,
            payment_handler,
            event_store,
        }
    }

    /// Execute a single Inventory call and load resulting events.
    async fn execute_inventory_call(
        &self,
        command: InventoryCommand,
        before_version: Version,
    ) -> ReservationSagaCallResult {
        let stream_id = InventoryBusinessLogic::stream_id(&command);

        match self.inventory_handler.handle(command).await {
            Ok(_result) => {
                let events = load_events_since::<InventoryEvent, _>(
                    &self.event_store,
                    &stream_id,
                    before_version,
                )
                .await;
                ReservationSagaCallResult::Inventory { result: Ok(events) }
            },
            Err(e) => ReservationSagaCallResult::Inventory {
                result: Err(map_inventory_error(e)),
            },
        }
    }

    /// Execute a single Payment call and load resulting events.
    async fn execute_payment_call(
        &self,
        command: PaymentCommand,
        before_version: Version,
    ) -> ReservationSagaCallResult {
        let stream_id = PaymentBusinessLogic::stream_id(&command);

        match self.payment_handler.handle(command).await {
            Ok(_result) => {
                let events = load_events_since::<PaymentEvent, _>(
                    &self.event_store,
                    &stream_id,
                    before_version,
                )
                .await;
                ReservationSagaCallResult::Payment { result: Ok(events) }
            },
            Err(e) => ReservationSagaCallResult::Payment {
                result: Err(map_payment_error(e)),
            },
        }
    }

    /// Get current version for a stream (for loading events after).
    async fn get_current_version(&self, stream_id: &composable_rust_next::StreamId) -> Version {
        match self.event_store.load(stream_id, None).await {
            Ok(events) => {
                if events.is_empty() {
                    Version::initial()
                } else {
                    events
                        .last()
                        .and_then(|e| e.version)
                        .unwrap_or_else(|| Version::new(events.len() as u64))
                }
            },
            Err(_) => Version::initial(),
        }
    }
}

impl<ES> CallExecutor<ReservationSagaCall, ReservationSagaCallResult>
    for ReservationSagaCallExecutor<ES>
where
    ES: EventStore + Clone + Send + Sync,
{
    async fn execute(&self, calls: Vec<ReservationSagaCall>) -> Vec<ReservationSagaCallResult> {
        // Execute all calls in parallel
        let futures: Vec<_> = calls
            .into_iter()
            .map(|call| async move {
                match call {
                    ReservationSagaCall::Inventory(cmd) => {
                        let stream_id = InventoryBusinessLogic::stream_id(&cmd);
                        let before_version = self.get_current_version(&stream_id).await;
                        self.execute_inventory_call(cmd, before_version).await
                    },
                    ReservationSagaCall::Payment(cmd) => {
                        let stream_id = PaymentBusinessLogic::stream_id(&cmd);
                        let before_version = self.get_current_version(&stream_id).await;
                        self.execute_payment_call(cmd, before_version).await
                    },
                }
            })
            .collect();

        futures::future::join_all(futures).await
    }
}

// Manual Debug
impl<ES> std::fmt::Debug for ReservationSagaCallExecutor<ES>
where
    ES: EventStore,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReservationSagaCallExecutor")
            .finish_non_exhaustive()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Helper: Load Events Since Version
// ═══════════════════════════════════════════════════════════════════════════

/// Load events from a stream since a given version and deserialize them.
///
/// This is used by call executors to retrieve the events that were persisted
/// by a handler call, so sagas can inspect them.
async fn load_events_since<E, ES>(
    event_store: &ES,
    stream_id: &composable_rust_next::StreamId,
    since_version: Version,
) -> Vec<E>
where
    E: serde::de::DeserializeOwned,
    ES: EventStore,
{
    // Load from the version AFTER the before version
    let from_version = if since_version == Version::initial() {
        None
    } else {
        Some(since_version)
    };

    match event_store.load(stream_id, from_version).await {
        Ok(serialized_events) => serialized_events
            .into_iter()
            .filter_map(|se| bincode::deserialize(&se.payload).ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Error Mapping Helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Map handler errors to domain errors for Event aggregate.
fn map_event_error<E>(e: composable_rust_next::HandlerError<E>) -> EventError
where
    E: std::error::Error,
{
    match e {
        composable_rust_next::HandlerError::Business(business_err) => {
            EventError::ValidationFailed {
                message: business_err.to_string(),
            }
        },
        composable_rust_next::HandlerError::Load(store_err) => EventError::ValidationFailed {
            message: format!("Load error: {store_err}"),
        },
        composable_rust_next::HandlerError::Persist(store_err) => EventError::ValidationFailed {
            message: format!("Persist error: {store_err}"),
        },
        composable_rust_next::HandlerError::Projection(proj_err) => EventError::ValidationFailed {
            message: format!("Projection error: {proj_err}"),
        },
        composable_rust_next::HandlerError::Broadcast(bus_err) => EventError::ValidationFailed {
            message: format!("Broadcast error: {bus_err}"),
        },
        composable_rust_next::HandlerError::Serialization(ser_err) => {
            EventError::ValidationFailed {
                message: format!("Serialization error: {ser_err}"),
            }
        },
        composable_rust_next::HandlerError::QueryFetch(fetch_err) => EventError::ValidationFailed {
            message: format!("Query fetch error: {fetch_err}"),
        },
        composable_rust_next::HandlerError::SagaIterationsExceeded { max_iterations } => {
            EventError::ValidationFailed {
                message: format!("Saga exceeded max iterations: {max_iterations}"),
            }
        },
    }
}

/// Map handler errors to domain errors for Inventory aggregate.
fn map_inventory_error<E>(e: composable_rust_next::HandlerError<E>) -> InventoryError
where
    E: std::error::Error,
{
    let message = match e {
        composable_rust_next::HandlerError::Business(business_err) => business_err.to_string(),
        composable_rust_next::HandlerError::Load(store_err) => format!("Load error: {store_err}"),
        composable_rust_next::HandlerError::Persist(store_err) => {
            format!("Persist error: {store_err}")
        },
        composable_rust_next::HandlerError::Projection(proj_err) => {
            format!("Projection error: {proj_err}")
        },
        composable_rust_next::HandlerError::Broadcast(bus_err) => {
            format!("Broadcast error: {bus_err}")
        },
        composable_rust_next::HandlerError::Serialization(ser_err) => {
            format!("Serialization error: {ser_err}")
        },
        composable_rust_next::HandlerError::QueryFetch(fetch_err) => {
            format!("Query fetch error: {fetch_err}")
        },
        composable_rust_next::HandlerError::SagaIterationsExceeded { max_iterations } => {
            format!("Saga exceeded max iterations: {max_iterations}")
        },
    };
    InventoryError::ValidationFailed(message)
}

/// Map handler errors to domain errors for Payment aggregate.
fn map_payment_error<E>(e: composable_rust_next::HandlerError<E>) -> PaymentError
where
    E: std::error::Error,
{
    let message = match e {
        composable_rust_next::HandlerError::Business(business_err) => business_err.to_string(),
        composable_rust_next::HandlerError::Load(store_err) => format!("Load error: {store_err}"),
        composable_rust_next::HandlerError::Persist(store_err) => {
            format!("Persist error: {store_err}")
        },
        composable_rust_next::HandlerError::Projection(proj_err) => {
            format!("Projection error: {proj_err}")
        },
        composable_rust_next::HandlerError::Broadcast(bus_err) => {
            format!("Broadcast error: {bus_err}")
        },
        composable_rust_next::HandlerError::Serialization(ser_err) => {
            format!("Serialization error: {ser_err}")
        },
        composable_rust_next::HandlerError::QueryFetch(fetch_err) => {
            format!("Query fetch error: {fetch_err}")
        },
        composable_rust_next::HandlerError::SagaIterationsExceeded { max_iterations } => {
            format!("Saga exceeded max iterations: {max_iterations}")
        },
    };
    PaymentError::ValidationFailed(message)
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::match_wildcard_for_single_variants
)]
mod tests {
    use super::*;
    use crate::next::TicketingEnvironment;
    use crate::types::{Capacity, EventId};
    use chrono::Utc;
    use composable_rust_next::testing::{InMemoryEventBus, InMemoryEventStore, InMemoryProjector};
    use composable_rust_next::{FixedClock, NoOpCallExecutor, NoOpQueryFetcher};

    type TestEnv =
        TicketingEnvironment<FixedClock, InMemoryEventStore, InMemoryProjector, InMemoryEventBus>;

    fn create_test_env(event_store: InMemoryEventStore) -> TestEnv {
        let clock = FixedClock::new(Utc::now());
        let projector = InMemoryProjector::new();
        let event_bus = InMemoryEventBus::new();

        TicketingEnvironment::new(
            clock,
            event_store,
            Some(projector),
            Some(event_bus),
            "test-events",
        )
    }

    #[tokio::test]
    async fn event_inventory_executor_handles_inventory_call() {
        // Shared event store for all handlers and the executor
        let event_store = InMemoryEventStore::new();

        let event_env = create_test_env(event_store.clone());
        let inventory_env = create_test_env(event_store.clone());

        let event_handler: Arc<dyn EventHandler> = Arc::new(Handler::new(
            EventBusinessLogic,
            NoOpCallExecutor,
            NoOpQueryFetcher,
            event_env,
        ));
        let inventory_handler: Arc<dyn InventoryHandler> = Arc::new(Handler::new(
            InventoryBusinessLogic,
            NoOpCallExecutor,
            NoOpQueryFetcher,
            inventory_env,
        ));

        let executor =
            EventInventorySagaCallExecutor::new(event_handler, inventory_handler, event_store);

        // Execute an inventory initialization call
        let event_id = EventId::new();
        let calls = vec![SagaCall::Inventory {
            section: "VIP".to_string(),
            command: InventoryCommand::Initialize {
                event_id,
                section: "VIP".to_string(),
                capacity: Capacity::new(100),
                fetched: None,
            },
        }];

        let results = executor.execute(calls).await;

        assert_eq!(results.len(), 1);
        // The call should succeed
        match &results[0] {
            SagaCallResult::Inventory { section, result } => {
                assert_eq!(section, "VIP");
                assert!(result.is_ok());
                // Should have loaded the InventoryInitialized event
                let events = result.as_ref().unwrap();
                assert_eq!(events.len(), 1);
                assert!(matches!(events[0], InventoryEvent::Initialized { .. }));
            },
            _ => panic!("Expected Inventory result"),
        }
    }

    #[tokio::test]
    async fn reservation_executor_handles_inventory_call() {
        // Shared event store
        let event_store = InMemoryEventStore::new();

        let inventory_env = create_test_env(event_store.clone());
        let payment_env = create_test_env(event_store.clone());

        let inventory_handler: Arc<dyn InventoryHandler> = Arc::new(Handler::new(
            InventoryBusinessLogic,
            NoOpCallExecutor,
            NoOpQueryFetcher,
            inventory_env,
        ));
        let payment_handler: Arc<dyn PaymentHandler> = Arc::new(Handler::new(
            PaymentBusinessLogic,
            NoOpCallExecutor,
            NoOpQueryFetcher,
            payment_env,
        ));

        let executor =
            ReservationSagaCallExecutor::new(inventory_handler, payment_handler, event_store);

        // Execute an inventory initialization call
        let event_id = EventId::new();
        let calls = vec![ReservationSagaCall::Inventory(
            InventoryCommand::Initialize {
                event_id,
                section: "GA".to_string(),
                capacity: Capacity::new(50),
                fetched: None,
            },
        )];

        let results = executor.execute(calls).await;

        assert_eq!(results.len(), 1);
        match &results[0] {
            ReservationSagaCallResult::Inventory { result } => {
                assert!(result.is_ok());
                // Should have loaded events
                let events = result.as_ref().unwrap();
                assert_eq!(events.len(), 1);
                assert!(matches!(events[0], InventoryEvent::Initialized { .. }));
            },
            _ => panic!("Expected Inventory result"),
        }
    }
}
