//! Mock user repository for testing.

use crate::error::{AuthError, Result};
use crate::providers::{User, UserRepository, OAuthLink, MagicLinkToken, PasskeyCredential};
use crate::state::{OAuthProvider, UserId};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

/// Mock user repository.
///
/// Uses in-memory storage for testing.
#[derive(Debug, Clone)]
pub struct MockUserRepository {
    users: Arc<Mutex<HashMap<UserId, User>>>,
    users_by_email: Arc<Mutex<HashMap<String, User>>>,
}

impl MockUserRepository {
    /// Create a new mock user repository.
    #[must_use]
    pub fn new() -> Self {
        Self {
            users: Arc::new(Mutex::new(HashMap::new())),
            users_by_email: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for MockUserRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl UserRepository for MockUserRepository {
    fn get_user_by_id(
        &self,
        user_id: UserId,
    ) -> impl Future<Output = Result<User>> + Send {
        let users = Arc::clone(&self.users);

        async move {
            users
                .lock()
                .map_err(|_| AuthError::InternalError)?
                .get(&user_id)
                .cloned()
                .ok_or(AuthError::ResourceNotFound)
        }
    }

    fn get_user_by_email(
        &self,
        email: &str,
    ) -> impl Future<Output = Result<User>> + Send {
        let users_by_email = Arc::clone(&self.users_by_email);
        let email = email.to_string();

        async move {
            users_by_email
                .lock()
                .map_err(|_| AuthError::InternalError)?
                .get(&email)
                .cloned()
                .ok_or(AuthError::ResourceNotFound)
        }
    }

    fn create_user(
        &self,
        user: &User,
    ) -> impl Future<Output = Result<User>> + Send {
        let users = Arc::clone(&self.users);
        let users_by_email = Arc::clone(&self.users_by_email);
        let user = user.clone();

        async move {
            let mut users_guard = users.lock().map_err(|_| AuthError::InternalError)?;
            let mut email_guard = users_by_email.lock().map_err(|_| AuthError::InternalError)?;

            // Check if email already exists
            if email_guard.contains_key(&user.email) {
                return Err(AuthError::DatabaseError("Email already exists".to_string()));
            }

            users_guard.insert(user.user_id, user.clone());
            email_guard.insert(user.email.clone(), user.clone());

            Ok(user)
        }
    }

    fn update_user(
        &self,
        user: &User,
    ) -> impl Future<Output = Result<User>> + Send {
        let users = Arc::clone(&self.users);
        let users_by_email = Arc::clone(&self.users_by_email);
        let user = user.clone();

        async move {
            let mut users_guard = users.lock().map_err(|_| AuthError::InternalError)?;
            let mut email_guard = users_by_email.lock().map_err(|_| AuthError::InternalError)?;

            if !users_guard.contains_key(&user.user_id) {
                return Err(AuthError::ResourceNotFound);
            }

            users_guard.insert(user.user_id, user.clone());
            email_guard.insert(user.email.clone(), user.clone());

            Ok(user)
        }
    }

    fn email_exists(
        &self,
        email: &str,
    ) -> impl Future<Output = Result<bool>> + Send {
        let users_by_email = Arc::clone(&self.users_by_email);
        let email = email.to_string();

        async move {
            Ok(users_by_email
                .lock()
                .map_err(|_| AuthError::InternalError)?
                .contains_key(&email))
        }
    }

    // OAuth links - simplified implementations
    fn get_oauth_link(
        &self,
        _user_id: UserId,
        _provider: OAuthProvider,
    ) -> impl Future<Output = Result<OAuthLink>> + Send {
        async move { Err(AuthError::ResourceNotFound) }
    }

    fn get_oauth_link_by_provider_id(
        &self,
        _provider: OAuthProvider,
        _provider_user_id: &str,
    ) -> impl Future<Output = Result<OAuthLink>> + Send {
        async move { Err(AuthError::ResourceNotFound) }
    }

    fn upsert_oauth_link(
        &self,
        link: &OAuthLink,
    ) -> impl Future<Output = Result<OAuthLink>> + Send {
        let link = link.clone();
        async move { Ok(link) }
    }

    fn delete_oauth_link(
        &self,
        _user_id: UserId,
        _provider: OAuthProvider,
    ) -> impl Future<Output = Result<()>> + Send {
        async move { Ok(()) }
    }

    // Magic link tokens - simplified implementations
    fn create_magic_link_token(
        &self,
        _token: &MagicLinkToken,
    ) -> impl Future<Output = Result<()>> + Send {
        async move { Ok(()) }
    }

    fn get_magic_link_token(
        &self,
        _token_hash: &str,
    ) -> impl Future<Output = Result<MagicLinkToken>> + Send {
        async move { Err(AuthError::MagicLinkInvalid) }
    }

    fn mark_magic_link_used(
        &self,
        _token_hash: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        async move { Ok(()) }
    }

    fn delete_expired_magic_links(
        &self,
    ) -> impl Future<Output = Result<usize>> + Send {
        async move { Ok(0) }
    }

    // Passkey credentials - simplified implementations
    fn get_passkey_credential(
        &self,
        _credential_id: &str,
    ) -> impl Future<Output = Result<PasskeyCredential>> + Send {
        async move { Err(AuthError::PasskeyNotFound) }
    }

    fn get_user_passkey_credentials(
        &self,
        _user_id: UserId,
    ) -> impl Future<Output = Result<Vec<PasskeyCredential>>> + Send {
        async move { Ok(Vec::new()) }
    }

    fn create_passkey_credential(
        &self,
        _credential: &PasskeyCredential,
    ) -> impl Future<Output = Result<()>> + Send {
        async move { Ok(()) }
    }

    fn update_passkey_counter(
        &self,
        _credential_id: &str,
        _counter: u32,
    ) -> impl Future<Output = Result<()>> + Send {
        async move { Ok(()) }
    }

    fn delete_passkey_credential(
        &self,
        _credential_id: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        async move { Ok(()) }
    }
}
