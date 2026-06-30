//! Resource management for infrastructure setup.
//!
//! This module centralizes all infrastructure initialization (databases, event bus,
//! authentication) into a single `ResourceManager` struct.
//!
//! # Design Philosophy
//!
//! The `ResourceManager` is a **framework-level abstraction** that handles the
//! boilerplate of setting up infrastructure resources. Different applications
//! can have different resource configurations (e.g., MySQL vs PostgreSQL,
//! Kafka vs Redpanda), but they all follow the same pattern:
//!
//! 1. Load configuration
//! 2. Connect to databases (with migrations)
//! 3. Initialize shared services
//!
//! # Example
//!
//! ```rust,ignore
//! let config = Config::from_env();
//! let resources = ResourceManager::from_config(&config).await?;
//!
//! // Resources are now ready to use:
//! // - resources.event_store
//! // - resources.projections_pool
//! // - resources.auth_pool
//! // - resources.clock
//! ```

use crate::config::Config;
use composable_rust_core::environment::SystemClock;
use composable_rust_postgres::PostgresEventStore;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::info;

/// Central resource manager for all infrastructure components.
///
/// This struct owns all the infrastructure resources needed by the application:
/// - Databases (event store, projections, auth)
/// - System services (clock)
///
/// # Thread Safety
///
/// All resources are wrapped in `Arc` so they can be safely shared across
/// async tasks.
#[derive(Clone)]
pub struct ResourceManager {
    /// Application configuration
    pub config: Arc<Config>,

    /// System clock for timestamps
    pub clock: Arc<SystemClock>,

    /// Event store (write side)
    pub event_store: Arc<PostgresEventStore>,

    /// Projections database (read side)
    pub projections_pool: Arc<PgPool>,

    /// Authentication database
    pub auth_pool: Arc<PgPool>,
}

impl ResourceManager {
    /// Initialize all infrastructure resources from configuration.
    ///
    /// This method:
    /// 1. Connects to PostgreSQL databases (event store, projections, auth)
    /// 2. Runs database migrations
    /// 3. Initializes shared services (clock)
    ///
    /// # Arguments
    ///
    /// * `config` - Application configuration
    ///
    /// # Returns
    ///
    /// A `ResourceManager` with all infrastructure ready to use.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database connection fails
    /// - Database migrations fail
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = Config::from_env();
    /// let resources = ResourceManager::from_config(&config).await?;
    /// ```
    pub async fn from_config(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        // Single PostgreSQL database for the event log, all projections, and the
        // durable saga_state (so a saga's events and state commit in one transaction).
        info!("Connecting to database...");
        let pool = PgPool::connect(&config.postgres.url).await?;

        // Run the merged migrations: event log + projections + saga_state + idempotency.
        info!("Running database migrations...");
        sqlx::migrate!("./migrations").run(&pool).await?;
        info!("Database migrations complete");

        let event_store = Arc::new(PostgresEventStore::from_pool(pool.clone()));
        // Projections share the same pool/database as the event log.
        let projections_pool = pool;
        info!("Database connected");

        // Setup system clock
        let clock = Arc::new(SystemClock);

        // Setup auth database WITH MIGRATIONS
        info!("Connecting to auth database...");
        let auth_database_url = std::env::var("AUTH_DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://postgres:postgres@localhost:5435/ticketing_auth".to_string()
        });
        let auth_pool = PgPool::connect(&auth_database_url).await?;

        // Run auth migrations
        info!("Running auth migrations...");
        sqlx::migrate!("./migrations_auth")
            .run(&auth_pool)
            .await?;
        info!("Auth migrations complete");

        // Setup analytics database WITH MIGRATIONS
        info!("Connecting to analytics database...");
        let analytics_database_url = std::env::var("ANALYTICS_DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://postgres:postgres@localhost:5434/ticketing_analytics".to_string()
        });
        let analytics_pool = PgPool::connect(&analytics_database_url).await?;

        // Run analytics migrations
        info!("Running analytics migrations...");
        sqlx::migrate!("./migrations_analytics")
            .run(&analytics_pool)
            .await?;
        info!("Analytics migrations complete");

        // Analytics pool is currently not stored - will be used in future
        drop(analytics_pool);

        Ok(Self {
            config: Arc::new(config.clone()),
            clock,
            event_store,
            projections_pool: Arc::new(projections_pool),
            auth_pool: Arc::new(auth_pool),
        })
    }
}
