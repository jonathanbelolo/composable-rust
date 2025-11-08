//! Application coordinator - wires together all components.
//!
//! This module provides the main application structure that coordinates:
//! - Event store (PostgreSQL)
//! - Event bus (RedPanda)
//! - Aggregate services (command handlers)
//! - Projection managers (read model subscribers)

mod coordinator;
mod services;

pub use coordinator::TicketingApp;
pub use services::{InventoryService, ReservationService, PaymentService};
