//! OAuth token storage trait.
//!
//! Stores OAuth access tokens and refresh tokens securely.

use crate::error::Result;
use crate::state::{OAuthProvider, UserId};
use chrono::{DateTime, Utc};

/// `OAuth` token data.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OAuthTokenData {
    /// User ID.
    pub user_id: UserId,

    /// `OAuth` provider.
    pub provider: OAuthProvider,

    /// Access token (encrypted at rest).
    pub access_token: String,

    /// Refresh token (encrypted at rest).
    pub refresh_token: Option<String>,

    /// Token expiration timestamp.
    pub expires_at: Option<DateTime<Utc>>,

    /// When the token was stored.
    pub stored_at: DateTime<Utc>,
}

/// `OAuth` token store.
///
/// This trait abstracts over `OAuth` token storage.
///
/// # Security
///
/// **CRITICAL**: Tokens MUST be encrypted at rest. Use AES-256-GCM or similar.
///
/// # Implementation Notes
///
/// **Production**:
/// - `PostgreSQL`: Store in `oauth_tokens` table with encryption
/// - `Redis`: Store with `TTL` matching token expiry
///
/// **Testing**:
/// - In-memory store with plain-text tokens (tests only!)
///
/// # Example
///
/// ```ignore
/// // Store tokens after `OAuth` success
/// let token_data = OAuthTokenData {
///     `user_id`,
///     provider: OAuthProvider::Google,
///     access_token: "encrypted_access_token".to_string(),
///     refresh_token: Some("encrypted_refresh_token".to_string()),
///     expires_at: Some(Utc::now() + Duration::hours(1)),
///     stored_at: Utc::now(),
/// };
///
/// token_store.store_tokens(&token_data).await?;
///
/// // Retrieve tokens later
/// let tokens = token_store.get_tokens(`user_id`, OAuthProvider::Google).await?;
/// ```
pub trait OAuthTokenStore: Send + Sync {
    /// Store `OAuth` tokens for a user.
    ///
    /// # Security
    ///
    /// Tokens MUST be encrypted before storage.
    ///
    /// # Errors
    ///
    /// Returns error if storage fails.
    fn store_tokens(
        &self,
        tokens: &OAuthTokenData,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Get `OAuth` tokens for a user and provider.
    ///
    /// # Returns
    ///
    /// - `Some(tokens)` if found
    /// - `None` if no tokens exist for this user/provider
    ///
    /// # Errors
    ///
    /// Returns error if retrieval fails.
    fn get_tokens(
        &self,
        user_id: UserId,
        provider: OAuthProvider,
    ) -> impl std::future::Future<Output = Result<Option<OAuthTokenData>>> + Send;

    /// Delete `OAuth` tokens for a user and provider.
    ///
    /// Used during logout or token revocation.
    ///
    /// # Errors
    ///
    /// Returns error if deletion fails.
    fn delete_tokens(
        &self,
        user_id: UserId,
        provider: OAuthProvider,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}
