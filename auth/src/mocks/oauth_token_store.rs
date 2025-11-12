//! Mock OAuth token store.

use crate::error::Result;
use crate::providers::{OAuthTokenData, OAuthTokenStore};
use crate::state::{OAuthProvider, UserId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Mock `OAuth` token store.
///
/// In-memory implementation for testing. Tokens stored in plain-text.
///
/// **WARNING**: Do NOT use in production. This stores tokens unencrypted!
#[derive(Clone)]
pub struct MockOAuthTokenStore {
    tokens: Arc<Mutex<HashMap<(UserId, OAuthProvider), OAuthTokenData>>>,
}

impl MockOAuthTokenStore {
    /// Create a new mock `OAuth` token store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for MockOAuthTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

impl OAuthTokenStore for MockOAuthTokenStore {
    #[allow(clippy::unwrap_used)] // Test mock: mutex poisoning is a test failure
    async fn store_tokens(&self, tokens: &OAuthTokenData) -> Result<()> {
        let mut store = self.tokens.lock().unwrap();
        let key = (tokens.user_id, tokens.provider);
        store.insert(key, tokens.clone());
        Ok(())
    }

    #[allow(clippy::unwrap_used)] // Test mock: mutex poisoning is a test failure
    async fn get_tokens(
        &self,
        user_id: UserId,
        provider: OAuthProvider,
    ) -> Result<Option<OAuthTokenData>> {
        let store = self.tokens.lock().unwrap();
        let key = (user_id, provider);
        Ok(store.get(&key).cloned())
    }

    #[allow(clippy::unwrap_used)] // Test mock: mutex poisoning is a test failure
    async fn delete_tokens(&self, user_id: UserId, provider: OAuthProvider) -> Result<()> {
        let mut store = self.tokens.lock().unwrap();
        let key = (user_id, provider);
        store.remove(&key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    #[tokio::test]
    async fn test_store_and_retrieve_tokens() {
        let store = MockOAuthTokenStore::new();
        let user_id = UserId::new();

        let token_data = OAuthTokenData {
            user_id,
            provider: OAuthProvider::Google,
            access_token: "test_access_token".to_string(),
            refresh_token: Some("test_refresh_token".to_string()),
            expires_at: Some(Utc::now() + Duration::hours(1)),
            stored_at: Utc::now(),
        };

        // Store tokens
        store.store_tokens(&token_data).await.unwrap();

        // Retrieve tokens
        let retrieved = store
            .get_tokens(user_id, OAuthProvider::Google)
            .await
            .unwrap();

        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.access_token, "test_access_token");
        assert_eq!(
            retrieved.refresh_token,
            Some("test_refresh_token".to_string())
        );
    }

    #[tokio::test]
    async fn test_delete_tokens() {
        let store = MockOAuthTokenStore::new();
        let user_id = UserId::new();

        let token_data = OAuthTokenData {
            user_id,
            provider: OAuthProvider::GitHub,
            access_token: "test_token".to_string(),
            refresh_token: None,
            expires_at: None,
            stored_at: Utc::now(),
        };

        // Store then delete
        store.store_tokens(&token_data).await.unwrap();
        store
            .delete_tokens(user_id, OAuthProvider::GitHub)
            .await
            .unwrap();

        // Verify deleted
        let retrieved = store
            .get_tokens(user_id, OAuthProvider::GitHub)
            .await
            .unwrap();

        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_multiple_providers() {
        let store = MockOAuthTokenStore::new();
        let user_id = UserId::new();

        let google_tokens = OAuthTokenData {
            user_id,
            provider: OAuthProvider::Google,
            access_token: "google_token".to_string(),
            refresh_token: None,
            expires_at: None,
            stored_at: Utc::now(),
        };

        let github_tokens = OAuthTokenData {
            user_id,
            provider: OAuthProvider::GitHub,
            access_token: "github_token".to_string(),
            refresh_token: None,
            expires_at: None,
            stored_at: Utc::now(),
        };

        // Store both
        store.store_tokens(&google_tokens).await.unwrap();
        store.store_tokens(&github_tokens).await.unwrap();

        // Retrieve separately
        let google = store
            .get_tokens(user_id, OAuthProvider::Google)
            .await
            .unwrap()
            .unwrap();
        let github = store
            .get_tokens(user_id, OAuthProvider::GitHub)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(google.access_token, "google_token");
        assert_eq!(github.access_token, "github_token");
    }
}
