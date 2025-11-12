//! Mock ` WebAuthn` challenge store for testing.

use crate::error::Result;
use crate::providers::{ChallengeData, ChallengeStore};
use crate::state::UserId;
use chrono::{Duration, Utc};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Mock `WebAuthn` challenge store.
///
/// In-memory implementation for testing. Challenges stored with expiration.
///
/// **WARNING**: Do NOT use in production. This is for testing only!
#[derive(Clone)]
pub struct MockChallengeStore {
    challenges: Arc<Mutex<HashMap<(UserId, String), ChallengeData>>>,
}

impl MockChallengeStore {
    /// Create a new mock challenge store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            challenges: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Clean up expired challenges (called internally on access).
    #[allow(clippy::unwrap_used)] // Test mock: mutex poisoning is a test failure
    fn cleanup_expired(&self) {
        let mut store = self.challenges.lock().unwrap();
        let now = Utc::now();
        store.retain(|_, data| data.expires_at > now);
    }
}

impl Default for MockChallengeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ChallengeStore for MockChallengeStore {
    #[allow(clippy::unwrap_used)] // Test mock: mutex poisoning is a test failure
    async fn store_challenge(
        &self,
        user_id: UserId,
        challenge: String,
        ttl: Duration,
    ) -> Result<()> {
        self.cleanup_expired();

        let now = Utc::now();
        let challenge_data = ChallengeData {
            user_id,
            challenge: challenge.clone(),
            created_at: now,
            expires_at: now + ttl,
        };

        let mut store = self.challenges.lock().unwrap();
        let key = (user_id, challenge);
        store.insert(key, challenge_data);
        Ok(())
    }

    #[allow(clippy::unwrap_used)] // Test mock: mutex poisoning is a test failure
    async fn consume_challenge(
        &self,
        user_id: UserId,
        challenge: &str,
    ) -> Result<Option<ChallengeData>> {
        self.cleanup_expired();

        let mut store = self.challenges.lock().unwrap();
        let key = (user_id, challenge.to_string());

        // Atomic get-and-remove
        if let Some(data) = store.remove(&key) {
            let now = Utc::now();
            // Double-check expiration
            if data.expires_at > now {
                Ok(Some(data))
            } else {
                // Expired during this call
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    #[allow(clippy::unwrap_used)] // Test mock: mutex poisoning is a test failure
    async fn delete_challenge(&self, user_id: UserId, challenge: &str) -> Result<()> {
        let mut store = self.challenges.lock().unwrap();
        let key = (user_id, challenge.to_string());
        store.remove(&key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_store_and_consume_challenge() {
        let store = MockChallengeStore::new();
        let user_id = UserId::new();
        let challenge = "test_challenge_abc123".to_string();

        // Store challenge
        store
            .store_challenge(user_id, challenge.clone(), Duration::minutes(5))
            .await
            .unwrap();

        // Consume challenge
        let result = store.consume_challenge(user_id, &challenge).await.unwrap();
        assert!(result.is_some());
        let data = result.unwrap();
        assert_eq!(data.user_id, user_id);
        assert_eq!(data.challenge, challenge);

        // Try to consume again (should fail - single-use)
        let result = store.consume_challenge(user_id, &challenge).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_consume_nonexistent_challenge() {
        let store = MockChallengeStore::new();
        let user_id = UserId::new();

        let result = store
            .consume_challenge(user_id, "nonexistent")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_consume_expired_challenge() {
        let store = MockChallengeStore::new();
        let user_id = UserId::new();
        let challenge = "expired_challenge".to_string();

        // Store with very short TTL
        store
            .store_challenge(user_id, challenge.clone(), Duration::milliseconds(1))
            .await
            .unwrap();

        // Wait for expiration
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Try to consume (should fail - expired)
        let result = store.consume_challenge(user_id, &challenge).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_delete_challenge() {
        let store = MockChallengeStore::new();
        let user_id = UserId::new();
        let challenge = "delete_me".to_string();

        // Store challenge
        store
            .store_challenge(user_id, challenge.clone(), Duration::minutes(5))
            .await
            .unwrap();

        // Delete challenge
        store.delete_challenge(user_id, &challenge).await.unwrap();

        // Try to consume (should fail - deleted)
        let result = store.consume_challenge(user_id, &challenge).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_concurrent_consume_atomicity() {
        let store = MockChallengeStore::new();
        let user_id = UserId::new();
        let challenge = "concurrent_test".to_string();

        // Store challenge
        store
            .store_challenge(user_id, challenge.clone(), Duration::minutes(5))
            .await
            .unwrap();

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

        // Exactly one should succeed
        let successes = results.iter().filter(|r| r.is_some()).count();
        assert_eq!(successes, 1, "Exactly one consume should succeed");
    }

    #[tokio::test]
    async fn test_different_users_different_challenges() {
        let store = MockChallengeStore::new();
        let user1 = UserId::new();
        let user2 = UserId::new();
        let challenge = "shared_challenge".to_string();

        // Store same challenge for two different users
        store
            .store_challenge(user1, challenge.clone(), Duration::minutes(5))
            .await
            .unwrap();
        store
            .store_challenge(user2, challenge.clone(), Duration::minutes(5))
            .await
            .unwrap();

        // User 1 consumes their challenge
        let result = store.consume_challenge(user1, &challenge).await.unwrap();
        assert!(result.is_some());

        // User 2 can still consume their challenge
        let result = store.consume_challenge(user2, &challenge).await.unwrap();
        assert!(result.is_some());
    }
}
