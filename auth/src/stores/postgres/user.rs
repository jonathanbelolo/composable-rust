//! PostgreSQL user repository implementation.
//!
//! This module provides persistent storage for user accounts, OAuth links,
//! magic link tokens, and passkey credentials using PostgreSQL.
//!
//! # Architecture
//!
//! **Event Sourcing**: This repository reads from projection tables built from events.
//! All write operations should go through event emission in reducers.
//!
//! # Example
//!
//! ```no_run
//! use composable_rust_auth::stores::postgres::PostgresUserRepository;
//! use sqlx::PgPool;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let pool = PgPool::connect("postgresql://localhost/auth").await?;
//! let repo = PostgresUserRepository::new(pool);
//! # Ok(())
//! # }
//! ```

use crate::error::{AuthError, Result};
use crate::providers::{User, UserRepository, OAuthLink, MagicLinkToken, PasskeyCredential};
use crate::state::{OAuthProvider, UserId, DeviceId};
use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// PostgreSQL user repository.
///
/// Provides persistent storage for user accounts and related data using PostgreSQL.
#[derive(Clone)]
pub struct PostgresUserRepository {
    /// PostgreSQL connection pool.
    pool: PgPool,
}

impl PostgresUserRepository {
    /// Create a new PostgreSQL user repository.
    ///
    /// # Arguments
    ///
    /// * `pool` - PostgreSQL connection pool
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Run database migrations.
    ///
    /// # Errors
    ///
    /// Returns error if migrations fail.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| AuthError::DatabaseError(format!("Migration failed: {e}")))?;
        Ok(())
    }
}

impl UserRepository for PostgresUserRepository {
    async fn get_user_by_id(&self, user_id: UserId) -> Result<User> {
        let row = sqlx::query!(
            r#"
            SELECT user_id, email, name, email_verified, created_at, updated_at
            FROM users_projection
            WHERE user_id = $1
            "#,
            user_id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to get user: {e}")))?
        .ok_or(AuthError::ResourceNotFound)?;

        Ok(User {
            user_id: UserId(row.user_id),
            email: row.email,
            name: row.name,
            email_verified: row.email_verified,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    async fn get_user_by_email(&self, email: &str) -> Result<User> {
        // Validate email before querying
        crate::utils::validate_email(email)?;

        let row = sqlx::query!(
            r#"
            SELECT user_id, email, name, email_verified, created_at, updated_at
            FROM users_projection
            WHERE email = $1
            "#,
            email
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to get user: {e}")))?
        .ok_or(AuthError::ResourceNotFound)?;

        Ok(User {
            user_id: UserId(row.user_id),
            email: row.email,
            name: row.name,
            email_verified: row.email_verified,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    async fn create_user(&self, user: &User) -> Result<User> {
        // Validate email before insertion
        crate::utils::validate_email(&user.email)?;

        sqlx::query!(
            r#"
            INSERT INTO users_projection
                (user_id, email, name, email_verified, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
            user.user_id.0,
            user.email,
            user.name,
            user.email_verified,
            user.created_at,
            user.updated_at,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            // Check for duplicate email constraint
            if let sqlx::Error::Database(db_err) = &e {
                if db_err.is_unique_violation() {
                    return AuthError::DatabaseError("Email already exists".to_string());
                }
            }
            AuthError::DatabaseError(format!("Failed to create user: {e}"))
        })?;

        Ok(user.clone())
    }

    async fn update_user(&self, user: &User) -> Result<User> {
        let result = sqlx::query!(
            r#"
            UPDATE users_projection
            SET email = $2,
                name = $3,
                email_verified = $4,
                updated_at = $5
            WHERE user_id = $1
            "#,
            user.user_id.0,
            user.email,
            user.name,
            user.email_verified,
            user.updated_at,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to update user: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AuthError::ResourceNotFound);
        }

        Ok(user.clone())
    }

    async fn email_exists(&self, email: &str) -> Result<bool> {
        // Validate email before querying
        crate::utils::validate_email(email)?;

        let row = sqlx::query!(
            r#"
            SELECT EXISTS(SELECT 1 FROM users_projection WHERE email = $1) AS "exists!"
            "#,
            email
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to check email: {e}")))?;

        Ok(row.exists)
    }

    // ═══════════════════════════════════════════════════════════════════════
    // OAuth Links
    // ═══════════════════════════════════════════════════════════════════════

    async fn get_oauth_link(&self, user_id: UserId, provider: OAuthProvider) -> Result<OAuthLink> {
        let provider_str = provider.as_str();

        let row = sqlx::query!(
            r#"
            SELECT user_id, provider, provider_user_id, linked_at
            FROM oauth_links_projection
            WHERE user_id = $1 AND provider = $2
            "#,
            user_id.0,
            provider_str
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to get OAuth link: {e}")))?
        .ok_or(AuthError::ResourceNotFound)?;

        Ok(OAuthLink {
            user_id: UserId(row.user_id),
            provider,
            provider_user_id: row.provider_user_id,
            access_token: String::new(), // TODO: Not stored in projection, load from token store
            refresh_token: None, // TODO: Not stored in projection
            expires_at: None, // TODO: Not stored in projection
            created_at: row.linked_at,
            updated_at: row.linked_at,
        })
    }

    async fn get_oauth_link_by_provider_id(
        &self,
        provider: OAuthProvider,
        provider_user_id: &str,
    ) -> Result<OAuthLink> {
        let provider_str = provider.as_str();

        let row = sqlx::query!(
            r#"
            SELECT user_id, provider, provider_user_id, linked_at
            FROM oauth_links_projection
            WHERE provider = $1 AND provider_user_id = $2
            "#,
            provider_str,
            provider_user_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to get OAuth link: {e}")))?
        .ok_or(AuthError::ResourceNotFound)?;

        Ok(OAuthLink {
            user_id: UserId(row.user_id),
            provider,
            provider_user_id: row.provider_user_id,
            access_token: String::new(), // TODO: Not stored in projection, load from token store
            refresh_token: None, // TODO: Not stored in projection
            expires_at: None, // TODO: Not stored in projection
            created_at: row.linked_at,
            updated_at: row.linked_at,
        })
    }

    async fn upsert_oauth_link(&self, link: &OAuthLink) -> Result<OAuthLink> {
        let provider_str = link.provider.as_str();

        sqlx::query!(
            r#"
            INSERT INTO oauth_links_projection
                (user_id, provider, provider_user_id, linked_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (user_id, provider)
            DO UPDATE SET
                provider_user_id = EXCLUDED.provider_user_id,
                linked_at = EXCLUDED.linked_at
            "#,
            link.user_id.0,
            provider_str,
            link.provider_user_id,
            link.created_at,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to upsert OAuth link: {e}")))?;

        Ok(link.clone())
    }

    async fn delete_oauth_link(&self, user_id: UserId, provider: OAuthProvider) -> Result<()> {
        let provider_str = provider.as_str();

        sqlx::query!(
            r#"
            DELETE FROM oauth_links_projection
            WHERE user_id = $1 AND provider = $2
            "#,
            user_id.0,
            provider_str
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to delete OAuth link: {e}")))?;

        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Magic Link Tokens
    // ═══════════════════════════════════════════════════════════════════════

    async fn create_magic_link_token(&self, _token: &MagicLinkToken) -> Result<()> {
        // NOTE: Magic link tokens are stored in Redis (RedisTokenStore) for ephemeral storage.
        // PostgreSQL is not used for magic link tokens as they have short TTL.
        // This method is kept for trait compatibility but should not be used.
        Err(AuthError::InternalError(
            "Magic link tokens should be stored in Redis, not PostgreSQL".to_string(),
        ))
    }

    async fn get_magic_link_token(&self, _token_hash: &str) -> Result<MagicLinkToken> {
        // NOTE: Magic link tokens are stored in Redis (RedisTokenStore).
        Err(AuthError::MagicLinkInvalid)
    }

    async fn mark_magic_link_used(&self, _token_hash: &str) -> Result<()> {
        // NOTE: Magic link tokens are stored in Redis (RedisTokenStore).
        Err(AuthError::InternalError(
            "Magic link tokens should be stored in Redis, not PostgreSQL".to_string(),
        ))
    }

    async fn delete_expired_magic_links(&self) -> Result<usize> {
        // NOTE: Magic link tokens are stored in Redis with TTL.
        Ok(0)
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Passkey Credentials
    // ═══════════════════════════════════════════════════════════════════════

    async fn get_passkey_credential(&self, credential_id: &str) -> Result<PasskeyCredential> {
        let row = sqlx::query!(
            r#"
            SELECT credential_id, user_id, device_id, public_key, counter, registered_at, last_used
            FROM passkeys_projection
            WHERE credential_id = $1
            "#,
            credential_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to get passkey credential: {e}")))?
        .ok_or(AuthError::PasskeyNotFound)?;

        // Convert INTEGER (i32) counter to u32 with proper validation
        // Migration 003 ensures counter is always non-negative and <= i32::MAX
        let counter = u32::try_from(row.counter).map_err(|_| {
            AuthError::DatabaseError(format!(
                "Invalid passkey counter value: {} (must be 0-2147483647)",
                row.counter
            ))
        })?;

        Ok(PasskeyCredential {
            credential_id: row.credential_id,
            user_id: UserId(row.user_id),
            device_id: DeviceId(row.device_id),
            public_key: row.public_key,
            counter,
            created_at: row.registered_at,
            last_used: row.last_used,
        })
    }

    async fn get_user_passkey_credentials(&self, user_id: UserId) -> Result<Vec<PasskeyCredential>> {
        let rows = sqlx::query!(
            r#"
            SELECT credential_id, user_id, device_id, public_key, counter, registered_at, last_used
            FROM passkeys_projection
            WHERE user_id = $1
            ORDER BY registered_at DESC
            "#,
            user_id.0
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to get passkey credentials: {e}")))?;

        // Convert rows to credentials, validating counter values
        let mut credentials = Vec::new();
        for row in rows {
            // Convert INTEGER (i32) counter to u32 with proper validation
            // Migration 003 ensures counter is always non-negative and <= i32::MAX
            let counter = u32::try_from(row.counter).map_err(|_| {
                AuthError::DatabaseError(format!(
                    "Invalid passkey counter value: {} (must be 0-2147483647)",
                    row.counter
                ))
            })?;

            credentials.push(PasskeyCredential {
                credential_id: row.credential_id,
                user_id: UserId(row.user_id),
                device_id: DeviceId(row.device_id),
                public_key: row.public_key,
                counter,
                created_at: row.registered_at,
                last_used: row.last_used,
            });
        }

        Ok(credentials)
    }

    async fn create_passkey_credential(&self, credential: &PasskeyCredential) -> Result<()> {
        // Convert u32 counter to i32 for database
        // This is safe because u32::MAX > i32::MAX, but WebAuthn counters
        // will never reach i32::MAX in practice (would require 2 billion authentications)
        let counter_i32 = i32::try_from(credential.counter).map_err(|_| {
            AuthError::DatabaseError(format!(
                "Counter value {} exceeds i32::MAX (implementation limit)",
                credential.counter
            ))
        })?;

        sqlx::query!(
            r#"
            INSERT INTO passkeys_projection
                (credential_id, user_id, device_id, public_key, counter, registered_at, last_used)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            credential.credential_id,
            credential.user_id.0,
            credential.device_id.0,
            credential.public_key,
            counter_i32,
            credential.created_at,
            credential.last_used,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to create passkey credential: {e}")))?;

        Ok(())
    }

    async fn update_passkey_counter(&self, credential_id: &str, counter: u32) -> Result<()> {
        // Convert u32 counter to i32 for database
        let counter_i32 = i32::try_from(counter).map_err(|_| {
            AuthError::DatabaseError(format!(
                "Counter value {} exceeds i32::MAX (implementation limit)",
                counter
            ))
        })?;

        let result = sqlx::query!(
            r#"
            UPDATE passkeys_projection
            SET counter = $2,
                last_used = NOW()
            WHERE credential_id = $1
            "#,
            credential_id,
            counter_i32
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to update passkey counter: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AuthError::PasskeyNotFound);
        }

        Ok(())
    }

    async fn update_passkey_counter_atomic(
        &self,
        credential_id: &str,
        expected_old_counter: u32,
        new_counter: u32,
    ) -> Result<bool> {
        // Convert u32 counters to i32 for database
        let expected_old_counter_i32 = i32::try_from(expected_old_counter).map_err(|_| {
            AuthError::DatabaseError(format!(
                "Expected counter value {} exceeds i32::MAX",
                expected_old_counter
            ))
        })?;
        let new_counter_i32 = i32::try_from(new_counter).map_err(|_| {
            AuthError::DatabaseError(format!(
                "New counter value {} exceeds i32::MAX",
                new_counter
            ))
        })?;

        // ✅ SECURITY FIX (BLOCKER #6): Atomic compare-and-swap with explicit transaction
        //
        // This implementation uses SELECT FOR UPDATE to acquire an exclusive row lock,
        // ensuring true atomicity even under high concurrency.
        //
        // Transaction flow:
        // 1. BEGIN TRANSACTION (automatic with sqlx::Transaction)
        // 2. SELECT ... FOR UPDATE (acquire exclusive lock on row)
        // 3. UPDATE ... WHERE counter = $expected (atomic check-then-act)
        // 4. COMMIT (release lock)
        //
        // Why SELECT FOR UPDATE?
        // - Prevents race conditions under READ COMMITTED isolation (PostgreSQL default)
        // - Explicit row-level locking ensures no concurrent modifications
        // - More efficient than SERIALIZABLE isolation (no transaction retries)
        //
        // Example race condition prevented:
        //   Request A: SELECT FOR UPDATE (acquires lock)
        //   Request B: SELECT FOR UPDATE (blocks, waits for A's lock)
        //   Request A: UPDATE counter 100→101, COMMIT (releases lock)
        //   Request B: SELECT FOR UPDATE completes, reads counter=101
        //   Request B: UPDATE WHERE counter=100 (fails, 0 rows affected)
        //   Request B: COMMIT, returns Ok(false)

        let mut tx = self.pool.begin().await
            .map_err(|e| AuthError::DatabaseError(format!("Failed to start transaction: {e}")))?;

        // Step 1: Acquire exclusive lock on the credential row
        // This blocks concurrent updates until our transaction completes
        let _lock = sqlx::query!(
            r#"
            SELECT counter
            FROM passkeys_projection
            WHERE credential_id = $1
            FOR UPDATE
            "#,
            credential_id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to lock credential: {e}")))?;

        // If credential doesn't exist, rollback and return error
        if _lock.is_none() {
            let _ = tx.rollback().await; // Ignore rollback errors
            return Err(AuthError::PasskeyNotFound);
        }

        // Step 2: Perform atomic compare-and-swap update
        // Only succeeds if counter still matches expected_old_counter
        let result = sqlx::query!(
            r#"
            UPDATE passkeys_projection
            SET counter = $2,
                last_used = NOW()
            WHERE credential_id = $1 AND counter = $3
            "#,
            credential_id,
            new_counter_i32,
            expected_old_counter_i32
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to update passkey counter: {e}")))?;

        // Step 3: Commit transaction (releases lock)
        tx.commit().await
            .map_err(|e| AuthError::DatabaseError(format!("Failed to commit transaction: {e}")))?;

        // Return true if CAS succeeded (counter matched expected value)
        // Return false if CAS failed (counter was changed between lock and update)
        Ok(result.rows_affected() == 1)
    }

    async fn delete_passkey_credential(&self, credential_id: &str) -> Result<()> {
        sqlx::query!(
            r#"
            DELETE FROM passkeys_projection
            WHERE credential_id = $1
            "#,
            credential_id
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to delete passkey credential: {e}")))?;

        Ok(())
    }
}
