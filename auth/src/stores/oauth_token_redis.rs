//! Redis-based OAuth token store implementation with encryption at rest.
//!
//! This module provides secure storage for OAuth access/refresh tokens using Redis with:
//! - **AES-256-GCM encryption** for all tokens stored
//! - **TTL-based expiration** aligned with token lifetime
//! - **Atomic updates** for token refresh
//!
//! # Security
//!
//! ⚠️ **CRITICAL**: All tokens are encrypted at rest using AES-256-GCM before storing in Redis.
//! The encryption key MUST be:
//! - Generated using a CSPRNG (cryptographically secure random number generator)
//! - Stored securely (e.g., AWS Secrets Manager, HashiCorp Vault, environment variable)
//! - Rotated periodically (e.g., every 90 days)
//! - Never committed to version control
//!
//! # Architecture
//!
//! Tokens are stored in Redis with:
//! - **Primary key**: `oauth_token:{user_id}:{provider}` → encrypted bincode-serialized OAuthTokenData
//! - **TTL**: Aligned with token expiration (or 30 days default if no expiration)
//!
//! # Example
//!
//! ```no_run
//! use composable_rust_auth::stores::RedisOAuthTokenStore;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Generate a secure encryption key (32 bytes for AES-256)
//! let encryption_key = vec![0u8; 32]; // Replace with actual secure key!
//!
//! let store = RedisOAuthTokenStore::new(
//!     "redis://127.0.0.1:6379",
//!     encryption_key
//! ).await?;
//! # Ok(())
//! # }
//! ```

use crate::error::{AuthError, Result};
use crate::providers::{OAuthTokenData, OAuthTokenStore};
use crate::state::{OAuthProvider, UserId};
use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use chrono::Utc;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use std::sync::Arc;

/// Redis-based OAuth token store with AES-256-GCM encryption at rest.
///
/// Provides:
/// - Secure token storage with encryption
/// - Automatic expiration aligned with token lifetime
/// - Connection pooling via `ConnectionManager`
///
/// This is a pure storage implementation. Token refresh logic belongs in reducers.
pub struct RedisOAuthTokenStore {
    /// Connection manager for connection pooling.
    conn_manager: ConnectionManager,
    /// AES-256-GCM cipher for token encryption.
    /// Wrapped in Arc for safe cloning without nonce reuse risks.
    cipher: Arc<Aes256Gcm>,
}

impl RedisOAuthTokenStore {
    /// Create a new Redis OAuth token store with encryption.
    ///
    /// # Arguments
    ///
    /// * `redis_url` - Redis connection URL (e.g., "redis://127.0.0.1:6379")
    /// * `encryption_key` - 32-byte AES-256 encryption key (MUST be from secure source)
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Connection to Redis fails
    /// - Encryption key is invalid (not 32 bytes)
    ///
    /// # Security
    ///
    /// The encryption key MUST:
    /// - Be exactly 32 bytes (256 bits)
    /// - Come from a secure source (Secrets Manager, Vault, etc.)
    /// - Never be hardcoded or committed to version control
    pub async fn new(redis_url: &str, encryption_key: Vec<u8>) -> Result<Self> {
        // Validate encryption key length
        if encryption_key.len() != 32 {
            return Err(AuthError::InternalError(
                "Encryption key must be exactly 32 bytes (256 bits) for AES-256-GCM".to_string(),
            ));
        }

        let client = Client::open(redis_url).map_err(|e| {
            AuthError::InternalError(format!("Failed to create Redis client: {e}"))
        })?;

        let conn_manager = ConnectionManager::new(client).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to create Redis connection manager: {e}"))
        })?;

        // Initialize AES-256-GCM cipher
        let cipher = Aes256Gcm::new_from_slice(&encryption_key).map_err(|e| {
            AuthError::InternalError(format!("Failed to initialize AES-256-GCM cipher: {e}"))
        })?;

        Ok(Self {
            conn_manager,
            cipher: Arc::new(cipher),
        })
    }

    /// Get the Redis key for OAuth tokens.
    fn token_key(user_id: &UserId, provider: &OAuthProvider) -> String {
        format!("oauth_token:{}:{}", user_id.0, provider.as_str())
    }

    /// Encrypt data using AES-256-GCM.
    ///
    /// Returns (nonce, ciphertext) tuple.
    fn encrypt(&self, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        // Generate random nonce (96 bits for GCM)
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

        // Encrypt
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| AuthError::InternalError(format!("Encryption failed: {e}")))?;

        Ok((nonce.to_vec(), ciphertext))
    }

    /// Decrypt data using AES-256-GCM.
    fn decrypt(&self, nonce_bytes: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
        // Convert nonce bytes back to Nonce type
        // Nonce for AES-GCM is 96 bits (12 bytes)
        if nonce_bytes.len() != 12 {
            return Err(AuthError::InternalError(
                "Invalid nonce length (expected 12 bytes)".to_string(),
            ));
        }

        let nonce = Nonce::clone_from_slice(nonce_bytes);

        // Decrypt
        let plaintext = self
            .cipher
            .decrypt(&nonce, ciphertext)
            .map_err(|e| AuthError::InternalError(format!("Decryption failed: {e}")))?;

        Ok(plaintext)
    }

    /// Calculate TTL for token storage.
    ///
    /// If token has expiration, use it. Otherwise, default to 30 days.
    fn calculate_ttl(token_data: &OAuthTokenData) -> u64 {
        if let Some(expires_at) = token_data.expires_at {
            let now = Utc::now();
            let remaining = expires_at.signed_duration_since(now);
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let seconds = remaining.num_seconds().max(60) as u64; // Minimum 60 seconds
            seconds
        } else {
            // Default: 30 days for tokens without expiration
            30 * 24 * 60 * 60
        }
    }
}

impl Clone for RedisOAuthTokenStore {
    fn clone(&self) -> Self {
        // Arc allows safe sharing of the cipher across clones.
        // Each encrypt() call generates a fresh nonce, so nonce reuse is not a concern.
        Self {
            conn_manager: self.conn_manager.clone(),
            cipher: Arc::clone(&self.cipher),
        }
    }
}

impl OAuthTokenStore for RedisOAuthTokenStore {
    async fn store_tokens(&self, tokens: &OAuthTokenData) -> Result<()> {
        let mut conn = self.conn_manager.clone();
        let token_key = Self::token_key(&tokens.user_id, &tokens.provider);

        // Serialize token data
        let token_bytes =
            bincode::serialize(tokens).map_err(|e| AuthError::SerializationError(e.to_string()))?;

        // Encrypt the serialized data
        let (nonce, ciphertext) = self.encrypt(&token_bytes)?;

        // Combine nonce + ciphertext for storage
        // Format: [nonce (12 bytes)][ciphertext (variable)]
        let mut encrypted_data = Vec::with_capacity(nonce.len() + ciphertext.len());
        encrypted_data.extend_from_slice(&nonce);
        encrypted_data.extend_from_slice(&ciphertext);

        // Calculate TTL
        let ttl_seconds = Self::calculate_ttl(tokens);

        // Store encrypted data with TTL
        let _: () = conn
            .set_ex(&token_key, encrypted_data, ttl_seconds)
            .await
            .map_err(|e| AuthError::InternalError(format!("Failed to store OAuth tokens: {e}")))?;

        tracing::info!(
            user_id = %tokens.user_id.0,
            provider = %tokens.provider.as_str(),
            ttl_seconds = ttl_seconds,
            has_refresh_token = tokens.refresh_token.is_some(),
            "Stored OAuth tokens in Redis (encrypted)"
        );

        Ok(())
    }

    async fn get_tokens(
        &self,
        user_id: UserId,
        provider: OAuthProvider,
    ) -> Result<Option<OAuthTokenData>> {
        let mut conn = self.conn_manager.clone();
        let token_key = Self::token_key(&user_id, &provider);

        let encrypted_data: Option<Vec<u8>> = conn.get(&token_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to get OAuth tokens from Redis: {e}"))
        })?;

        match encrypted_data {
            Some(data) => {
                // Extract nonce and ciphertext
                if data.len() < 12 {
                    return Err(AuthError::InternalError(
                        "Encrypted data too short (missing nonce)".to_string(),
                    ));
                }

                let (nonce, ciphertext) = data.split_at(12);

                // Decrypt
                let plaintext = self.decrypt(nonce, ciphertext)?;

                // Deserialize
                let tokens: OAuthTokenData = bincode::deserialize(&plaintext)
                    .map_err(|e| AuthError::SerializationError(e.to_string()))?;

                Ok(Some(tokens))
            }
            None => Ok(None),
        }
    }

    async fn delete_tokens(&self, user_id: UserId, provider: OAuthProvider) -> Result<()> {
        let mut conn = self.conn_manager.clone();
        let token_key = Self::token_key(&user_id, &provider);

        let _: () = conn.del(&token_key).await.map_err(|e| {
            AuthError::InternalError(format!("Failed to delete OAuth tokens from Redis: {e}"))
        })?;

        tracing::info!(
            user_id = %user_id.0,
            provider = %provider.as_str(),
            "Deleted OAuth tokens from Redis"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::OAuthProvider;
    use chrono::Duration;

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    async fn test_redis_oauth_token_lifecycle() {
        // Generate test encryption key (32 bytes)
        let encryption_key = vec![42u8; 32];

        let store = RedisOAuthTokenStore::new("redis://127.0.0.1:6379", encryption_key)
            .await
            .expect("Failed to create store");

        let user_id = UserId::new();
        let provider = OAuthProvider::Google;

        let token_data = OAuthTokenData {
            user_id,
            provider,
            access_token: "test_access_token_12345".to_string(),
            refresh_token: Some("test_refresh_token_67890".to_string()),
            expires_at: Some(Utc::now() + Duration::hours(1)),
            stored_at: Utc::now(),
        };

        // Store
        store
            .store_tokens(&token_data)
            .await
            .expect("Failed to store tokens");

        // Retrieve
        let retrieved = store
            .get_tokens(user_id, provider)
            .await
            .expect("Failed to get tokens")
            .expect("Tokens not found");

        assert_eq!(retrieved.access_token, token_data.access_token);
        assert_eq!(retrieved.refresh_token, token_data.refresh_token);
        assert_eq!(retrieved.provider, provider);

        // Delete
        store
            .delete_tokens(user_id, provider)
            .await
            .expect("Failed to delete tokens");

        // Verify deleted
        let after_delete = store
            .get_tokens(user_id, provider)
            .await
            .expect("Failed to get tokens after delete");
        assert!(after_delete.is_none());
    }

    #[tokio::test]
    #[ignore] // Requires Redis running
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    async fn test_encryption_at_rest() {
        let encryption_key = vec![99u8; 32];

        let store = RedisOAuthTokenStore::new("redis://127.0.0.1:6379", encryption_key)
            .await
            .expect("Failed to create store");

        let user_id = UserId::new();
        let provider = OAuthProvider::GitHub;

        let token_data = OAuthTokenData {
            user_id,
            provider,
            access_token: "secret_access_token".to_string(),
            refresh_token: Some("secret_refresh_token".to_string()),
            expires_at: Some(Utc::now() + Duration::hours(1)),
            stored_at: Utc::now(),
        };

        // Store tokens
        store
            .store_tokens(&token_data)
            .await
            .expect("Failed to store tokens");

        // Manually retrieve raw encrypted data from Redis
        let mut conn = store.conn_manager.clone();
        let token_key = RedisOAuthTokenStore::token_key(&user_id, &provider);
        let raw_data: Vec<u8> = conn
            .get(&token_key)
            .await
            .expect("Failed to get raw data");

        // Verify the raw data does NOT contain the plaintext tokens
        let raw_string = String::from_utf8_lossy(&raw_data);
        assert!(
            !raw_string.contains("secret_access_token"),
            "Access token found in plaintext!"
        );
        assert!(
            !raw_string.contains("secret_refresh_token"),
            "Refresh token found in plaintext!"
        );

        // But decryption should work
        let retrieved = store
            .get_tokens(user_id, provider)
            .await
            .expect("Failed to decrypt tokens")
            .expect("Tokens not found");

        assert_eq!(retrieved.access_token, "secret_access_token");
        assert_eq!(
            retrieved.refresh_token,
            Some("secret_refresh_token".to_string())
        );

        // Cleanup
        store
            .delete_tokens(user_id, provider)
            .await
            .expect("Failed to delete tokens");
    }
}
