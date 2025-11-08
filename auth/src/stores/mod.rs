//! Storage implementations for auth system.
//!
//! This module provides persistent and ephemeral storage for authentication state:
//!
//! - **Session Store** (Redis) - Ephemeral session storage with TTL
//! - **Device Registry** (PostgreSQL) - Persistent device tracking (planned)
//! - **Challenge Store** (Redis) - WebAuthn challenge storage (planned)

pub mod session_redis;

// Re-exports
pub use session_redis::RedisSessionStore;
