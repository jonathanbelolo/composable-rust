//! Application coordinator - main application lifecycle manager.

use super::services::{InventoryService, ReservationService, PaymentService};
use crate::config::Config;
use crate::projections::{
    AvailableSeatsProjection, CustomerHistoryProjection, SalesAnalyticsProjection,
    TicketingEvent, Projection,
};
use composable_rust_core::event_bus::EventBus;
use composable_rust_postgres::PostgresEventStore;
use composable_rust_redpanda::RedpandaEventBus;
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::RwLock;
use thiserror::Error;

/// Application errors
#[derive(Error, Debug)]
pub enum AppError {
    /// Database connection failed
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Database migration failed
    #[error("Migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    /// Event bus connection failed
    #[error("Event bus error: {0}")]
    EventBus(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),
}

/// Main ticketing application.
///
/// Coordinates all components:
/// - Event store (PostgreSQL)
/// - Event bus (RedPanda)
/// - Aggregate services
/// - Projection managers
pub struct TicketingApp {
    /// Event store
    event_store: Arc<PostgresEventStore>,
    /// Event bus
    event_bus: Arc<dyn EventBus>,
    /// Inventory service
    pub inventory: Arc<InventoryService>,
    /// Reservation service
    pub reservation: Arc<ReservationService>,
    /// Payment service
    pub payment: Arc<PaymentService>,
    /// Available seats projection
    pub available_seats: Arc<RwLock<AvailableSeatsProjection>>,
    /// Sales analytics projection
    pub sales_analytics: Arc<RwLock<SalesAnalyticsProjection>>,
    /// Customer history projection
    pub customer_history: Arc<RwLock<CustomerHistoryProjection>>,
    /// Configuration
    config: Config,
}

impl TicketingApp {
    /// Initialize the application with all components.
    ///
    /// # Errors
    ///
    /// Returns error if database or event bus connection fails.
    pub async fn new(config: Config) -> Result<Self, AppError> {
        tracing::info!("Initializing Ticketing Application...");

        // 1. Initialize PostgreSQL event store
        tracing::info!("Connecting to PostgreSQL: {}", config.postgres.url);
        let pool = PgPool::connect(&config.postgres.url).await?;

        // Run migrations
        tracing::info!("Running database migrations...");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await?;

        let event_store = Arc::new(PostgresEventStore::from_pool(pool));
        tracing::info!("✓ Event store initialized");

        // 2. Initialize RedPanda event bus
        tracing::info!("Connecting to RedPanda: {}", config.redpanda.brokers);
        let event_bus = Arc::new(
            RedpandaEventBus::builder()
                .brokers(&config.redpanda.brokers)
                .consumer_group(&config.redpanda.consumer_group)
                .build()
                .map_err(|e| AppError::EventBus(e.to_string()))?
        ) as Arc<dyn EventBus>;
        tracing::info!("✓ Event bus connected");

        // 3. Initialize aggregate services
        let inventory = Arc::new(InventoryService::new(
            event_store.clone(),
            event_bus.clone(),
            config.redpanda.inventory_topic.clone(),
        ));

        let reservation = Arc::new(ReservationService::new(
            event_store.clone(),
            event_bus.clone(),
            config.redpanda.reservation_topic.clone(),
        ));

        let payment = Arc::new(PaymentService::new(
            event_store.clone(),
            event_bus.clone(),
            config.redpanda.payment_topic.clone(),
        ));

        tracing::info!("✓ Aggregate services initialized");

        // 4. Initialize projections
        let available_seats = Arc::new(RwLock::new(AvailableSeatsProjection::new()));
        let sales_analytics = Arc::new(RwLock::new(SalesAnalyticsProjection::new()));
        let customer_history = Arc::new(RwLock::new(CustomerHistoryProjection::new()));

        tracing::info!("✓ Projections initialized");

        Ok(Self {
            event_store,
            event_bus,
            inventory,
            reservation,
            payment,
            available_seats,
            sales_analytics,
            customer_history,
            config,
        })
    }

    /// Start the application event processing.
    ///
    /// This spawns background tasks for:
    /// - Projection subscription and updates
    /// - Health monitoring
    ///
    /// # Errors
    ///
    /// Returns error if subscription fails.
    pub async fn start(&self) -> Result<(), AppError> {
        tracing::info!("Starting Ticketing Application...");

        // Subscribe to all event topics
        let topics: Vec<&str> = vec![
            &self.config.redpanda.inventory_topic,
            &self.config.redpanda.reservation_topic,
            &self.config.redpanda.payment_topic,
        ];

        tracing::info!("Subscribing to topics: {:?}", topics);

        let mut stream = self.event_bus
            .subscribe(&topics)
            .await
            .map_err(|e| AppError::EventBus(e.to_string()))?;

        tracing::info!("✓ Subscribed to event bus");

        // Clone projections for background task
        let available_seats = self.available_seats.clone();
        let sales_analytics = self.sales_analytics.clone();
        let customer_history = self.customer_history.clone();

        // Spawn projection update task
        tokio::spawn(async move {
            tracing::info!("Projection update task started");

            while let Some(result) = stream.next().await {
                match result {
                    Ok(serialized_event) => {
                        // Deserialize to TicketingEvent
                        match serde_json::from_slice::<TicketingEvent>(&serialized_event.data) {
                            Ok(event) => {
                                // Update all projections
                                if let Err(e) = available_seats.write().await.handle_event(&event) {
                                    tracing::error!("AvailableSeats projection error: {}", e);
                                }

                                if let Err(e) = sales_analytics.write().await.handle_event(&event) {
                                    tracing::error!("SalesAnalytics projection error: {}", e);
                                }

                                if let Err(e) = customer_history.write().await.handle_event(&event) {
                                    tracing::error!("CustomerHistory projection error: {}", e);
                                }

                                tracing::debug!(
                                    event_type = %serialized_event.event_type,
                                    "Projection updated"
                                );
                            }
                            Err(e) => {
                                tracing::error!("Failed to deserialize event: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Event stream error: {}", e);
                    }
                }
            }

            tracing::warn!("Projection update task ended");
        });

        tracing::info!("✓ Application started successfully");
        tracing::info!("  - Event store: Ready");
        tracing::info!("  - Event bus: Subscribed");
        tracing::info!("  - Projections: Updating");

        Ok(())
    }

    /// Get the application configuration
    pub fn config(&self) -> &Config {
        &self.config
    }
}
