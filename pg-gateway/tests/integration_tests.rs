//! Integration tests for `pg-gateway` using testcontainers.
//!
//! These tests use a real `PostgreSQL` database to validate:
//! - Pool creation and configuration
//! - Health check endpoints
//! - Identity context propagation
//! - Error code mapping
//!
//! # Requirements
//!
//! Docker must be running to execute these tests.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)] // Test code

use composable_rust_pg_gateway::{
    DbConfig, HealthChecks, HealthResponse, HealthStatus, Identity, check_database, create_pool,
    execute_with_identity, set_identity_context,
};
use sqlx::PgPool;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;

// ═══════════════════════════════════════════════════════════════════════════
// Test Types
// ═══════════════════════════════════════════════════════════════════════════

/// Result type for identity context queries.
#[derive(Debug, sqlx::FromRow)]
struct IdentityResult {
    user_id: Option<String>,
    tenant_id: Option<String>,
    roles: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Test Setup
// ═══════════════════════════════════════════════════════════════════════════

/// Start a `PostgreSQL` container and return a configured pool.
///
/// # Panics
/// Panics if container setup fails (test environment issue).
async fn setup_postgres() -> (ContainerAsync<Postgres>, PgPool) {
    let container = Postgres::default()
        .start()
        .await
        .expect("Failed to start postgres container");

    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("Failed to get postgres port");

    let database_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    // Wait for postgres to be ready with retry logic
    let mut retries = 0;
    let max_retries = 60;
    loop {
        if let Ok(pool) = sqlx::PgPool::connect(&database_url).await {
            if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                return (container, pool);
            }
        }

        assert!(
            retries < max_retries,
            "Failed to connect after {max_retries} retries"
        );
        retries += 1;
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

/// Create test tables and functions for identity context tests.
async fn setup_identity_schema(pool: &PgPool) {
    sqlx::query(
        r"
        CREATE OR REPLACE FUNCTION test_get_identity()
        RETURNS TABLE(user_id TEXT, tenant_id TEXT, roles TEXT)
        AS $$
        BEGIN
            RETURN QUERY SELECT
                current_setting('app.user_id', true),
                current_setting('app.tenant_id', true),
                current_setting('app.roles', true);
        END;
        $$ LANGUAGE plpgsql;
        ",
    )
    .execute(pool)
    .await
    .expect("Failed to create test function");
}

/// Create test functions for error code mapping.
async fn setup_error_functions(pool: &PgPool) {
    // Function that raises P0001 (Bad Request)
    sqlx::query(
        r"
        CREATE OR REPLACE FUNCTION test_bad_request()
        RETURNS void AS $$
        BEGIN
            RAISE EXCEPTION 'Validation failed' USING ERRCODE = 'P0001';
        END;
        $$ LANGUAGE plpgsql;
        ",
    )
    .execute(pool)
    .await
    .expect("Failed to create bad_request function");

    // Function that raises P0401 (Unauthorized)
    sqlx::query(
        r"
        CREATE OR REPLACE FUNCTION test_unauthorized()
        RETURNS void AS $$
        BEGIN
            RAISE EXCEPTION 'Not authenticated' USING ERRCODE = 'P0401';
        END;
        $$ LANGUAGE plpgsql;
        ",
    )
    .execute(pool)
    .await
    .expect("Failed to create unauthorized function");

    // Function that raises P0403 (Forbidden)
    sqlx::query(
        r"
        CREATE OR REPLACE FUNCTION test_forbidden()
        RETURNS void AS $$
        BEGIN
            RAISE EXCEPTION 'Permission denied' USING ERRCODE = 'P0403';
        END;
        $$ LANGUAGE plpgsql;
        ",
    )
    .execute(pool)
    .await
    .expect("Failed to create forbidden function");

    // Function that raises P0404 (Not Found)
    sqlx::query(
        r"
        CREATE OR REPLACE FUNCTION test_not_found()
        RETURNS void AS $$
        BEGIN
            RAISE EXCEPTION 'Resource not found' USING ERRCODE = 'P0404';
        END;
        $$ LANGUAGE plpgsql;
        ",
    )
    .execute(pool)
    .await
    .expect("Failed to create not_found function");

    // Function that raises P0409 (Conflict)
    sqlx::query(
        r"
        CREATE OR REPLACE FUNCTION test_conflict()
        RETURNS void AS $$
        BEGIN
            RAISE EXCEPTION 'Already exists' USING ERRCODE = 'P0409';
        END;
        $$ LANGUAGE plpgsql;
        ",
    )
    .execute(pool)
    .await
    .expect("Failed to create conflict function");
}

// ═══════════════════════════════════════════════════════════════════════════
// Pool and Config Tests
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_create_pool_success() {
    let (container, _) = setup_postgres().await;

    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("Failed to get port");

    let config = DbConfig::with_url(format!(
        "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
    ))
    .max_connections(5)
    .connect_timeout(30);

    let pool = create_pool(&config).await.expect("Should create pool");

    // Verify pool works
    let result: (i32,) = sqlx::query_as("SELECT 1")
        .fetch_one(&pool)
        .await
        .expect("Query should work");

    assert_eq!(result.0, 1);
}

#[tokio::test]
async fn test_db_config_builder() {
    let config = DbConfig::with_url("postgres://localhost/test")
        .max_connections(20)
        .connect_timeout(60)
        .idle_timeout(600);

    assert_eq!(config.database_url, "postgres://localhost/test");
    assert_eq!(config.max_connections, 20);
    assert_eq!(config.connect_timeout_secs, 60);
    assert_eq!(config.idle_timeout_secs, 600);
}

// ═══════════════════════════════════════════════════════════════════════════
// Health Check Tests
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_check_database_healthy() {
    let (_container, pool) = setup_postgres().await;

    let healthy = check_database(&pool).await;
    assert!(healthy, "Database should be healthy");
}

#[tokio::test]
async fn test_health_response_serialization() {
    let response = HealthResponse {
        status: HealthStatus::Healthy,
        checks: HealthChecks {
            database: true,
            outbox: None,
        },
    };

    let json = serde_json::to_string(&response).expect("Should serialize");
    assert!(json.contains("\"healthy\""));
    assert!(json.contains("\"database\":true"));
    // outbox should be omitted when None
    assert!(!json.contains("outbox"));
}

#[tokio::test]
async fn test_health_response_with_outbox() {
    let response = HealthResponse {
        status: HealthStatus::Unhealthy,
        checks: HealthChecks {
            database: true,
            outbox: Some(false),
        },
    };

    let json = serde_json::to_string(&response).expect("Should serialize");
    assert!(json.contains("\"unhealthy\""));
    assert!(json.contains("\"outbox\":false"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Identity Context Tests
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_set_identity_context() {
    let (_container, pool) = setup_postgres().await;
    setup_identity_schema(&pool).await;

    let identity = Identity::new(
        "user-123",
        "tenant-456",
        vec!["admin".to_string(), "user".to_string()],
    )
    .with_session("session-789");

    // Use transaction since set_identity_context uses transaction-local settings
    let mut tx = pool.begin().await.expect("Should begin transaction");

    // Set identity context
    set_identity_context(&mut tx, &identity)
        .await
        .expect("Should set identity");

    // Verify context was set
    let result: IdentityResult = sqlx::query_as("SELECT * FROM test_get_identity()")
        .fetch_one(&mut *tx)
        .await
        .expect("Should query identity");

    assert_eq!(result.user_id.as_deref(), Some("user-123"));
    assert_eq!(result.tenant_id.as_deref(), Some("tenant-456"));
    assert!(result.roles.as_ref().is_some_and(|r| r.contains("admin")));
}

#[tokio::test]
async fn test_execute_with_identity() {
    let (_container, pool) = setup_postgres().await;
    setup_identity_schema(&pool).await;

    let identity = Identity::new("exec-user", "exec-tenant", vec!["reader".to_string()]);

    let result: IdentityResult = execute_with_identity(&pool, &identity, |conn| {
        Box::pin(async move {
            sqlx::query_as("SELECT * FROM test_get_identity()")
                .fetch_one(conn)
                .await
        })
    })
    .await
    .expect("Should execute with identity");

    assert_eq!(result.user_id.as_deref(), Some("exec-user"));
    assert_eq!(result.tenant_id.as_deref(), Some("exec-tenant"));
    assert!(result.roles.as_ref().is_some_and(|r| r.contains("reader")));
}

#[tokio::test]
async fn test_identity_builder() {
    let identity =
        Identity::new("user-1", "tenant-1", vec!["admin".to_string()]).with_session("session-1");

    assert_eq!(identity.user_id, "user-1");
    assert_eq!(identity.tenant_id, "tenant-1");
    assert_eq!(identity.roles, vec!["admin"]);
    assert_eq!(identity.session_id, Some("session-1".to_string()));
}

#[tokio::test]
async fn test_identity_has_role() {
    let identity = Identity::new(
        "user",
        "tenant",
        vec!["admin".to_string(), "user".to_string()],
    );

    assert!(identity.has_role("admin"));
    assert!(identity.has_role("user"));
    assert!(!identity.has_role("superadmin"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Error Code Mapping Tests
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_error_code_p0001_bad_request() {
    use composable_rust_pg_gateway::ApiError;

    let (_container, pool) = setup_postgres().await;
    setup_error_functions(&pool).await;

    let result: Result<(), sqlx::Error> = sqlx::query("SELECT test_bad_request()")
        .execute(&pool)
        .await
        .map(|_| ());

    let api_error: ApiError = result.unwrap_err().into();

    match api_error {
        ApiError::BadRequest(msg) => {
            assert!(msg.contains("Validation failed"));
        },
        other => panic!("Expected BadRequest, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_error_code_p0401_unauthorized() {
    use composable_rust_pg_gateway::ApiError;

    let (_container, pool) = setup_postgres().await;
    setup_error_functions(&pool).await;

    let result: Result<(), sqlx::Error> = sqlx::query("SELECT test_unauthorized()")
        .execute(&pool)
        .await
        .map(|_| ());

    let api_error: ApiError = result.unwrap_err().into();

    assert!(matches!(api_error, ApiError::Unauthorized));
}

#[tokio::test]
async fn test_error_code_p0403_forbidden() {
    use composable_rust_pg_gateway::ApiError;

    let (_container, pool) = setup_postgres().await;
    setup_error_functions(&pool).await;

    let result: Result<(), sqlx::Error> = sqlx::query("SELECT test_forbidden()")
        .execute(&pool)
        .await
        .map(|_| ());

    let api_error: ApiError = result.unwrap_err().into();

    match api_error {
        ApiError::Forbidden(msg) => {
            assert!(msg.contains("Permission denied"));
        },
        other => panic!("Expected Forbidden, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_error_code_p0404_not_found() {
    use composable_rust_pg_gateway::ApiError;

    let (_container, pool) = setup_postgres().await;
    setup_error_functions(&pool).await;

    let result: Result<(), sqlx::Error> = sqlx::query("SELECT test_not_found()")
        .execute(&pool)
        .await
        .map(|_| ());

    let api_error: ApiError = result.unwrap_err().into();

    assert!(matches!(api_error, ApiError::NotFound));
}

#[tokio::test]
async fn test_error_code_p0409_conflict() {
    use composable_rust_pg_gateway::ApiError;

    let (_container, pool) = setup_postgres().await;
    setup_error_functions(&pool).await;

    let result: Result<(), sqlx::Error> = sqlx::query("SELECT test_conflict()")
        .execute(&pool)
        .await
        .map(|_| ());

    let api_error: ApiError = result.unwrap_err().into();

    match api_error {
        ApiError::Conflict(msg) => {
            assert!(msg.contains("Already exists"));
        },
        other => panic!("Expected Conflict, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_error_code_23505_unique_violation() {
    use composable_rust_pg_gateway::ApiError;

    let (_container, pool) = setup_postgres().await;

    // Create table with unique constraint
    sqlx::query("CREATE TABLE test_unique (id INT PRIMARY KEY)")
        .execute(&pool)
        .await
        .expect("Should create table");

    // Insert first row
    sqlx::query("INSERT INTO test_unique VALUES (1)")
        .execute(&pool)
        .await
        .expect("Should insert first row");

    // Try to insert duplicate
    let result: Result<(), sqlx::Error> = sqlx::query("INSERT INTO test_unique VALUES (1)")
        .execute(&pool)
        .await
        .map(|_| ());

    let api_error: ApiError = result.unwrap_err().into();

    match api_error {
        ApiError::Conflict(msg) => {
            assert!(msg.contains("unique") || msg.contains("duplicate"));
        },
        other => panic!("Expected Conflict, got: {other:?}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Claims Tests (auth-handlers feature)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "auth-handlers")]
mod auth_tests {
    use composable_rust_pg_gateway::Claims;

    #[test]
    fn test_claims_into_identity() {
        let claims = Claims {
            sub: "user-from-claims".to_string(),
            tenant_id: "tenant-from-claims".to_string(),
            roles: vec!["role1".to_string(), "role2".to_string()],
            iat: 0,
            exp: 9_999_999_999,
            jti: "session-id".to_string(),
        };

        let identity = claims.into_identity();

        assert_eq!(identity.user_id, "user-from-claims");
        assert_eq!(identity.tenant_id, "tenant-from-claims");
        assert_eq!(identity.roles, vec!["role1", "role2"]);
        // into_identity doesn't set session_id from jti
        assert!(identity.session_id.is_none());
    }
}
