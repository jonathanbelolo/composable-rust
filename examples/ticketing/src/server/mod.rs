//! HTTP server module for the ticketing system.
//!
//! This module provides the Axum-based HTTP server with:
//! - Application state management
//! - Health check endpoints
//! - Metrics endpoint
//! - HTTP middleware (metrics, correlation ID)
//! - Graceful shutdown handling
//! - Router configuration (using next-generation handlers)

pub mod health;
pub mod metrics;
pub mod middleware;
pub mod routes;
pub mod state;

pub use health::health_check;
pub use metrics::metrics_routes;
pub use middleware::metrics_middleware;
pub use routes::build_router;
pub use state::AppState;
