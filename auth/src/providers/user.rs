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
    fn get_user_by_id(
        &self,
        user_id: UserId,
    ) -> impl std::future::Future<Output = Result<User>> + Send;

    /// Get user by email.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - User not found → `AuthError::ResourceNotFound`
    fn get_user_by_email(
        &self,
        email: &str,
    ) -> impl std::future::Future<Output = Result<User>> + Send;

    /// Create user.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - Email already exists
    fn create_user(
        &self,
        user: &User,
    ) -> impl std::future::Future<Output = Result<User>> + Send;

    /// Update user.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - User not found
    fn update_user(
        &self,
        user: &User,
    ) -> impl std::future::Future<Output = Result<User>> + Send;

    /// Check if email exists.
    ///
    /// # Returns
    ///
    /// `true` if email is already registered.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn email_exists(
        &self,
        email: &str,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

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
    fn get_oauth_link(
        &self,
        user_id: UserId,
        provider: OAuthProvider,
    ) -> impl std::future::Future<Output = Result<OAuthLink>> + Send;

    /// Get OAuth link by provider user ID.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - Link not found → `AuthError::ResourceNotFound`
    fn get_oauth_link_by_provider_id(
        &self,
        provider: OAuthProvider,
        provider_user_id: &str,
    ) -> impl std::future::Future<Output = Result<OAuthLink>> + Send;

    /// Create or update OAuth link.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn upsert_oauth_link(
        &self,
        link: &OAuthLink,
    ) -> impl std::future::Future<Output = Result<OAuthLink>> + Send;

    /// Delete OAuth link.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn delete_oauth_link(
        &self,
        user_id: UserId,
        provider: OAuthProvider,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    // ═══════════════════════════════════════════════════════════════════════
    // Magic Link Tokens
    // ═══════════════════════════════════════════════════════════════════════
    /// Create magic link token.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn create_magic_link_token(
        &self,
        token: &MagicLinkToken,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Get magic link token.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - Token not found → `AuthError::MagicLinkInvalid`
    /// - Token expired → `AuthError::MagicLinkExpired`
    /// - Token already used → `AuthError::MagicLinkAlreadyUsed`
    fn get_magic_link_token(
        &self,
        token_hash: &str,
    ) -> impl std::future::Future<Output = Result<MagicLinkToken>> + Send;

    /// Mark magic link token as used.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn mark_magic_link_used(
        &self,
        token_hash: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Delete expired magic link tokens.
    ///
    /// # Returns
    ///
    /// Number of tokens deleted.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn delete_expired_magic_links(
        &self,
    ) -> impl std::future::Future<Output = Result<usize>> + Send;

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
    fn get_passkey_credential(
        &self,
        credential_id: &str,
    ) -> impl std::future::Future<Output = Result<PasskeyCredential>> + Send;

    /// Get all passkey credentials for a user.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn get_user_passkey_credentials(
        &self,
        user_id: UserId,
    ) -> impl std::future::Future<Output = Result<Vec<PasskeyCredential>>> + Send;

    /// Create passkey credential.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn create_passkey_credential(
        &self,
        credential: &PasskeyCredential,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Update passkey counter.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn update_passkey_counter(
        &self,
        credential_id: &str,
        counter: u32,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Delete passkey credential.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn delete_passkey_credential(
        &self,
        credential_id: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}
