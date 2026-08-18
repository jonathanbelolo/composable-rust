//! Storage implementations for auth system.
//!
//! This module provides persistent and ephemeral storage for authentication state:
//!
//! - **Session Store** (Redis) - Ephemeral session storage with TTL
//! - **Device Registry** (` PostgreSQL`) - Persistent device tracking
//! - **OAuth Token Store** (Redis) - Encrypted OAuth token storage with refresh
//! - **Challenge Store** (Redis) - `WebAuthn` challenge storage with atomic consumption
//! - **Token Store** (Redis) - Magic link token storage with atomic consumption

pub mod challenge_redis;
pub mod oauth_token_redis;
#[cfg(feature = "postgres")]
pub mod postgres;
pub mod rate_limiter_redis;
pub mod session_redis;
pub mod token_redis;

use redis::aio::{ConnectionManager, ConnectionManagerConfig};
use redis::{Client, RedisResult};
use std::time::Duration;

/// Longest a single Redis command may take before it is treated as failed.
///
/// Every command these stores issue — `GET`, `SETEX`, `GETDEL`, `EVALSHA` —
/// completes in well under a millisecond, so this does not bound a slow
/// OPERATION. It bounds a transient STALL: a reconnect handshake, a scheduler
/// pause, an fsync during an AOF rewrite. Hence the wide margin — it should
/// trip only on something genuinely wrong.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);

/// Longest a single connection ATTEMPT may take. Covers DNS plus a TLS
/// handshake on a cold start; the manager retries with backoff on top of this.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);

/// Open a Redis connection manager with EXPLICIT timeouts.
///
/// redis 1.0 gave async connections default timeouts — 1s to connect, **500ms
/// per response** — where the 0.x line had none at all. Those defaults are a
/// real improvement over hanging forever, but INHERITING them silently is the
/// wrong shape: the number that decides whether a session lookup fails under
/// load should be written down where it can be read and argued with, not
/// acquired as a side effect of upgrading a dependency.
///
/// # Errors
/// Returns the underlying Redis error if the connection cannot be established.
pub(crate) async fn connect_manager(client: Client) -> RedisResult<ConnectionManager> {
    let config = ConnectionManagerConfig::new()
        .set_response_timeout(Some(RESPONSE_TIMEOUT))
        .set_connection_timeout(Some(CONNECTION_TIMEOUT));
    ConnectionManager::new_with_config(client, config).await
}

// Re-exports
pub use challenge_redis::RedisChallengeStore;
pub use oauth_token_redis::RedisOAuthTokenStore;
#[cfg(feature = "postgres")]
pub use postgres::{PostgresDeviceRepository, PostgresUserRepository};
pub use rate_limiter_redis::RedisRateLimiter;
pub use session_redis::RedisSessionStore;
pub use token_redis::RedisTokenStore;
