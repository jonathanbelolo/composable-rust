//! Test infrastructure for handler-level integration tests.
//!
//! This module provides in-memory implementations of projectors and query fetchers
//! that share state, enabling fast, deterministic testing of handlers without a database.
//!
//! # Architecture
//!
//! ```text
//! Handler.handle(command)
//!     │
//!     ├── persist events to InMemoryEventStore
//!     │
//!     ├── Projector.project(events)
//!     │        │
//!     │        └── writes to InMemory*Projections
//!     │
//!     └── (next iteration) QueryFetcher.fetch(command)
//!              │
//!              └── reads from InMemory*Projections
//! ```
//!
//! # Test Levels
//!
//! This module enables Level 1-3 tests from the test guidelines:
//!
//! - **Level 1**: Handler with NoOpQueryFetcher (tests event persistence)
//! - **Level 2**: Handler with real QueryFetcher reading from projections
//! - **Level 3**: Saga with child handlers sharing infrastructure
//!
//! # Usage
//!
//! ```ignore
//! use ticketing::next::testing::{
//!     InMemoryInventoryProjections, InMemoryInventoryProjector, InMemoryInventoryQueryFetcher,
//!     InventoryTestHarness,
//! };
//!
//! // Create a fully-wired test harness
//! let harness = InventoryTestHarness::new();
//!
//! // Initialize inventory
//! harness.handle(InventoryCommand::Initialize { ... }).await?;
//!
//! // Verify state via projections
//! let dto = harness.get_inventory(event_id, "VIP").await.unwrap();
//! assert!(dto.initialized);
//!
//! // Reserve seats - the query fetcher will see the initialized inventory
//! harness.handle(InventoryCommand::Reserve { ... }).await?;
//! ```

mod harness;
mod inventory;
mod payment;

pub use harness::ReservationSagaTestHarness;
pub use inventory::{
    InMemoryInventoryProjections, InMemoryInventoryProjector, InMemoryInventoryQueryFetcher,
    InventoryTestEnvironment, InventoryTestHarness,
};
pub use payment::{
    InMemoryPaymentProjections, InMemoryPaymentProjector, InMemoryPaymentQueryFetcher,
    PaymentTestEnvironment, PaymentTestHarness,
};
