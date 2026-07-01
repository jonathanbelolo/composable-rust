//! Database connection pool management.
//!
//! This module provides configuration and creation of `PostgreSQL` connection pools
//! optimized for the thin server pattern.
//!
//! # Example
//!
//! ```rust,ignore
//! use composable_rust_pg_gateway::{DbConfig, create_pool};
//!
//! let config = DbConfig::from_env()?;
//! let pool = create_pool(&config).await?;
//!
//! // Use pool in handlers
//! let result = sqlx::query("SELECT 1")
//!     .fetch_one(&pool)
//!     .await?;
//! ```

mod config;
#[cfg(feature = "http")]
mod health;

pub use config::DbConfig;

#[cfg(feature = "http")]
pub use health::{
    HealthChecks, HealthConfig, HealthResponse, HealthStatus, check_database, check_outbox,
    health_check, health_check_with_outbox,
};

use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

/// Create a `PostgreSQL` connection pool from configuration.
///
/// This function creates a pool with settings optimized for web applications:
/// - Connection timeout
/// - Idle timeout
/// - Max connections based on config
///
/// # Errors
///
/// Returns an error if the database connection cannot be established.
///
/// # Example
///
/// ```rust,ignore
/// let config = DbConfig::from_env()?;
/// let pool = create_pool(&config).await?;
/// ```
pub async fn create_pool(config: &DbConfig) -> Result<PgPool, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(Duration::from_secs(config.connect_timeout_secs))
        .idle_timeout(Some(Duration::from_secs(config.idle_timeout_secs)))
        .connect(&config.database_url)
        .await?;

    tracing::info!(
        max_connections = config.max_connections,
        "Database pool created"
    );

    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::DbConfig;

    #[test]
    fn config_default_values() {
        let config = DbConfig {
            database_url: "postgres://localhost/test".to_string(),
            ..Default::default()
        };

        assert_eq!(config.max_connections, 10);
        assert_eq!(config.connect_timeout_secs, 30);
        assert_eq!(config.idle_timeout_secs, 600);
    }
}
