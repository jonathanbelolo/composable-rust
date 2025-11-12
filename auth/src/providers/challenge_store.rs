//! `WebAuthn` challenge storage trait.
//!
//! Stores ` WebAuthn` challenges securely with expiration and atomic consumption.
//!
//! # Security
//!
//! ` WebAuthn` challenges must be:
//! - **Single-use**: Consumed atomically to prevent replay attacks
//! - **Ephemeral**: Expire after 5 minutes (configurable)
//! - **Unique**: Cryptographically random (256 bits minimum)
//! - **Associated**: Tied to specific user/device
//!
//! # Implementation
//!
//! **Production**: Use Redis with atomic operations (GET + DELETE)
//! **Testing**: Use in-memory `HashMap` with Mutex
//!
//! # Example
//!
//! ```ignore
//! // Store challenge
//! let challenge = "base64url_encoded_challenge_256_bits";
//! let ttl = Duration::minutes(5);
//! challenge_store.store_challenge(user_id, challenge, ttl).await?;
//!
//! // Consume challenge (atomic, single-use)
//! match challenge_store.consume_challenge(user_id, challenge).await? {
//!     Some(stored_challenge) => {
//!         // Challenge valid and consumed
//!         verify_webauthn_assertion(stored_challenge)?;
//!     }
//!     None => {
//!         // Challenge expired, already used, or never existed
//!         return Err(AuthError::ChallengeNotFound);
//!     }
//! }
//! ```

use crate::error::Result;
use crate::state::UserId;
use chrono::Duration;

/// `WebAuthn` challenge data.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChallengeData {
    /// User ID associated with this challenge.
    pub user_id: UserId,

    /// Challenge string (base64url encoded).
    pub challenge: String,

    /// Challenge created timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// Challenge expiration timestamp.
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// `WebAuthn` challenge store.
///
/// This trait abstracts over `WebAuthn` challenge storage with atomic consumption.
///
/// # Security Properties
///
/// 1. **Single-use**: Challenges are consumed atomically (removed on first use)
/// 2. **Expiration**: Challenges expire after `TTL` (default 5 minutes)
/// 3. **No replay**: Challenge cannot be reused after consumption
/// 4. **Isolation**: Challenges are user-specific
///
/// # Implementation Notes
///
/// **Production** (`Redis`):
/// ```ignore
/// // Store: SET key value EX ttl_seconds
/// redis.set_ex(key, value, ttl_seconds).await?;
///
/// // Consume: GETDEL key (atomic get-and-delete)
/// redis.getdel(key).await?;
/// ```
///
/// **Testing** (In-memory):
/// ```ignore
/// // Use HashMap with Mutex for thread safety
/// // Remove expired challenges on access
/// ```
pub trait ChallengeStore: Send + Sync {
    /// Store a `WebAuthn` challenge with expiration.
    ///
    /// # Arguments
    ///
    /// * ``user_id`` - User ID associated with this challenge
    /// * `challenge` - Challenge string (base64url encoded)
    /// * `ttl` - Time-to-live for this challenge
    ///
    /// # Security
    ///
    /// Challenges MUST expire after `TTL` to prevent indefinite replay windows.
    ///
    /// # Errors
    ///
    /// Returns error if storage fails.
    fn store_challenge(
        &self,
        user_id: UserId,
        challenge: String,
        ttl: Duration,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Consume a `WebAuthn` challenge atomically (single-use).
    ///
    /// This operation is **atomic**: the challenge is removed from storage
    /// if and only if it exists and is valid. Multiple concurrent attempts
    /// to consume the same challenge will result in exactly one success.
    ///
    /// # Arguments
    ///
    /// * ``user_id`` - User ID to verify challenge ownership
    /// * `challenge` - Challenge string to consume
    ///
    /// # Returns
    ///
    /// - `Some(ChallengeData)` if challenge exists, is valid, and not expired
    /// - `None` if challenge doesn't exist, already consumed, or expired
    ///
    /// # Security
    ///
    /// This method MUST be atomic to prevent race conditions where
    /// multiple requests could use the same challenge.
    ///
    /// # Errors
    ///
    /// Returns error only on storage failures, not on missing/expired challenges.
    fn consume_challenge(
        &self,
        user_id: UserId,
        challenge: &str,
    ) -> impl std::future::Future<Output = Result<Option<ChallengeData>>> + Send;

    /// Delete a specific challenge (for cleanup or cancellation).
    ///
    /// # Arguments
    ///
    /// * ``user_id`` - User ID
    /// * `challenge` - Challenge string to delete
    ///
    /// # Errors
    ///
    /// Returns error if deletion fails (not found is OK).
    fn delete_challenge(
        &self,
        user_id: UserId,
        challenge: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}
