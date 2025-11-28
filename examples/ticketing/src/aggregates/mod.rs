//! Aggregate reducers for the Event Ticketing System.
//!
//! This module contains all aggregate implementations:
//! - Analytics: Sales and customer metrics queries (query-only)
//! - Event: Event creation and lifecycle management
//! - `EventInventorySaga`: Saga coordinator for event creation with inventory initialization
//! - Inventory: Seat availability and reservation tracking
//! - `ReservationSaga`: Saga coordinator for ticket purchases with compensation
//! - Payment: Payment processing and refunds

pub mod analytics;
pub mod event;
pub mod event_inventory_saga;
pub mod inventory;
pub mod payment;
pub mod reservation_saga;

pub use analytics::{AnalyticsAction, AnalyticsReducer};
pub use event::{EventAction, EventEnvironment, EventReducer};
pub use event_inventory_saga::{EventInventorySagaAction, EventInventorySaga};
pub use inventory::{InventoryAction, InventoryEnvironment, InventoryReducer};
pub use payment::{PaymentAction, PaymentReducer};
pub use reservation_saga::{ReservationAction, ReservationReducer};
