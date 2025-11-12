//! Application state for the ticketing HTTP server.
//!
//! Contains all shared resources needed by HTTP handlers:
//! - Authentication store (for session validation)
//! - Event stores (for aggregates)
//! - Projections (for read queries)
//! - Event bus (for saga coordination)

use crate::auth::setup::TicketingAuthStore;
use crate::projections::PostgresAvailableSeatsProjection;
use composable_rust_core::event_bus::EventBus;
use composable_rust_postgres::PostgresEventStore;
use axum::extract::FromRef;
use std::sync::Arc;

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
#[derive(Clone)]
pub struct AppState {
    /// Authentication store for session validation and user management
    pub auth_store: Arc<TicketingAuthStore>,

    /// Event store for event-sourced aggregates (write side)
    pub event_store: Arc<PostgresEventStore>,

    /// Event bus for publishing events to sagas and projections
    pub event_bus: Arc<dyn EventBus>,

    /// Available seats projection for fast seat availability queries
    pub available_seats_projection: Arc<PostgresAvailableSeatsProjection>,
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
    #[must_use]
    pub fn new(
        auth_store: Arc<TicketingAuthStore>,
        event_store: Arc<PostgresEventStore>,
        event_bus: Arc<dyn EventBus>,
        available_seats_projection: Arc<PostgresAvailableSeatsProjection>,
    ) -> Self {
        Self {
            auth_store,
            event_store,
            event_bus,
            available_seats_projection,
        }
    }
}

// Implement FromRef to allow extractors to get auth_store from AppState
impl FromRef<AppState> for Arc<TicketingAuthStore> {
    fn from_ref(app_state: &AppState) -> Self {
        app_state.auth_store.clone()
    }
}
