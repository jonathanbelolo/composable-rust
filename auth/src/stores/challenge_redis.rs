//! Redis-based ` WebAuthn` challenge store implementation.
//!
//! This module provides secure, single-use challenge storage for `WebAuthn` operations using Redis.
//!
//! # Architecture
//!
//! Challenges are stored in Redis with:
//! - **Primary key**: `webauthn_challenge:{user_id}:{challenge}` → bincode-serialized `ChallengeData`
//! - **TTL**: Configurable (default 5 minutes)
//! - **Atomic consumption**: Uses GETDEL command for single-use guarantee
//!
//! # Security
//!
//! - **Single-use**: Challenges consumed atomically via GETDEL (get + delete in one operation)
//! - **Expiration**: Challenges automatically expire after TTL
//! - **User isolation**: Challenges keyed by (`user_id`, challenge) to prevent cross-user attacks
//! - **Replay protection**: Once consumed, challenge cannot be reused
//!
//! # Example
//!
//! ```no_run
//! use composable_rust_auth::stores::RedisChallengeStore;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let store = RedisChallengeStore::new("redis://127.0.0.1:6379").await?;
//! # Ok(())
//! # }
//! ```

use crate::error::{AuthError, Result};
use crate::providers::{ChallengeData, ChallengeStore};
use crate::state::UserId;
use chrono::{Duration, Utc};
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};

/// `Redis`-based `WebAuthn` challenge store with atomic consumption.
///
/// Provides:
/// - Single-use challenge storage (atomic GETDEL)
/// - Automatic expiration via TTL
/// - User isolation (challenges scoped to ``user_id``)
/// - Connection pooling via `ConnectionManager`
pub struct RedisChallengeStore {
    /// Connection manager for connection pooling.
    conn_manager: ConnectionManager,
}

impl RedisChallengeStore {
    /// Create a new `Redis` challenge store.
    ///
    /// # Arguments
    ///
    /// * `redis_url` - `Redis` connection URL (e.g., "<redis://127.0.0.1:6379>")
    ///
    /// # Errors
    ///
    /// Returns error if connection to `Redis` fails.
    pub async fn new(redis_url: &str) -> Result<Self> {
        let client = Client::open(redis_url).map_err(|e| {
            AuthError::InternalError(format!("Failed to create Redis client: {e}"))
        })?;

        let conn_manager = ConnectionManager::new(client).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to create Redis connection manager: {e}"))
        })?;

        Ok(Self { conn_manager })
    }

    /// Get the `Redis` key for a challenge.
    fn challenge_key(user_id: &UserId, challenge: &str) -> String {
        format!("webauthn_challenge:{}:{}", user_id.0, challenge)
    }
}

impl Clone for RedisChallengeStore {
    fn clone(&self) -> Self {
        Self {
            conn_manager: self.conn_manager.clone(),
        }
    }
}

impl ChallengeStore for RedisChallengeStore {
    async fn store_challenge(
        &self,
        user_id: UserId,
        challenge: String,
        ttl: Duration,
    ) -> Result<()> {
        let mut conn = self.conn_manager.clone();
        let challenge_key = Self::challenge_key(&user_id, &challenge);

        let now = Utc::now();
        let challenge_data = ChallengeData {
            user_id,
            challenge: challenge.clone(),
            created_at: now,
            expires_at: now + ttl,
        };

        // Serialize challenge data
        let challenge_bytes = bincode::serialize(&challenge_data)
            .map_err(|e| AuthError::SerializationError(e.to_string()))?;

        // Calculate TTL in seconds
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let ttl_seconds = ttl.num_seconds().max(1) as u64;

        // Store with TTL
        let _: () = conn
            .set_ex(&challenge_key, challenge_bytes, ttl_seconds)
            .await
            .map_err(|e| AuthError::InternalError(format!("Failed to store challenge: {e}")))?;

        tracing::info!(
            user_id = %user_id.0,
            challenge_len = challenge.len(),
            ttl_seconds = ttl_seconds,
            "Stored `WebAuthn` challenge in Redis"
        );

        Ok(())
    }

    async fn consume_challenge(
        &self,
        user_id: UserId,
        challenge: &str,
    ) -> Result<Option<ChallengeData>> {
        let mut conn = self.conn_manager.clone();
        let challenge_key = Self::challenge_key(&user_id, challenge);

        // GETDEL is atomic: get + delete in one operation
        // This ensures single-use semantics (no race conditions)
        let challenge_bytes: Option<Vec<u8>> = conn
            .get_del(&challenge_key)
            .await
            .map_err(|e| AuthError::InternalError(format!("Failed to consume challenge: {e}")))?;

        match challenge_bytes {
            Some(bytes) => {
                // Deserialize
                let challenge_data: ChallengeData = bincode::deserialize(&bytes)
                    .map_err(|e| AuthError::SerializationError(e.to_string()))?;

                // Verify not expired (double-check, TTL should handle this)
                let now = Utc::now();
                if challenge_data.expires_at <= now {
                    tracing::warn!(
                        user_id = %user_id.0,
                        "Challenge expired (TTL should have cleaned this up)"
                    );
                    return Ok(None);
                }

                // Verify user_id matches (defense in depth)
                if challenge_data.user_id != user_id {
                    tracing::error!(
                        "🚨 SECURITY ALERT: Challenge user_id mismatch! Expected {}, got {}",
                        user_id.0,
                        challenge_data.user_id.0
                    );
                    return Ok(None);
                }

                tracing::info!(
                    user_id = %user_id.0,
                    "Consumed `WebAuthn` challenge (single-use)"
                );

                Ok(Some(challenge_data))
            }
            None => {
                // Challenge not found (already consumed, expired, or never existed)
                Ok(None)
            }
        }
    }

    async fn delete_challenge(&self, user_id: UserId, challenge: &str) -> Result<()> {
        let mut conn = self.conn_manager.clone();
        let challenge_key = Self::challenge_key(&user_id, challenge);

        let _: () = conn.del(&challenge_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to delete challenge from Redis: {e}"))
        })?;

        tracing::debug!(
            user_id = %user_id.0,
            "Deleted `WebAuthn` challenge from Redis"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    async fn test_redis_challenge_lifecycle() {
        let store = RedisChallengeStore::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to create store");

        let user_id = UserId::new();
        let challenge = "test_challenge_abc123".to_string();

        // Store challenge
        store
            .store_challenge(user_id, challenge.clone(), Duration::minutes(5))
            .await
            .expect("Failed to store challenge");

        // Consume challenge (should succeed)
        let consumed = store
            .consume_challenge(user_id, &challenge)
            .await
            .expect("Failed to consume challenge");

        assert!(consumed.is_some());
        let data = consumed.unwrap();
        assert_eq!(data.user_id, user_id);
        assert_eq!(data.challenge, challenge);

        // Try to consume again (should fail - single use)
        let second_consume = store
            .consume_challenge(user_id, &challenge)
            .await
            .expect("Failed on second consume");

        assert!(second_consume.is_none());
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    async fn test_challenge_expiration() {
        let store = RedisChallengeStore::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to create store");

        let user_id = UserId::new();
        let challenge = "expiring_challenge".to_string();

        // Store with very short TTL (1 second)
        store
            .store_challenge(user_id, challenge.clone(), Duration::seconds(1))
            .await
            .expect("Failed to store challenge");

        // Wait for expiration
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Try to consume (should fail - expired)
        let result = store
            .consume_challenge(user_id, &challenge)
            .await
            .expect("Failed to consume challenge");

        assert!(result.is_none());
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    async fn test_user_isolation() {
        let store = RedisChallengeStore::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to create store");

        let user1 = UserId::new();
        let user2 = UserId::new();
        let challenge = "shared_challenge".to_string();

        // Store same challenge for two different users
        store
            .store_challenge(user1, challenge.clone(), Duration::minutes(5))
            .await
            .expect("Failed to store challenge for user1");

        store
            .store_challenge(user2, challenge.clone(), Duration::minutes(5))
            .await
            .expect("Failed to store challenge for user2");

        // User1 consumes their challenge
        let user1_result = store
            .consume_challenge(user1, &challenge)
            .await
            .expect("Failed to consume user1 challenge");
        assert!(user1_result.is_some());

        // User2 can still consume their challenge
        let user2_result = store
            .consume_challenge(user2, &challenge)
            .await
            .expect("Failed to consume user2 challenge");
        assert!(user2_result.is_some());
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    async fn test_delete_challenge() {
        let store = RedisChallengeStore::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to create store");

        let user_id = UserId::new();
        let challenge = "delete_me".to_string();

        // Store challenge
        store
            .store_challenge(user_id, challenge.clone(), Duration::minutes(5))
            .await
            .expect("Failed to store challenge");

        // Delete challenge
        store
            .delete_challenge(user_id, &challenge)
            .await
            .expect("Failed to delete challenge");

        // Try to consume (should fail - deleted)
        let result = store
            .consume_challenge(user_id, &challenge)
            .await
            .expect("Failed to consume challenge");

        assert!(result.is_none());
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    async fn test_atomic_consumption() {
        let store = RedisChallengeStore::new("redis://127.0.0.1:6379")
            .await
            .expect("Failed to create store");

        let user_id = UserId::new();
        let challenge = "concurrent_test".to_string();

        // Store challenge
        store
            .store_challenge(user_id, challenge.clone(), Duration::minutes(5))
            .await
            .expect("Failed to store challenge");

        // Spawn 10 concurrent tasks trying to consume the same challenge
        let mut handles = vec![];
        for _ in 0..10 {
            let store_clone = store.clone();
            let challenge_clone = challenge.clone();
            let handle = tokio::spawn(async move {
                store_clone
                    .consume_challenge(user_id, &challenge_clone)
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
    }
}
