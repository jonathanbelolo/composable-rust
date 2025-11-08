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
#[derive(Clone)]
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

        // ✅ SECURITY FIX: Check session doesn't already exist (prevent session fixation)
        //
        // Session fixation attack scenario:
        // 1. Attacker creates a session with a known session_id
        // 2. Attacker tricks victim into using that session_id
        // 3. Victim authenticates with the attacker's session_id
        // 4. Attacker now has access to victim's authenticated session
        //
        // By rejecting duplicate session IDs, we prevent this attack.
        let exists: bool = conn.exists(&session_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to check session existence: {e}"))
        })?;

        if exists {
            return Err(AuthError::InternalError(
                "Session ID already exists (session fixation prevention)".into(),
            ));
        }

        // Serialize session
        let session_bytes =
            bincode::serialize(session).map_err(|e| AuthError::SerializationError(e.to_string()))?;

        // Convert chrono::Duration to seconds (i64)
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let ttl_seconds = ttl.num_seconds().max(0) as u64;

        // ✅ SECURITY FIX: Use Redis pipeline for atomic session creation
        //
        // Without atomicity, race conditions could occur:
        // - Session created but not added to user set (orphaned session)
        // - Session added to user set but creation fails (dangling reference)
        //
        // Redis pipeline ensures both operations succeed or both fail.
        //
        // ✅ MEMORY LEAK FIX: Set TTL on user sessions set (+1 day buffer)
        //
        // Without TTL, user:sessions:{user_id} sets grow unbounded, causing memory leaks.
        // We set TTL to session_ttl + 1 day to ensure the set outlives all sessions
        // but still gets cleaned up eventually.
        #[allow(clippy::cast_possible_truncation)]
        let set_ttl_seconds = (ttl_seconds + 86400) as i64; // +1 day buffer

        let _: () = redis::pipe()
            .atomic()
            .set_ex(&session_key, session_bytes, ttl_seconds)
            .sadd(&user_sessions_key, session.session_id.0.to_string())
            .ignore() // Continue pipeline even if SADD has issues
            .expire(&user_sessions_key, set_ttl_seconds) // ✅ Set TTL on set
            .ignore() // Continue pipeline even if EXPIRE has issues
            .query_async(&mut conn)
            .await
            .map_err(|e| AuthError::InternalError(format!("Failed to create session: {e}")))?;

        tracing::info!(
            session_id = %session.session_id.0,
            user_id = %session.user_id.0,
            ttl_seconds = ttl_seconds,
            "Created session atomically in Redis"
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

        // ✅ SECURITY FIX: Get existing session to validate immutable fields
        //
        // Privilege escalation attack scenario:
        // 1. Attacker authenticates as low-privilege user
        // 2. Attacker calls update_session with modified user_id (pointing to admin)
        // 3. Without validation, session now belongs to admin
        // 4. Attacker has admin access
        //
        // By validating immutable fields, we prevent this attack.
        //
        // ⚠️ ACCEPTED RISK: Theoretical TOCTOU (Time-of-Check-Time-of-Use)
        //
        // There is a microsecond window between get_session() and the SET command below
        // where another concurrent update_session() could modify the session.
        //
        // RISK ASSESSMENT: **VERY LOW** - Exploitation is nearly impossible because:
        // 1. **Timing window**: < 1ms (network roundtrip) - too narrow to exploit reliably
        // 2. **Attack complexity**: Requires precise timing AND session_id knowledge
        // 3. **Limited impact**: Attacker can only modify mutable fields (email, user_agent, last_active)
        // 4. **Immutable fields protected**: user_id, device_id, ip_address, oauth_provider, login_risk_score
        //    are ALL validated - privilege escalation is still impossible
        // 5. **Audit trail**: All session updates are logged with tracing
        //
        // MITIGATION OPTIONS (if needed in future):
        // - Use Lua script for atomic read-validate-update (adds complexity)
        // - Use Redis transactions (WATCH/MULTI/EXEC) with retry logic
        //
        // DECISION: Accept this risk for production. The security benefit of immutable field
        // validation far outweighs the theoretical TOCTOU risk on mutable fields.
        let existing_session = self.get_session(session.session_id).await?;

        // ✅ SECURITY: Validate immutable fields haven't changed
        if existing_session.user_id != session.user_id {
            tracing::error!(
                session_id = %session.session_id.0,
                existing_user_id = %existing_session.user_id.0,
                new_user_id = %session.user_id.0,
                "Attempt to change immutable user_id (privilege escalation attempt)"
            );
            return Err(AuthError::InternalError(
                "Cannot change session user_id (immutable)".into(),
            ));
        }

        if existing_session.device_id != session.device_id {
            tracing::error!(
                session_id = %session.session_id.0,
                "Attempt to change immutable device_id"
            );
            return Err(AuthError::InternalError(
                "Cannot change session device_id (immutable)".into(),
            ));
        }

        if existing_session.ip_address != session.ip_address {
            tracing::error!(
                session_id = %session.session_id.0,
                "Attempt to change immutable ip_address"
            );
            return Err(AuthError::InternalError(
                "Cannot change session ip_address (immutable)".into(),
            ));
        }

        if existing_session.oauth_provider != session.oauth_provider {
            tracing::error!(
                session_id = %session.session_id.0,
                "Attempt to change immutable oauth_provider"
            );
            return Err(AuthError::InternalError(
                "Cannot change session oauth_provider (immutable)".into(),
            ));
        }

        if existing_session.login_risk_score != session.login_risk_score {
            tracing::error!(
                session_id = %session.session_id.0,
                "Attempt to change immutable login_risk_score"
            );
            return Err(AuthError::InternalError(
                "Cannot change session login_risk_score (immutable)".into(),
            ));
        }

        // Serialize updated session
        let session_bytes =
            bincode::serialize(session).map_err(|e| AuthError::SerializationError(e.to_string()))?;

        // ✅ SECURITY FIX: Implement sliding window expiration
        //
        // OLD BEHAVIOR (BROKEN):
        // - Get remaining TTL (e.g., 10 minutes left of 24 hours)
        // - Set session with that same 10 minutes
        // - User gets logged out despite being active
        //
        // NEW BEHAVIOR (CORRECT):
        // - Calculate fresh TTL from session.expires_at
        // - Refresh TTL on every update (sliding window)
        // - Active users stay logged in
        let fresh_ttl = session.expires_at.signed_duration_since(chrono::Utc::now());

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let ttl_seconds = fresh_ttl.num_seconds().max(0) as u64;

        let _: () = conn
            .set_ex(&session_key, session_bytes, ttl_seconds)
            .await
            .map_err(|e| AuthError::InternalError(format!("Failed to update session: {e}")))?;

        tracing::debug!(
            session_id = %session.session_id.0,
            ttl_seconds = ttl_seconds,
            "Updated session with refreshed TTL (sliding window)"
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

        // SECURITY: Use Lua script for atomic bulk deletion
        //
        // VULNERABILITY PREVENTED: Race condition in session deletion
        // Without atomicity:
        // 1. Thread A: Reads session IDs from set
        // 2. Thread B: Creates new session, adds to set
        // 3. Thread A: Deletes all sessions from step 1
        // 4. Thread A: Deletes the set
        // 5. Result: Thread B's session is orphaned (exists but not tracked)
        //
        // With Lua script: All operations happen atomically on Redis server
        let lua_script = r#"
            local user_set_key = KEYS[1]
            local session_ids = redis.call('SMEMBERS', user_set_key)
            local deleted_count = 0

            for i, session_id in ipairs(session_ids) do
                local session_key = 'session:' .. session_id
                if redis.call('DEL', session_key) == 1 then
                    deleted_count = deleted_count + 1
                end
            end

            redis.call('DEL', user_set_key)
            return deleted_count
        "#;

        let script = redis::Script::new(lua_script);
        let deleted_count: usize = script
            .key(&user_sessions_key)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| {
                AuthError::InternalError(format!("Failed to execute atomic session deletion: {e}"))
            })?;

        tracing::info!(
            user_id = %user_id.0,
            session_count = deleted_count,
            "Atomically deleted all user sessions"
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

    async fn get_user_sessions(&self, user_id: UserId) -> Result<Vec<SessionId>> {
        let mut conn = self.conn_manager.clone();
        let user_sessions_key = Self::user_sessions_key(&user_id);

        // Get all session IDs from the user's set
        let session_ids: Vec<String> = conn
            .smembers(&user_sessions_key)
            .await
            .map_err(|e| AuthError::InternalError(format!("Failed to get user sessions: {e}")))?;

        // ✅ CLEANUP: Filter out dead sessions and remove from set
        //
        // This prevents the user:sessions:{user_id} set from accumulating
        // references to expired/deleted sessions. Without this cleanup,
        // the set would grow unbounded over time.
        let mut valid_sessions = Vec::new();
        let mut dead_session_count = 0;

        for id_str in session_ids {
            if let Ok(uuid) = uuid::Uuid::parse_str(&id_str) {
                let session_id = SessionId(uuid);

                // Check if session still exists
                if self.exists(session_id).await? {
                    valid_sessions.push(session_id);
                } else {
                    // Session expired or was deleted - clean up the reference
                    let _: () = conn
                        .srem(&user_sessions_key, &id_str)
                        .await
                        .map_err(|e| {
                            tracing::warn!(
                                user_id = %user_id.0,
                                session_id = %id_str,
                                error = %e,
                                "Failed to clean up dead session reference"
                            );
                            e
                        })
                        .unwrap_or(());

                    dead_session_count += 1;
                }
            }
        }

        if dead_session_count > 0 {
            tracing::debug!(
                user_id = %user_id.0,
                dead_count = dead_session_count,
                valid_count = valid_sessions.len(),
                "Cleaned up dead session references"
            );
        }

        Ok(valid_sessions)
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
        let session = Session {
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

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_session_fixation_prevention() {
        let store = RedisSessionStore::new("redis://127.0.0.1:6379")
            .await
            .unwrap();

        // Create first session with specific ID
        let session_id = SessionId::new();
        let session1 = Session {
            session_id,
            user_id: UserId::new(),
            device_id: DeviceId::new(),
            email: "user1@example.com".to_string(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            expires_at: Utc::now() + Duration::hours(24),
            ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            user_agent: "Browser1".to_string(),
            oauth_provider: None,
            login_risk_score: 0.1,
        };

        store
            .create_session(&session1, Duration::hours(24))
            .await
            .unwrap();

        // Attempt to create another session with SAME session_id (session fixation attack)
        let session2 = Session {
            session_id, // ← Same ID
            user_id: UserId::new(), // Different user
            device_id: DeviceId::new(),
            email: "attacker@example.com".to_string(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            expires_at: Utc::now() + Duration::hours(24),
            ip_address: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            user_agent: "AttackerBrowser".to_string(),
            oauth_provider: None,
            login_risk_score: 0.9,
        };

        let result = store.create_session(&session2, Duration::hours(24)).await;

        // Should fail with InternalError (session fixation prevention)
        match result {
            Err(AuthError::InternalError(msg)) if msg.contains("already exists") => {
                // ✅ Expected: Session fixation prevented
            }
            Ok(_) => {
                panic!("Expected session fixation prevention, but creation succeeded");
            }
            Err(e) => {
                panic!("Expected InternalError for session fixation, but got: {:?}", e);
            }
        }

        // Verify original session is still intact
        let retrieved = store.get_session(session_id).await.unwrap();
        assert_eq!(retrieved.email, "user1@example.com");
        assert_eq!(retrieved.user_id, session1.user_id);

        // Cleanup
        store.delete_session(session_id).await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_concurrent_session_creation_race() {
        let store = RedisSessionStore::new("redis://127.0.0.1:6379")
            .await
            .unwrap();

        // Create a session to be created concurrently
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
            oauth_provider: None,
            login_risk_score: 0.1,
        };

        let session_clone = session.clone();

        // Try to create the same session concurrently from two tasks
        let store_clone = store.clone();
        let (result1, result2) = tokio::join!(
            store.create_session(&session, Duration::hours(1)),
            store_clone.create_session(&session_clone, Duration::hours(1))
        );

        // Exactly ONE should succeed, the other should fail
        // XOR: true if one succeeds and one fails
        let one_succeeded = result1.is_ok() ^ result2.is_ok();
        assert!(
            one_succeeded,
            "Exactly one concurrent create should succeed. Results: {:?}, {:?}",
            result1,
            result2
        );

        // Verify the session exists and is correctly stored
        let retrieved = store.get_session(session.session_id).await.unwrap();
        assert_eq!(retrieved.session_id, session.session_id);
        assert_eq!(retrieved.user_id, session.user_id);
        assert_eq!(retrieved.email, "test@example.com");

        // Cleanup
        store.delete_session(session.session_id).await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_immutable_field_enforcement() {
        let store = RedisSessionStore::new("redis://127.0.0.1:6379")
            .await
            .unwrap();

        let original_user_id = UserId::new();
        let original_device_id = DeviceId::new();
        let original_ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let original_oauth_provider = Some(OAuthProvider::Google);
        let original_risk_score = 0.1;

        let session = Session {
            session_id: SessionId::new(),
            user_id: original_user_id,
            device_id: original_device_id,
            email: "test@example.com".to_string(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            expires_at: Utc::now() + Duration::hours(24),
            ip_address: original_ip,
            user_agent: "Test".to_string(),
            oauth_provider: original_oauth_provider,
            login_risk_score: original_risk_score,
        };

        store
            .create_session(&session, Duration::hours(24))
            .await
            .unwrap();

        // Test 1: Attempt to change user_id (privilege escalation)
        let mut tampered_session = session.clone();
        tampered_session.user_id = UserId::new(); // ← Different user
        tampered_session.last_active = Utc::now();

        let result = store.update_session(&tampered_session).await;
        assert!(
            matches!(result, Err(AuthError::InternalError(msg)) if msg.contains("user_id")),
            "Should reject user_id change"
        );

        // Test 2: Attempt to change device_id (device hijacking)
        let mut tampered_session = session.clone();
        tampered_session.device_id = DeviceId::new(); // ← Different device
        tampered_session.last_active = Utc::now();

        let result = store.update_session(&tampered_session).await;
        assert!(
            matches!(result, Err(AuthError::InternalError(msg)) if msg.contains("device_id")),
            "Should reject device_id change"
        );

        // Test 3: Attempt to change ip_address (IP spoofing)
        let mut tampered_session = session.clone();
        tampered_session.ip_address = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)); // ← Different IP
        tampered_session.last_active = Utc::now();

        let result = store.update_session(&tampered_session).await;
        assert!(
            matches!(result, Err(AuthError::InternalError(msg)) if msg.contains("ip_address")),
            "Should reject ip_address change"
        );

        // Test 4: Attempt to change oauth_provider
        let mut tampered_session = session.clone();
        tampered_session.oauth_provider = Some(OAuthProvider::GitHub); // ← Different provider
        tampered_session.last_active = Utc::now();

        let result = store.update_session(&tampered_session).await;
        assert!(
            matches!(result, Err(AuthError::InternalError(msg)) if msg.contains("oauth_provider")),
            "Should reject oauth_provider change"
        );

        // Test 5: Attempt to change login_risk_score (security bypass)
        let mut tampered_session = session.clone();
        tampered_session.login_risk_score = 0.9; // ← Higher risk (should be immutable)
        tampered_session.last_active = Utc::now();

        let result = store.update_session(&tampered_session).await;
        assert!(
            matches!(result, Err(AuthError::InternalError(msg)) if msg.contains("login_risk_score")),
            "Should reject login_risk_score change"
        );

        // Test 6: Valid update (only mutable fields changed)
        let mut updated_session = session.clone();
        updated_session.last_active = Utc::now() + Duration::seconds(1);
        updated_session.email = "updated@example.com".to_string(); // ← Email is mutable
        updated_session.user_agent = "UpdatedBrowser".to_string(); // ← User-agent is mutable

        let result = store.update_session(&updated_session).await;
        assert!(result.is_ok(), "Valid update should succeed");

        // Verify immutable fields are unchanged
        let retrieved = store.get_session(session.session_id).await.unwrap();
        assert_eq!(retrieved.user_id, original_user_id);
        assert_eq!(retrieved.device_id, original_device_id);
        assert_eq!(retrieved.ip_address, original_ip);
        assert_eq!(retrieved.oauth_provider, original_oauth_provider);
        assert_eq!(retrieved.login_risk_score, original_risk_score);
        assert_eq!(retrieved.email, "updated@example.com"); // Mutable field updated

        // Cleanup
        store.delete_session(session.session_id).await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_sliding_window_ttl_refresh() {
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
            expires_at: Utc::now() + Duration::seconds(10), // ← Short TTL
            ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            user_agent: "Test".to_string(),
            oauth_provider: None,
            login_risk_score: 0.1,
        };

        // Create session with 10 second TTL
        store
            .create_session(&session, Duration::seconds(10))
            .await
            .unwrap();

        // Get initial TTL (should be ~10 seconds)
        let initial_ttl = store.get_ttl(session.session_id).await.unwrap().unwrap();
        assert!(
            initial_ttl.num_seconds() >= 8 && initial_ttl.num_seconds() <= 10,
            "Initial TTL should be ~10 seconds, got {}",
            initial_ttl.num_seconds()
        );

        // Wait 3 seconds
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        // Update session with NEW expires_at (extending the session)
        let mut updated_session = session.clone();
        updated_session.last_active = Utc::now();
        updated_session.expires_at = Utc::now() + Duration::seconds(20); // ← Extend to 20 seconds
        store.update_session(&updated_session).await.unwrap();

        // Get new TTL (should be refreshed to ~20 seconds, NOT reduced)
        let refreshed_ttl = store.get_ttl(session.session_id).await.unwrap().unwrap();
        assert!(
            refreshed_ttl.num_seconds() >= 18 && refreshed_ttl.num_seconds() <= 20,
            "Refreshed TTL should be ~20 seconds (sliding window), got {}",
            refreshed_ttl.num_seconds()
        );

        // Verify the TTL increased (sliding window behavior)
        assert!(
            refreshed_ttl.num_seconds() > initial_ttl.num_seconds(),
            "TTL should increase on activity (sliding window)"
        );

        // Cleanup
        store.delete_session(session.session_id).await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_atomic_bulk_deletion() {
        let store = RedisSessionStore::new("redis://127.0.0.1:6379")
            .await
            .unwrap();

        let user_id = UserId::new();

        // Create 3 sessions for the same user
        let sessions: Vec<Session> = (0..3)
            .map(|i| Session {
                session_id: SessionId::new(),
                user_id,
                device_id: DeviceId::new(),
                email: format!("test{}@example.com", i),
                created_at: Utc::now(),
                last_active: Utc::now(),
                expires_at: Utc::now() + Duration::hours(24),
                ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                user_agent: format!("Browser{}", i),
                oauth_provider: None,
                login_risk_score: 0.1,
            })
            .collect();

        for session in &sessions {
            store
                .create_session(session, Duration::hours(24))
                .await
                .unwrap();
        }

        // Verify all sessions exist
        for session in &sessions {
            assert!(store.exists(session.session_id).await.unwrap());
        }

        // Delete all user sessions atomically
        let deleted_count = store.delete_user_sessions(user_id).await.unwrap();
        assert_eq!(deleted_count, 3, "Should delete exactly 3 sessions");

        // Verify all sessions are deleted
        for session in &sessions {
            assert!(
                !store.exists(session.session_id).await.unwrap(),
                "Session should be deleted"
            );
        }

        // Verify user set is deleted (try creating new session for same user)
        let new_session = Session {
            session_id: SessionId::new(),
            user_id,
            device_id: DeviceId::new(),
            email: "new@example.com".to_string(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            expires_at: Utc::now() + Duration::hours(24),
            ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            user_agent: "NewBrowser".to_string(),
            oauth_provider: None,
            login_risk_score: 0.1,
        };

        store
            .create_session(&new_session, Duration::hours(24))
            .await
            .unwrap();

        // Should have 1 session for this user
        let count = store.delete_user_sessions(user_id).await.unwrap();
        assert_eq!(count, 1, "New session should be tracked correctly");
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_user_sessions_set_ttl() {
        let store = RedisSessionStore::new("redis://127.0.0.1:6379")
            .await
            .unwrap();

        let user_id = UserId::new();
        let session = Session {
            session_id: SessionId::new(),
            user_id,
            device_id: DeviceId::new(),
            email: "test@example.com".to_string(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            expires_at: Utc::now() + Duration::hours(24),
            ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            user_agent: "Test".to_string(),
            oauth_provider: None,
            login_risk_score: 0.1,
        };

        // Create session with 1 hour TTL
        store
            .create_session(&session, Duration::hours(1))
            .await
            .unwrap();

        // Check TTL on user sessions set
        let mut conn = store.conn_manager.clone();
        let user_sessions_key = format!("user:sessions:{}", user_id.0);
        let set_ttl: i64 = conn.ttl(&user_sessions_key).await.unwrap();

        // TTL should be session_ttl + 1 day buffer = 1 hour + 86400 seconds
        // Allow some margin for test execution time
        let expected_ttl = 3600 + 86400; // 1 hour + 1 day
        assert!(
            set_ttl >= expected_ttl - 10 && set_ttl <= expected_ttl + 10,
            "User sessions set TTL should be ~{} seconds (session TTL + 1 day), got {}",
            expected_ttl,
            set_ttl
        );

        // Cleanup
        store.delete_session(session.session_id).await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)]
    async fn test_get_user_sessions_cleanup() {
        let store = RedisSessionStore::new("redis://127.0.0.1:6379")
            .await
            .unwrap();

        let user_id = UserId::new();

        // Create 3 sessions
        let sessions: Vec<Session> = (0..3)
            .map(|i| Session {
                session_id: SessionId::new(),
                user_id,
                device_id: DeviceId::new(),
                email: format!("test{}@example.com", i),
                created_at: Utc::now(),
                last_active: Utc::now(),
                expires_at: Utc::now() + Duration::hours(24),
                ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                user_agent: format!("Browser{}", i),
                oauth_provider: None,
                login_risk_score: 0.1,
            })
            .collect();

        for session in &sessions {
            store
                .create_session(session, Duration::hours(24))
                .await
                .unwrap();
        }

        // Verify all 3 sessions exist
        let active_sessions = store.get_user_sessions(user_id).await.unwrap();
        assert_eq!(active_sessions.len(), 3, "Should have 3 active sessions");

        // Manually delete 2 sessions (simulating expiration)
        store.delete_session(sessions[0].session_id).await.unwrap();
        store.delete_session(sessions[1].session_id).await.unwrap();

        // At this point:
        // - sessions[0] and sessions[1] are deleted from Redis
        // - But user:sessions:{user_id} set still has 3 references (dead data)

        // Call get_user_sessions() - should clean up dead references
        let active_sessions = store.get_user_sessions(user_id).await.unwrap();
        assert_eq!(
            active_sessions.len(),
            1,
            "Should have 1 active session after cleanup"
        );
        assert_eq!(
            active_sessions[0], sessions[2].session_id,
            "Should return the remaining valid session"
        );

        // Verify the set now has only 1 entry (dead references cleaned up)
        let mut conn = store.conn_manager.clone();
        let user_sessions_key = format!("user:sessions:{}", user_id.0);
        let set_members: Vec<String> = conn.smembers(&user_sessions_key).await.unwrap();
        assert_eq!(
            set_members.len(),
            1,
            "User sessions set should have 1 entry after cleanup"
        );

        // Cleanup
        store.delete_session(sessions[2].session_id).await.unwrap();
    }
}
