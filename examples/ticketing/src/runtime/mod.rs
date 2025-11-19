//! Runtime components for event-driven applications.
//!
//! This module provides generic, reusable components for building event-driven
//! applications with Composable Rust:
//!
//! - **`consumer`**: Generic event bus consumer with automatic reconnection
//! - **`handlers`**: Trait and implementations for processing events
//! - **`lifecycle`**: Application lifecycle management and graceful shutdown
//!
//! These components are designed to be framework-level abstractions that can
//! be reused across different applications, not just the ticketing example.

pub mod consumer;
pub mod handlers;
pub mod lifecycle;

pub use consumer::EventConsumer;
pub use handlers::EventHandler;
pub use lifecycle::Application;
