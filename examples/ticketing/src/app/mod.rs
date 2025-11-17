//! Application coordinator - wires together all components.
//!
//! This module provides the main application structure that coordinates:
//! - Event store (`PostgreSQL`)
//! - Event bus (`RedPanda`)
//! - Aggregate stores (Composable Rust Store runtime)
//! - Projection managers (read model subscribers)

mod coordinator;

pub use coordinator::TicketingApp;
