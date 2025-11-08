//! Session store trait.

use crate::error::Result;
use crate::state::{Session, SessionId, UserId};
use chrono::Duration;

/// Session store.
///
/// This trait abstracts over session storage (Redis).
///
/// # Implementation Notes
///
/// - Sessions are ephemeral (24-hour TTL)
/// - Sliding expiration on each access
/// - Fast lookups (<5ms target)
pub trait SessionStore: Send + Sync {
    /// Create session.
    ///
    /// # Arguments
    ///
    /// - `session`: Session to create
    /// - `ttl`: Time to live (typically 24 hours)
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Network request fails
    /// - Session ID already exists
    async fn create_session(
        &self,
        session: &Session,
        ttl: Duration,
    ) -> Result<()>;

    /// Get session.
    ///
    /// # Returns
    ///
    /// The session if found and not expired.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Network request fails
    /// - Session not found → `AuthError::SessionNotFound`
    /// - Session expired → `AuthError::SessionExpired`
    async fn get_session(
        &self,
        session_id: SessionId,
    ) -> Result<Session>;

    /// Update session.
    ///
    /// Updates last_active and refreshes TTL.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Network request fails
    /// - Session not found
    async fn update_session(
        &self,
        session: &Session,
    ) -> Result<()>;

    /// Delete session.
    ///
    /// # Errors
    ///
    /// Returns error if network request fails.
    async fn delete_session(
        &self,
        session_id: SessionId,
    ) -> Result<()>;

    /// Delete all sessions for a user.
    ///
    /// # Errors
    ///
    /// Returns error if network request fails.
    async fn delete_user_sessions(
        &self,
        user_id: UserId,
    ) -> Result<usize>;

    /// Check if session exists.
    ///
    /// # Returns
    ///
    /// `true` if session exists and is not expired.
    ///
    /// # Errors
    ///
    /// Returns error if network request fails.
    async fn exists(
        &self,
        session_id: SessionId,
    ) -> Result<bool>;

    /// Get remaining TTL for session.
    ///
    /// # Returns
    ///
    /// Remaining time to live, or `None` if session doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns error if network request fails.
    async fn get_ttl(
        &self,
        session_id: SessionId,
    ) -> Result<Option<Duration>>;
}
