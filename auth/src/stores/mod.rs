//! Storage implementations for auth system.
//!
//! This module provides persistent and ephemeral storage for authentication state:
//!
//! - **Session Store** (Redis) - Ephemeral session storage with TTL
//! - **Device Registry** (PostgreSQL) - Persistent device tracking
//! - **Challenge Store** (Redis) - WebAuthn challenge storage (planned)

#[cfg(feature = "postgres")]
pub mod postgres;
pub mod session_redis;

// Re-exports
#[cfg(feature = "postgres")]
pub use postgres::PostgresDeviceRepository;
pub use session_redis::RedisSessionStore;
