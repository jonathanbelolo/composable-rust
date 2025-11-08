//! Mock session store for testing.

use crate::error::{AuthError, Result};
use crate::providers::SessionStore;
use crate::state::{Session, SessionId, UserId};
use chrono::Duration;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

/// Mock session store.
///
/// Uses in-memory storage for testing.
#[derive(Debug, Clone)]
pub struct MockSessionStore {
    sessions: Arc<Mutex<HashMap<SessionId, Session>>>,
}

impl MockSessionStore {
    /// Create a new mock session store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get count of stored sessions (for testing).
    ///
    /// # Errors
    ///
    /// Returns error if lock is poisoned.
    pub fn session_count(&self) -> Result<usize> {
        Ok(self
            .sessions
            .lock()
            .map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?
            .len())
    }
}

impl Default for MockSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStore for MockSessionStore {
    fn create_session(
        &self,
        session: &Session,
        _ttl: Duration,
        max_concurrent_sessions: usize,
    ) -> impl Future<Output = Result<()>> + Send {
        let sessions = Arc::clone(&self.sessions);
        let session = session.clone();

        async move {
            let mut sessions_guard = sessions.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            if sessions_guard.contains_key(&session.session_id) {
                return Err(AuthError::DatabaseError(
                    "Session ID already exists".to_string(),
                ));
            }

            // ✅ SECURITY FIX (MEDIUM): Enforce concurrent session limits
            //
            // Count active sessions for this user
            let user_sessions: Vec<SessionId> = sessions_guard
                .iter()
                .filter(|(_, s)| s.user_id == session.user_id && s.expires_at >= chrono::Utc::now())
                .map(|(id, _)| *id)
                .collect();

            // If at limit, delete the oldest session
            if user_sessions.len() >= max_concurrent_sessions {
                // Find oldest session
                let mut oldest_id: Option<SessionId> = None;
                let mut oldest_created_at: Option<chrono::DateTime<chrono::Utc>> = None;

                for session_id in &user_sessions {
                    if let Some(s) = sessions_guard.get(session_id) {
                        if oldest_created_at.is_none() || s.created_at < oldest_created_at.unwrap() {
                            oldest_created_at = Some(s.created_at);
                            oldest_id = Some(*session_id);
                        }
                    }
                }

                // Delete oldest session
                if let Some(oldest) = oldest_id {
                    tracing::info!(
                        user_id = %session.user_id.0,
                        oldest_session_id = %oldest.0,
                        "Mock revoking oldest session (concurrent session limit reached)"
                    );
                    sessions_guard.remove(&oldest);
                }
            }

            sessions_guard.insert(session.session_id, session);
            Ok(())
        }
    }

    fn get_session(
        &self,
        session_id: SessionId,
    ) -> impl Future<Output = Result<Session>> + Send {
        let sessions = Arc::clone(&self.sessions);

        async move {
            let mut sessions_guard = sessions.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            let session = sessions_guard
                .get_mut(&session_id)
                .ok_or(AuthError::SessionNotFound)?;

            let now = chrono::Utc::now();

            // Check if expired
            if session.expires_at < now {
                return Err(AuthError::SessionExpired);
            }

            // ✅ SECURITY FIX (CRITICAL): Idle timeout enforcement
            //
            // Check if session has been idle too long (configured per session).
            // This matches the Redis implementation.
            let idle_timeout = session.idle_timeout;
            let idle_duration = now.signed_duration_since(session.last_active);

            if idle_duration > idle_timeout {
                tracing::warn!(
                    session_id = %session_id.0,
                    last_active = %session.last_active,
                    idle_duration_minutes = idle_duration.num_minutes(),
                    "Mock session idle timeout exceeded"
                );
                return Err(AuthError::SessionExpired);
            }

            // Update last_active timestamp (sliding window)
            session.last_active = now;

            Ok(session.clone())
        }
    }

    fn update_session(
        &self,
        session: &Session,
    ) -> impl Future<Output = Result<()>> + Send {
        let sessions = Arc::clone(&self.sessions);
        let session = session.clone();

        async move {
            let mut sessions_guard = sessions.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            // Get existing session for validation
            let existing_session = sessions_guard
                .get(&session.session_id)
                .cloned()
                .ok_or(AuthError::SessionNotFound)?;

            // ✅ SECURITY: Validate immutable fields haven't changed
            // This matches the RedisSessionStore validation logic
            if existing_session.user_id != session.user_id {
                tracing::error!(
                    session_id = %session.session_id.0,
                    existing_user_id = %existing_session.user_id.0,
                    new_user_id = %session.user_id.0,
                    "Mock: Attempt to change immutable user_id (privilege escalation attempt)"
                );
                return Err(AuthError::InternalError(
                    "Cannot change session user_id (immutable)".into(),
                ));
            }

            if existing_session.device_id != session.device_id {
                tracing::error!(
                    session_id = %session.session_id.0,
                    "Mock: Attempt to change immutable device_id"
                );
                return Err(AuthError::InternalError(
                    "Cannot change session device_id (immutable)".into(),
                ));
            }

            if existing_session.ip_address != session.ip_address {
                tracing::error!(
                    session_id = %session.session_id.0,
                    "Mock: Attempt to change immutable ip_address"
                );
                return Err(AuthError::InternalError(
                    "Cannot change session ip_address (immutable)".into(),
                ));
            }

            if existing_session.oauth_provider != session.oauth_provider {
                tracing::error!(
                    session_id = %session.session_id.0,
                    "Mock: Attempt to change immutable oauth_provider"
                );
                return Err(AuthError::InternalError(
                    "Cannot change session oauth_provider (immutable)".into(),
                ));
            }

            if existing_session.login_risk_score != session.login_risk_score {
                tracing::error!(
                    session_id = %session.session_id.0,
                    "Mock: Attempt to change immutable login_risk_score"
                );
                return Err(AuthError::InternalError(
                    "Cannot change session login_risk_score (immutable)".into(),
                ));
            }

            // All validations passed - update the session
            sessions_guard.insert(session.session_id, session);
            Ok(())
        }
    }

    fn delete_session(
        &self,
        session_id: SessionId,
    ) -> impl Future<Output = Result<()>> + Send {
        let sessions = Arc::clone(&self.sessions);

        async move {
            sessions
                .lock()
                .map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?
                .remove(&session_id);
            Ok(())
        }
    }

    fn delete_user_sessions(
        &self,
        user_id: UserId,
    ) -> impl Future<Output = Result<usize>> + Send {
        let sessions = Arc::clone(&self.sessions);

        async move {
            let mut sessions_guard = sessions.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            let session_ids_to_delete: Vec<SessionId> = sessions_guard
                .iter()
                .filter(|(_, s)| s.user_id == user_id)
                .map(|(id, _)| *id)
                .collect();

            let count = session_ids_to_delete.len();

            for session_id in session_ids_to_delete {
                sessions_guard.remove(&session_id);
            }

            Ok(count)
        }
    }

    fn exists(
        &self,
        session_id: SessionId,
    ) -> impl Future<Output = Result<bool>> + Send {
        let sessions = Arc::clone(&self.sessions);

        async move {
            let sessions_guard = sessions.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            if let Some(session) = sessions_guard.get(&session_id) {
                // Check if expired
                Ok(session.expires_at >= chrono::Utc::now())
            } else {
                Ok(false)
            }
        }
    }

    fn get_ttl(
        &self,
        session_id: SessionId,
    ) -> impl Future<Output = Result<Option<Duration>>> + Send {
        let sessions = Arc::clone(&self.sessions);

        async move {
            let sessions_guard = sessions.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            if let Some(session) = sessions_guard.get(&session_id) {
                let now = chrono::Utc::now();
                if session.expires_at > now {
                    Ok(Some(session.expires_at.signed_duration_since(now)))
                } else {
                    Ok(Some(Duration::zero()))
                }
            } else {
                Ok(None)
            }
        }
    }

    fn get_user_sessions(
        &self,
        user_id: UserId,
    ) -> impl Future<Output = Result<Vec<SessionId>>> + Send {
        let sessions = Arc::clone(&self.sessions);

        async move {
            let sessions_guard = sessions.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            // Return all non-expired sessions for this user
            let session_ids: Vec<SessionId> = sessions_guard
                .iter()
                .filter(|(_, session)| {
                    session.user_id == user_id && session.expires_at >= chrono::Utc::now()
                })
                .map(|(id, _)| *id)
                .collect();

            Ok(session_ids)
        }
    }

    fn rotate_session(
        &self,
        old_session_id: SessionId,
    ) -> impl Future<Output = Result<SessionId>> + Send {
        let sessions = Arc::clone(&self.sessions);

        async move {
            let mut sessions_guard = sessions.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            // Get existing session
            let mut session = sessions_guard
                .get(&old_session_id)
                .cloned()
                .ok_or(AuthError::SessionNotFound)?;

            // Generate new session ID
            let new_session_id = SessionId::new();

            // Update session ID
            session.session_id = new_session_id;

            // Atomically remove old and add new
            sessions_guard.remove(&old_session_id);
            sessions_guard.insert(new_session_id, session);

            tracing::info!(
                old_session_id = %old_session_id.0,
                new_session_id = %new_session_id.0,
                "Mock session ID rotated"
            );

            Ok(new_session_id)
        }
    }
}
