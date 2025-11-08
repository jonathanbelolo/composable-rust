//! Mock token store for testing.

use crate::error::{AuthError, Result};
use crate::providers::{TokenData, TokenStore};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Mock token store.
///
/// In-memory token store for testing with atomic single-use semantics.
#[derive(Debug, Clone)]
pub struct MockTokenStore {
    tokens: Arc<Mutex<HashMap<String, TokenData>>>,
}

impl MockTokenStore {
    /// Create a new mock token store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get all stored tokens (for testing).
    #[must_use]
    pub fn get_all(&self) -> HashMap<String, TokenData> {
        self.tokens.lock().unwrap().clone()
    }

    /// Clear all tokens (for testing).
    pub fn clear(&self) {
        self.tokens.lock().unwrap().clear();
    }
}

impl Default for MockTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenStore for MockTokenStore {
    async fn store_token(&self, token_id: &str, token_data: TokenData) -> Result<()> {
        let mut tokens = self.tokens.lock().unwrap();
        tokens.insert(token_id.to_string(), token_data);
        Ok(())
    }

    async fn consume_token(&self, token_id: &str, token: &str) -> Result<Option<TokenData>> {
        let mut tokens = self.tokens.lock().unwrap();

        // Atomic check-and-delete under mutex protection
        let Some(stored_data) = tokens.get(token_id) else {
            return Ok(None);
        };

        // ⚡ SECURITY FIX (BLOCKER #5): Constant-time validation to prevent timing attacks
        //
        // VULNERABILITY: Early returns create timing side-channel
        // - Wrong token: Fast return (no time check)
        // - Expired token: Slow return (time check + token removal)
        // → Attacker can distinguish between wrong vs expired tokens
        //
        // FIX: Always perform ALL checks, regardless of early failures

        // 1. Check token match (constant-time comparison)
        let token_matches = constant_time_eq::constant_time_eq(
            token.as_bytes(),
            stored_data.token.as_bytes(),
        );

        // 2. Check expiration (always execute, even if token doesn't match)
        let now = Utc::now();
        let is_expired = now > stored_data.expires_at;

        // 3. Determine if valid (both conditions must pass)
        let is_valid = token_matches && !is_expired;

        // 4. Always remove token if expired (regardless of match)
        //    This ensures cleanup happens in constant time
        if is_expired {
            tokens.remove(token_id);
        }

        // 5. Consume token only if fully valid
        if is_valid {
            // Token is valid and not expired - consume it
            let token_data = tokens.remove(token_id).ok_or_else(|| {
                AuthError::TokenConsumed("Token was consumed by another request".to_string())
            })?;
            Ok(Some(token_data))
        } else {
            // Invalid or expired - same return path for both cases
            Ok(None)
        }
    }

    async fn delete_token(&self, token_id: &str) -> Result<()> {
        let mut tokens = self.tokens.lock().unwrap();
        tokens.remove(token_id);
        Ok(())
    }

    async fn exists(&self, token_id: &str) -> Result<bool> {
        let tokens = self.tokens.lock().unwrap();

        let Some(token_data) = tokens.get(token_id) else {
            return Ok(false);
        };

        // Check if expired
        if Utc::now() > token_data.expires_at {
            return Ok(false);
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::TokenType;
    use chrono::Duration;

    #[tokio::test]
    async fn test_store_and_consume_token() {
        let store = MockTokenStore::new();

        let token_data = TokenData::new(
            TokenType::MagicLink,
            "test-token-123".to_string(),
            serde_json::json!({"email": "test@example.com"}),
            Utc::now() + Duration::minutes(10),
        );

        // Store token
        store
            .store_token("token-id-1", token_data.clone())
            .await
            .unwrap();

        // Verify exists
        assert!(store.exists("token-id-1").await.unwrap());

        // Consume token
        let consumed = store
            .consume_token("token-id-1", "test-token-123")
            .await
            .unwrap();

        assert!(consumed.is_some());
        assert_eq!(consumed.as_ref().unwrap().token, "test-token-123");

        // Token should no longer exist
        assert!(!store.exists("token-id-1").await.unwrap());

        // Second consume should fail
        let second = store
            .consume_token("token-id-1", "test-token-123")
            .await
            .unwrap();

        assert!(second.is_none());
    }

    #[tokio::test]
    async fn test_consume_wrong_token() {
        let store = MockTokenStore::new();

        let token_data = TokenData::new(
            TokenType::MagicLink,
            "correct-token".to_string(),
            serde_json::json!({"email": "test@example.com"}),
            Utc::now() + Duration::minutes(10),
        );

        store.store_token("token-id-1", token_data).await.unwrap();

        // Try to consume with wrong token
        let result = store
            .consume_token("token-id-1", "wrong-token")
            .await
            .unwrap();

        assert!(result.is_none());

        // Token should still exist
        assert!(store.exists("token-id-1").await.unwrap());
    }

    #[tokio::test]
    async fn test_consume_expired_token() {
        let store = MockTokenStore::new();

        let token_data = TokenData::new(
            TokenType::MagicLink,
            "test-token".to_string(),
            serde_json::json!({"email": "test@example.com"}),
            Utc::now() - Duration::seconds(1), // Already expired
        );

        store.store_token("token-id-1", token_data).await.unwrap();

        // Try to consume expired token
        let result = store
            .consume_token("token-id-1", "test-token")
            .await
            .unwrap();

        assert!(result.is_none());

        // Token should be removed
        assert!(!store.exists("token-id-1").await.unwrap());
    }

    #[tokio::test]
    async fn test_concurrent_consume_atomicity() {
        let store = MockTokenStore::new();

        let token_data = TokenData::new(
            TokenType::MagicLink,
            "test-token".to_string(),
            serde_json::json!({"email": "test@example.com"}),
            Utc::now() + Duration::minutes(10),
        );

        store.store_token("token-id-1", token_data).await.unwrap();

        // Simulate concurrent consumption attempts
        let store1 = store.clone();
        let store2 = store.clone();

        let (result1, result2) = tokio::join!(
            store1.consume_token("token-id-1", "test-token"),
            store2.consume_token("token-id-1", "test-token"),
        );

        // Exactly one should succeed
        let success_count = [result1.unwrap(), result2.unwrap()]
            .iter()
            .filter(|r| r.is_some())
            .count();

        assert_eq!(success_count, 1, "Exactly one concurrent request should succeed");

        // Token should no longer exist
        assert!(!store.exists("token-id-1").await.unwrap());
    }
}
