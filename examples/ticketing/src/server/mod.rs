//! HTTP server module for the ticketing system.
//!
//! This module provides the Axum-based HTTP server with:
//! - Health check endpoints
//! - Metrics endpoint
//! - Router state types

pub mod health;
pub mod metrics;
pub mod routes;

pub use health::health_check;
pub use metrics::metrics_routes;
pub use routes::AuthAppState;
