//! Application coordinator - main application lifecycle manager.

use crate::aggregates::{
    inventory::{InventoryEnvironment, InventoryReducer},
    payment::{PaymentEnvironment, PaymentReducer},
    reservation::{ReservationEnvironment, ReservationReducer},
};
use crate::config::Config;
use crate::projections::{
    query_adapters::{PostgresInventoryQuery, PostgresPaymentQuery, PostgresReservationQuery},
    AvailableSeatsProjection, CustomerHistoryProjection, PostgresAvailableSeatsProjection,
    PostgresPaymentsProjection, PostgresReservationsProjection, Projection,
    SalesAnalyticsProjection, TicketingEvent,
};
use crate::types::{InventoryState, PaymentState, ReservationState};
use composable_rust_core::environment::SystemClock;
use composable_rust_core::event_bus::EventBus;
use composable_rust_core::stream::StreamId;
use composable_rust_postgres::PostgresEventStore;
use composable_rust_redpanda::RedpandaEventBus;
use composable_rust_runtime::Store;
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

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
/// - Event store (`PostgreSQL`)
/// - Event bus (`RedPanda`)
/// - Aggregate stores (Composable Rust Store runtime)
/// - Projection managers
pub struct TicketingApp {
    /// Event store
    #[allow(dead_code)] // Will be used for event sourcing in future
    event_store: Arc<PostgresEventStore>,
    /// Event bus
    event_bus: Arc<dyn EventBus>,
    /// Inventory store (child aggregate)
    pub inventory: Arc<
        Store<
            InventoryState,
            crate::aggregates::InventoryAction,
            InventoryEnvironment,
            InventoryReducer,
        >,
    >,
    /// Payment store (child aggregate)
    pub payment: Arc<
        Store<
            PaymentState,
            crate::aggregates::PaymentAction,
            PaymentEnvironment,
            PaymentReducer,
        >,
    >,
    /// Reservation store (saga coordinator / parent aggregate)
    pub reservation: Arc<
        Store<
            ReservationState,
            crate::aggregates::ReservationAction,
            ReservationEnvironment,
            ReservationReducer,
        >,
    >,
    /// PostgreSQL available seats projection (for state loading)
    pub postgres_available_seats: Arc<PostgresAvailableSeatsProjection>,
    /// PostgreSQL payments projection (for state loading)
    pub postgres_payments: Arc<PostgresPaymentsProjection>,
    /// Available seats projection (in-memory, for compatibility)
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
    #[allow(clippy::cognitive_complexity)] // Application initialization with multiple components
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

        // 2. Initialize PostgreSQL projections database (read side - CQRS separation)
        tracing::info!("Connecting to projections database: {}", config.projections.url);
        let projections_pool = PgPool::connect(&config.projections.url).await?;

        // Run projection migrations (using consolidated migrations)
        tracing::info!("Running projection migrations...");
        sqlx::migrate!("./migrations_projections")
            .run(&projections_pool)
            .await?;
        tracing::info!("✓ Projection database initialized");

        // 3. Initialize RedPanda event bus
        tracing::info!("Connecting to RedPanda: {}", config.redpanda.brokers);
        let event_bus = Arc::new(
            RedpandaEventBus::builder()
                .brokers(&config.redpanda.brokers)
                .consumer_group(&config.redpanda.consumer_group)
                .build()
                .map_err(|e| AppError::EventBus(e.to_string()))?
        ) as Arc<dyn EventBus>;
        tracing::info!("✓ Event bus connected");

        // 4. Initialize PostgreSQL projections for state loading (using separate read DB)
        let postgres_available_seats =
            Arc::new(PostgresAvailableSeatsProjection::new(Arc::new(projections_pool.clone())));
        let postgres_payments =
            Arc::new(PostgresPaymentsProjection::new(Arc::new(projections_pool.clone())));
        let postgres_reservations =
            Arc::new(PostgresReservationsProjection::new(Arc::new(projections_pool)));
        tracing::info!("✓ PostgreSQL projections initialized");

        // 5. Create query adapters
        let inventory_query = Arc::new(PostgresInventoryQuery::new(postgres_available_seats.clone()));
        let payment_query = Arc::new(PostgresPaymentQuery::new(postgres_payments.clone()));
        let reservation_query = Arc::new(PostgresReservationQuery::new(postgres_reservations));
        tracing::info!("✓ Query adapters created");

        // 5. Initialize aggregate stores (Composable Rust architecture)
        // Create child stores first (Inventory and Payment)
        let clock = Arc::new(SystemClock);

        // Inventory Store (child)
        let inventory_env = InventoryEnvironment::new(
            clock.clone(),
            event_store.clone(),
            event_bus.clone(),
            StreamId::new("inventory"),
            inventory_query,
        );
        let inventory = Arc::new(Store::new(
            InventoryState::new(),
            InventoryReducer::new(),
            inventory_env,
        ));

        // Payment Store (child)
        let payment_env = PaymentEnvironment::new(
            clock.clone(),
            event_store.clone(),
            event_bus.clone(),
            StreamId::new("payment"),
            payment_query,
        );
        let payment = Arc::new(Store::new(
            PaymentState::new(),
            PaymentReducer::new(),
            payment_env,
        ));

        // Reservation Store (parent / saga coordinator)
        // Note: ReservationState holds child STATE, not child stores (TCA pattern)
        let reservation_env = ReservationEnvironment::new(
            clock.clone(),
            event_store.clone(),
            event_bus.clone(),
            StreamId::new("reservation"),
            reservation_query,
        );
        let reservation = Arc::new(Store::new(
            ReservationState::new(),
            ReservationReducer::new(),
            reservation_env,
        ));

        tracing::info!("✓ Aggregate stores initialized (Composable Rust architecture)");

        // 6. Initialize in-memory projections (for compatibility)
        let available_seats = Arc::new(RwLock::new(AvailableSeatsProjection::new()));
        let sales_analytics = Arc::new(RwLock::new(SalesAnalyticsProjection::new()));
        let customer_history = Arc::new(RwLock::new(CustomerHistoryProjection::new()));

        tracing::info!("✓ In-memory projections initialized");

        Ok(Self {
            event_store,
            event_bus,
            inventory,
            payment,
            reservation,
            postgres_available_seats,
            postgres_payments,
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
    #[allow(clippy::cognitive_complexity)] // Event processing loop with multiple event types
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
    #[must_use] 
    pub const fn config(&self) -> &Config {
        &self.config
    }
}
