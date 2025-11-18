//! Configuration management for the ticketing application.
//!
//! Loads configuration from environment variables with sensible defaults.

use serde::{Deserialize, Serialize};
use std::env;

/// Application configuration loaded from environment variables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// `PostgreSQL` configuration (event store - write side)
    pub postgres: PostgresConfig,
    /// `PostgreSQL` projection configuration (read side - separate DB for CQRS)
    pub projections: PostgresConfig,
    /// RedPanda/Kafka configuration
    pub redpanda: RedpandaConfig,
    /// Application server configuration
    pub server: ServerConfig,
    /// Redis configuration (for auth sessions/tokens)
    pub redis: RedisConfig,
    /// Authentication configuration
    pub auth: AuthConfig,
}

/// `PostgreSQL` configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresConfig {
    /// `PostgreSQL` connection URL
    pub url: String,
    /// Maximum number of connections in the pool
    pub max_connections: u32,
    /// Minimum number of idle connections in the pool
    pub min_connections: u32,
    /// Connection timeout in seconds
    pub connect_timeout: u64,
    /// Statement timeout in seconds
    pub statement_timeout: u64,
    /// Idle timeout in seconds (connections idle longer than this are closed)
    pub idle_timeout: u64,
    /// SSL mode: disable, prefer, require (default: prefer)
    pub ssl_mode: String,
    /// Path to SSL root certificate (for verify-ca and verify-full modes)
    pub ssl_root_cert: Option<String>,
}

/// RedPanda/Kafka configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedpandaConfig {
    /// Broker addresses (comma-separated)
    pub brokers: String,
    /// Consumer group for projections
    pub consumer_group: String,
    /// Topic for inventory events
    pub inventory_topic: String,
    /// Topic for reservation events
    pub reservation_topic: String,
    /// Topic for payment events
    pub payment_topic: String,
    /// Security protocol: plaintext, ssl, `sasl_plaintext`, `sasl_ssl`
    pub security_protocol: String,
    /// SASL mechanism: PLAIN, SCRAM-SHA-256, SCRAM-SHA-512
    pub sasl_mechanism: Option<String>,
    /// SASL username
    pub sasl_username: Option<String>,
    /// SASL password
    pub sasl_password: Option<String>,
    /// Path to SSL CA certificate
    pub ssl_ca_location: Option<String>,
    /// Session timeout in milliseconds (default: 45000)
    pub session_timeout_ms: u32,
    /// Heartbeat interval in milliseconds (default: 3000)
    pub heartbeat_interval_ms: u32,
    /// Max poll interval in milliseconds (default: 300000)
    pub max_poll_interval_ms: u32,
    /// Enable auto commit (default: true)
    pub enable_auto_commit: bool,
    /// Auto commit interval in milliseconds (default: 5000)
    pub auto_commit_interval_ms: u32,
}

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Host to bind to
    pub host: String,
    /// Port to bind to
    pub port: u16,
    /// Log level (trace, debug, info, warn, error)
    pub log_level: String,
    /// Metrics server host (for Prometheus scraping)
    pub metrics_host: String,
    /// Metrics server port
    pub metrics_port: u16,
    /// Graceful shutdown timeout in seconds
    pub shutdown_timeout: u64,
}

/// Redis configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfig {
    /// Redis connection URL
    pub url: String,
    /// Maximum number of connections in the pool
    pub max_connections: u32,
    /// Connection timeout in seconds
    pub connect_timeout: u64,
}

/// Authentication configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Base URL for magic links and OAuth callbacks
    pub base_url: String,
    /// JWT secret for signing session tokens
    pub jwt_secret: String,
    /// Session TTL in seconds (default: 7 days)
    pub session_ttl: u64,
    /// Magic link token TTL in seconds (default: 15 minutes)
    pub magic_link_ttl: u64,
    /// Maximum concurrent sessions per user (0 = unlimited)
    pub max_concurrent_sessions: usize,
    /// Rate limit: requests per window
    pub rate_limit_requests: u32,
    /// Rate limit: window duration in seconds
    pub rate_limit_window: u64,
    /// **TESTING ONLY**: Expose magic links in API responses for automated testing.
    ///
    /// # Security Warning
    ///
    /// This MUST be `false` in production! Setting this to `true` defeats the security
    /// purpose of magic links (email ownership verification) by exposing the link to
    /// any API caller. Only enable this in testing/development environments.
    ///
    /// When `true`, the `/auth/magic-link/request` endpoint will include the magic link
    /// in the response body, allowing automated tests to complete the auth flow.
    ///
    /// Default: `false`
    pub expose_magic_links_for_testing: bool,
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// # Panics
    ///
    /// Panics if required environment variables are missing or invalid.
    #[must_use]
    #[allow(clippy::too_many_lines)] // Config loading is naturally long but simple
    pub fn from_env() -> Self {
        Self {
            postgres: PostgresConfig {
                url: env::var("DATABASE_URL")
                    .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/ticketing_events".to_string()),
                max_connections: env::var("DATABASE_MAX_CONNECTIONS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(10),
                min_connections: env::var("DATABASE_MIN_CONNECTIONS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(2),
                connect_timeout: env::var("DATABASE_CONNECT_TIMEOUT")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(30),
                statement_timeout: env::var("DATABASE_STATEMENT_TIMEOUT")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(60),
                idle_timeout: env::var("DATABASE_IDLE_TIMEOUT")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(600),
                ssl_mode: env::var("DATABASE_SSL_MODE")
                    .unwrap_or_else(|_| "prefer".to_string()),
                ssl_root_cert: env::var("DATABASE_SSL_ROOT_CERT").ok(),
            },
            projections: PostgresConfig {
                url: env::var("PROJECTION_DATABASE_URL")
                    .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/ticketing_projections".to_string()),
                max_connections: env::var("PROJECTION_DATABASE_MAX_CONNECTIONS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(10),
                min_connections: env::var("PROJECTION_DATABASE_MIN_CONNECTIONS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(2),
                connect_timeout: env::var("PROJECTION_DATABASE_CONNECT_TIMEOUT")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(30),
                statement_timeout: env::var("PROJECTION_DATABASE_STATEMENT_TIMEOUT")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(60),
                idle_timeout: env::var("PROJECTION_DATABASE_IDLE_TIMEOUT")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(600),
                ssl_mode: env::var("PROJECTION_DATABASE_SSL_MODE")
                    .unwrap_or_else(|_| "prefer".to_string()),
                ssl_root_cert: env::var("PROJECTION_DATABASE_SSL_ROOT_CERT").ok(),
            },
            redpanda: RedpandaConfig {
                brokers: env::var("REDPANDA_BROKERS")
                    .unwrap_or_else(|_| "localhost:9092".to_string()),
                consumer_group: env::var("CONSUMER_GROUP")
                    .unwrap_or_else(|_| "ticketing-projections".to_string()),
                inventory_topic: env::var("INVENTORY_TOPIC")
                    .unwrap_or_else(|_| "ticketing-inventory-events".to_string()),
                reservation_topic: env::var("RESERVATION_TOPIC")
                    .unwrap_or_else(|_| "ticketing-reservation-events".to_string()),
                payment_topic: env::var("PAYMENT_TOPIC")
                    .unwrap_or_else(|_| "ticketing-payment-events".to_string()),
                security_protocol: env::var("REDPANDA_SECURITY_PROTOCOL")
                    .unwrap_or_else(|_| "plaintext".to_string()),
                sasl_mechanism: env::var("REDPANDA_SASL_MECHANISM").ok(),
                sasl_username: env::var("REDPANDA_SASL_USERNAME").ok(),
                sasl_password: env::var("REDPANDA_SASL_PASSWORD").ok(),
                ssl_ca_location: env::var("REDPANDA_SSL_CA_LOCATION").ok(),
                session_timeout_ms: env::var("REDPANDA_SESSION_TIMEOUT_MS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(45000),
                heartbeat_interval_ms: env::var("REDPANDA_HEARTBEAT_INTERVAL_MS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(3000),
                max_poll_interval_ms: env::var("REDPANDA_MAX_POLL_INTERVAL_MS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(300_000),
                enable_auto_commit: env::var("REDPANDA_ENABLE_AUTO_COMMIT")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(true),
                auto_commit_interval_ms: env::var("REDPANDA_AUTO_COMMIT_INTERVAL_MS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(5000),
            },
            server: ServerConfig {
                host: env::var("HOST")
                    .unwrap_or_else(|_| "0.0.0.0".to_string()),
                port: env::var("PORT")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(8080),
                log_level: env::var("RUST_LOG")
                    .unwrap_or_else(|_| "info".to_string()),
                metrics_host: env::var("METRICS_HOST")
                    .unwrap_or_else(|_| "0.0.0.0".to_string()),
                metrics_port: env::var("METRICS_PORT")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(9090),
                shutdown_timeout: env::var("SHUTDOWN_TIMEOUT")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(30),
            },
            redis: RedisConfig {
                url: env::var("REDIS_URL")
                    .unwrap_or_else(|_| "redis://localhost:6379".to_string()),
                max_connections: env::var("REDIS_MAX_CONNECTIONS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(10),
                connect_timeout: env::var("REDIS_CONNECT_TIMEOUT")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(30),
            },
            auth: AuthConfig {
                base_url: env::var("AUTH_BASE_URL")
                    .unwrap_or_else(|_| "http://localhost:8080".to_string()),
                jwt_secret: env::var("AUTH_JWT_SECRET")
                    .unwrap_or_else(|_| "dev-secret-change-in-production".to_string()),
                session_ttl: env::var("AUTH_SESSION_TTL")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(604_800), // 7 days
                magic_link_ttl: env::var("AUTH_MAGIC_LINK_TTL")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(900), // 15 minutes
                max_concurrent_sessions: env::var("AUTH_MAX_CONCURRENT_SESSIONS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(5),
                rate_limit_requests: env::var("AUTH_RATE_LIMIT_REQUESTS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(10),
                rate_limit_window: env::var("AUTH_RATE_LIMIT_WINDOW")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(60), // 1 minute
                expose_magic_links_for_testing: env::var("AUTH_EXPOSE_MAGIC_LINKS_FOR_TESTING")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(false), // CRITICAL: Default to false (secure by default)
            },
        }
    }

    /// Get all event topics
    #[must_use] 
    pub fn all_topics(&self) -> Vec<&str> {
        vec![
            &self.redpanda.inventory_topic,
            &self.redpanda.reservation_topic,
            &self.redpanda.payment_topic,
        ]
    }
}
