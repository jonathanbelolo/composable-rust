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
//! # Saga Coordination
//!
//! For multi-aggregate workflows, use [`EventInventorySagaLogic`]:
//!
//! ```text
//! CreateEventWithInventory
//!     │
//!     ▼
//! SagaLogic.process() ──► Continue { events, calls: [CreateEvent] }
//!     │
//!     ▼ (Handler executes)
//! EventLogic ──► Success
//!     │
//!     ▼ (feedback_input)
//! SagaLogic.process() ──► Continue { events, calls: [InitInventory...] }
//!     │
//!     ▼ (Handler executes in parallel)
//! InventoryLogic ──► Success/Failure
//!     │
//!     ▼
//! SagaLogic.process() ──► Done (or compensation)
//! ```
//!
//! [`BusinessLogic`]: composable_rust_next::BusinessLogic
//! [`Handler`]: composable_rust_next::Handler
//! [`EventInventorySagaLogic`]: saga::EventInventorySagaLogic

pub mod environment;
pub mod event;
pub mod http;
pub mod inventory;
pub mod projector;
pub mod saga;

pub use environment::TicketingEnvironment;
pub use event::{EventBusinessLogic, EventCommand, EventError, EventEvent, EventState};
pub use http::{events_v2_routes, EventHandler, NextAppState};
pub use inventory::{
    InventoryBusinessLogic, InventoryCommand, InventoryError, InventoryEvent, InventoryState,
};
pub use projector::EventProjector;
pub use saga::{
    EventInventorySagaLogic, SagaCall, SagaCallResult, SagaError, SagaEvent, SagaInput, SagaPhase,
    SagaState,
};
