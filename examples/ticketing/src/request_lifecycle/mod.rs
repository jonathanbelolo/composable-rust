//! Request lifecycle management for HTTP-based event-sourced applications.
//!
//! This module provides a **Request Lifecycle Store** that tracks the complete lifecycle
//! of HTTP requests from initiation through domain event processing, projection updates,
//! and external operations (emails, notifications, webhooks).
//!
//! # Architecture
//!
//! ```text
//! HTTP Request → RequestLifecycleStore
//!                 ↓
//!                 Creates RequestLifecycle aggregate with correlation_id
//!                 ↓
//!                 Dispatches business domain action
//!                 ↓
//! Business Domain Reducer emits events → EventBus
//!                 ↓
//! Projections consume → Update → Emit ProjectionCompleted
//!                 ↓
//! RequestLifecycleStore receives ProjectionCompleted
//!                 ↓
//!                 Marks projection as done
//!                 ↓
//!                 When ALL done → Emits RequestCompleted
//!                 ↓
//! WebSocket broadcasts → Tests/clients know request is fully processed
//! ```
//!
//! # Key Insight
//!
//! **Request lifecycle tracking is NOT business domain logic.** It's an infrastructure
//! concern orthogonal to domain aggregates like Event, Reservation, Inventory.
//!
//! This keeps the business domain clean and reusable across different interfaces
//! (HTTP, CLI, gRPC, message queues).

pub mod actions;
pub mod environment;
pub mod reducer;
pub mod store;
#[cfg(test)]
mod tests;
pub mod types;

pub use actions::RequestLifecycleAction;
pub use environment::RequestLifecycleEnvironment;
pub use reducer::RequestLifecycleReducer;
pub use store::RequestLifecycleStore;
pub use types::{
    CorrelationId, RequestLifecycle, RequestLifecycleState, RequestMetadata, RequestStatus,
};
