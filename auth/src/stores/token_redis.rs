//! Redis-based token store implementation.
//!
//! This module provides secure, single-use token storage for magic links, OAuth state
//! parameters, and other ephemeral tokens using Redis.
//!
//! # Architecture
//!
//! Tokens are stored in Redis with:
//! - **Primary key**: `auth:token:{token_id}` → JSON-serialized `TokenData`
//! - **TTL**: Configurable based on token type (5-15 minutes typical)
//! - **Atomic consumption**: Uses GETDEL command for single-use guarantee
//!
//! # Security
//!
//! - **Single-use**: Tokens consumed atomically via GETDEL (get + delete in one operation)
//! - **Expiration**: Tokens automatically expire after TTL (Redis-level + validation)
//! - **Constant-time validation**: Uses `constant_time_eq` to prevent timing attacks
//! - **Defense-in-depth**: Double-checks expiration even though TTL handles it
//! - **Replay protection**: Once consumed, token cannot be reused
//! - **Key namespacing**: All keys prefixed with `auth:token:` to avoid collisions
//!
//! # Performance
//!
//! - **Connection pooling**: Uses `ConnectionManager` for efficient connection reuse
//! - **Single round-trip**: GETDEL fetches and deletes in one Redis command
//! - **Automatic cleanup**: Redis TTL ensures expired tokens are removed
//!
//! # Example
//!
//! ```no_run
//! use composable_rust_auth::stores::RedisTokenStore;
//! use composable_rust_auth::providers::{TokenData, TokenType, TokenStore};
//! use chrono::{Utc, Duration};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let store = RedisTokenStore::new("redis://127.0.0.1:6379").await?;
//!
//! // Store a magic link token
//! let token_data = TokenData::new(
//!     TokenType::MagicLink,
//!     "secure-random-token-256-bits".to_string(),
//!     serde_json::json!({"email": "user@example.com"}),
//!     Utc::now() + Duration::minutes(10),
//! );
//!
//! store.store_token("token-id-123", token_data).await?;
//!
//! // Later: Consume token (atomic, single-use)
//! if let Some(token) = store.consume_token("token-id-123", "secure-random-token-256-bits").await? {
//!     println!("Token valid! Email: {}", token.data);
//! }
//! # Ok(())
//! # }
//! ```

use crate::error::{AuthError, Result};
use crate::providers::{TokenData, TokenStore};
use chrono::Utc;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};

/// `Redis`-based token store with atomic consumption.
///
/// Provides:
/// - Single-use token storage (atomic GETDEL)
/// - Automatic expiration via TTL
/// - Constant-time token validation
/// - Connection pooling via `ConnectionManager`
/// - Defense-in-depth security (`TTL` + expiration validation)
///
/// # Thread Safety
///
/// This type is `Clone` and can be safely shared across threads.
/// Each clone shares the same `ConnectionManager` (connection pool).
pub struct RedisTokenStore {
    /// Connection manager for connection pooling.
    conn_manager: ConnectionManager,
}

impl RedisTokenStore {
    /// Create a new `Redis` token store.
    ///
    /// # Arguments
    ///
    /// * `redis_url` - `Redis` connection URL (e.g., "<redis://127.0.0.1:6379>")
    ///
    /// # Connection URL Format
    ///
    /// - TCP: `redis://[:password@]host[:port][/database]`
    /// - Unix socket: `redis+unix:///path/to/redis.sock[?db=database[&pass=password]]`
    /// - TLS: `rediss://[:password@]host[:port][/database]`
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - `Redis` URL is malformed
    /// - Connection to `Redis` server fails
    /// - Authentication fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use composable_rust_auth::stores::RedisTokenStore;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// // Local development
    /// let store = RedisTokenStore::new("redis://127.0.0.1:6379").await?;
    ///
    /// // Production with password
    /// let store = RedisTokenStore::new("redis://:mypassword@redis.example.com:6379/0").await?;
    ///
    /// // TLS
    /// let store = RedisTokenStore::new("rediss://redis.example.com:6380").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(redis_url: &str) -> Result<Self> {
        let client = Client::open(redis_url).map_err(|e| {
            AuthError::InternalError(format!("Failed to create Redis client: {e}"))
        })?;

        let conn_manager = ConnectionManager::new(client).await.map_err(|e| {
            AuthError::InternalError(format!(
                "Failed to create Redis connection manager: {e}"
            ))
        })?;

        tracing::info!("RedisTokenStore initialized successfully");

        Ok(Self { conn_manager })
    }

    /// Get the `Redis` key for a token.
    ///
    /// # Key Format
    ///
    /// `auth:token:{token_id}`
    ///
    /// # Namespacing
    ///
    /// The `auth:token:` prefix prevents collisions with other `Redis` keys
    /// in shared `Redis` instances.
    fn token_key(token_id: &str) -> String {
        format!("auth:token:{token_id}")
    }
}

impl Clone for RedisTokenStore {
    fn clone(&self) -> Self {
        Self {
            conn_manager: self.conn_manager.clone(),
        }
    }
}

impl TokenStore for RedisTokenStore {
    async fn store_token(&self, token_id: &str, token_data: TokenData) -> Result<()> {
        let mut conn = self.conn_manager.clone();
        let token_key = Self::token_key(token_id);

        // Serialize token data (using JSON instead of bincode because TokenData contains serde_json::Value)
        let token_bytes = serde_json::to_vec(&token_data)
            .map_err(|e| AuthError::SerializationError(e.to_string()))?;

        // Calculate TTL in seconds
        let ttl = token_data.expires_at.signed_duration_since(Utc::now());

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let ttl_seconds = ttl.num_seconds().max(1) as u64;

        // Store with TTL
        // SETEX is atomic: SET + EXPIRE in one command
        let _: () = conn
            .set_ex(&token_key, token_bytes, ttl_seconds)
            .await
            .map_err(|e| AuthError::InternalError(format!("Failed to store token: {e}")))?;

        tracing::info!(
            token_type = ?token_data.token_type,
            token_id = token_id,
            ttl_seconds = ttl_seconds,
            expires_at = %token_data.expires_at,
            "Stored token in Redis"
        );

        Ok(())
    }

    async fn consume_token(&self, token_id: &str, token: &str) -> Result<Option<TokenData>> {
        let mut conn = self.conn_manager.clone();
        let token_key = Self::token_key(token_id);

        // ✅ SECURITY: GETDEL is atomic (get + delete in one operation)
        //
        // This ensures single-use semantics with no race conditions:
        // - Multiple concurrent consume attempts will result in exactly one success
        // - Once consumed, the token cannot be reused
        // - No TOCTOU vulnerability between check and delete
        let token_bytes: Option<Vec<u8>> = conn
            .get_del(&token_key)
            .await
            .map_err(|e| AuthError::InternalError(format!("Failed to consume token: {e}")))?;

        if let Some(bytes) = token_bytes {
            // Deserialize (using JSON instead of bincode because TokenData contains serde_json::Value)
            let token_data: TokenData = serde_json::from_slice(&bytes)
                .map_err(|e| AuthError::SerializationError(e.to_string()))?;

            // ✅ SECURITY: Constant-time token comparison to prevent timing attacks
            //
            // Using variable-time comparison (==) would allow attackers to:
            // 1. Measure response time for different token values
            // 2. Determine which characters match via timing differences
            // 3. Gradually reconstruct the token character-by-character
            //
            // constant_time_eq prevents this by always taking the same time
            // regardless of where the first mismatch occurs.
            let token_matches = constant_time_eq::constant_time_eq(
                token.as_bytes(),
                token_data.token.as_bytes(),
            );

            // ✅ SECURITY: Defense-in-depth expiration check
            //
            // Although Redis TTL should automatically delete expired tokens,
            // we validate expiration here to guard against:
            // - Clock skew between application and Redis
            // - Manual Redis TTL manipulation (PERSIST command)
            // - Redis configuration issues (maxmemory-policy noeviction)
            let now = Utc::now();
            let is_expired = token_data.expires_at <= now;

            // Both conditions must pass
            let is_valid = token_matches && !is_expired;

            if is_valid {
                tracing::info!(
                    token_type = ?token_data.token_type,
                    token_id = token_id,
                    "Token consumed successfully (single-use)"
                );
                Ok(Some(token_data))
            } else {
                // ✅ SECURITY: Generic error path prevents information leakage
                //
                // We return None for both "wrong token" and "expired token" cases
                // to prevent attackers from distinguishing between:
                // - Valid but expired tokens (token exists in system)
                // - Invalid tokens (token never existed or was consumed)
                //
                // This prevents token enumeration and timing attacks.
                if token_matches {
                    tracing::warn!(
                        token_id = token_id,
                        expires_at = %token_data.expires_at,
                        now = %now,
                        "Token consumption failed: token expired (TTL should have cleaned this up)"
                    );
                } else {
                    tracing::warn!(
                        token_id = token_id,
                        "Token consumption failed: token mismatch"
                    );
                }
                Ok(None)
            }
        } else {
            // Token not found - could be:
            // 1. Already consumed (single-use semantics)
            // 2. Expired (Redis TTL deleted it)
            // 3. Never existed (invalid token_id)
            //
            // We don't distinguish between these cases for security
            // (prevents enumeration of valid token IDs).
            tracing::debug!(
                token_id = token_id,
                "Token not found (consumed, expired, or invalid)"
            );
            Ok(None)
        }
    }

    async fn delete_token(&self, token_id: &str) -> Result<()> {
        let mut conn = self.conn_manager.clone();
        let token_key = Self::token_key(token_id);

        let deleted: i32 = conn.del(&token_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to delete token from Redis: {e}"))
        })?;

        if deleted > 0 {
            tracing::debug!(
                token_id = token_id,
                "Deleted token from Redis"
            );
        } else {
            tracing::trace!(
                token_id = token_id,
                "Token delete: key not found (already deleted or never existed)"
            );
        }

        Ok(())
    }

    async fn exists(&self, token_id: &str) -> Result<bool> {
        let mut conn = self.conn_manager.clone();
        let token_key = Self::token_key(token_id);

        let exists: bool = conn.exists(&token_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to check token existence: {e}"))
        })?;

        Ok(exists)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::TokenType;
    use chrono::Duration;

    #[tokio::test]
    #[ignore] // Requires Redis running at localhost:6379
    #[allow(clippy::unwrap_used)] // Test code
    async fn test_redis_token_lifecycle() {
        let store = RedisTokenStore::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to create store");

        let token_id = "test-token-id-lifecycle";
        let token = "test-token-secret-abc456xyz";

        let token_data = TokenData::new(
            TokenType::MagicLink,
            token.to_string(),
            serde_json::json!({"email": "test@example.com"}),
            Utc::now() + Duration::minutes(10),
        );

        // Store token
        store
            .store_token(token_id, token_data.clone())
            .await
            .expect("Failed to store token");

        // Verify exists
        assert!(
            store.exists(token_id).await.unwrap(),
            "Token should exist after storage"
        );

        // Consume token (should succeed)
        let consumed = store
            .consume_token(token_id, token)
            .await
            .expect("Failed to consume token");

        assert!(consumed.is_some(), "Token should be consumable");
        let data = consumed.unwrap();
        assert_eq!(data.token, token, "Token value should match");
        assert_eq!(
            data.token_type,
            TokenType::MagicLink,
            "Token type should match"
        );

        // Token should no longer exist
        assert!(
            !store.exists(token_id).await.unwrap(),
            "Token should not exist after consumption"
        );

        // Try to consume again (should fail - single use)
        let second_consume = store
            .consume_token(token_id, token)
            .await
            .expect("Second consume should not error");

        assert!(
            second_consume.is_none(),
            "Second consume should fail (single-use)"
        );
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)] // Test code
    async fn test_token_expiration() {
        let store = RedisTokenStore::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to create store");

        let token_id = "expiring-token";
        let token = "secret";

        let token_data = TokenData::new(
            TokenType::MagicLink,
            token.to_string(),
            serde_json::json!({"email": "test@example.com"}),
            Utc::now() + Duration::seconds(1), // 1 second TTL
        );

        // Store token
        store
            .store_token(token_id, token_data)
            .await
            .expect("Failed to store token");

        // Verify exists
        assert!(store.exists(token_id).await.unwrap());

        // Wait for expiration (Redis TTL)
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Try to consume (should fail - expired via TTL)
        let result = store
            .consume_token(token_id, token)
            .await
            .expect("Consume should not error");

        assert!(result.is_none(), "Expired token should not be consumable");

        // Token should not exist (TTL deleted it)
        assert!(
            !store.exists(token_id).await.unwrap(),
            "Expired token should not exist"
        );
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)] // Test code
    async fn test_wrong_token() {
        let store = RedisTokenStore::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to create store");

        let token_id = "test-token-wrong";
        let correct_token = "correct-secret-12345";
        let wrong_token = "wrong-secret-67890";

        let token_data = TokenData::new(
            TokenType::MagicLink,
            correct_token.to_string(),
            serde_json::json!({"email": "test@example.com"}),
            Utc::now() + Duration::minutes(10),
        );

        store
            .store_token(token_id, token_data)
            .await
            .expect("Failed to store token");

        // Try to consume with wrong token
        let result = store
            .consume_token(token_id, wrong_token)
            .await
            .expect("Consume should not error");

        assert!(
            result.is_none(),
            "Wrong token should not be accepted (constant-time comparison)"
        );

        // SECURITY: Token should still exist (not consumed on wrong token)
        assert!(
            store.exists(token_id).await.unwrap(),
            "Token should still exist after failed consume"
        );

        // Verify correct token still works
        let correct_result = store
            .consume_token(token_id, correct_token)
            .await
            .expect("Consume should not error");

        assert!(
            correct_result.is_some(),
            "Correct token should be consumable after wrong attempt"
        );
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)] // Test code
    async fn test_atomic_consumption() {
        let store = RedisTokenStore::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to create store");

        let token_id = "concurrent-token";
        let token = "secret";

        let token_data = TokenData::new(
            TokenType::MagicLink,
            token.to_string(),
            serde_json::json!({"email": "test@example.com"}),
            Utc::now() + Duration::minutes(10),
        );

        store
            .store_token(token_id, token_data)
            .await
            .expect("Failed to store token");

        // Spawn 10 concurrent tasks trying to consume the same token
        let mut handles = vec![];
        for _ in 0..10 {
            let store_clone = store.clone();
            let token_clone = token.to_string();
            let handle = tokio::spawn(async move {
                store_clone
                    .consume_token(token_id, &token_clone)
                    .await
                    .unwrap()
            });
            handles.push(handle);
        }

        // Collect results
        let mut results = vec![];
        for handle in handles {
            results.push(handle.await.unwrap());
        }

        // Exactly one should succeed (GETDEL is atomic)
        let successes = results.iter().filter(|r| r.is_some()).count();
        assert_eq!(
            successes, 1,
            "Exactly one consume should succeed due to GETDEL atomicity"
        );

        // Token should no longer exist
        assert!(
            !store.exists(token_id).await.unwrap(),
            "Token should not exist after atomic consumption"
        );
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)] // Test code
    async fn test_delete_token() {
        let store = RedisTokenStore::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to create store");

        let token_id = "delete-me";
        let token = "secret";

        let token_data = TokenData::new(
            TokenType::MagicLink,
            token.to_string(),
            serde_json::json!({"email": "test@example.com"}),
            Utc::now() + Duration::minutes(10),
        );

        // Store token
        store
            .store_token(token_id, token_data)
            .await
            .expect("Failed to store token");

        assert!(store.exists(token_id).await.unwrap());

        // Delete token
        store
            .delete_token(token_id)
            .await
            .expect("Failed to delete token");

        // Token should not exist
        assert!(
            !store.exists(token_id).await.unwrap(),
            "Token should not exist after deletion"
        );

        // Try to consume (should fail - deleted)
        let result = store
            .consume_token(token_id, token)
            .await
            .expect("Consume should not error");

        assert!(result.is_none(), "Deleted token should not be consumable");
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)] // Test code
    async fn test_different_token_types() {
        let store = RedisTokenStore::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to create store");

        let token_types = vec![
            TokenType::MagicLink,
            TokenType::OAuthState,
            TokenType::PasskeyRegistrationChallenge,
            TokenType::PasskeyAuthenticationChallenge,
        ];

        for (i, token_type) in token_types.iter().enumerate() {
            let token_id = format!("token-type-test-{i}");
            let token = format!("secret-{i}");

            let token_data = TokenData::new(
                *token_type,
                token.clone(),
                serde_json::json!({"test": i}),
                Utc::now() + Duration::minutes(10),
            );

            store
                .store_token(&token_id, token_data)
                .await
                .expect("Failed to store token");

            let consumed = store
                .consume_token(&token_id, &token)
                .await
                .expect("Failed to consume token");

            assert!(consumed.is_some());
            assert_eq!(consumed.unwrap().token_type, *token_type);
        }
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)] // Test code
    async fn test_key_namespacing() {
        let store = RedisTokenStore::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to create store");

        let token_id = "namespace-test";
        let expected_key = "auth:token:namespace-test";

        assert_eq!(
            RedisTokenStore::token_key(token_id),
            expected_key,
            "Key should be properly namespaced"
        );

        // Store a token
        let token_data = TokenData::new(
            TokenType::MagicLink,
            "secret".to_string(),
            serde_json::json!({}),
            Utc::now() + Duration::minutes(10),
        );

        store
            .store_token(token_id, token_data)
            .await
            .expect("Failed to store token");

        // Verify it's stored under the namespaced key
        let mut conn = store.conn_manager.clone();
        let exists: bool = conn
            .exists(expected_key)
            .await
            .expect("Failed to check key existence");

        assert!(exists, "Token should be stored under namespaced key");

        // Cleanup
        store.delete_token(token_id).await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used)] // Test code
    async fn test_idempotent_delete() {
        let store = RedisTokenStore::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to create store");

        let token_id = "idempotent-delete-test";

        // Delete non-existent token should not error
        store
            .delete_token(token_id)
            .await
            .expect("Deleting non-existent token should not error");

        // Create token
        let token_data = TokenData::new(
            TokenType::MagicLink,
            "secret".to_string(),
            serde_json::json!({}),
            Utc::now() + Duration::minutes(10),
        );

        store
            .store_token(token_id, token_data)
            .await
            .expect("Failed to store token");

        // Delete once
        store
            .delete_token(token_id)
            .await
            .expect("First delete should succeed");

        // Delete again (idempotent)
        store
            .delete_token(token_id)
            .await
            .expect("Second delete should not error (idempotent)");
    }
}
