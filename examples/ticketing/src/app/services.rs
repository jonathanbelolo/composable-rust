//! Aggregate services - command handlers that persist and publish events.
//!
//! Services coordinate between reducers, event store, and event bus:
//! 1. Execute reducer with command
//! 2. Persist resulting events to PostgreSQL (source of truth)
//! 3. Publish events to RedPanda (distribution)
//! 4. Return result

use crate::aggregates::{
    InventoryAction, InventoryReducer,
    ReservationAction, ReservationReducer,
    PaymentAction, PaymentReducer,
};
use crate::aggregates::inventory::InventoryEnvironment;
use crate::aggregates::reservation::ReservationEnvironment;
use crate::aggregates::payment::PaymentEnvironment;
use crate::projections::TicketingEvent;
use crate::types::{InventoryState, ReservationState, PaymentState};
use composable_rust_core::environment::SystemClock;
use composable_rust_core::event::SerializedEvent;
use composable_rust_core::event_bus::{EventBus, EventBusError};
use composable_rust_core::event_store::{EventStore, EventStoreError};
use composable_rust_core::reducer::Reducer;
use composable_rust_core::stream::StreamId;
use composable_rust_postgres::PostgresEventStore;
use std::sync::Arc;
use thiserror::Error;

/// Errors that can occur in aggregate services
#[derive(Error, Debug)]
pub enum ServiceError {
    /// Event store operation failed
    #[error("Event store error: {0}")]
    EventStore(#[from] EventStoreError),

    /// Event bus operation failed
    #[error("Event bus error: {0}")]
    EventBus(#[from] EventBusError),

    /// Reducer returned an error
    #[error("Reducer error: {0}")]
    Reducer(String),

    /// Serialization failed
    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Inventory aggregate service
pub struct InventoryService {
    event_store: Arc<PostgresEventStore>,
    event_bus: Arc<dyn EventBus>,
    topic: String,
    reducer: InventoryReducer,
    env: InventoryEnvironment,
}

impl InventoryService {
    /// Create a new inventory service
    pub fn new(
        event_store: Arc<PostgresEventStore>,
        event_bus: Arc<dyn EventBus>,
        topic: String,
    ) -> Self {
        Self {
            event_store,
            event_bus,
            topic,
            reducer: InventoryReducer,
            env: InventoryEnvironment::new(Arc::new(SystemClock)),
        }
    }

    /// Handle an inventory command
    ///
    /// 1. Load current state from event store
    /// 2. Execute reducer
    /// 3. Persist new events
    /// 4. Publish to event bus
    pub async fn handle(
        &self,
        stream_id: StreamId,
        action: InventoryAction,
    ) -> Result<(), ServiceError> {
        // 1. Load current state
        let events = self.event_store.load_events(stream_id.clone(), None).await?;
        let mut state = InventoryState::default();

        // Rebuild state from events
        for event in &events {
            let ticketing_event: TicketingEvent = serde_json::from_slice(&event.data)
                .map_err(|e| ServiceError::Serialization(e.to_string()))?;

            // Extract the inventory action from the wrapper
            if let TicketingEvent::Inventory(inventory_action) = ticketing_event {
                self.reducer.reduce(&mut state, inventory_action, &self.env);
            }
        }

        // 2. Execute reducer with new command
        let _effects = self.reducer.reduce(&mut state, action.clone(), &self.env);

        // 3. Serialize and persist events
        let serialized = self.serialize_action(&action)?;
        let _version = self.event_store.append_events(
            stream_id,
            None, // Optimistic concurrency - would use version in production
            vec![serialized.clone()],
        ).await?;

        // 4. Publish to event bus
        self.event_bus.publish(&self.topic, &serialized).await?;

        tracing::info!(
            action = ?action,
            "Inventory command handled"
        );

        Ok(())
    }

    fn serialize_action(&self, action: &InventoryAction) -> Result<SerializedEvent, ServiceError> {
        let event_type = format!("Inventory{:?}", action).split('(').next().unwrap_or("Unknown").to_string();

        // Wrap in TicketingEvent for unified event stream
        let ticketing_event = TicketingEvent::Inventory(action.clone());
        let data = serde_json::to_vec(&ticketing_event)
            .map_err(|e| ServiceError::Serialization(e.to_string()))?;

        Ok(SerializedEvent::new(event_type, data, None))
    }
}

/// Reservation aggregate service (Saga)
pub struct ReservationService {
    event_store: Arc<PostgresEventStore>,
    event_bus: Arc<dyn EventBus>,
    topic: String,
    reducer: ReservationReducer,
    env: ReservationEnvironment,
}

impl ReservationService {
    /// Create a new reservation service
    pub fn new(
        event_store: Arc<PostgresEventStore>,
        event_bus: Arc<dyn EventBus>,
        topic: String,
    ) -> Self {
        Self {
            event_store,
            event_bus,
            topic,
            reducer: ReservationReducer,
            env: ReservationEnvironment::new(Arc::new(SystemClock)),
        }
    }

    /// Handle a reservation command
    pub async fn handle(
        &self,
        stream_id: StreamId,
        action: ReservationAction,
    ) -> Result<(), ServiceError> {
        // 1. Load current state
        let events = self.event_store.load_events(stream_id.clone(), None).await?;
        let mut state = ReservationState::default();

        for event in &events {
            let ticketing_event: TicketingEvent = serde_json::from_slice(&event.data)
                .map_err(|e| ServiceError::Serialization(e.to_string()))?;

            // Extract the reservation action from the wrapper
            if let TicketingEvent::Reservation(reservation_action) = ticketing_event {
                self.reducer.reduce(&mut state, reservation_action, &self.env);
            }
        }

        // 2. Execute reducer with new command
        let _effects = self.reducer.reduce(&mut state, action.clone(), &self.env);

        // 3. Persist and publish
        let serialized = self.serialize_action(&action)?;
        let _version = self.event_store.append_events(
            stream_id,
            None,
            vec![serialized.clone()],
        ).await?;

        self.event_bus.publish(&self.topic, &serialized).await?;

        tracing::info!(
            action = ?action,
            "Reservation command handled"
        );

        Ok(())
    }

    fn serialize_action(&self, action: &ReservationAction) -> Result<SerializedEvent, ServiceError> {
        let event_type = format!("Reservation{:?}", action).split('(').next().unwrap_or("Unknown").to_string();

        // Wrap in TicketingEvent for unified event stream
        let ticketing_event = TicketingEvent::Reservation(action.clone());
        let data = serde_json::to_vec(&ticketing_event)
            .map_err(|e| ServiceError::Serialization(e.to_string()))?;

        Ok(SerializedEvent::new(event_type, data, None))
    }
}

/// Payment aggregate service
pub struct PaymentService {
    event_store: Arc<PostgresEventStore>,
    event_bus: Arc<dyn EventBus>,
    topic: String,
    reducer: PaymentReducer,
    env: PaymentEnvironment,
}

impl PaymentService {
    /// Create a new payment service
    pub fn new(
        event_store: Arc<PostgresEventStore>,
        event_bus: Arc<dyn EventBus>,
        topic: String,
    ) -> Self {
        Self {
            event_store,
            event_bus,
            topic,
            reducer: PaymentReducer,
            env: PaymentEnvironment::new(Arc::new(SystemClock)),
        }
    }

    /// Handle a payment command
    pub async fn handle(
        &self,
        stream_id: StreamId,
        action: PaymentAction,
    ) -> Result<(), ServiceError> {
        // 1. Load current state
        let events = self.event_store.load_events(stream_id.clone(), None).await?;
        let mut state = PaymentState::default();

        for event in &events {
            let ticketing_event: TicketingEvent = serde_json::from_slice(&event.data)
                .map_err(|e| ServiceError::Serialization(e.to_string()))?;

            // Extract the payment action from the wrapper
            if let TicketingEvent::Payment(payment_action) = ticketing_event {
                self.reducer.reduce(&mut state, payment_action, &self.env);
            }
        }

        // 2. Execute reducer with new command
        let _effects = self.reducer.reduce(&mut state, action.clone(), &self.env);

        // 3. Persist and publish
        let serialized = self.serialize_action(&action)?;
        let _version = self.event_store.append_events(
            stream_id,
            None,
            vec![serialized.clone()],
        ).await?;

        self.event_bus.publish(&self.topic, &serialized).await?;

        tracing::info!(
            action = ?action,
            "Payment command handled"
        );

        Ok(())
    }

    fn serialize_action(&self, action: &PaymentAction) -> Result<SerializedEvent, ServiceError> {
        let event_type = format!("Payment{:?}", action).split('(').next().unwrap_or("Unknown").to_string();

        // Wrap in TicketingEvent for unified event stream
        let ticketing_event = TicketingEvent::Payment(action.clone());
        let data = serde_json::to_vec(&ticketing_event)
            .map_err(|e| ServiceError::Serialization(e.to_string()))?;

        Ok(SerializedEvent::new(event_type, data, None))
    }
}
