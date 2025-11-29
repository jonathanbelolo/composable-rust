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
pub mod event_inventory_saga;
pub mod http;
pub mod inventory;
pub mod payment;
pub mod projector;
pub mod reservation_saga;

pub use environment::{
    NoOpEventBus, NoOpProjector, ProductionEnvironment, ProductionEnvironmentWithProjector,
    TicketingEnvironment,
};
pub use event::{EventBusinessLogic, EventCommand, EventError, EventEvent, EventState};
pub use event_inventory_saga::{
    EventInventorySagaLogic, SagaCall as EventInventorySagaCall,
    SagaCallResult as EventInventorySagaCallResult, SagaError as EventInventorySagaError,
    SagaEvent as EventInventorySagaEvent, SagaInput as EventInventorySagaInput,
    SagaPhase as EventInventorySagaPhase, SagaState as EventInventorySagaState,
};
pub use http::{events_v2_routes, EventHandler, NextAppState};
pub use inventory::{
    InventoryBusinessLogic, InventoryCommand, InventoryError, InventoryEvent, InventoryState,
};
pub use payment::{PaymentBusinessLogic, PaymentCommand, PaymentError, PaymentEvent, PaymentState};
pub use projector::EventProjector;
pub use reservation_saga::{
    ReservationSagaLogic, ReservationSagaCall, ReservationSagaCallResult, ReservationSagaError,
    ReservationSagaEvent, ReservationSagaInput, ReservationSagaPhase, ReservationSagaState,
};
