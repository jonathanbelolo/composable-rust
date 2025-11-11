//! Configuration Management for Agent Systems
//!
//! Provides environment-based configuration with validation and secrets management.
//!
//! # Features
//!
//! - Environment-based configs (dev, staging, production)
//! - Config validation with clear error messages
//! - Secrets management via environment variables
//! - Sensible defaults for all environments
//! - Type-safe configuration with serde
//!
//! # Example
//!
//! ```no_run
//! use composable_rust_agent_patterns::config::{AgentConfig, Environment};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Load from environment variable CONFIG_ENV (defaults to dev)
//! let config = AgentConfig::from_env()?;
//!
//! // Or load explicitly
//! let config = AgentConfig::load(Environment::Production)?;
//!
//! println!("LLM Model: {}", config.llm.model);
//! println!("Max tokens: {}", config.llm.max_tokens);
//! # Ok(())
//! # }
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// Configuration error
#[derive(Debug)]
pub enum ConfigError {
    /// Environment variable not set
    EnvVarNotSet(String),
    /// Invalid environment value
    InvalidEnvironment(String),
    /// Configuration validation failed
    ValidationError(String),
    /// Failed to parse configuration
    ParseError(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EnvVarNotSet(var) => write!(f, "Environment variable not set: {var}"),
            Self::InvalidEnvironment(env) => write!(f, "Invalid environment: {env}"),
            Self::ValidationError(msg) => write!(f, "Configuration validation failed: {msg}"),
            Self::ParseError(msg) => write!(f, "Failed to parse configuration: {msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Deployment environment
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    /// Development environment (local)
    Development,
    /// Staging environment (pre-production)
    Staging,
    /// Production environment
    Production,
}

impl Environment {
    /// Get environment from string
    ///
    /// # Errors
    ///
    /// Returns error if environment string is invalid
    pub fn from_str(s: &str) -> Result<Self, ConfigError> {
        match s.to_lowercase().as_str() {
            "dev" | "development" => Ok(Self::Development),
            "staging" | "stage" => Ok(Self::Staging),
            "prod" | "production" => Ok(Self::Production),
            _ => Err(ConfigError::InvalidEnvironment(s.to_string())),
        }
    }

    /// Check if this is production environment
    #[must_use]
    pub const fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }

    /// Check if this is development environment
    #[must_use]
    pub const fn is_development(self) -> bool {
        matches!(self, Self::Development)
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Development => write!(f, "development"),
            Self::Staging => write!(f, "staging"),
            Self::Production => write!(f, "production"),
        }
    }
}

/// LLM configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// LLM model to use
    pub model: String,
    /// Maximum tokens per request
    pub max_tokens: u32,
    /// Temperature for generation (0.0-1.0)
    pub temperature: f32,
    /// API timeout in seconds
    pub timeout_secs: u64,
    /// Maximum retries for transient failures
    pub max_retries: u32,
}

impl LlmConfig {
    /// Validate LLM configuration
    ///
    /// # Errors
    ///
    /// Returns error if configuration is invalid
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.model.is_empty() {
            return Err(ConfigError::ValidationError("model cannot be empty".to_string()));
        }
        if self.max_tokens == 0 {
            return Err(ConfigError::ValidationError("max_tokens must be > 0".to_string()));
        }
        if !(0.0..=1.0).contains(&self.temperature) {
            return Err(ConfigError::ValidationError(
                "temperature must be between 0.0 and 1.0".to_string(),
            ));
        }
        if self.timeout_secs == 0 {
            return Err(ConfigError::ValidationError("timeout_secs must be > 0".to_string()));
        }
        Ok(())
    }

    /// Get timeout as Duration
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-5-20250929".to_string(),
            max_tokens: 4096,
            temperature: 0.7,
            timeout_secs: 30,
            max_retries: 3,
        }
    }
}

/// Observability configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    /// Enable tracing
    pub tracing_enabled: bool,
    /// Jaeger endpoint (e.g., "localhost:6831")
    pub jaeger_endpoint: Option<String>,
    /// Enable metrics
    pub metrics_enabled: bool,
    /// Metrics port
    pub metrics_port: u16,
    /// Log level (trace, debug, info, warn, error)
    pub log_level: String,
}

impl ObservabilityConfig {
    /// Validate observability configuration
    ///
    /// # Errors
    ///
    /// Returns error if configuration is invalid
    pub fn validate(&self) -> Result<(), ConfigError> {
        let valid_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_levels.contains(&self.log_level.to_lowercase().as_str()) {
            return Err(ConfigError::ValidationError(format!(
                "invalid log_level: {}. Must be one of: {}",
                self.log_level,
                valid_levels.join(", ")
            )));
        }
        Ok(())
    }
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            tracing_enabled: true,
            jaeger_endpoint: None,
            metrics_enabled: true,
            metrics_port: 9090,
            log_level: "info".to_string(),
        }
    }
}

/// Resilience configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResilienceConfig {
    /// Circuit breaker failure threshold
    pub circuit_breaker_threshold: u32,
    /// Circuit breaker timeout in seconds
    pub circuit_breaker_timeout_secs: u64,
    /// Rate limiter requests per second
    pub rate_limit_rps: u32,
    /// Bulkhead max concurrent requests
    pub bulkhead_max_concurrent: usize,
}

impl ResilienceConfig {
    /// Validate resilience configuration
    ///
    /// # Errors
    ///
    /// Returns error if configuration is invalid
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.circuit_breaker_threshold == 0 {
            return Err(ConfigError::ValidationError(
                "circuit_breaker_threshold must be > 0".to_string(),
            ));
        }
        if self.circuit_breaker_timeout_secs == 0 {
            return Err(ConfigError::ValidationError(
                "circuit_breaker_timeout_secs must be > 0".to_string(),
            ));
        }
        if self.rate_limit_rps == 0 {
            return Err(ConfigError::ValidationError("rate_limit_rps must be > 0".to_string()));
        }
        if self.bulkhead_max_concurrent == 0 {
            return Err(ConfigError::ValidationError(
                "bulkhead_max_concurrent must be > 0".to_string(),
            ));
        }
        Ok(())
    }

    /// Get circuit breaker timeout as Duration
    #[must_use]
    pub const fn circuit_breaker_timeout(&self) -> Duration {
        Duration::from_secs(self.circuit_breaker_timeout_secs)
    }
}

impl Default for ResilienceConfig {
    fn default() -> Self {
        Self {
            circuit_breaker_threshold: 5,
            circuit_breaker_timeout_secs: 60,
            rate_limit_rps: 10,
            bulkhead_max_concurrent: 100,
        }
    }
}

/// Database configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Database URL (from environment variable for security)
    #[serde(skip)]
    pub url: Option<String>,
    /// Maximum connections in pool
    pub max_connections: u32,
    /// Connection timeout in seconds
    pub connect_timeout_secs: u64,
    /// Idle timeout in seconds
    pub idle_timeout_secs: u64,
}

impl DatabaseConfig {
    /// Validate database configuration
    ///
    /// # Errors
    ///
    /// Returns error if configuration is invalid
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.max_connections == 0 {
            return Err(ConfigError::ValidationError(
                "max_connections must be > 0".to_string(),
            ));
        }
        if self.connect_timeout_secs == 0 {
            return Err(ConfigError::ValidationError(
                "connect_timeout_secs must be > 0".to_string(),
            ));
        }
        Ok(())
    }

    /// Get connect timeout as Duration
    #[must_use]
    pub const fn connect_timeout(&self) -> Duration {
        Duration::from_secs(self.connect_timeout_secs)
    }

    /// Get idle timeout as Duration
    #[must_use]
    pub const fn idle_timeout(&self) -> Duration {
        Duration::from_secs(self.idle_timeout_secs)
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: None,
            max_connections: 10,
            connect_timeout_secs: 10,
            idle_timeout_secs: 600,
        }
    }
}

/// Agent system configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Deployment environment
    pub environment: Environment,
    /// LLM configuration
    pub llm: LlmConfig,
    /// Observability configuration
    pub observability: ObservabilityConfig,
    /// Resilience configuration
    pub resilience: ResilienceConfig,
    /// Database configuration
    pub database: DatabaseConfig,
}

impl AgentConfig {
    /// Load configuration from environment
    ///
    /// Reads `CONFIG_ENV` environment variable (defaults to "development")
    ///
    /// # Errors
    ///
    /// Returns error if configuration cannot be loaded or is invalid
    pub fn from_env() -> Result<Self, ConfigError> {
        let env_str = std::env::var("CONFIG_ENV").unwrap_or_else(|_| "development".to_string());
        let environment = Environment::from_str(&env_str)?;
        Self::load(environment)
    }

    /// Load configuration for specific environment
    ///
    /// # Errors
    ///
    /// Returns error if configuration is invalid
    pub fn load(environment: Environment) -> Result<Self, ConfigError> {
        let mut config = Self {
            environment,
            llm: LlmConfig::default(),
            observability: ObservabilityConfig::default(),
            resilience: ResilienceConfig::default(),
            database: DatabaseConfig::default(),
        };

        // Environment-specific overrides
        match environment {
            Environment::Development => {
                config.observability.log_level = "debug".to_string();
                config.resilience.rate_limit_rps = 100; // More permissive in dev
            }
            Environment::Staging => {
                config.observability.log_level = "info".to_string();
                config.resilience.rate_limit_rps = 50;
            }
            Environment::Production => {
                config.observability.log_level = "warn".to_string();
                config.resilience.rate_limit_rps = 10; // Conservative in prod
                config.resilience.circuit_breaker_threshold = 3; // Stricter in prod
            }
        }

        // Load secrets from environment variables
        config.load_secrets()?;

        // Validate
        config.validate()?;

        Ok(config)
    }

    /// Load secrets from environment variables
    ///
    /// # Errors
    ///
    /// Returns error if required secrets are missing
    fn load_secrets(&mut self) -> Result<(), ConfigError> {
        // Database URL (required in staging/prod)
        if let Ok(url) = std::env::var("DATABASE_URL") {
            self.database.url = Some(url);
        } else if !self.environment.is_development() {
            return Err(ConfigError::EnvVarNotSet("DATABASE_URL".to_string()));
        }

        // Jaeger endpoint (optional)
        if let Ok(endpoint) = std::env::var("JAEGER_ENDPOINT") {
            self.observability.jaeger_endpoint = Some(endpoint);
        }

        Ok(())
    }

    /// Validate entire configuration
    ///
    /// # Errors
    ///
    /// Returns error if any configuration section is invalid
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.llm.validate()?;
        self.observability.validate()?;
        self.resilience.validate()?;
        self.database.validate()?;
        Ok(())
    }

    /// Check if running in production
    #[must_use]
    pub const fn is_production(&self) -> bool {
        self.environment.is_production()
    }

    /// Check if running in development
    #[must_use]
    pub const fn is_development(&self) -> bool {
        self.environment.is_development()
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            environment: Environment::Development,
            llm: LlmConfig::default(),
            observability: ObservabilityConfig::default(),
            resilience: ResilienceConfig::default(),
            database: DatabaseConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_environment_from_str() {
        assert!(matches!(
            Environment::from_str("dev").unwrap(),
            Environment::Development
        ));
        assert!(matches!(
            Environment::from_str("development").unwrap(),
            Environment::Development
        ));
        assert!(matches!(
            Environment::from_str("staging").unwrap(),
            Environment::Staging
        ));
        assert!(matches!(
            Environment::from_str("prod").unwrap(),
            Environment::Production
        ));
        assert!(Environment::from_str("invalid").is_err());
    }

    #[test]
    fn test_environment_display() {
        assert_eq!(Environment::Development.to_string(), "development");
        assert_eq!(Environment::Staging.to_string(), "staging");
        assert_eq!(Environment::Production.to_string(), "production");
    }

    #[test]
    fn test_llm_config_validation() {
        let mut config = LlmConfig::default();
        assert!(config.validate().is_ok());

        config.model = String::new();
        assert!(config.validate().is_err());

        config.model = "claude".to_string();
        config.temperature = 1.5;
        assert!(config.validate().is_err());

        config.temperature = 0.7;
        config.max_tokens = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_observability_config_validation() {
        let mut config = ObservabilityConfig::default();
        assert!(config.validate().is_ok());

        config.log_level = "invalid".to_string();
        assert!(config.validate().is_err());

        config.log_level = "debug".to_string();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_resilience_config_validation() {
        let mut config = ResilienceConfig::default();
        assert!(config.validate().is_ok());

        config.circuit_breaker_threshold = 0;
        assert!(config.validate().is_err());

        config.circuit_breaker_threshold = 5;
        config.rate_limit_rps = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_database_config_validation() {
        let mut config = DatabaseConfig::default();
        assert!(config.validate().is_ok());

        config.max_connections = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_agent_config_defaults() {
        let config = AgentConfig::default();
        assert!(matches!(config.environment, Environment::Development));
        assert_eq!(config.llm.model, "claude-sonnet-4-5-20250929");
        assert!(config.observability.tracing_enabled);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_agent_config_load_development() {
        let config = AgentConfig::load(Environment::Development).unwrap();
        assert_eq!(config.observability.log_level, "debug");
        assert_eq!(config.resilience.rate_limit_rps, 100);
    }

    #[test]
    fn test_agent_config_load_production() {
        let config = AgentConfig::load(Environment::Production);
        // May fail if DATABASE_URL not set, which is expected
        if let Ok(config) = config {
            assert_eq!(config.observability.log_level, "warn");
            assert_eq!(config.resilience.rate_limit_rps, 10);
            assert_eq!(config.resilience.circuit_breaker_threshold, 3);
        }
    }

    #[test]
    fn test_duration_conversions() {
        let llm_config = LlmConfig::default();
        assert_eq!(llm_config.timeout(), Duration::from_secs(30));

        let resilience_config = ResilienceConfig::default();
        assert_eq!(
            resilience_config.circuit_breaker_timeout(),
            Duration::from_secs(60)
        );

        let db_config = DatabaseConfig::default();
        assert_eq!(db_config.connect_timeout(), Duration::from_secs(10));
        assert_eq!(db_config.idle_timeout(), Duration::from_secs(600));
    }

    #[test]
    fn test_config_error_display() {
        let err = ConfigError::EnvVarNotSet("TEST_VAR".to_string());
        assert_eq!(err.to_string(), "Environment variable not set: TEST_VAR");

        let err = ConfigError::ValidationError("test failed".to_string());
        assert_eq!(err.to_string(), "Configuration validation failed: test failed");
    }
}
