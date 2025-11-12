//! HTTP server module for the ticketing system.
//!
//! This module provides the Axum-based HTTP server with:
//! - Application state management
//! - Health check endpoints
//! - Graceful shutdown handling
//! - Router configuration

pub mod state;
pub mod health;
pub mod routes;

pub use state::AppState;
pub use health::health_check;
pub use routes::build_router;
