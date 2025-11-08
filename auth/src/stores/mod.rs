//! Storage implementations for auth system.
//!
//! This module provides persistent and ephemeral storage for authentication state:
//!
//! - **Session Store** (Redis) - Ephemeral session storage with TTL
//! - **Device Registry** (PostgreSQL) - Persistent device tracking
//! - **OAuth Token Store** (Redis) - Encrypted OAuth token storage with refresh
//! - **Challenge Store** (Redis) - WebAuthn challenge storage with atomic consumption
//! - **Token Store** (Redis) - Magic link token storage with atomic consumption

#[cfg(feature = "postgres")]
pub mod postgres;
pub mod session_redis;
pub mod oauth_token_redis;
pub mod challenge_redis;
pub mod token_redis;

// Re-exports
#[cfg(feature = "postgres")]
pub use postgres::PostgresDeviceRepository;
pub use session_redis::RedisSessionStore;
pub use oauth_token_redis::RedisOAuthTokenStore;
pub use challenge_redis::RedisChallengeStore;
pub use token_redis::RedisTokenStore;
