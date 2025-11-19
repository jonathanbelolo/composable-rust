//! Resource management for infrastructure setup.
//!
//! This module centralizes all infrastructure initialization (databases, event bus,
//! authentication, payment gateway) into a single `ResourceManager` struct.
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
//! 3. Connect to event bus
//! 4. Initialize shared services (auth, payment gateway, etc.)
//!
//! # Example
//!
//! ```rust,ignore
//! let config = Config::from_env();
//! let resources = ResourceManager::from_config(&config).await?;
//!
//! // Resources are now ready to use:
//! // - resources.event_store
//! // - resources.event_bus
//! // - resources.clock
//! // ... etc
//! ```

use crate::config::Config;
use composable_rust_core::environment::SystemClock;
use composable_rust_core::event_bus::EventBus;
use composable_rust_postgres::PostgresEventStore;
use composable_rust_redpanda::RedpandaEventBus;
use composable_rust_runtime::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

/// Central resource manager for all infrastructure components.
///
/// This struct owns all the infrastructure resources needed by the application:
/// - Databases (event store, projections, auth)
/// - Event bus (Redpanda/Kafka)
/// - System services (clock, payment gateway)
/// - Circuit breakers for resilience
///
/// # Thread Safety
///
/// All resources are wrapped in `Arc` so they can be safely shared across
/// async tasks and event consumers.
#[derive(Clone)]
pub struct ResourceManager {
    /// Application configuration
    pub config: Arc<Config>,

    /// System clock for timestamps
    pub clock: Arc<SystemClock>,

    /// Event store (write side)
    pub event_store: Arc<PostgresEventStore>,

    /// Event bus for cross-aggregate coordination
    pub event_bus: Arc<dyn EventBus>,

    /// Projections database (read side)
    pub projections_pool: Arc<PgPool>,

    /// Authentication database
    pub auth_pool: Arc<PgPool>,

    /// Payment gateway (mock in development, real in production)
    pub payment_gateway: Arc<dyn crate::payment_gateway::PaymentGateway>,

    /// Circuit breaker for payment gateway
    pub payment_gateway_breaker: Arc<CircuitBreaker>,
}

impl ResourceManager {
    /// Initialize all infrastructure resources from configuration.
    ///
    /// This method:
    /// 1. Connects to PostgreSQL databases (event store, projections, auth)
    /// 2. Runs database migrations
    /// 3. Connects to Redpanda event bus
    /// 4. Initializes shared services (clock, payment gateway)
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
    /// - Event bus connection fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = Config::from_env();
    /// let resources = ResourceManager::from_config(&config).await?;
    /// ```
    pub async fn from_config(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        // Setup event store (write side) WITH MIGRATIONS
        info!("Connecting to event store database...");
        let event_store_pool = PgPool::connect(&config.postgres.url).await?;

        // Run event store migrations
        info!("Running event store migrations...");
        sqlx::migrate!("../../migrations")
            .run(&event_store_pool)
            .await?;
        info!("Event store migrations complete");

        let event_store = Arc::new(PostgresEventStore::from_pool(event_store_pool));
        info!("Event store connected");

        // Setup projections database WITH MIGRATIONS
        info!("Connecting to projections database...");
        let projections_pool = PgPool::connect(&config.projections.url).await?;

        // Run projection migrations
        info!("Running projection migrations...");
        sqlx::migrate!("./migrations_projections")
            .run(&projections_pool)
            .await?;
        info!("Projection migrations complete");

        // Setup event bus
        info!("Connecting to Redpanda event bus...");
        let event_bus: Arc<dyn EventBus> = Arc::new(
            RedpandaEventBus::builder()
                .brokers(&config.redpanda.brokers)
                .consumer_group(&config.redpanda.consumer_group)
                .build()?,
        );
        info!("Event bus connected");

        // Setup system clock
        let clock = Arc::new(SystemClock);

        // Setup auth database
        info!("Connecting to auth database...");
        let auth_database_url = std::env::var("AUTH_DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://postgres:postgres@localhost:5435/ticketing_auth".to_string()
        });
        let auth_pool = PgPool::connect(&auth_database_url).await?;
        info!("Auth database connected");

        // Initialize payment gateway and circuit breaker
        info!("Initializing payment gateway...");
        let payment_gateway = crate::payment_gateway::MockPaymentGateway::shared();
        let payment_gateway_breaker = Arc::new(CircuitBreaker::new(
            CircuitBreakerConfig::builder()
                .failure_threshold(5) // Open after 5 failures
                .timeout(Duration::from_secs(30)) // Try again after 30s
                .success_threshold(2) // Close after 2 successes in half-open
                .build(),
        ));
        info!("Payment gateway initialized (using mock)");

        Ok(Self {
            config: Arc::new(config.clone()),
            clock,
            event_store,
            event_bus,
            projections_pool: Arc::new(projections_pool),
            auth_pool: Arc::new(auth_pool),
            payment_gateway,
            payment_gateway_breaker,
        })
    }
}
