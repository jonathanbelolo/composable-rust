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
    fn create_session(
        &self,
        session: &Session,
        ttl: Duration,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

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
    fn get_session(
        &self,
        session_id: SessionId,
    ) -> impl std::future::Future<Output = Result<Session>> + Send;

    /// Update session.
    ///
    /// Updates last_active and refreshes TTL.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Network request fails
    /// - Session not found
    fn update_session(
        &self,
        session: &Session,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Delete session.
    ///
    /// # Errors
    ///
    /// Returns error if network request fails.
    fn delete_session(
        &self,
        session_id: SessionId,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Delete all sessions for a user.
    ///
    /// # Errors
    ///
    /// Returns error if network request fails.
    fn delete_user_sessions(
        &self,
        user_id: UserId,
    ) -> impl std::future::Future<Output = Result<usize>> + Send;

    /// Check if session exists.
    ///
    /// # Returns
    ///
    /// `true` if session exists and is not expired.
    ///
    /// # Errors
    ///
    /// Returns error if network request fails.
    fn exists(
        &self,
        session_id: SessionId,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Get remaining TTL for session.
    ///
    /// # Returns
    ///
    /// Remaining time to live, or `None` if session doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns error if network request fails.
    fn get_ttl(
        &self,
        session_id: SessionId,
    ) -> impl std::future::Future<Output = Result<Option<Duration>>> + Send;

    /// Get all active sessions for a user.
    ///
    /// This method returns only valid sessions and automatically cleans up
    /// any dead references from the user's session set.
    ///
    /// # Returns
    ///
    /// List of valid session IDs for the user.
    ///
    /// # Errors
    ///
    /// Returns error if network request fails.
    fn get_user_sessions(
        &self,
        user_id: UserId,
    ) -> impl std::future::Future<Output = Result<Vec<SessionId>>> + Send;
}
