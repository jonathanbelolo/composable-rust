//! Projection implementations for Composable Rust.
//!
//! # Overview
//!
//! This crate provides concrete implementations of the projection system:
//! - **`PostgreSQL`**: Persistent projection store with JSONB support
//! - **Checkpointing**: PostgreSQL-backed checkpoint tracking
//! - **`ProjectionStream`**: Type-agnostic event stream helper for building projections
//!
//! # CQRS Separation
//!
//! For true CQRS, use **separate databases** for event store and projections:
//!
//! ```text
//! Event Store DB (Write)     →  Event Bus  →  Projection DB (Read)
//! ```
//!
//! # Building Projections
//!
//! Use `ProjectionStream` for consuming events with checkpoint tracking:
//!
//! ```ignore
//! use composable_rust_projections::ProjectionStream;
//!
//! let mut stream = ProjectionStream::new(
//!     event_bus,
//!     checkpoint,
//!     "inventory-events",
//!     "available-seats-projection",
//!     "available-seats",
//! ).await?;
//!
//! while let Some(result) = stream.next().await {
//!     let serialized = result?;
//!
//!     // Client knows the concrete type
//!     let event: InventoryEvent = bincode::deserialize(&serialized.data)?;
//!
//!     // Update projection
//!     projection.handle_event(&event)?;
//!
//!     // Commit checkpoint
//!     stream.commit().await?;
//! }
//! ```

pub mod manager;
pub mod postgres;
pub mod stream;

// Re-export main types for convenience
#[deprecated(
    since = "0.1.0",
    note = "Use ProjectionStream instead for better type safety with bincode deserialization"
)]
pub use manager::ProjectionManager;
pub use postgres::{PostgresProjectionCheckpoint, PostgresProjectionStore};
pub use stream::ProjectionStream;
