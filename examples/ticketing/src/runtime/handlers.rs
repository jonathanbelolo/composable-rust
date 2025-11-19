//! Event handler trait and implementations.
//!
//! This module provides the `EventHandler` trait which allows pluggable
//! event processing logic in the generic `EventConsumer`.
//!
//! # Design Philosophy
//!
//! The `EventHandler` trait is intentionally generic and accepts `Vec<u8>`
//! (raw bytes) rather than a specific event type. This makes the framework
//! reusable across different applications with different event types.
//!
//! Each application implements concrete handlers that:
//! 1. Deserialize the raw bytes into their application-specific event type
//! 2. Process the event (update projections, dispatch to stores, etc.)
//! 3. Return success or error
//!
//! # Example
//!
//! ```rust,ignore
//! use async_trait::async_trait;
//!
//! struct MyHandler {
//!     // Application-specific dependencies
//! }
//!
//! #[async_trait]
//! impl EventHandler for MyHandler {
//!     async fn handle(&self, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
//!         // Deserialize into application-specific event type
//!         let event: MyEvent = bincode::deserialize(data)?;
//!
//!         // Process event
//!         self.process(event).await?;
//!
//!         Ok(())
//!     }
//! }
//! ```

use async_trait::async_trait;
use composable_rust_core::{environment::SystemClock, event_bus::EventBus, stream::StreamId};
use composable_rust_postgres::PostgresEventStore;
use composable_rust_runtime::Store;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{info, warn};

use crate::{
    aggregates::{
        inventory::{InventoryEnvironment, InventoryReducer},
        payment::{PaymentEnvironment, PaymentReducer},
        PaymentAction, ReservationAction,
    },
    projections::{
        query_adapters::{PostgresInventoryQuery, PostgresPaymentQuery},
        CustomerHistoryProjection, Projection, SalesAnalyticsProjection, TicketingEvent,
    },
    types::{CustomerId, InventoryState, PaymentId, PaymentState, ReservationId},
};

/// Handler for processing deserialized events.
///
/// This trait is the core abstraction that makes the `EventConsumer` generic
/// and reusable across different applications. Each application implements
/// concrete handlers for their specific event types and processing logic.
///
/// # Type Parameters
///
/// The trait accepts raw bytes (`&[u8]`) rather than a generic event type
/// to avoid polluting the trait with application-specific generics. This
/// makes the framework more flexible and easier to use.
///
/// # Error Handling
///
/// Handlers should return `Result<(), Box<dyn std::error::Error>>`. Errors
/// are logged by the `EventConsumer` but do not stop event processing.
/// The consumer will continue processing subsequent events.
///
/// # Thread Safety
///
/// Implementors must be `Send + Sync + 'static` because handlers are shared
/// across async tasks.
#[async_trait]
pub trait EventHandler: Send + Sync + 'static {
    /// Handle a raw event (serialized bytes).
    ///
    /// # Arguments
    ///
    /// * `data` - Raw event bytes (typically bincode-serialized)
    ///
    /// # Returns
    ///
    /// - `Ok(())` if event was processed successfully
    /// - `Err(...)` if event processing failed
    ///
    /// # Errors
    ///
    /// Common error cases:
    /// - Deserialization failure (malformed data)
    /// - Business logic errors
    /// - Infrastructure failures (database, network)
    async fn handle(&self, data: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

// ============================================================================
// Aggregate Handlers (Create Store Per Message Pattern)
// ============================================================================

/// Handler for inventory aggregate events.
///
/// This handler demonstrates the **per-message store pattern** where a fresh
/// `Store` is created for each message, ensuring:
/// - Privacy: No state shared across different users/messages
/// - Memory efficiency: State cleared after each message
/// - Event sourcing: Each store loads only the data it needs
///
/// # Architecture
///
/// When an inventory command arrives from the event bus:
/// 1. Deserialize event → extract `InventoryAction`
/// 2. Create fresh `Store` with empty state
/// 3. Dispatch action to store (store loads state from event store if needed)
/// 4. Store processes action and persists events
/// 5. Store dropped - memory freed
pub struct InventoryHandler {
    /// System clock for timestamps
    pub clock: Arc<SystemClock>,

    /// Event store for persistence
    pub event_store: Arc<PostgresEventStore>,

    /// Event bus for publishing events
    pub event_bus: Arc<dyn EventBus>,

    /// Query adapter for loading inventory state on-demand
    pub query: Arc<PostgresInventoryQuery>,
}

#[async_trait]
impl EventHandler for InventoryHandler {
    async fn handle(&self, data: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Deserialize into application-specific event type
        let event: TicketingEvent = bincode::deserialize(data)?;

        // Extract inventory action (if this event is for inventory)
        if let TicketingEvent::Inventory(action) = event {
            info!(action = ?action, "Inventory consumer received command");

            // Create fresh store per message (per-request pattern)
            let env = InventoryEnvironment::new(
                self.clock.clone(),
                self.event_store.clone(),
                self.event_bus.clone(),
                StreamId::new("inventory"),
                self.query.clone(),
            );

            let store = Store::new(InventoryState::new(), InventoryReducer::new(), env);

            // Dispatch action to fresh store
            store.send(action).await?;

            // Store dropped here - memory freed
        }

        Ok(())
    }
}

/// Handler for payment aggregate events.
///
/// Similar to `InventoryHandler`, this implements the per-message store pattern
/// for payment processing. Each payment command gets a fresh store that loads
/// only the payment state it needs from the event store.
pub struct PaymentHandler {
    /// System clock for timestamps
    pub clock: Arc<SystemClock>,

    /// Event store for persistence
    pub event_store: Arc<PostgresEventStore>,

    /// Event bus for publishing events
    pub event_bus: Arc<dyn EventBus>,

    /// Query adapter for loading payment state on-demand
    pub query: Arc<PostgresPaymentQuery>,
}

#[async_trait]
impl EventHandler for PaymentHandler {
    async fn handle(&self, data: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Deserialize into application-specific event type
        let event: TicketingEvent = bincode::deserialize(data)?;

        // Extract payment action (if this event is for payment)
        if let TicketingEvent::Payment(action) = event {
            info!(action = ?action, "Payment consumer received command");

            // Create fresh store per message (per-request pattern)
            let env = PaymentEnvironment::new(
                self.clock.clone(),
                self.event_store.clone(),
                self.event_bus.clone(),
                StreamId::new("payment"),
                self.query.clone(),
            );

            let store = Store::new(PaymentState::new(), PaymentReducer::new(), env);

            // Dispatch action to fresh store
            store.send(action).await?;

            // Store dropped here - memory freed
        }

        Ok(())
    }
}

// ============================================================================
// Projection Handlers (Update Read Models)
// ============================================================================

/// Handler for sales analytics projection events.
///
/// This handler updates the in-memory sales analytics projection and also
/// maintains ownership indices for authorization in WebSocket notifications.
///
/// # Ownership Tracking
///
/// The handler tracks two ownership relationships:
/// - `ReservationId` → `CustomerId` (who owns each reservation)
/// - `PaymentId` → `ReservationId` (which reservation each payment belongs to)
///
/// This enables the WebSocket handler to filter notifications by ownership.
pub struct SalesAnalyticsHandler {
    /// Sales analytics projection (in-memory)
    pub projection: Arc<RwLock<SalesAnalyticsProjection>>,

    /// Ownership index: ReservationId → CustomerId
    pub reservation_ownership: Arc<RwLock<HashMap<ReservationId, CustomerId>>>,

    /// Ownership index: PaymentId → ReservationId
    pub payment_ownership: Arc<RwLock<HashMap<PaymentId, ReservationId>>>,
}

#[async_trait]
impl EventHandler for SalesAnalyticsHandler {
    async fn handle(&self, data: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Deserialize into application-specific event type
        let event: TicketingEvent = bincode::deserialize(data)?;

        // Track ownership for security (WebSocket notification filtering)
        match &event {
            TicketingEvent::Reservation(ReservationAction::ReservationInitiated {
                reservation_id,
                customer_id,
                ..
            }) => {
                if let Ok(mut index) = self.reservation_ownership.write() {
                    index.insert(*reservation_id, *customer_id);
                    info!(
                        reservation_id = %reservation_id.as_uuid(),
                        customer_id = %customer_id.as_uuid(),
                        "Tracked reservation ownership"
                    );
                }
            }
            TicketingEvent::Payment(PaymentAction::PaymentProcessed {
                payment_id,
                reservation_id,
                ..
            }) => {
                if let Ok(mut index) = self.payment_ownership.write() {
                    index.insert(*payment_id, *reservation_id);
                    info!(
                        payment_id = %payment_id.as_uuid(),
                        reservation_id = %reservation_id.as_uuid(),
                        "Tracked payment ownership"
                    );
                }
            }
            _ => {}
        }

        // Update projection
        if let Ok(mut projection) = self.projection.write() {
            projection.handle_event(&event)?;
        } else {
            warn!("Failed to acquire write lock on sales projection");
        }

        Ok(())
    }
}

/// Handler for customer history projection events.
///
/// This handler updates the in-memory customer history projection which tracks
/// all reservations made by each customer for analytics purposes.
///
/// Also maintains a backup copy of the reservation ownership index.
pub struct CustomerHistoryHandler {
    /// Customer history projection (in-memory)
    pub projection: Arc<RwLock<CustomerHistoryProjection>>,

    /// Ownership index: ReservationId → CustomerId (backup tracking)
    pub reservation_ownership: Arc<RwLock<HashMap<ReservationId, CustomerId>>>,
}

#[async_trait]
impl EventHandler for CustomerHistoryHandler {
    async fn handle(&self, data: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Deserialize into application-specific event type
        let event: TicketingEvent = bincode::deserialize(data)?;

        // Track ownership for security (backup tracking)
        if let TicketingEvent::Reservation(ReservationAction::ReservationInitiated {
            reservation_id,
            customer_id,
            ..
        }) = &event
        {
            if let Ok(mut index) = self.reservation_ownership.write() {
                index.insert(*reservation_id, *customer_id);
            }
        }

        // Update projection
        if let Ok(mut projection) = self.projection.write() {
            projection.handle_event(&event)?;
        } else {
            warn!("Failed to acquire write lock on customer projection");
        }

        Ok(())
    }
}
