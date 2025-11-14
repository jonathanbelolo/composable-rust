//! API endpoints for the ticketing system.
//!
//! This module contains all HTTP API handlers organized by domain:
//! - Events: CRUD operations for events
//! - Availability: Querying seat availability (projections)
//! - Reservations: Creating and managing reservations (saga)
//! - Payments: Payment processing
//! - Analytics: Business intelligence and reporting
//! - WebSocket: Real-time updates and notifications

pub mod analytics;
pub mod availability;
pub mod events;
pub mod payments;
pub mod reservations;
pub mod websocket;

pub use analytics::{
    get_customer_profile, get_event_sales, get_popular_sections, get_top_spenders,
    get_total_revenue,
};
pub use availability::{get_event_availability, get_section_availability, get_total_available};
pub use events::{create_event, delete_event, get_event, list_events, update_event};
pub use payments::{get_payment, list_user_payments, process_payment, refund_payment};
pub use reservations::{
    cancel_reservation, create_reservation, get_reservation, list_user_reservations,
};
pub use websocket::{active_connection_count, availability_updates, personal_notifications};
