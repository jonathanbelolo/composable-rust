//! Aggregate reducers for the Event Ticketing System.
//!
//! This module contains all aggregate implementations:
//! - Event: Event creation and lifecycle management
//! - Inventory: Seat availability and reservation tracking
//! - Reservation: Saga coordinator for ticket purchases
//! - Payment: Payment processing and refunds

pub mod event;
pub mod inventory;
pub mod payment;
pub mod reservation;

pub use event::{EventAction, EventReducer};
pub use inventory::{InventoryAction, InventoryReducer};
pub use payment::{PaymentAction, PaymentReducer};
pub use reservation::{ReservationAction, ReservationReducer};
