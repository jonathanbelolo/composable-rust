//! Advanced Resilience Patterns (Phase 8.4 Part 3)
//!
//! Production-ready resilience patterns for agent systems:
//! - Circuit breakers: Prevent cascading failures
//! - Rate limiters: Prevent resource exhaustion
//! - Bulkheads: Isolate failures

pub mod bulkhead;
pub mod circuit_breaker;
pub mod rate_limiter;

// Re-export common types
pub use bulkhead::{Bulkhead, BulkheadConfig, BulkheadRegistry};
pub use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState};
pub use rate_limiter::{RateLimiter, RateLimiterConfig};
