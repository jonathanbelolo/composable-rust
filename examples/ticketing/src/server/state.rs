//! Application state for the ticketing HTTP server.
//!
//! Contains all shared resources needed by HTTP handlers:
//! - Authentication store (for session validation)
//! - Event stores (for aggregates)
//! - Projections (for read queries)
//! - Event bus (for saga coordination)

use crate::auth::setup::TicketingAuthStore;
use crate::projections::{
    CustomerHistoryProjection, PostgresAvailableSeatsProjection, SalesAnalyticsProjection,
};
use crate::types::{CustomerId, PaymentId, ReservationId};
use composable_rust_core::event_bus::EventBus;
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
/// The state follows the Composable Rust pattern:
/// - **Stores**: For sending actions to aggregates (event sourcing)
/// - **Projections**: For querying read models (CQRS)
/// - **Event Bus**: For publishing cross-aggregate events (sagas)
///
/// # Projections
///
/// - **PostgreSQL-backed**: `available_seats_projection` (persistent, crash-safe)
/// - **In-memory**: `sales_analytics_projection`, `customer_history_projection`
///   (fast, but need rebuilding on restart - consumed from EventBus)
///
/// # Security Indices
///
/// - **Ownership tracking**: `reservation_ownership`, `payment_ownership`
///   (in-memory indices for fast authorization checks in WebSocket notifications)
#[derive(Clone)]
pub struct AppState {
    /// Authentication store for session validation and user management
    pub auth_store: Arc<TicketingAuthStore>,

    /// Event store for event-sourced aggregates (write side)
    pub event_store: Arc<PostgresEventStore>,

    /// Event bus for publishing events to sagas and projections
    pub event_bus: Arc<dyn EventBus>,

    /// Available seats projection for fast seat availability queries (PostgreSQL-backed)
    pub available_seats_projection: Arc<PostgresAvailableSeatsProjection>,

    /// Sales analytics projection for revenue and sales metrics (in-memory)
    pub sales_analytics_projection: Arc<RwLock<SalesAnalyticsProjection>>,

    /// Customer history projection for purchase tracking (in-memory)
    pub customer_history_projection: Arc<RwLock<CustomerHistoryProjection>>,

    /// Ownership index: ReservationId → CustomerId (for WebSocket notification filtering)
    pub reservation_ownership: Arc<RwLock<HashMap<ReservationId, CustomerId>>>,

    /// Ownership index: PaymentId → ReservationId (for WebSocket notification filtering)
    pub payment_ownership: Arc<RwLock<HashMap<PaymentId, ReservationId>>>,
}

impl AppState {
    /// Create a new application state.
    ///
    /// # Arguments
    ///
    /// - `auth_store`: Authentication store for session management
    /// - `event_store`: Event store for event sourcing
    /// - `event_bus`: Event bus for cross-aggregate communication
    /// - `available_seats_projection`: Projection for seat availability queries
    /// - `sales_analytics_projection`: Projection for sales and revenue analytics
    /// - `customer_history_projection`: Projection for customer purchase history
    /// - `reservation_ownership`: Ownership index for reservation authorization
    /// - `payment_ownership`: Ownership index for payment authorization
    #[must_use]
    #[allow(clippy::too_many_arguments)] // AppState construction requires all dependencies
    pub fn new(
        auth_store: Arc<TicketingAuthStore>,
        event_store: Arc<PostgresEventStore>,
        event_bus: Arc<dyn EventBus>,
        available_seats_projection: Arc<PostgresAvailableSeatsProjection>,
        sales_analytics_projection: Arc<RwLock<SalesAnalyticsProjection>>,
        customer_history_projection: Arc<RwLock<CustomerHistoryProjection>>,
        reservation_ownership: Arc<RwLock<HashMap<ReservationId, CustomerId>>>,
        payment_ownership: Arc<RwLock<HashMap<PaymentId, ReservationId>>>,
    ) -> Self {
        Self {
            auth_store,
            event_store,
            event_bus,
            available_seats_projection,
            sales_analytics_projection,
            customer_history_projection,
            reservation_ownership,
            payment_ownership,
        }
    }
}

// Implement FromRef to allow extractors to get auth_store from AppState
impl FromRef<AppState> for Arc<TicketingAuthStore> {
    fn from_ref(app_state: &AppState) -> Self {
        app_state.auth_store.clone()
    }
}
