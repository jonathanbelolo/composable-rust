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
    /// Passkey credentials storage for atomic counter update testing
    passkey_credentials: Arc<Mutex<HashMap<String, PasskeyCredential>>>,
}

impl MockUserRepository {
    /// Create a new mock user repository.
    #[must_use]
    pub fn new() -> Self {
        Self {
            users: Arc::new(Mutex::new(HashMap::new())),
            users_by_email: Arc::new(Mutex::new(HashMap::new())),
            passkey_credentials: Arc::new(Mutex::new(HashMap::new())),
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
                .map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?
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
                .map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?
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
            let mut users_guard = users.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;
            let mut email_guard = users_by_email.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

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
            let mut users_guard = users.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;
            let mut email_guard = users_by_email.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

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
                .map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?
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
        credential_id: &str,
    ) -> impl Future<Output = Result<PasskeyCredential>> + Send {
        let credentials = Arc::clone(&self.passkey_credentials);
        let credential_id = credential_id.to_string();

        async move {
            credentials
                .lock()
                .map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?
                .get(&credential_id)
                .cloned()
                .ok_or(AuthError::PasskeyNotFound)
        }
    }

    fn get_user_passkey_credentials(
        &self,
        user_id: UserId,
    ) -> impl Future<Output = Result<Vec<PasskeyCredential>>> + Send {
        let credentials = Arc::clone(&self.passkey_credentials);

        async move {
            let credentials_guard = credentials
                .lock()
                .map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            // Filter credentials by user_id and collect into Vec
            let user_credentials: Vec<PasskeyCredential> = credentials_guard
                .values()
                .filter(|cred| cred.user_id == user_id)
                .cloned()
                .collect();

            Ok(user_credentials)
        }
    }

    fn create_passkey_credential(
        &self,
        credential: &PasskeyCredential,
    ) -> impl Future<Output = Result<()>> + Send {
        let credentials = Arc::clone(&self.passkey_credentials);
        let credential = credential.clone();

        async move {
            let mut credentials_guard = credentials
                .lock()
                .map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            credentials_guard.insert(credential.credential_id.clone(), credential);
            Ok(())
        }
    }

    fn update_passkey_counter(
        &self,
        _credential_id: &str,
        _counter: u32,
    ) -> impl Future<Output = Result<()>> + Send {
        async move { Ok(()) }
    }

    fn update_passkey_counter_atomic(
        &self,
        credential_id: &str,
        expected_old_counter: u32,
        new_counter: u32,
    ) -> impl Future<Output = Result<bool>> + Send {
        let credentials = Arc::clone(&self.passkey_credentials);
        let credential_id = credential_id.to_string();

        async move {
            let mut credentials_guard = credentials
                .lock()
                .map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            // Get mutable reference to credential
            let credential = credentials_guard
                .get_mut(&credential_id)
                .ok_or(AuthError::PasskeyNotFound)?;

            // Atomic compare-and-swap: only update if counter matches expected value
            if credential.counter == expected_old_counter {
                credential.counter = new_counter;
                Ok(true) // CAS succeeded
            } else {
                Ok(false) // CAS failed - counter was changed by concurrent request
            }
        }
    }

    fn delete_passkey_credential(
        &self,
        credential_id: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        let credentials = Arc::clone(&self.passkey_credentials);
        let credential_id = credential_id.to_string();

        async move {
            let mut credentials_guard = credentials
                .lock()
                .map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            // Remove the credential from the HashMap
            // Returns None if credential doesn't exist (idempotent delete)
            credentials_guard.remove(&credential_id);

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DeviceId;

    #[tokio::test]
    async fn test_atomic_counter_update_exactly_once_semantics() {
        // This test validates that update_passkey_counter_atomic() provides
        // exactly-once semantics under concurrent authentication attempts.
        //
        // SECURITY RATIONALE:
        // Without atomic updates, cloned authenticators could bypass detection
        // by authenticating concurrently before either updates the counter.

        let repo = MockUserRepository::new();
        let user_id = UserId::new();
        let device_id = DeviceId::new();
        let credential_id = "test_credential_concurrent".to_string();

        // Create initial credential with counter=100
        let credential = PasskeyCredential {
            credential_id: credential_id.clone(),
            user_id,
            device_id,
            public_key: vec![1, 2, 3, 4],
            counter: 100,
            created_at: chrono::Utc::now(),
            last_used: None,
        };
        repo.create_passkey_credential(&credential).await.unwrap();

        // Spawn 10 concurrent authentication attempts
        // All try to update counter from 100 → 101
        let mut handles = vec![];
        for _ in 0..10 {
            let repo_clone = repo.clone();
            let cred_id = credential_id.clone();
            let handle = tokio::spawn(async move {
                repo_clone
                    .update_passkey_counter_atomic(&cred_id, 100, 101)
                    .await
            });
            handles.push(handle);
        }

        // Collect results
        let mut successful_updates = 0;
        let mut failed_updates = 0;

        for handle in handles {
            match handle.await.unwrap() {
                Ok(true) => successful_updates += 1,  // CAS succeeded
                Ok(false) => failed_updates += 1,     // CAS failed (concurrent modification)
                Err(_) => panic!("Unexpected error during concurrent update"),
            }
        }

        // ✅ CRITICAL ASSERTION: Exactly ONE update should succeed
        assert_eq!(
            successful_updates, 1,
            "Expected exactly 1 successful atomic update, got {}",
            successful_updates
        );
        assert_eq!(
            failed_updates, 9,
            "Expected 9 failed updates due to concurrent modification, got {}",
            failed_updates
        );

        // Verify final counter value is 101 (not 100 or corrupted)
        let final_credential = repo.get_passkey_credential(&credential_id).await.unwrap();
        assert_eq!(
            final_credential.counter, 101,
            "Final counter should be 101 (exactly one increment)"
        );
    }

    #[tokio::test]
    async fn test_atomic_counter_update_sequential_success() {
        // This test validates that sequential atomic updates work correctly.

        let repo = MockUserRepository::new();
        let user_id = UserId::new();
        let device_id = DeviceId::new();
        let credential_id = "test_credential_sequential".to_string();

        // Create initial credential with counter=100
        let credential = PasskeyCredential {
            credential_id: credential_id.clone(),
            user_id,
            device_id,
            public_key: vec![1, 2, 3, 4],
            counter: 100,
            created_at: chrono::Utc::now(),
            last_used: None,
        };
        repo.create_passkey_credential(&credential).await.unwrap();

        // First update: 100 → 101
        let result = repo
            .update_passkey_counter_atomic(&credential_id, 100, 101)
            .await
            .unwrap();
        assert!(result, "First atomic update should succeed");

        // Second update: 101 → 102
        let result = repo
            .update_passkey_counter_atomic(&credential_id, 101, 102)
            .await
            .unwrap();
        assert!(result, "Second atomic update should succeed");

        // Third update: 102 → 103
        let result = repo
            .update_passkey_counter_atomic(&credential_id, 102, 103)
            .await
            .unwrap();
        assert!(result, "Third atomic update should succeed");

        // Verify final counter
        let final_credential = repo.get_passkey_credential(&credential_id).await.unwrap();
        assert_eq!(final_credential.counter, 103);
    }

    #[tokio::test]
    async fn test_atomic_counter_update_detects_stale_counter() {
        // This test validates that stale counter values are rejected.

        let repo = MockUserRepository::new();
        let user_id = UserId::new();
        let device_id = DeviceId::new();
        let credential_id = "test_credential_stale".to_string();

        // Create initial credential with counter=100
        let credential = PasskeyCredential {
            credential_id: credential_id.clone(),
            user_id,
            device_id,
            public_key: vec![1, 2, 3, 4],
            counter: 100,
            created_at: chrono::Utc::now(),
            last_used: None,
        };
        repo.create_passkey_credential(&credential).await.unwrap();

        // Update counter to 105
        let result = repo
            .update_passkey_counter_atomic(&credential_id, 100, 105)
            .await
            .unwrap();
        assert!(result, "Initial update should succeed");

        // Attempt to update with stale counter (expects 100, but actual is 105)
        let result = repo
            .update_passkey_counter_atomic(&credential_id, 100, 106)
            .await
            .unwrap();
        assert!(!result, "Update with stale counter should fail (CAS rejection)");

        // Verify counter unchanged (still 105)
        let final_credential = repo.get_passkey_credential(&credential_id).await.unwrap();
        assert_eq!(final_credential.counter, 105);
    }

    #[tokio::test]
    async fn test_atomic_counter_update_nonexistent_credential() {
        // This test validates proper error handling for nonexistent credentials.

        let repo = MockUserRepository::new();

        // Attempt to update counter for credential that doesn't exist
        let result = repo
            .update_passkey_counter_atomic("nonexistent_credential", 100, 101)
            .await;

        assert!(
            matches!(result, Err(AuthError::PasskeyNotFound)),
            "Expected PasskeyNotFound error, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_list_user_passkey_credentials() {
        // This test validates listing all credentials for a user.

        let repo = MockUserRepository::new();
        let user_id = UserId::new();
        let device_id_1 = DeviceId::new();
        let device_id_2 = DeviceId::new();

        // Create two credentials for the user
        let credential_1 = PasskeyCredential {
            credential_id: "credential_1".to_string(),
            user_id,
            device_id: device_id_1,
            public_key: vec![1, 2, 3, 4],
            counter: 10,
            created_at: chrono::Utc::now(),
            last_used: None,
        };
        repo.create_passkey_credential(&credential_1).await.unwrap();

        let credential_2 = PasskeyCredential {
            credential_id: "credential_2".to_string(),
            user_id,
            device_id: device_id_2,
            public_key: vec![5, 6, 7, 8],
            counter: 20,
            created_at: chrono::Utc::now(),
            last_used: None,
        };
        repo.create_passkey_credential(&credential_2).await.unwrap();

        // List credentials for user
        let credentials = repo.get_user_passkey_credentials(user_id).await.unwrap();

        // Should return both credentials
        assert_eq!(credentials.len(), 2);
        assert!(credentials.iter().any(|c| c.credential_id == "credential_1"));
        assert!(credentials.iter().any(|c| c.credential_id == "credential_2"));
    }

    #[tokio::test]
    async fn test_list_user_passkey_credentials_empty() {
        // This test validates listing credentials when user has none.

        let repo = MockUserRepository::new();
        let user_id = UserId::new();

        // List credentials for user (no credentials exist)
        let credentials = repo.get_user_passkey_credentials(user_id).await.unwrap();

        // Should return empty list
        assert_eq!(credentials.len(), 0);
    }

    #[tokio::test]
    async fn test_list_user_passkey_credentials_isolation() {
        // This test validates that users can only see their own credentials.

        let repo = MockUserRepository::new();
        let user_1 = UserId::new();
        let user_2 = UserId::new();
        let device_id = DeviceId::new();

        // Create credential for user_1
        let credential_1 = PasskeyCredential {
            credential_id: "user1_credential".to_string(),
            user_id: user_1,
            device_id,
            public_key: vec![1, 2, 3, 4],
            counter: 10,
            created_at: chrono::Utc::now(),
            last_used: None,
        };
        repo.create_passkey_credential(&credential_1).await.unwrap();

        // Create credential for user_2
        let credential_2 = PasskeyCredential {
            credential_id: "user2_credential".to_string(),
            user_id: user_2,
            device_id,
            public_key: vec![5, 6, 7, 8],
            counter: 20,
            created_at: chrono::Utc::now(),
            last_used: None,
        };
        repo.create_passkey_credential(&credential_2).await.unwrap();

        // List credentials for user_1
        let user_1_credentials = repo.get_user_passkey_credentials(user_1).await.unwrap();

        // Should only return user_1's credential
        assert_eq!(user_1_credentials.len(), 1);
        assert_eq!(user_1_credentials[0].credential_id, "user1_credential");

        // List credentials for user_2
        let user_2_credentials = repo.get_user_passkey_credentials(user_2).await.unwrap();

        // Should only return user_2's credential
        assert_eq!(user_2_credentials.len(), 1);
        assert_eq!(user_2_credentials[0].credential_id, "user2_credential");
    }

    #[tokio::test]
    async fn test_delete_passkey_credential() {
        // This test validates deleting a credential.

        let repo = MockUserRepository::new();
        let user_id = UserId::new();
        let device_id = DeviceId::new();

        // Create credential
        let credential = PasskeyCredential {
            credential_id: "test_credential".to_string(),
            user_id,
            device_id,
            public_key: vec![1, 2, 3, 4],
            counter: 10,
            created_at: chrono::Utc::now(),
            last_used: None,
        };
        repo.create_passkey_credential(&credential).await.unwrap();

        // Verify credential exists
        let retrieved = repo.get_passkey_credential("test_credential").await.unwrap();
        assert_eq!(retrieved.credential_id, "test_credential");

        // Delete credential
        repo.delete_passkey_credential("test_credential").await.unwrap();

        // Verify credential is deleted
        let result = repo.get_passkey_credential("test_credential").await;
        assert!(
            matches!(result, Err(AuthError::PasskeyNotFound)),
            "Expected PasskeyNotFound after deletion, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_delete_passkey_credential_nonexistent() {
        // This test validates proper error handling when deleting nonexistent credential.

        let repo = MockUserRepository::new();

        // Attempt to delete credential that doesn't exist
        let result = repo.delete_passkey_credential("nonexistent_credential").await;

        // Should succeed (idempotent delete)
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_delete_passkey_credential_updates_list() {
        // This test validates that deletion is reflected in list results.

        let repo = MockUserRepository::new();
        let user_id = UserId::new();
        let device_id = DeviceId::new();

        // Create two credentials
        let credential_1 = PasskeyCredential {
            credential_id: "credential_1".to_string(),
            user_id,
            device_id,
            public_key: vec![1, 2, 3, 4],
            counter: 10,
            created_at: chrono::Utc::now(),
            last_used: None,
        };
        repo.create_passkey_credential(&credential_1).await.unwrap();

        let credential_2 = PasskeyCredential {
            credential_id: "credential_2".to_string(),
            user_id,
            device_id,
            public_key: vec![5, 6, 7, 8],
            counter: 20,
            created_at: chrono::Utc::now(),
            last_used: None,
        };
        repo.create_passkey_credential(&credential_2).await.unwrap();

        // Verify both credentials exist
        let credentials = repo.get_user_passkey_credentials(user_id).await.unwrap();
        assert_eq!(credentials.len(), 2);

        // Delete one credential
        repo.delete_passkey_credential("credential_1").await.unwrap();

        // Verify only one credential remains
        let credentials = repo.get_user_passkey_credentials(user_id).await.unwrap();
        assert_eq!(credentials.len(), 1);
        assert_eq!(credentials[0].credential_id, "credential_2");
    }
}
