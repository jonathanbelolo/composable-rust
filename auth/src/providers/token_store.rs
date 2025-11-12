//! Token store trait.
//!
//! This module defines the trait for storing and consuming one-time tokens
//! (magic link tokens, OAuth states, ` WebAuthn` challenges) with atomic
//! single-use semantics.

use crate::error::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Token types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenType {
    /// Magic link email verification token.
    MagicLink,

    /// `OAuth` `CSRF` state parameter.
    OAuthState,

    /// `WebAuthn` registration challenge.
    PasskeyRegistrationChallenge,

    /// `WebAuthn` authentication challenge.
    PasskeyAuthenticationChallenge,
}

/// Token data stored in the token store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenData {
    /// Token type.
    pub token_type: TokenType,

    /// Token value (the actual token string or state parameter).
    pub token: String,

    /// Associated data (e.g., email for magic link, provider for `OAuth`).
    pub data: serde_json::Value,

    /// Expiration time.
    pub expires_at: DateTime<Utc>,

    /// Creation time.
    pub created_at: DateTime<Utc>,
}

impl TokenData {
    /// Create new token data.
    #[must_use]
    pub fn new(
        token_type: TokenType,
        token: String,
        data: serde_json::Value,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            token_type,
            token,
            data,
            expires_at,
            created_at: Utc::now(),
        }
    }
}

/// Token store.
///
/// This trait abstracts over one-time token storage with atomic single-use semantics.
///
/// # Implementation Notes
///
/// - Tokens are ephemeral (5-15 minute `TTL` typically)
/// - **CRITICAL**: `consume_token()` MUST be atomic (use `Redis` GETDEL or database transactions)
/// - Tokens are single-use (consumed on first successful verification)
/// - Fast lookups (<5ms target)
///
/// # Security Requirements
///
/// 1. **Atomicity**: `consume_token()` must atomically check and delete
/// 2. **Single-use**: Once consumed, token cannot be reused
/// 3. **Expiration**: Expired tokens must be rejected
/// 4. **Constant-time**: Operations should not leak timing information
pub trait TokenStore: Send + Sync {
    /// Store a token.
    ///
    /// # Arguments
    ///
    /// - `token_id`: Unique identifier for this token (e.g., random `UUID`)
    /// - `token_data`: Token data to store
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Network request fails
    /// - Storage operation fails
    fn store_token(
        &self,
        token_id: &str,
        token_data: TokenData,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Consume a token atomically.
    ///
    /// This operation MUST be atomic:
    /// 1. Check if token exists and matches
    /// 2. Check if token is not expired
    /// 3. Delete token (ensuring single-use)
    ///
    /// All three steps must happen atomically to prevent race conditions.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(TokenData))`: Token was valid, not expired, and has been consumed
    /// - `Ok(None)`: Token not found, expired, or doesn't match
    /// - `Err(...)`: Storage operation failed
    ///
    /// # Security
    ///
    /// **CRITICAL**: This must be implemented using:
    /// - `Redis`: `GETDEL` command (atomic get-and-delete)
    /// - `PostgreSQL`: `DELETE ... RETURNING` in a transaction
    /// - In-memory: Mutex-protected check-and-delete
    ///
    /// Non-atomic implementations will allow token reuse attacks.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Network request fails
    /// - Storage operation fails
    fn consume_token(
        &self,
        token_id: &str,
        token: &str,
    ) -> impl std::future::Future<Output = Result<Option<TokenData>>> + Send;

    /// Delete a token without returning it.
    ///
    /// Used for cleanup or explicit revocation.
    ///
    /// # Errors
    ///
    /// Returns error if network request fails.
    fn delete_token(
        &self,
        token_id: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Check if a token exists.
    ///
    /// # Returns
    ///
    /// `true` if token exists and is not expired.
    ///
    /// # Errors
    ///
    /// Returns error if network request fails.
    fn exists(
        &self,
        token_id: &str,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;
}
