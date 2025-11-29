//! Next-generation architecture implementation for the ticketing system.
//!
//! This module implements the separated architecture from `compiler-target-architecture.md`:
//! - Pure business logic implementing [`BusinessLogic`] trait
//! - Infrastructure handled by the generic [`Handler`]
//!
//! [`BusinessLogic`]: composable_rust_next::BusinessLogic
//! [`Handler`]: composable_rust_next::Handler

pub mod event;
pub mod environment;
pub mod projector;

pub use event::{EventBusinessLogic, EventCommand, EventError, EventEvent, EventState};
pub use environment::TicketingEnvironment;
pub use projector::EventProjector;
