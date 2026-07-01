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

pub mod analytics;
pub mod call_executor;
pub mod environment;
pub mod event;
pub mod event_inventory_saga;
pub mod expiration;
pub mod http;
pub mod inventory;
pub mod payment;
pub mod projection_queries;
pub mod projector;
pub mod reservation;
pub mod reservation_saga;
pub mod testing;

pub use analytics::{
    AnalyticsBusinessLogic, AnalyticsCommand, AnalyticsError, AnalyticsEvent, AnalyticsResponse,
    AnalyticsState, CustomerProfileDto, CustomerSpendingDto, EventSalesDto, PopularSectionsDto,
    PurchaseRecordDto, SectionPopularityDto, SectionSalesDto, TopSpendersDto, TotalRevenueDto,
};
pub use call_executor::{EventInventorySagaCallExecutor, ReservationSagaCallExecutor};
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
pub use http::{
    analytics_routes, availability_routes, events_v2_routes, health_routes, payment_routes,
    query_routes, reservation_query_routes, reservation_routes, AnalyticsAppState,
    AnalyticsQueryEnvironment, EventAggregateHandler, FullQueryAppState, InventoryAggregateHandler,
    InventoryQueryEnvironment, NextAppState, PaymentAggregateHandler, PaymentQueryEnvironment,
    QueryAppState, QueryEnabledAnalyticsHandler, QueryEnabledEnvironment, QueryEnabledEventHandler,
    QueryEnabledInventoryHandler, QueryEnabledPaymentHandler,
    QueryEnabledReservationQueryHandler, ReservationAppState, ReservationQueryAppState,
    ReservationQueryEnvironment, ReservationSagaHandler,
};
pub use inventory::{
    InventoryBusinessLogic, InventoryCommand, InventoryError, InventoryEvent, InventoryResponse,
    InventoryState, ReleaseReason, SectionAvailabilityDto,
};
pub use payment::{
    PaymentBusinessLogic, PaymentCommand, PaymentDto, PaymentDtoStatus, PaymentError,
    PaymentEvent, PaymentResponse, PaymentState,
};
pub use expiration::ReservationExpirationWorker;
pub use projection_queries::{
    AnalyticsProjectionQueries, AnalyticsQueryFetcher, EventInventorySagaProjectionQueries,
    EventInventorySagaQueryFetcher, EventProjectionQueries, EventQueryFetcher,
    InMemoryEventProjectionQueries, InMemoryEventQueryFetcher, InventoryProjectionQueries,
    InventoryQueryFetcher, PaymentProjectionQueries, PaymentQueryFetcher,
    ReservationProjectionQueries, ReservationQueryFetcher, ReservationSagaProjectionQueries,
    ReservationSagaQueryFetcher,
};
pub use projector::{
    EventProjector, InventoryProjector, PaymentProjector, PgAtomicPersist,
    PgEventInventorySagaAtomicPersist, PgEventInventorySagaStateProjector,
    PgReservationSagaStateProjector,
};
pub use reservation::{
    ReservationDto, ReservationListDto, ReservationQueryCommand, ReservationQueryError,
    ReservationQueryEvent, ReservationQueryLogic, ReservationQueryResponse, ReservationQueryState,
    ReservationSummaryDto,
};
pub use reservation_saga::{
    ReservationSagaLogic, ReservationSagaCall, ReservationSagaCallResult, ReservationSagaError,
    ReservationSagaEvent, ReservationSagaInput, ReservationSagaPhase, ReservationSagaState,
};
