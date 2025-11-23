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

use crate::aggregates::{EventAction, InventoryAction, PaymentAction, ReservationAction};
use crate::config::Config;
use composable_rust_core::environment::SystemClock;
use composable_rust_core::event_bus::EventBus;
use composable_rust_postgres::PostgresEventStore;
use composable_rust_redpanda::RedpandaEventBus;
use composable_rust_runtime::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
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

    /// Global action channels for cross-aggregate coordination
    /// Channel capacity: 1000 (sufficient for high-throughput monolith)

    /// Event aggregate action channel
    pub event_actions: broadcast::Sender<EventAction>,

    /// Inventory aggregate action channel
    pub inventory_actions: broadcast::Sender<InventoryAction>,

    /// Reservation aggregate action channel
    pub reservation_actions: broadcast::Sender<ReservationAction>,

    /// Payment aggregate action channel
    pub payment_actions: broadcast::Sender<PaymentAction>,
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

        // Create Redpanda topics if they don't exist
        // Redpanda auto-creates topics on first publish, but we need topics to exist
        // before subscription (which happens later). So we publish a dummy event
        // to each topic to trigger auto-creation.
        info!("Ensuring Redpanda topics exist...");
        let projection_completed_topic = "projection.completed".to_string();
        let topics_to_create = vec![
            (&config.redpanda.inventory_topic, "inventory"),
            (&config.redpanda.reservation_topic, "reservation"),
            (&config.redpanda.payment_topic, "payment"),
            (&projection_completed_topic, "projection-completion"),
        ];

        for (topic, name) in &topics_to_create {
            tracing::debug!("Ensuring topic exists: {}", topic);
            // Publish a bootstrap event to trigger topic creation
            let bootstrap_event = composable_rust_core::event::SerializedEvent::new(
                format!("{}Bootstrap", name),
                b"{}".to_vec(), // Empty JSON object
                None,
            );

            if let Err(e) = event_bus.publish(topic, &bootstrap_event).await {
                tracing::warn!("Failed to create topic {}: {} (may already exist)", topic, e);
            } else {
                tracing::debug!("✓ Topic {} created or verified", topic);
            }
        }
        info!("Redpanda topics initialized");

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

        // Initialize global action channels (capacity: 1000)
        info!("Initializing global action channels...");
        let (event_actions, _) = broadcast::channel(1000);
        let (inventory_actions, _) = broadcast::channel(1000);
        let (reservation_actions, _) = broadcast::channel(1000);
        let (payment_actions, _) = broadcast::channel(1000);
        info!("Global action channels initialized");

        Ok(Self {
            config: Arc::new(config.clone()),
            clock,
            event_store,
            event_bus,
            projections_pool: Arc::new(projections_pool),
            auth_pool: Arc::new(auth_pool),
            payment_gateway,
            payment_gateway_breaker,
            event_actions,
            inventory_actions,
            reservation_actions,
            payment_actions,
        })
    }
}
