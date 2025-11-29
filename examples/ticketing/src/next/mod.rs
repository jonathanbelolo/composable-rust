//! Next-generation architecture implementation for the ticketing system.
//!
//! This module implements the separated architecture from `compiler-target-architecture.md`:
//! - Pure business logic implementing [`BusinessLogic`] trait
//! - Infrastructure handled by the generic [`Handler`]
//! - HTTP handlers for API endpoints
//!
//! # Architecture
//!
//! ```text
//! HTTP Request → HTTP Handler → Handler.handle() → BusinessLogic → Events
//!                                    │
//!                                    ├─► EventStore (persist)
//!                                    ├─► Projector (read model)
//!                                    └─► EventBus (broadcast)
//! ```
//!
//! [`BusinessLogic`]: composable_rust_next::BusinessLogic
//! [`Handler`]: composable_rust_next::Handler

pub mod environment;
pub mod event;
pub mod http;
pub mod projector;

pub use environment::TicketingEnvironment;
pub use event::{EventBusinessLogic, EventCommand, EventError, EventEvent, EventState};
pub use http::{events_v2_routes, EventHandler, NextAppState};
pub use projector::EventProjector;
