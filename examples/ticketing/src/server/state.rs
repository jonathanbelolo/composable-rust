//! Application state for the ticketing HTTP server.
//!
//! Contains all shared resources needed by HTTP handlers:
//! - Authentication store (for session validation)
//! - Event stores (for aggregates)
//! - Projections (for read queries)
//! - Event bus (for saga coordination)
//!
//! # Per-Request Store Pattern
//!
//! This module implements the **per-request store pattern** for privacy and security:
//! - AppState holds **dependencies** (not Store instances)
//! - Each HTTP request creates a **fresh Store** with empty state
//! - Stores load only the data they need from event store
//! - Stores are dropped when request completes
//!
//! This ensures:
//! - **Privacy**: No state shared across different users
//! - **Memory efficiency**: State cleared after each request
//! - **Event sourcing**: Each store rebuilds state from events

use crate::aggregates::{
    inventory::InventoryReducer,
    payment::PaymentReducer,
    reservation::ReservationReducer,
};
use crate::auth::setup::TicketingAuthStore;
use crate::config::Config;
use crate::projections::{
    query_adapters::{PostgresInventoryQuery, PostgresPaymentQuery, PostgresReservationQuery},
    CustomerHistoryProjection, PostgresAvailableSeatsProjection, PostgresEventsProjection,
    PostgresReservationsProjection, ProjectionCompletionTracker, SalesAnalyticsProjection,
};
use crate::types::{CustomerId, PaymentId, ReservationId};
use composable_rust_core::{environment::Clock, event_bus::EventBus};
use composable_rust_postgres::PostgresEventStore;
use axum::extract::FromRef;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Application state shared across all HTTP handlers.
///
/// This struct contains all the dependencies needed by the API endpoints.
/// It's cloned (cheaply via Arc) for each request.
///
/// # Architecture
///
/// The state follows the **per-request store pattern**:
/// - **Dependencies**: Clock, event store, event bus, queries (shared, reusable)
/// - **Stores**: Created fresh for each request (not stored in AppState)
/// - **Projections**: For querying read models (CQRS)
///
/// # Projections
///
/// - **PostgreSQL-backed**: `available_seats_projection` (persistent, crash-safe)
/// - **In-memory**: `sales_analytics_projection`, `customer_history_projection`
///   (fast, but need rebuilding on restart - consumed from `EventBus`)
///
/// # Security Indices
///
/// - **Ownership tracking**: `reservation_ownership`, `payment_ownership`
///   (in-memory indices for fast authorization checks in WebSocket notifications)
#[derive(Clone)]
pub struct AppState {
    /// Configuration (for accessing settings in handlers)
    pub config: Arc<Config>,

    /// Authentication store for session validation and user management
    pub auth_store: Arc<TicketingAuthStore>,

    /// Authentication database pool for querying user roles and auth data
    pub auth_pool: Arc<sqlx::PgPool>,

    // ===== Store Dependencies (shared across requests) =====
    /// Clock for timestamps
    pub clock: Arc<dyn Clock>,

    /// Event store for event-sourced aggregates (write side)
    pub event_store: Arc<PostgresEventStore>,

    /// Event bus for publishing events to sagas and projections
    pub event_bus: Arc<dyn EventBus>,

    // ===== Projection Queries (for on-demand state loading) =====
    /// Inventory projection query
    pub inventory_query: Arc<PostgresInventoryQuery>,

    /// Payment projection query
    pub payment_query: Arc<PostgresPaymentQuery>,

    /// Reservation projection query
    pub reservation_query: Arc<PostgresReservationQuery>,

    // ===== Projections (CQRS read side) =====
    /// Events projection for querying event data (PostgreSQL-backed)
    pub events_projection: Arc<PostgresEventsProjection>,

    /// Reservations projection for querying reservation data (PostgreSQL-backed)
    pub reservations_projection: Arc<PostgresReservationsProjection>,

    /// Available seats projection for fast seat availability queries (PostgreSQL-backed)
    pub available_seats_projection: Arc<PostgresAvailableSeatsProjection>,

    /// Sales analytics projection for revenue and sales metrics (in-memory)
    pub sales_analytics_projection: Arc<RwLock<SalesAnalyticsProjection>>,

    /// Customer history projection for purchase tracking (in-memory)
    pub customer_history_projection: Arc<RwLock<CustomerHistoryProjection>>,

    // ===== Security Indices =====
    /// Ownership index: `ReservationId` → `CustomerId` (for WebSocket notification filtering)
    pub reservation_ownership: Arc<RwLock<HashMap<ReservationId, CustomerId>>>,

    /// Ownership index: `PaymentId` → `ReservationId` (for WebSocket notification filtering)
    pub payment_ownership: Arc<RwLock<HashMap<PaymentId, ReservationId>>>,

    // ===== Projection Completion Tracking =====
    /// Singleton projection completion tracker (ONE consumer for entire app)
    /// Tracks when projections complete processing events for read-after-write consistency
    pub projection_completion_tracker: Arc<ProjectionCompletionTracker>,
}

impl AppState {
    /// Create a new application state.
    ///
    /// # Arguments
    ///
    /// - `config`: Application configuration
    /// - `auth_store`: Authentication store for session management
    /// - `auth_pool`: Authentication database pool for role queries
    /// - `clock`: Clock for timestamps
    /// - `event_store`: Event store for event sourcing
    /// - `event_bus`: Event bus for cross-aggregate communication
    /// - `inventory_query`: Projection query for inventory state loading
    /// - `payment_query`: Projection query for payment state loading
    /// - `reservation_query`: Projection query for reservation state loading
    /// - `events_projection`: Projection for event data queries
    /// - `reservations_projection`: Projection for reservation data queries
    /// - `available_seats_projection`: Projection for seat availability queries
    /// - `sales_analytics_projection`: Projection for sales and revenue analytics
    /// - `customer_history_projection`: Projection for customer purchase history
    /// - `reservation_ownership`: Ownership index for reservation authorization
    /// - `payment_ownership`: Ownership index for payment authorization
    /// - `projection_completion_tracker`: Singleton tracker for projection completion events
    #[must_use]
    #[allow(clippy::too_many_arguments)] // AppState construction requires all dependencies
    pub fn new(
        config: Arc<Config>,
        auth_store: Arc<TicketingAuthStore>,
        auth_pool: Arc<sqlx::PgPool>,
        clock: Arc<dyn Clock>,
        event_store: Arc<PostgresEventStore>,
        event_bus: Arc<dyn EventBus>,
        inventory_query: Arc<PostgresInventoryQuery>,
        payment_query: Arc<PostgresPaymentQuery>,
        reservation_query: Arc<PostgresReservationQuery>,
        events_projection: Arc<PostgresEventsProjection>,
        reservations_projection: Arc<PostgresReservationsProjection>,
        available_seats_projection: Arc<PostgresAvailableSeatsProjection>,
        sales_analytics_projection: Arc<RwLock<SalesAnalyticsProjection>>,
        customer_history_projection: Arc<RwLock<CustomerHistoryProjection>>,
        reservation_ownership: Arc<RwLock<HashMap<ReservationId, CustomerId>>>,
        payment_ownership: Arc<RwLock<HashMap<PaymentId, ReservationId>>>,
        projection_completion_tracker: Arc<ProjectionCompletionTracker>,
    ) -> Self {
        Self {
            config,
            auth_store,
            auth_pool,
            clock,
            event_store,
            event_bus,
            inventory_query,
            payment_query,
            reservation_query,
            events_projection,
            reservations_projection,
            available_seats_projection,
            sales_analytics_projection,
            customer_history_projection,
            reservation_ownership,
            payment_ownership,
            projection_completion_tracker,
        }
    }

    /// Create a fresh Inventory store for this request.
    ///
    /// Each call creates a new Store with empty state. The store will load
    /// only the data it needs from the event store when processing actions.
    ///
    /// # Returns
    ///
    /// A new Inventory store instance for this request.
    #[must_use]
    pub fn create_inventory_store(
        &self,
    ) -> composable_rust_runtime::Store<
        crate::types::InventoryState,
        crate::aggregates::InventoryAction,
        crate::aggregates::inventory::InventoryEnvironment,
        InventoryReducer,
    > {
        use crate::aggregates::inventory::InventoryEnvironment;
        use crate::types::InventoryState;
        use composable_rust_core::stream::StreamId;
        use composable_rust_runtime::Store;

        let env = InventoryEnvironment::new(
            self.clock.clone(),
            self.event_store.clone(),
            self.event_bus.clone(),
            StreamId::new("inventory"),
            self.inventory_query.clone(),
        );

        Store::new(InventoryState::new(), InventoryReducer::new(), env)
    }

    /// Create a fresh Payment store for this request.
    ///
    /// Each call creates a new Store with empty state. The store will load
    /// only the data it needs from the event store when processing actions.
    ///
    /// # Returns
    ///
    /// A new Payment store instance for this request.
    #[must_use]
    pub fn create_payment_store(
        &self,
    ) -> composable_rust_runtime::Store<
        crate::types::PaymentState,
        crate::aggregates::PaymentAction,
        crate::aggregates::payment::PaymentEnvironment,
        PaymentReducer,
    > {
        use crate::aggregates::payment::PaymentEnvironment;
        use crate::types::PaymentState;
        use composable_rust_core::stream::StreamId;
        use composable_rust_runtime::Store;

        let env = PaymentEnvironment::new(
            self.clock.clone(),
            self.event_store.clone(),
            self.event_bus.clone(),
            StreamId::new("payment"),
            self.payment_query.clone(),
        );

        Store::new(PaymentState::new(), PaymentReducer::new(), env)
    }

    /// Create a fresh Reservation store for this request.
    ///
    /// Each call creates a new Store with empty state. The store will load
    /// only the data it needs from the event store when processing actions.
    ///
    /// # Returns
    ///
    /// A new Reservation store instance for this request.
    #[must_use]
    pub fn create_reservation_store(
        &self,
    ) -> composable_rust_runtime::Store<
        crate::types::ReservationState,
        crate::aggregates::ReservationAction,
        crate::aggregates::reservation::ReservationEnvironment,
        ReservationReducer,
    > {
        use crate::aggregates::reservation::ReservationEnvironment;
        use crate::types::ReservationState;
        use composable_rust_core::stream::StreamId;
        use composable_rust_runtime::Store;

        let env = ReservationEnvironment::new(
            self.clock.clone(),
            self.event_store.clone(),
            self.event_bus.clone(),
            StreamId::new("reservation"),
            self.reservation_query.clone(),
        );

        Store::new(ReservationState::new(), ReservationReducer::new(), env)
    }

    /// Create a fresh Event store for this request.
    ///
    /// Each call creates a new Store with empty state. The store will load
    /// only the data it needs from the event store when processing actions.
    ///
    /// # Returns
    ///
    /// A new Event store instance for this request.
    #[must_use]
    pub fn create_event_store(
        &self,
    ) -> composable_rust_runtime::Store<
        crate::types::EventState,
        crate::aggregates::event::EventAction,
        crate::aggregates::event::EventEnvironment,
        crate::aggregates::event::EventReducer,
    > {
        use crate::aggregates::event::{EventEnvironment, EventReducer};
        use crate::types::EventState;
        use composable_rust_core::stream::StreamId;
        use composable_rust_runtime::Store;

        let env = EventEnvironment::new(
            self.clock.clone(),
            self.event_store.clone(),
            self.event_bus.clone(),
            StreamId::new("event"),
        );

        Store::new(EventState::new(), EventReducer::new(), env)
    }
}

// Implement FromRef to allow extractors to get auth_store from AppState
impl FromRef<AppState> for Arc<TicketingAuthStore> {
    fn from_ref(app_state: &AppState) -> Self {
        app_state.auth_store.clone()
    }
}

// Implement FromRef to allow extractors to get config from AppState
impl FromRef<AppState> for Arc<Config> {
    fn from_ref(app_state: &AppState) -> Self {
        app_state.config.clone()
    }
}
