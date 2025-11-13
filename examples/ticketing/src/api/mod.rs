//! API endpoints for the ticketing system.
//!
//! This module contains all HTTP API handlers organized by domain:
//! - Events: CRUD operations for events
//! - Availability: Querying seat availability (projections)
//! - Reservations: Creating and managing reservations (saga)
//! - Payments: Payment processing

pub mod availability;
pub mod events;

pub use availability::{get_event_availability, get_section_availability, get_total_available};
pub use events::{create_event, delete_event, get_event, list_events, update_event};
