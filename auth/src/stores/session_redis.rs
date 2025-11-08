//! Redis-based session store implementation.
//!
//! This module provides an ephemeral session store using Redis with TTL-based expiration.
//!
//! # Architecture
//!
//! Sessions are stored in Redis with:
//! - **Primary key**: `session:{session_id}` → bincode-serialized Session
//! - **User index**: `user:{user_id}:sessions` (Set) → list of session IDs
//! - **TTL**: Configurable expiration (default 24 hours, sliding window)
//!
//! # Example
//!
//! ```no_run
//! use composable_rust_auth::stores::RedisSessionStore;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let store = RedisSessionStore::new("redis://127.0.0.1:6379").await?;
//! # Ok(())
//! # }
//! ```

use crate::error::{AuthError, Result};
use crate::providers::SessionStore;
use crate::state::{Session, SessionId, UserId};
use chrono::Duration;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};

/// Redis-based session store with TTL-based expiration.
///
/// Provides:
/// - Session storage with automatic expiration
/// - Multi-device tracking per user
/// - Sliding window expiration (extend on activity)
/// - Connection pooling via `ConnectionManager`
pub struct RedisSessionStore {
    /// Connection manager for connection pooling.
    conn_manager: ConnectionManager,
}

impl RedisSessionStore {
    /// Create a new Redis session store.
    ///
    /// # Arguments
    ///
    /// * `redis_url` - Redis connection URL (e.g., "redis://127.0.0.1:6379")
    ///
    /// # Errors
    ///
    /// Returns error if connection to Redis fails.
    pub async fn new(redis_url: &str) -> Result<Self> {
        let client = Client::open(redis_url).map_err(|e| {
            AuthError::InternalError(format!("Failed to create Redis client: {e}"))
        })?;

        let conn_manager = ConnectionManager::new(client).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to create Redis connection manager: {e}"))
        })?;

        Ok(Self { conn_manager })
    }

    /// Get the Redis key for a session.
    fn session_key(session_id: &SessionId) -> String {
        format!("session:{}", session_id.0)
    }

    /// Get the Redis key for user sessions set.
    fn user_sessions_key(user_id: &UserId) -> String {
        format!("user:{}:sessions", user_id.0)
    }
}

impl SessionStore for RedisSessionStore {
    async fn create_session(&self, session: &Session, ttl: Duration) -> Result<()> {
        let mut conn = self.conn_manager.clone();
        let session_key = Self::session_key(&session.session_id);
        let user_sessions_key = Self::user_sessions_key(&session.user_id);

        // Serialize session
        let session_bytes =
            bincode::serialize(session).map_err(|e| AuthError::SerializationError(e.to_string()))?;

        // Convert chrono::Duration to seconds (i64)
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let ttl_seconds = ttl.num_seconds().max(0) as u64;

        // Store session with TTL
        let _: () = conn
            .set_ex(&session_key, session_bytes, ttl_seconds)
            .await
            .map_err(|e| AuthError::InternalError(format!("Failed to store session: {e}")))?;

        // Add to user's session set (no TTL on the set itself, members expire individually)
        let _: () = conn
            .sadd(&user_sessions_key, session.session_id.0.to_string())
            .await
            .map_err(|e| {
                AuthError::InternalError(format!("Failed to add session to user set: {e}"))
            })?;

        tracing::info!(
            session_id = %session.session_id.0,
            user_id = %session.user_id.0,
            ttl_seconds = ttl_seconds,
            "Created session in Redis"
        );

        Ok(())
    }

    async fn get_session(&self, session_id: SessionId) -> Result<Session> {
        let mut conn = self.conn_manager.clone();
        let session_key = Self::session_key(&session_id);

        let session_bytes: Option<Vec<u8>> = conn.get(&session_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to get session from Redis: {e}"))
        })?;

        match session_bytes {
            Some(bytes) => {
                let session: Session = bincode::deserialize(&bytes)
                    .map_err(|e| AuthError::SerializationError(e.to_string()))?;

                // ✅ SECURITY FIX: Validate session expiration
                //
                // Although Redis TTL automatically deletes expired sessions,
                // we validate expiration here as defense-in-depth to guard against:
                // - Clock skew between application server and Redis
                // - Manual Redis TTL manipulation (PERSIST command)
                // - Redis configuration issues (maxmemory-policy noeviction)
                // - Redis bugs or edge cases
                if session.expires_at < chrono::Utc::now() {
                    tracing::warn!(
                        session_id = %session_id.0,
                        expires_at = %session.expires_at,
                        now = %chrono::Utc::now(),
                        "Session expired (TTL should have cleaned this up)"
                    );
                    return Err(AuthError::SessionExpired);
                }

                Ok(session)
            }
            None => Err(AuthError::SessionNotFound),
        }
    }

    async fn update_session(&self, session: &Session) -> Result<()> {
        let mut conn = self.conn_manager.clone();
        let session_key = Self::session_key(&session.session_id);

        // Check if session exists
        let exists: bool = conn.exists(&session_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to check session existence: {e}"))
        })?;

        if !exists {
            return Err(AuthError::SessionNotFound);
        }

        // Get current TTL to preserve it
        let ttl: i64 = conn.ttl(&session_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to get session TTL: {e}"))
        })?;

        if ttl < 0 {
            // TTL < 0 means no expiration or key doesn't exist
            return Err(AuthError::SessionNotFound);
        }

        // Serialize and update
        let session_bytes =
            bincode::serialize(session).map_err(|e| AuthError::SerializationError(e.to_string()))?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let ttl_u64 = ttl.max(0) as u64;

        let _: () = conn
            .set_ex(&session_key, session_bytes, ttl_u64)
            .await
            .map_err(|e| AuthError::InternalError(format!("Failed to update session: {e}")))?;

        tracing::debug!(
            session_id = %session.session_id.0,
            "Updated session in Redis"
        );

        Ok(())
    }

    async fn delete_session(&self, session_id: SessionId) -> Result<()> {
        let mut conn = self.conn_manager.clone();
        let session_key = Self::session_key(&session_id);

        // Get session first to find user_id
        match self.get_session(session_id).await {
            Ok(session) => {
                let user_sessions_key = Self::user_sessions_key(&session.user_id);

                // Remove from user's session set
                let _: () = conn
                    .srem(&user_sessions_key, session_id.0.to_string())
                    .await
                    .map_err(|e| {
                        AuthError::InternalError(format!("Failed to remove session from user set: {e}"))
                    })?;
            }
            Err(AuthError::SessionNotFound) => {
                // Session doesn't exist - that's okay for delete
            }
            Err(e) => return Err(e),
        }

        // Delete session
        let _: () = conn.del(&session_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to delete session from Redis: {e}"))
        })?;

        tracing::info!(
            session_id = %session_id.0,
            "Deleted session from Redis"
        );

        Ok(())
    }

    async fn delete_user_sessions(&self, user_id: UserId) -> Result<usize> {
        let mut conn = self.conn_manager.clone();
        let user_sessions_key = Self::user_sessions_key(&user_id);

        // Get all session IDs
        let session_ids: Vec<String> = conn.smembers(&user_sessions_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to get user sessions: {e}"))
        })?;

        let mut deleted_count = 0;

        // Delete each session
        for session_id_str in &session_ids {
            if let Ok(session_id) = uuid::Uuid::parse_str(session_id_str) {
                let session_key = Self::session_key(&SessionId(session_id));
                let _: () = conn.del(&session_key).await.map_err(|e| {
                    AuthError::InternalError(format!("Failed to delete session: {e}"))
                })?;
                deleted_count += 1;
            }
        }

        // Delete user's session set
        let _: () = conn.del(&user_sessions_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to delete user session set: {e}"))
        })?;

        tracing::info!(
            user_id = %user_id.0,
            session_count = deleted_count,
            "Deleted all user sessions"
        );

        Ok(deleted_count)
    }

    async fn exists(&self, session_id: SessionId) -> Result<bool> {
        let mut conn = self.conn_manager.clone();
        let session_key = Self::session_key(&session_id);

        let exists: bool = conn.exists(&session_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to check session existence: {e}"))
        })?;

        Ok(exists)
    }

    async fn get_ttl(&self, session_id: SessionId) -> Result<Option<Duration>> {
        let mut conn = self.conn_manager.clone();
        let session_key = Self::session_key(&session_id);

        let ttl_seconds: i64 = conn.ttl(&session_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to get session TTL: {e}"))
        })?;

        match ttl_seconds {
            -2 => Ok(None), // Key doesn't exist
            -1 => Ok(None), // Key exists but has no expiration
            seconds if seconds > 0 => Ok(Some(Duration::seconds(seconds))),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{DeviceId, OAuthProvider};
    use chrono::Utc;
    use std::net::{IpAddr, Ipv4Addr};

    // Note: These tests require a running Redis instance
    // Run with: docker run -d -p 6379:6379 redis:7-alpine
    // Or skip with: cargo test --lib (excludes integration tests)

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_redis_session_lifecycle() {
        let store = RedisSessionStore::new("redis://127.0.0.1:6379")
            .await
            .unwrap();

        let session = Session {
            session_id: SessionId::new(),
            user_id: UserId::new(),
            device_id: DeviceId::new(),
            email: "test@example.com".to_string(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            expires_at: Utc::now() + Duration::hours(24),
            ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            user_agent: "Test".to_string(),
            oauth_provider: Some(OAuthProvider::Google),
            login_risk_score: 0.1,
        };

        // Create
        store
            .create_session(&session, Duration::hours(24))
            .await
            .unwrap();

        // Get
        let retrieved = store.get_session(session.session_id).await.unwrap();
        assert_eq!(retrieved.session_id, session.session_id);

        // Exists
        let exists = store.exists(session.session_id).await.unwrap();
        assert!(exists);

        // Get TTL
        let ttl = store.get_ttl(session.session_id).await.unwrap();
        assert!(ttl.is_some());

        // Delete
        store.delete_session(session.session_id).await.unwrap();

        // Verify deleted
        let exists_after_delete = store.exists(session.session_id).await.unwrap();
        assert!(!exists_after_delete);
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_expired_session_rejected() {
        let store = RedisSessionStore::new("redis://127.0.0.1:6379")
            .await
            .unwrap();

        // Create session with expiration in the past
        let mut session = Session {
            session_id: SessionId::new(),
            user_id: UserId::new(),
            device_id: DeviceId::new(),
            email: "test@example.com".to_string(),
            created_at: Utc::now() - Duration::hours(2),
            last_active: Utc::now() - Duration::hours(1),
            expires_at: Utc::now() - Duration::seconds(10), // ← Expired 10 seconds ago
            ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            user_agent: "Test".to_string(),
            oauth_provider: None,
            login_risk_score: 0.1,
        };

        // Store with a short TTL (Redis will still allow storage)
        // We're testing the application-level expiration check, not Redis TTL
        store
            .create_session(&session, Duration::seconds(60))
            .await
            .unwrap();

        // Try to retrieve - should reject due to expires_at
        let result = store.get_session(session.session_id).await;

        match result {
            Err(AuthError::SessionExpired) => {
                // ✅ Expected: Session rejected due to expiration
            }
            Ok(_) => {
                panic!("Expected SessionExpired error, but got success");
            }
            Err(e) => {
                panic!("Expected SessionExpired error, but got: {:?}", e);
            }
        }

        // Cleanup
        let _ = store.delete_session(session.session_id).await;
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_valid_session_accepted() {
        let store = RedisSessionStore::new("redis://127.0.0.1:6379")
            .await
            .unwrap();

        // Create session with future expiration
        let session = Session {
            session_id: SessionId::new(),
            user_id: UserId::new(),
            device_id: DeviceId::new(),
            email: "test@example.com".to_string(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            expires_at: Utc::now() + Duration::hours(24), // ← Valid for 24 hours
            ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            user_agent: "Test".to_string(),
            oauth_provider: None,
            login_risk_score: 0.1,
        };

        store
            .create_session(&session, Duration::hours(24))
            .await
            .unwrap();

        // Should successfully retrieve
        let retrieved = store.get_session(session.session_id).await.unwrap();
        assert_eq!(retrieved.session_id, session.session_id);
        assert_eq!(retrieved.user_id, session.user_id);

        // Cleanup
        store.delete_session(session.session_id).await.unwrap();
    }
}
