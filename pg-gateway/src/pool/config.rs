//! Database configuration types.

use serde::Deserialize;

/// Database connection configuration.
///
/// This struct contains all settings needed to configure a `PostgreSQL`
/// connection pool for the thin server pattern.
///
/// # Environment Variables
///
/// When using [`DbConfig::from_env`], the following environment variables are read:
///
/// | Variable | Default | Description |
/// |----------|---------|-------------|
/// | `DATABASE_URL` | (required) | `PostgreSQL` connection string |
/// | `DB_MAX_CONNECTIONS` | 10 | Maximum pool connections |
/// | `DB_CONNECT_TIMEOUT` | 30 | Connection timeout in seconds |
/// | `DB_IDLE_TIMEOUT` | 600 | Idle connection timeout in seconds |
///
/// # Example
///
/// ```rust,ignore
/// // From environment variables
/// let config = DbConfig::from_env()?;
///
/// // Or construct directly
/// let config = DbConfig {
///     database_url: "postgres://user:pass@localhost/db".to_string(),
///     max_connections: 20,
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct DbConfig {
    /// `PostgreSQL` connection URL.
    ///
    /// Format: `postgres://user:password@host:port/database`
    pub database_url: String,

    /// Maximum number of connections in the pool.
    ///
    /// Default: 10
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    /// Connection acquisition timeout in seconds.
    ///
    /// Default: 30
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,

    /// Idle connection timeout in seconds.
    ///
    /// Connections idle longer than this are closed.
    ///
    /// Default: 600 (10 minutes)
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,
}

const fn default_max_connections() -> u32 {
    10
}

const fn default_connect_timeout() -> u64 {
    30
}

const fn default_idle_timeout() -> u64 {
    600
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            database_url: String::new(),
            max_connections: default_max_connections(),
            connect_timeout_secs: default_connect_timeout(),
            idle_timeout_secs: default_idle_timeout(),
        }
    }
}

impl DbConfig {
    /// Create configuration from environment variables.
    ///
    /// # Environment Variables
    ///
    /// - `DATABASE_URL` (required): `PostgreSQL` connection string
    /// - `DB_MAX_CONNECTIONS` (optional): Maximum pool connections (default: 10)
    /// - `DB_CONNECT_TIMEOUT` (optional): Connection timeout in seconds (default: 30)
    /// - `DB_IDLE_TIMEOUT` (optional): Idle timeout in seconds (default: 600)
    ///
    /// # Errors
    ///
    /// Returns an error if `DATABASE_URL` is not set.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// std::env::set_var("DATABASE_URL", "postgres://localhost/mydb");
    /// let config = DbConfig::from_env()?;
    /// ```
    pub fn from_env() -> Result<Self, DbConfigError> {
        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| DbConfigError::MissingDatabaseUrl)?;

        let max_connections = std::env::var("DB_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(default_max_connections);

        let connect_timeout_secs = std::env::var("DB_CONNECT_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(default_connect_timeout);

        let idle_timeout_secs = std::env::var("DB_IDLE_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(default_idle_timeout);

        Ok(Self {
            database_url,
            max_connections,
            connect_timeout_secs,
            idle_timeout_secs,
        })
    }

    /// Create configuration with a specific database URL.
    ///
    /// Uses default values for other settings.
    #[must_use]
    pub fn with_url(database_url: impl Into<String>) -> Self {
        Self {
            database_url: database_url.into(),
            ..Default::default()
        }
    }

    /// Set the maximum number of connections.
    #[must_use]
    pub const fn max_connections(mut self, max: u32) -> Self {
        self.max_connections = max;
        self
    }

    /// Set the connection timeout in seconds.
    #[must_use]
    pub const fn connect_timeout(mut self, secs: u64) -> Self {
        self.connect_timeout_secs = secs;
        self
    }

    /// Set the idle connection timeout in seconds.
    #[must_use]
    pub const fn idle_timeout(mut self, secs: u64) -> Self {
        self.idle_timeout_secs = secs;
        self
    }
}

/// Errors that can occur when loading database configuration.
#[derive(Debug, thiserror::Error)]
pub enum DbConfigError {
    /// `DATABASE_URL` environment variable is not set.
    #[error("DATABASE_URL environment variable is not set")]
    MissingDatabaseUrl,
}

#[cfg(test)]
mod tests {
    use super::DbConfig;

    #[test]
    fn default_config() {
        let config = DbConfig::default();
        assert_eq!(config.max_connections, 10);
        assert_eq!(config.connect_timeout_secs, 30);
        assert_eq!(config.idle_timeout_secs, 600);
        assert!(config.database_url.is_empty());
    }

    #[test]
    fn with_url() {
        let config = DbConfig::with_url("postgres://localhost/test");
        assert_eq!(config.database_url, "postgres://localhost/test");
        assert_eq!(config.max_connections, 10);
    }

    #[test]
    fn builder_pattern() {
        let config = DbConfig::with_url("postgres://localhost/test")
            .max_connections(20)
            .connect_timeout(60)
            .idle_timeout(300);

        assert_eq!(config.database_url, "postgres://localhost/test");
        assert_eq!(config.max_connections, 20);
        assert_eq!(config.connect_timeout_secs, 60);
        assert_eq!(config.idle_timeout_secs, 300);
    }
}
