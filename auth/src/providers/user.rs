//! User repository trait.

use crate::error::Result;
use crate::state::{OAuthProvider, UserId};
use super::{User, OAuthLink, MagicLinkToken, PasskeyCredential};

/// User repository.
///
/// This trait abstracts over user database operations (PostgreSQL).
pub trait UserRepository: Send + Sync {
    /// Get user by ID.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - User not found → `AuthError::ResourceNotFound`
    async fn get_user_by_id(
        &self,
        user_id: UserId,
    ) -> Result<User>;

    /// Get user by email.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - User not found → `AuthError::ResourceNotFound`
    async fn get_user_by_email(
        &self,
        email: &str,
    ) -> Result<User>;

    /// Create user.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - Email already exists
    async fn create_user(
        &self,
        user: &User,
    ) -> Result<User>;

    /// Update user.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - User not found
    async fn update_user(
        &self,
        user: &User,
    ) -> Result<User>;

    /// Check if email exists.
    ///
    /// # Returns
    ///
    /// `true` if email is already registered.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    async fn email_exists(
        &self,
        email: &str,
    ) -> Result<bool>;

    // ═══════════════════════════════════════════════════════════════════════
    // OAuth Links
    // ═══════════════════════════════════════════════════════════════════════
    /// Get OAuth link.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - Link not found → `AuthError::ResourceNotFound`
    async fn get_oauth_link(
        &self,
        user_id: UserId,
        provider: OAuthProvider,
    ) -> Result<OAuthLink>;

    /// Get OAuth link by provider user ID.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - Link not found → `AuthError::ResourceNotFound`
    async fn get_oauth_link_by_provider_id(
        &self,
        provider: OAuthProvider,
        provider_user_id: &str,
    ) -> Result<OAuthLink>;

    /// Create or update OAuth link.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    async fn upsert_oauth_link(
        &self,
        link: &OAuthLink,
    ) -> Result<OAuthLink>;

    /// Delete OAuth link.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    async fn delete_oauth_link(
        &self,
        user_id: UserId,
        provider: OAuthProvider,
    ) -> Result<()>;

    // ═══════════════════════════════════════════════════════════════════════
    // Magic Link Tokens
    // ═══════════════════════════════════════════════════════════════════════
    /// Create magic link token.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    async fn create_magic_link_token(
        &self,
        token: &MagicLinkToken,
    ) -> Result<()>;

    /// Get magic link token.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - Token not found → `AuthError::MagicLinkInvalid`
    /// - Token expired → `AuthError::MagicLinkExpired`
    /// - Token already used → `AuthError::MagicLinkAlreadyUsed`
    async fn get_magic_link_token(
        &self,
        token_hash: &str,
    ) -> Result<MagicLinkToken>;

    /// Mark magic link token as used.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    async fn mark_magic_link_used(
        &self,
        token_hash: &str,
    ) -> Result<()>;

    /// Delete expired magic link tokens.
    ///
    /// # Returns
    ///
    /// Number of tokens deleted.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    async fn delete_expired_magic_links(
        &self,
    ) -> Result<usize>;

    // ═══════════════════════════════════════════════════════════════════════
    // Passkey Credentials
    // ═══════════════════════════════════════════════════════════════════════
    /// Get passkey credential.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - Credential not found → `AuthError::PasskeyNotFound`
    async fn get_passkey_credential(
        &self,
        credential_id: &str,
    ) -> Result<PasskeyCredential>;

    /// Get all passkey credentials for a user.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    async fn get_user_passkey_credentials(
        &self,
        user_id: UserId,
    ) -> Result<Vec<PasskeyCredential>>;

    /// Create passkey credential.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    async fn create_passkey_credential(
        &self,
        credential: &PasskeyCredential,
    ) -> Result<()>;

    /// Update passkey counter.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    async fn update_passkey_counter(
        &self,
        credential_id: &str,
        counter: u32,
    ) -> Result<()>;

    /// Delete passkey credential.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    async fn delete_passkey_credential(
        &self,
        credential_id: &str,
    ) -> Result<()>;
}
