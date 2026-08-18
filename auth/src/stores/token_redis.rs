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
//! - **Atomic consumption**: a Lua script compares and deletes in one command
//!
//! # Security
//!
//! - **The secret is never stored.** `store_token` persists a SHA-256 digest of
//!   it, so a Redis dump, replica or `KEYS`-capable operator comes away with
//!   nothing usable inside the token's lifetime.
//! - **Single-use, and only on a MATCH**: consumption is a Lua
//!   compare-then-delete. Concurrent consumes leave exactly one winner, and a
//!   WRONG secret leaves the token in place — presenting a bad secret for a
//!   known token id must not destroy the holder's valid token.
//! - **No timing side-channel that matters**: Lua string equality is
//!   variable-time, so the comparison is between DIGESTS. What leaks is
//!   information about a SHA-256 output, which is worthless without a preimage.
//!   (This is why `constant_time_eq` is no longer used here: it cannot run
//!   inside the script, and moving the comparison out of the script is what
//!   made the delete non-atomic in the first place.)
//! - **Expiration**: Redis TTL, plus a defense-in-depth check against clock
//!   skew, a manual `PERSIST`, or maxmemory-policy misconfiguration.
//! - **No enumeration**: absent, already-consumed, expired and mismatched are
//!   all one indistinguishable `Ok(None)`.
//! - **Key namespacing**: all keys prefixed with `auth:token:`.
//!
//! # Performance
//!
//! - **Connection pooling**: `ConnectionManager`, shareable with the host
//!   application via [`RedisTokenStore::from_connection_manager`]
//! - **Single round-trip**: the compare-and-delete is one `EVALSHA`
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
        let client = Client::open(redis_url)
            .map_err(|e| AuthError::InternalError(format!("Failed to create Redis client: {e}")))?;

        let conn_manager = super::connect_manager(client).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to create Redis connection manager: {e}"))
        })?;

        tracing::info!("RedisTokenStore initialized successfully");

        Ok(Self { conn_manager })
    }

    /// Build a token store over an **existing** connection manager.
    ///
    /// [`Self::new`] opens its own pool, which is right for a caller that has
    /// no Redis of its own. An application that already holds one — the
    /// generated apps keep their sessions in the same Redis the tokens go to —
    /// should pass it in rather than double the connection count for two key
    /// prefixes. `ConnectionManager` is cheaply cloneable and multiplexes, so
    /// sharing costs nothing and keeps one place where timeouts and retry
    /// policy are configured.
    ///
    /// Requires the caller to compile against the same `redis` version this
    /// crate does — the type is not convertible across major versions.
    #[must_use]
    pub const fn from_connection_manager(conn_manager: ConnectionManager) -> Self {
        Self { conn_manager }
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

    /// Hex SHA-256 of a token secret.
    ///
    /// Stored in place of the secret, and recomputed on the consume path so the
    /// comparison happens between two digests. A timing side-channel on that
    /// comparison leaks about the DIGEST, which is worthless without a
    /// preimage — which is what lets the comparison move into Lua, where it can
    /// be atomic with the delete but cannot be constant-time.
    fn digest(token: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hasher
            .finalize()
            .iter()
            .fold(String::with_capacity(64), |mut acc, b| {
                use std::fmt::Write as _;
                let _ = write!(acc, "{b:02x}");
                acc
            })
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

        // Persist a DIGEST of the secret, never the secret itself. Redis holds
        // these under a 5-15 minute TTL, but a dump, a replica, or an operator
        // with `KEYS` access should not come away with live credentials — and
        // the digest is also what makes the consume-side comparison safe to do
        // in Lua (see `consume_token`).
        let mut stored = token_data.clone();
        stored.token = Self::digest(&token_data.token);
        let token_bytes = serde_json::to_vec(&stored)
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

        // ✅ SECURITY: compare-then-delete, atomically, in ONE round trip.
        //
        // The previous implementation ran `GETDEL` and validated afterwards, so
        // presenting a WRONG secret for a known token id destroyed the holder's
        // valid token — a login denial, and the opposite of this module's own
        // contract. GETDEL cannot express "delete only if it matches", so the
        // comparison moves into the script, where it is atomic with the delete:
        //
        // - concurrent consumes: exactly one wins, DEL runs once
        // - a wrong secret: the token is LEFT IN PLACE
        // - no TOCTOU: check and delete are one Redis command
        //
        // The comparison is between DIGESTS (see `digest`), not secrets. Lua
        // string equality is variable-time, so comparing secrets here would
        // reintroduce exactly the timing side-channel `constant_time_eq` was
        // added to close; comparing digests leaks only about the digest, which
        // is worthless without a preimage.
        let lua_script = r"
            local raw = redis.call('GET', KEYS[1])
            if not raw then
                return nil
            end
            local ok, obj = pcall(cjson.decode, raw)
            if not ok or obj.token ~= ARGV[1] then
                return nil
            end
            redis.call('DEL', KEYS[1])
            return raw
        ";

        let script = redis::Script::new(lua_script);
        let raw: Option<String> = script
            .key(&token_key)
            .arg(Self::digest(token))
            .invoke_async(&mut conn)
            .await
            .map_err(|e| AuthError::InternalError(format!("Failed to consume token: {e}")))?;

        let Some(raw) = raw else {
            // Not found, already consumed, expired, or the secret did not
            // match. Deliberately indistinguishable: telling them apart would
            // let a caller enumerate valid token ids.
            tracing::debug!(
                token_id = token_id,
                "Token not consumed (absent, already used, expired, or mismatched)"
            );
            return Ok(None);
        };

        let mut token_data: TokenData =
            serde_json::from_str(&raw).map_err(|e| AuthError::SerializationError(e.to_string()))?;

        // ✅ SECURITY: defense-in-depth expiration check.
        //
        // Redis TTL should already have removed an expired token. This guards
        // against clock skew, a manual `PERSIST`, and maxmemory-policy
        // misconfiguration. The token has been deleted by now either way, which
        // is what we want for an expired one.
        let now = Utc::now();
        if token_data.expires_at <= now {
            tracing::warn!(
                token_id = token_id,
                expires_at = %token_data.expires_at,
                now = %now,
                "Token consumption failed: token expired (TTL should have cleaned this up)"
            );
            return Ok(None);
        }

        // Hand back the secret the caller presented, not the digest we store.
        // `TokenData.token` means "the token" to every caller; returning a
        // digest under that name would be a quiet trap.
        token_data.token = token.to_string();

        tracing::info!(
            token_type = ?token_data.token_type,
            token_id = token_id,
            "Token consumed successfully (single-use)"
        );
        Ok(Some(token_data))
    }

    async fn delete_token(&self, token_id: &str) -> Result<()> {
        let mut conn = self.conn_manager.clone();
        let token_key = Self::token_key(token_id);

        let deleted: i32 = conn.del(&token_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to delete token from Redis: {e}"))
        })?;

        if deleted > 0 {
            tracing::debug!(token_id = token_id, "Deleted token from Redis");
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

    /// The secret never reaches Redis in a readable form.
    ///
    /// Tokens live 5-15 minutes, but a dump, a replica, or an operator with
    /// `KEYS` access must not come away with a working credential inside that
    /// window. Asserted against the RAW stored bytes rather than through the
    /// store's own API, which would happily round-trip a plaintext secret and
    /// tell us nothing.
    #[tokio::test]
    #[ignore] // Requires Redis running
    async fn test_secret_is_not_stored_in_plaintext() {
        let store = RedisTokenStore::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to create store");

        let token_id = "digest-at-rest-test";
        let secret = "correct-horse-battery-staple-256-bits";
        store
            .store_token(
                token_id,
                TokenData::new(
                    TokenType::MagicLink,
                    secret.to_string(),
                    serde_json::json!({"email": "cargo@example.com"}),
                    Utc::now() + Duration::minutes(10),
                ),
            )
            .await
            .expect("Failed to store token");

        let mut conn = store.conn_manager.clone();
        let raw: String = conn
            .get(RedisTokenStore::token_key(token_id))
            .await
            .expect("Failed to read raw token record");

        assert!(
            !raw.contains(secret),
            "the secret must NOT be readable in the stored record, got: {raw}"
        );
        assert!(
            raw.contains(&RedisTokenStore::digest(secret)),
            "the stored record must carry the digest of the secret"
        );

        // And the digest is what makes it verifiable: the real secret still
        // consumes, so this is privacy at rest, not a broken store.
        let consumed = store
            .consume_token(token_id, secret)
            .await
            .expect("consume failed")
            .expect("the correct secret must still consume the token");
        assert_eq!(
            consumed.token, secret,
            "the caller gets back the secret it presented, never the digest"
        );
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
