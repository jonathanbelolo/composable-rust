//! Read model projections for the Event Ticketing System.
//!
//! This module implements the **read side** of CQRS (Command Query Responsibility Segregation).
//! Projections consume events from the event store and build denormalized views optimized
//! for queries.
//!
//! ## Architecture
//!
//! ```text
//! Write Side (Aggregates)          Event Store          Read Side (Projections)
//! ┌─────────────────┐             ┌──────────┐         ┌────────────────────┐
//! │  Event          │────events──>│PostgreSQL│────────>│ AvailableSeats     │
//! │  Inventory      │             │  Event   │         │ SalesAnalytics     │
//! │  Reservation    │             │  Store   │         │ CustomerHistory    │
//! │  Payment        │             └──────────┘         └────────────────────┘
//! └─────────────────┘                                           │
//!                                                                v
//!                                                         Fast Queries
//! ```
//!
//! ## Projections
//!
//! ### 1. [`AvailableSeatsProjection`]
//!
//! Denormalized view of seat availability for fast lookups:
//! - Query: "Show me all available VIP seats for Event X"
//! - No need to recompute from total - reserved - sold
//! - Updated on: `SeatsReserved`, `SeatsConfirmed`, `SeatsReleased`
//!
//! ### 2. [`SalesAnalyticsProjection`]
//!
//! Revenue and sales metrics:
//! - Query: "What's our total revenue for Event X?"
//! - Query: "Which pricing tier is most popular?"
//! - Updated on: `PaymentSucceeded`, `ReservationCompleted`
//!
//! ### 3. [`CustomerHistoryProjection`]
//!
//! Customer purchase history:
//! - Query: "Show all tickets purchased by Customer Y"
//! - Query: "Has this customer attended this venue before?"
//! - Updated on: `ReservationCompleted`, `PaymentSucceeded`
//!
//! ## Event Sourcing Integration
//!
//! Projections are **eventually consistent**:
//! 1. Aggregates emit events to event store
//! 2. Event store persists events (write side completes)
//! 3. Projections subscribe to event stream
//! 4. Projections update their views (read side catches up)
//!
//! This means queries might see slightly stale data (milliseconds behind), but:
//! - Writes are never blocked by read model updates
//! - Projections can be rebuilt from event history
//! - Multiple projections can run independently
//!
//! ## Projection Rebuilding
//!
//! Since projections are derived from events, they can be rebuilt:
//!
//! ```rust,ignore
//! // Rebuild projection from scratch
//! let mut projection = AvailableSeatsProjection::new();
//! for event in event_store.load_all_events() {
//!     projection.handle_event(&event)?;
//! }
//! ```
//!
//! This enables:
//! - Adding new projections to existing systems
//! - Fixing bugs in projection logic (replay with fix)
//! - Experimenting with different views (replay into new projection)

// Projection completion tracking
pub mod completion;

// PostgreSQL-backed projections (framework-compatible)
pub mod available_seats_postgres;
pub mod customer_history_postgres;
pub mod events_postgres;
pub mod manager;
pub mod payments_postgres;
pub mod query_adapters;
pub mod sales_analytics_postgres;

// In-memory projections (for testing and legacy compatibility)
pub mod available_seats;
pub mod customer_history;
pub mod event_projection;
pub mod sales_analytics;

pub use available_seats::{AvailableSeatsProjection, SeatAvailability};
pub use available_seats_postgres::{PostgresAvailableSeatsProjection, SectionAvailability};
pub use completion::{
    CorrelationId, ProjectionCompleted, ProjectionCompletionEvent, ProjectionCompletionTracker,
    ProjectionFailed, ProjectionResult,
};
pub use customer_history::{CustomerHistoryProjection, CustomerPurchase};
pub use customer_history_postgres::PostgresCustomerHistoryProjection;
pub use events_postgres::PostgresEventsProjection;
pub use manager::{setup_projection_managers, ProjectionManagers};
pub use payments_postgres::PostgresPaymentsProjection;
pub use sales_analytics::{SalesAnalyticsProjection, SalesMetrics};
pub use sales_analytics_postgres::PostgresSalesAnalyticsProjection;

use crate::aggregates::{EventAction, InventoryAction, PaymentAction, ReservationAction};
use composable_rust_core::event::SerializedEvent;
use serde::{Deserialize, Serialize};

/// Unified event type for all ticketing aggregates.
///
/// Projections consume this unified event stream to build their views.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TicketingEvent {
    /// Event from the Event aggregate
    Event(EventAction),
    /// Event from the Inventory aggregate
    Inventory(InventoryAction),
    /// Event from the Reservation aggregate
    Reservation(ReservationAction),
    /// Event from the Payment aggregate
    Payment(PaymentAction),
}

impl TicketingEvent {
    /// Serialize an action into a `SerializedEvent` for event store persistence
    ///
    /// # Errors
    ///
    /// Returns error if serialization fails
    pub fn serialize(self) -> Result<SerializedEvent, String> {
        let event_type = match &self {
            Self::Event(action) => format!("Event{action:?}"),
            Self::Inventory(action) => format!("Inventory{action:?}"),
            Self::Reservation(action) => format!("Reservation{action:?}"),
            Self::Payment(action) => format!("Payment{action:?}"),
        }
        .split('(')
        .next()
        .unwrap_or("Unknown")
        .to_string();

        let data = bincode::serialize(&self)
            .map_err(|e| format!("Serialization error: {e}"))?;

        Ok(SerializedEvent::new(event_type, data, None))
    }

    /// Deserialize a `SerializedEvent` back into a `TicketingEvent`
    ///
    /// # Errors
    ///
    /// Returns error if deserialization fails
    pub fn deserialize(event: &SerializedEvent) -> Result<Self, String> {
        bincode::deserialize(&event.data)
            .map_err(|e| format!("Deserialization error: {e}"))
    }
}

/// Trait for projections that consume events to build read models.
///
/// Projections are **event handlers** that update denormalized views.
pub trait Projection: Send + Sync {
    /// Handle a ticketing event and update the projection's view.
    ///
    /// This method is called for each event in the event stream.
    /// Projections should:
    /// - Check if the event is relevant to their view
    /// - Extract necessary data from the event
    /// - Update their internal state/database
    ///
    /// # Errors
    ///
    /// Returns an error if the projection fails to update (e.g., database error).
    fn handle_event(&mut self, event: &TicketingEvent) -> Result<(), String>;

    /// Get the projection's name (for logging/debugging).
    fn name(&self) -> &'static str;

    /// Reset the projection to initial state.
    ///
    /// Used for rebuilding projections from scratch.
    fn reset(&mut self);
}
