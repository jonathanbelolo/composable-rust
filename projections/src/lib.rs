//! Projection implementations for Composable Rust.
//!
//! # Overview
//!
//! This crate provides concrete implementations of the projection system:
//! - **`PostgreSQL`**: Persistent projection store with JSONB support
//! - **Checkpointing**: PostgreSQL-backed checkpoint tracking
//! - **`ProjectionManager`**: Orchestrates projection updates from events
//!
//! # CQRS Separation
//!
//! For true CQRS, use **separate databases** for event store and projections:
//!
//! ```text
//! Event Store DB (Write)     →  Event Bus  →  Projection DB (Read)
//! ```
//!
//! # Example
//!
//! ```ignore
//! use composable_rust_projections::postgres::*;
//!
//! // Connect to projection database (separate from event store)
//! let projection_store = PostgresProjectionStore::new_with_separate_db(
//!     "postgres://localhost/projections",
//!     "order_projections".to_string(),
//! ).await?;
//!
//! // Use in projection
//! projection_store.save("order:123", &order_data).await?;
//! ```

pub mod manager;
pub mod postgres;

// Re-export main types for convenience
pub use manager::ProjectionManager;
pub use postgres::{PostgresProjectionCheckpoint, PostgresProjectionStore};
