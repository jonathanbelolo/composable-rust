//! API endpoints for the ticketing system.
//!
//! This module contains all HTTP API handlers organized by domain:
//! - Events: CRUD operations for events
//! - Availability: Querying seat availability (projections)
//! - Reservations: Creating and managing reservations (saga)
//! - Payments: Payment processing

pub mod events;

pub use events::{create_event, get_event, list_events, update_event, delete_event};
