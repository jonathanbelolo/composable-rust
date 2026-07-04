//! Identity context propagation to `PostgreSQL`.
//!
//! This module provides functions to propagate user identity to `PostgreSQL`
//! via session variables. This enables:
//!
//! - Row Level Security (RLS) policies using `current_setting()`
//! - Audit logging with actor information
//! - Authorization checks in stored procedures
//!
//! # Session Variables
//!
//! The following session variables are set:
//!
//! | Variable | Type | Description |
//! |----------|------|-------------|
//! | `app.user_id` | `TEXT` | User identifier |
//! | `app.tenant_id` | `TEXT` | Tenant identifier |
//! | `app.roles` | `JSON` | Array of role strings |
//!
//! # Example: `PostgreSQL` Function
//!
//! ```sql
//! CREATE FUNCTION api.my_function() RETURNS void AS $$
//! DECLARE
//!     v_user_id text := current_setting('app.user_id', true);
//!     v_tenant_id text := current_setting('app.tenant_id', true);
//!     v_roles jsonb := current_setting('app.roles', true)::jsonb;
//! BEGIN
//!     -- Use identity in your logic
//!     IF NOT v_roles ? 'admin' THEN
//!         RAISE EXCEPTION 'Admin role required' USING ERRCODE = 'P0403';
//!     END IF;
//! END;
//! $$ LANGUAGE plpgsql;
//! ```

use super::types::Identity;
use futures::future::BoxFuture;
use sqlx::PgPool;
use sqlx::postgres::PgConnection;

/// Set identity context on a `PostgreSQL` connection.
///
/// Sets the following session variables using `set_config()`:
/// - `app.user_id`
/// - `app.tenant_id`
/// - `app.roles` (as JSON array)
///
/// The third parameter to `set_config()` is `true`, making the setting
/// transaction-local. This ensures identity is scoped to the current
/// transaction and cleaned up automatically.
///
/// # Errors
///
/// Returns an error if any of the `SET` commands fail.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::identity::{Identity, set_identity_context};
///
/// let identity = Identity::new("user-123", "tenant-456", vec!["admin".to_string()]);
///
/// let mut tx = pool.begin().await?;
/// set_identity_context(&mut *tx, &identity).await?;
///
/// // Now PostgreSQL functions can access:
/// // - current_setting('app.user_id', true) → 'user-123'
/// // - current_setting('app.tenant_id', true) → 'tenant-456'
/// // - current_setting('app.roles', true)::jsonb → '["admin"]'
/// ```
pub async fn set_identity_context(
    conn: &mut PgConnection,
    identity: &Identity,
) -> Result<(), sqlx::Error> {
    // Set user_id
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(&identity.user_id)
        .execute(&mut *conn)
        .await?;

    // Set tenant_id
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(&identity.tenant_id)
        .execute(&mut *conn)
        .await?;

    // Set roles as JSON array
    let roles_json = serde_json::to_string(&identity.roles).unwrap_or_else(|_| "[]".to_string());
    sqlx::query("SELECT set_config('app.roles', $1, true)")
        .bind(&roles_json)
        .execute(&mut *conn)
        .await?;

    tracing::trace!(
        user_id = %identity.user_id,
        tenant_id = %identity.tenant_id,
        roles = ?identity.roles,
        "Identity context set"
    );

    Ok(())
}

/// Execute a database operation with identity context.
///
/// This function:
/// 1. Begins a transaction
/// 2. Sets identity context (session variables)
/// 3. Executes your operation
/// 4. Commits the transaction on success
/// 5. Rolls back on error
///
/// The identity context is automatically cleaned up when the transaction ends.
///
/// # Type Parameters
///
/// - `T`: The return type of your operation
/// - `F`: The async closure that performs the database operation
///
/// # Errors
///
/// Returns an error if:
/// - The transaction cannot be started
/// - Setting identity context fails
/// - Your operation fails
/// - The transaction cannot be committed
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::identity::{Identity, execute_with_identity};
/// use sqlx::PgPool;
///
/// async fn submit_order(
///     pool: &PgPool,
///     identity: &Identity,
///     order_id: &str,
/// ) -> Result<OrderResult, sqlx::Error> {
///     execute_with_identity(pool, identity, |conn| {
///         Box::pin(async move {
///             sqlx::query_as("SELECT * FROM sales.submit_order($1)")
///                 .bind(order_id)
///                 .fetch_one(conn)
///                 .await
///         })
///     })
///     .await
/// }
/// ```
///
/// # Note on Closures
///
/// The closure receives a mutable reference to a `PgConnection` and must return
/// a `BoxFuture`. Use `Box::pin(async move { ... })` to create the future:
///
/// ```rust,ignore
/// execute_with_identity(pool, identity, |conn| {
///     Box::pin(async move {
///         // Your database operations here
///         sqlx::query("SELECT 1").execute(conn).await
///     })
/// })
/// ```
pub async fn execute_with_identity<T, F>(
    pool: &PgPool,
    identity: &Identity,
    f: F,
) -> Result<T, sqlx::Error>
where
    T: Send,
    F: for<'c> FnOnce(&'c mut PgConnection) -> BoxFuture<'c, Result<T, sqlx::Error>>,
{
    // Start transaction
    let mut tx = pool.begin().await?;

    // Set identity context
    set_identity_context(&mut tx, identity).await?;

    // Execute the operation
    let result = f(&mut tx).await?;

    // Commit transaction
    tx.commit().await?;

    Ok(result)
}

/// Execute a database operation with identity context, returning the raw result.
///
/// Similar to [`execute_with_identity`], but allows the caller to handle
/// errors from the operation separately from transaction errors.
///
/// This is useful when you want to map database errors before returning.
///
/// # Errors
///
/// Returns [`ExecuteError`] which distinguishes between:
/// - Transaction management errors (begin/commit)
/// - Context setting errors
/// - Operation errors (from your closure)
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::identity::{Identity, execute_with_identity_raw};
/// use composable_rust_pg_gateway::ApiError;
///
/// async fn get_order(
///     pool: &PgPool,
///     identity: &Identity,
///     order_id: &str,
/// ) -> Result<Order, ApiError> {
///     execute_with_identity_raw(pool, identity, |conn| {
///         Box::pin(async move {
///             sqlx::query_as("SELECT * FROM sales.get_order($1)")
///                 .bind(order_id)
///                 .fetch_optional(conn)
///                 .await
///         })
///     })
///     .await?
///     .ok_or(ApiError::NotFound)
/// }
/// ```
pub async fn execute_with_identity_raw<T, E, F>(
    pool: &PgPool,
    identity: &Identity,
    f: F,
) -> Result<T, ExecuteError<E>>
where
    T: Send,
    E: Send,
    F: for<'c> FnOnce(&'c mut PgConnection) -> BoxFuture<'c, Result<T, E>>,
{
    // Start transaction
    let mut tx = pool.begin().await.map_err(ExecuteError::Transaction)?;

    // Set identity context
    set_identity_context(&mut tx, identity)
        .await
        .map_err(ExecuteError::Context)?;

    // Execute the operation
    let result = f(&mut tx).await.map_err(ExecuteError::Operation)?;

    // Commit transaction
    tx.commit().await.map_err(ExecuteError::Transaction)?;

    Ok(result)
}

/// Errors from [`execute_with_identity_raw`].
#[derive(Debug, thiserror::Error)]
pub enum ExecuteError<E> {
    /// Transaction management error (begin/commit).
    #[error("Transaction error: {0}")]
    Transaction(sqlx::Error),

    /// Error setting identity context.
    #[error("Context error: {0}")]
    Context(sqlx::Error),

    /// Error from the user's operation.
    #[error("Operation error: {0}")]
    Operation(E),
}

impl<E> From<ExecuteError<E>> for sqlx::Error
where
    E: Into<sqlx::Error>,
{
    fn from(err: ExecuteError<E>) -> Self {
        match err {
            ExecuteError::Transaction(e) | ExecuteError::Context(e) => e,
            ExecuteError::Operation(e) => e.into(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn identity_roles_serializes_to_json() {
        let identity = Identity::new(
            "user-123",
            "tenant-456",
            vec!["admin".to_string(), "editor".to_string()],
        );

        let roles_json = serde_json::to_string(&identity.roles).unwrap();
        assert_eq!(roles_json, r#"["admin","editor"]"#);
    }

    #[test]
    fn empty_roles_serializes_to_empty_array() {
        let identity = Identity::new("user-123", "tenant-456", vec![]);

        let roles_json = serde_json::to_string(&identity.roles).unwrap();
        assert_eq!(roles_json, "[]");
    }
}
