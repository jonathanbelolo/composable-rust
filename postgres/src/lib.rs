//! `PostgreSQL` event store implementation for Composable Rust.
//!
//! This crate provides a production-ready PostgreSQL-based event store that implements
//! the `EventStore` trait from `composable-rust-core`. It uses sqlx for compile-time
//! checked queries and supports:
//!
//! - Event persistence with optimistic concurrency
//! - State snapshots for performance
//! - Connection pooling
//! - Transaction support
//!
//! # Example
//!
//! ```ignore
//! use composable_rust_postgres::PostgresEventStore;
//!
//! async fn example() -> Result<(), Box<dyn std::error::Error>> {
//!     let event_store = PostgresEventStore::new("postgres://localhost/mydb").await?;
//!     Ok(())
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

// Module structure will be added as we implement
