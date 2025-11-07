//! Order Projection Example
//!
//! This example demonstrates the projection system for building read models from events.
//!
//! # CQRS Architecture
//!
//! ```text
//! Write Side (Event Store)          Read Side (Projections)
//! ┌─────────────────────┐          ┌─────────────────────┐
//! │  PostgreSQL DB #1   │          │  PostgreSQL DB #2   │
//! │                     │          │                     │
//! │  events table       │          │  order_projections  │
//! │  snapshots          │   →→→    │  (denormalized)     │
//! │  (normalized)       │  Events  │  (optimized for     │
//! └─────────────────────┘          │   queries)          │
//!                                  └─────────────────────┘
//! ```
//!
//! # What This Example Shows
//!
//! 1. **Projection Trait Implementation**: How to build a read model from events
//! 2. **Event Processing**: Handling different event types (OrderPlaced, OrderCancelled, OrderShipped)
//! 3. **Query API**: Separate query methods for reading the projection
//! 4. **CQRS Separation**: Can use separate databases for events and projections
//! 5. **Checkpoint Resumption**: Projection resumes from last processed event after restart
//! 6. **Rebuild**: Can drop and rebuild the projection from scratch
//!
//! # Key Concepts
//!
//! - **Eventually Consistent**: Projections lag behind events (typically 10-100ms)
//! - **Optimized for Reads**: Schema designed for query patterns, not writes
//! - **Rebuildable**: Can be dropped and rebuilt from events at any time
//! - **Separate Storage**: Can use different database than event store

pub mod projection;

pub use projection::{CustomerOrderHistoryProjection, OrderSummary};
