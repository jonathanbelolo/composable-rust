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
            let sessions_guard = sessions.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            let session = sessions_guard
                .get(&session_id)
                .cloned()
                .ok_or(AuthError::SessionNotFound)?;

            // Check if expired
            if session.expires_at < chrono::Utc::now() {
                return Err(AuthError::SessionExpired);
            }

            Ok(session)
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

            if !sessions_guard.contains_key(&session.session_id) {
                return Err(AuthError::SessionNotFound);
            }

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
}
