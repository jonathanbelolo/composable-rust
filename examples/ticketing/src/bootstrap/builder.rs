//! Declarative application builder API.
//!
//! Provides a fluent builder interface for constructing and running the ticketing application.
//! This API is designed to be **framework-level reusable** across different applications.
//!
//! # Design Philosophy
//!
//! The builder follows a **declarative, step-by-step** initialization pattern:
//! 1. Configure (config, tracing)
//! 2. Initialize infrastructure (databases, event bus, auth)
//! 3. Register business logic (aggregates, projections)
//! 4. Build HTTP server (routes, state)
//! 5. Run application (start consumers, server, manage lifecycle)
//!
//! Each step returns `Result` for explicit error handling, making the initialization
//! flow clear and debuggable.
//!
//! # Example
//!
//! ```rust,ignore
//! ApplicationBuilder::new()
//!     .with_config(Config::from_env()?)
//!     .with_tracing()?
//!     .with_resources().await?
//!     .with_aggregates()
//!     .with_projections().await?
//!     .with_auth().await?
//!     .build().await?
//!     .run().await?;
//! ```
//!
//! # Framework-Level Reusability
//!
//! This builder is designed to work with different applications:
//! - **Configuration**: Application-specific Config type
//! - **Resources**: Different database URLs, event bus topics
//! - **Aggregates**: Different aggregate types (Order, Payment, etc.)
//! - **Projections**: Different projection schemas
//! - **Routes**: Application-specific HTTP endpoints
//!
//! The builder handles the **plumbing** (initialization, lifecycle, shutdown)
//! while allowing applications to customize the **business logic** (what aggregates,
//! what projections, what routes).

use crate::auth::setup::{build_auth_store, TicketingAuthStore};
use crate::bootstrap::{register_aggregate_consumers, register_projections, ProjectionSystem, ResourceManager};
use crate::config::Config;
use crate::next::{
    AnalyticsBusinessLogic, EventBusinessLogic, EventInventorySagaLogic, InventoryBusinessLogic,
    PaymentBusinessLogic, ReservationQueryLogic, ReservationSagaLogic,
    call_executor::{
        EventHandler as EventHandlerTrait,
        EventInventorySagaCallExecutor,
        InventoryHandler as InventoryHandlerTrait,
        PaymentHandler as PaymentHandlerTrait,
        ReservationSagaCallExecutor,
    },
    http::{
        AnalyticsAppState, EventCreationAppState, FullQueryAppState, NextAppState, QueryAppState,
        ReservationQueryAppState, ReservationAppState,
    },
    projection_queries::{
        AnalyticsProjectionQueries, AnalyticsQueryFetcher, EventProjectionQueries,
        EventQueryFetcher, InventoryProjectionQueries, InventoryQueryFetcher,
        PaymentProjectionQueries, PaymentQueryFetcher, ReservationProjectionQueries,
        ReservationQueryFetcher, ReservationSagaProjectionQueries, ReservationSagaQueryFetcher,
        SagaProjectionQueries, SagaQueryFetcher,
    },
    projector::{EventInventorySagaProjector, EventProjector, InventoryProjector, PaymentProjector, ReservationSagaProjector},
    TicketingEnvironment, NoOpEventBus, NoOpProjector,
};
use crate::projections::query_adapters::{PostgresAnalyticsQuery, PostgresInventoryQuery, PostgresPaymentQuery, PostgresReservationQuery};
use crate::projections::{
    PostgresAvailableSeatsProjection, PostgresEventsProjection, PostgresPaymentsProjection,
    PostgresReservationsProjection, ProjectionCompletionTracker,
};
use crate::runtime::{Application, EventConsumer};
use crate::server::{routes::AuthAppState, AppState};
use composable_rust_core::environment::SystemClock;
use composable_rust_next::{HandlerBuilder, NoOpCallExecutor, NoOpQueryFetcher, SystemClock as NextSystemClock};
use composable_rust_postgres_next::PostgresEventStore as NextPostgresEventStore;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Builder for creating a fully configured ticketing application.
///
/// This builder follows a **declarative, step-by-step** pattern for setting up
/// the application. Each method corresponds to a logical phase of initialization:
///
/// 1. **Configuration**: Load config, setup logging
/// 2. **Infrastructure**: Initialize databases, event bus, auth
/// 3. **Business Logic**: Register aggregates and projections
/// 4. **HTTP Server**: Build router and bind listener
/// 5. **Runtime**: Create application ready to run
///
/// # Type-State Pattern
///
/// The builder uses **Option fields** to track which components have been initialized.
/// This provides runtime validation that steps are called in the correct order.
///
/// # Framework-Level Design
///
/// While this implementation is ticketing-specific, the **structure** is reusable:
/// - Replace `Config` with your application's config type
/// - Replace `register_aggregate_consumers` with your aggregate registration
/// - Replace `register_projections` with your projection registration
/// - Replace `build_router` with your HTTP routes
///
/// The lifecycle management (shutdown coordination, graceful termination) is
/// completely generic and requires no customization.
pub struct ApplicationBuilder {
    /// Application configuration
    config: Option<Arc<Config>>,

    /// Infrastructure resources (databases, event bus, etc.)
    resources: Option<ResourceManager>,

    /// Aggregate event consumers (inventory, payment, reservation, event)
    aggregate_consumers: Vec<EventConsumer>,

    /// Complete projection system (managers + in-memory + consumers)
    projection_system: Option<ProjectionSystem>,

    /// Authentication store for session management
    auth_store: Option<Arc<TicketingAuthStore>>,

    /// Shutdown signal broadcaster
    shutdown_tx: broadcast::Sender<()>,

    /// Shutdown signal receiver (consumed during registration)
    shutdown_rx: Option<broadcast::Receiver<()>>,
}

impl ApplicationBuilder {
    /// Create a new application builder.
    ///
    /// Initializes the shutdown channel that will be used to coordinate
    /// graceful termination of all background tasks.
    ///
    /// # Returns
    ///
    /// A new `ApplicationBuilder` ready for configuration.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let builder = ApplicationBuilder::new();
    /// ```
    #[must_use]
    pub fn new() -> Self {
        let (shutdown_tx, shutdown_rx) = broadcast::channel(16);

        Self {
            config: None,
            resources: None,
            aggregate_consumers: Vec::new(),
            projection_system: None,
            auth_store: None,
            shutdown_tx,
            shutdown_rx: Some(shutdown_rx),
        }
    }

    /// Set application configuration.
    ///
    /// This should be called first, as other steps depend on the configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - Application configuration (typically from environment variables)
    ///
    /// # Returns
    ///
    /// Self for method chaining.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = Config::from_env()?;
    /// let builder = ApplicationBuilder::new()
    ///     .with_config(config);
    /// ```
    #[must_use]
    pub fn with_config(mut self, config: Config) -> Self {
        self.config = Some(Arc::new(config));
        self
    }

    /// Setup tracing and logging.
    ///
    /// Initializes `tracing-subscriber` with:
    /// - Environment-based filtering (RUST_LOG env var)
    /// - Formatted output for development
    ///
    /// # Returns
    ///
    /// `Ok(Self)` for method chaining, or error if tracing setup fails.
    ///
    /// # Errors
    ///
    /// Returns error if tracing subscriber cannot be initialized (e.g., already initialized).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let builder = ApplicationBuilder::new()
    ///     .with_config(config)
    ///     .with_tracing()?;
    /// ```
    pub fn with_tracing(self) -> Result<Self, Box<dyn std::error::Error>> {
        tracing_subscriber::registry()
            .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
            .with(tracing_subscriber::fmt::layer())
            .init();

        Ok(self)
    }

    /// Initialize infrastructure resources.
    ///
    /// This method:
    /// 1. Connects to PostgreSQL databases (event store, projections, auth)
    /// 2. Runs database migrations
    /// 3. Connects to Redpanda event bus
    /// 4. Creates system clock
    ///
    /// # Returns
    ///
    /// `Ok(Self)` for method chaining, or error if any infrastructure setup fails.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Config not set (call `with_config` first)
    /// - Database connection fails
    /// - Database migrations fail
    /// - Event bus connection fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let builder = ApplicationBuilder::new()
    ///     .with_config(config)
    ///     .with_tracing()?
    ///     .with_resources().await?;
    /// ```
    pub async fn with_resources(mut self) -> Result<Self, Box<dyn std::error::Error>> {
        let config = self
            .config
            .as_ref()
            .ok_or("Config must be set before initializing resources")?;

        let resources = ResourceManager::from_config(config.as_ref()).await?;
        self.resources = Some(resources);

        Ok(self)
    }

    /// Register aggregate event consumers.
    ///
    /// Creates consumers for:
    /// - Inventory aggregate (seat reservation/release)
    /// - Payment aggregate (payment processing)
    ///
    /// # Returns
    ///
    /// `Ok(Self)` for method chaining, or error if registration fails.
    ///
    /// # Errors
    ///
    /// Returns error if resources not initialized (call `with_resources` first).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let builder = ApplicationBuilder::new()
    ///     .with_config(config)
    ///     .with_resources().await?
    ///     .with_aggregates()?;
    /// ```
    pub fn with_aggregates(mut self) -> Result<Self, Box<dyn std::error::Error>> {
        let resources = self
            .resources
            .as_ref()
            .ok_or("Resources must be initialized before registering aggregates")?;

        let shutdown_rx = self
            .shutdown_rx
            .take()
            .ok_or("Shutdown receiver already consumed")?;

        let consumers = register_aggregate_consumers(resources, shutdown_rx);
        self.aggregate_consumers = consumers;

        // Create new receiver for projections
        self.shutdown_rx = Some(self.shutdown_tx.subscribe());

        Ok(self)
    }

    /// Register projection consumers and managers.
    ///
    /// Creates:
    /// - PostgreSQL projection managers (available seats)
    /// - In-memory projections (sales analytics, customer history)
    /// - Ownership indices for security
    /// - Event consumers that update projections
    ///
    /// # Returns
    ///
    /// `Ok(Self)` for method chaining, or error if registration fails.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Resources not initialized (call `with_resources` first)
    /// - Projection setup fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let builder = ApplicationBuilder::new()
    ///     .with_config(config)
    ///     .with_resources().await?
    ///     .with_aggregates()?
    ///     .with_projections().await?;
    /// ```
    pub async fn with_projections(mut self) -> Result<Self, Box<dyn std::error::Error>> {
        let resources = self
            .resources
            .as_ref()
            .ok_or("Resources must be initialized before registering projections")?;

        let shutdown_rx = self
            .shutdown_rx
            .take()
            .ok_or("Shutdown receiver already consumed")?;

        let projection_system = register_projections(resources, shutdown_rx).await?;
        self.projection_system = Some(projection_system);

        // Create new receiver for HTTP server
        self.shutdown_rx = Some(self.shutdown_tx.subscribe());

        Ok(self)
    }

    /// Setup authentication store.
    ///
    /// Initializes the authentication framework with:
    /// - Magic link authentication
    /// - Session management
    /// - Email sender configuration
    ///
    /// # Returns
    ///
    /// `Ok(Self)` for method chaining, or error if auth setup fails.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Config not set
    /// - Resources not initialized
    /// - Auth store initialization fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let builder = ApplicationBuilder::new()
    ///     .with_config(config)
    ///     .with_resources().await?
    ///     .with_auth().await?;
    /// ```
    pub async fn with_auth(mut self) -> Result<Self, Box<dyn std::error::Error>> {
        let config = self
            .config
            .as_ref()
            .ok_or("Config must be set before initializing auth")?;

        let resources = self
            .resources
            .as_ref()
            .ok_or("Resources must be initialized before auth")?;

        let auth_store = build_auth_store(config.as_ref(), (*resources.auth_pool).clone()).await?;
        self.auth_store = Some(auth_store);

        Ok(self)
    }

    /// Build the complete application.
    ///
    /// This method:
    /// 1. Creates projection query adapters
    /// 2. Creates projection completion tracker
    /// 3. Builds AppState with all dependencies
    /// 4. Builds HTTP router with all routes
    /// 5. Binds TCP listener
    /// 6. Combines all consumers (aggregates + projections)
    /// 7. Creates Application ready to run
    ///
    /// # Returns
    ///
    /// `Ok(Application)` ready to run, or error if build fails.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Any required component not initialized
    /// - HTTP server cannot bind to address
    /// - Router construction fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let app = ApplicationBuilder::new()
    ///     .with_config(config)
    ///     .with_tracing()?
    ///     .with_resources().await?
    ///     .with_aggregates()?
    ///     .with_projections().await?
    ///     .with_auth().await?
    ///     .build().await?;
    /// ```
    pub async fn build(self) -> Result<Application, Box<dyn std::error::Error>> {
        // Validate all components initialized
        let config = self.config.ok_or("Config must be set")?;
        let resources = self.resources.ok_or("Resources must be initialized")?;
        let projection_system = self.projection_system.ok_or("Projections must be registered")?;
        let auth_store = self.auth_store.ok_or("Auth must be initialized")?;

        // Create projection query adapters
        let events_projection = Arc::new(PostgresEventsProjection::new(
            resources.projections_pool.clone(),
        ));
        let reservations_projection = Arc::new(PostgresReservationsProjection::new(
            resources.projections_pool.clone(),
        ));
        let available_seats_projection = Arc::new(PostgresAvailableSeatsProjection::new(
            resources.projections_pool.clone(),
        ));
        let payments_projection = Arc::new(PostgresPaymentsProjection::new(
            resources.projections_pool.clone(),
        ));
        let inventory_query = Arc::new(PostgresInventoryQuery::new(available_seats_projection.clone()));
        let payment_query = Arc::new(PostgresPaymentQuery::new(payments_projection.clone()));
        let reservation_query = Arc::new(PostgresReservationQuery::new(reservations_projection.clone()));
        let analytics_query = Arc::new(PostgresAnalyticsQuery::new(
            projection_system.sales_analytics.clone(),
            projection_system.customer_history.clone(),
        ));

        // Create projection completion tracker (without EventBus tracking for channel-based architecture)
        let projection_completion_tracker = Arc::new(
            ProjectionCompletionTracker::new_without_tracking()
        );

        // Build AppState (kept for reference, but handlers now use next-gen architecture)
        let app_state = AppState::new(
            config.clone(),
            auth_store.clone(),
            resources.auth_pool.clone(),
            Arc::new(SystemClock),
            resources.event_store.clone(),
            inventory_query,
            payment_query,
            reservation_query,
            analytics_query.clone(),
            events_projection.clone(),
            reservations_projection.clone(),
            payments_projection.clone(),
            available_seats_projection.clone(),
            projection_system.sales_analytics.clone(),
            projection_system.customer_history.clone(),
            projection_system.reservation_ownership,
            projection_system.payment_ownership,
            projection_completion_tracker,
            resources.clone(),
        );
        let _app_state = app_state; // Kept for potential future use

        // ═══════════════════════════════════════════════════════════════════════
        // Create Next-Generation Handlers
        // ═══════════════════════════════════════════════════════════════════════

        // Create the next-gen PostgresEventStore from the existing pool
        let next_event_store = NextPostgresEventStore::from_pool(
            resources.event_store.pool().clone()
        );

        // Get projections pool for creating projectors
        let projections_pool = (*resources.projections_pool).clone();

        // ───────────────────────────────────────────────────────────────────────
        // Event Handler (write operations with projection)
        // ───────────────────────────────────────────────────────────────────────
        let event_projector = EventProjector::new(projections_pool.clone());
        type EventEnv = TicketingEnvironment<NextSystemClock, NextPostgresEventStore, EventProjector, NoOpEventBus>;
        let event_env: EventEnv = TicketingEnvironment::new(
            NextSystemClock,
            next_event_store.clone(),
            Some(event_projector),
            None::<NoOpEventBus>,
            "ticketing-events",
        );
        let event_handler = Arc::new(
            HandlerBuilder::new(EventBusinessLogic)
                .call_executor(NoOpCallExecutor)
                .query_fetcher(NoOpQueryFetcher)
                .environment(event_env)
                .build()
        );

        // ───────────────────────────────────────────────────────────────────────
        // Inventory Handler (write operations with projection)
        // ───────────────────────────────────────────────────────────────────────
        let inventory_projector = InventoryProjector::new(projections_pool.clone());
        type InventoryEnv = TicketingEnvironment<NextSystemClock, NextPostgresEventStore, InventoryProjector, NoOpEventBus>;
        let inventory_env: InventoryEnv = TicketingEnvironment::new(
            NextSystemClock,
            next_event_store.clone(),
            Some(inventory_projector),
            None::<NoOpEventBus>,
            "ticketing-inventory",
        );
        let inventory_handler = Arc::new(
            HandlerBuilder::new(InventoryBusinessLogic)
                .call_executor(NoOpCallExecutor)
                .query_fetcher(NoOpQueryFetcher)
                .environment(inventory_env)
                .build()
        );

        // ───────────────────────────────────────────────────────────────────────
        // Payment Handler (write operations with projection)
        // ───────────────────────────────────────────────────────────────────────
        let payment_projector = PaymentProjector::new(projections_pool.clone());
        type PaymentEnv = TicketingEnvironment<NextSystemClock, NextPostgresEventStore, PaymentProjector, NoOpEventBus>;
        let payment_env: PaymentEnv = TicketingEnvironment::new(
            NextSystemClock,
            next_event_store.clone(),
            Some(payment_projector),
            None::<NoOpEventBus>,
            "ticketing-payments",
        );
        let payment_handler = Arc::new(
            HandlerBuilder::new(PaymentBusinessLogic)
                .call_executor(NoOpCallExecutor)
                .query_fetcher(NoOpQueryFetcher)
                .environment(payment_env)
                .build()
        );

        // ───────────────────────────────────────────────────────────────────────
        // Query-Enabled Event Handler
        // ───────────────────────────────────────────────────────────────────────
        let event_projection_queries = EventProjectionQueries::new(
            projections_pool.clone(),
        );
        type QueryEventEnv = TicketingEnvironment<NextSystemClock, NextPostgresEventStore, NoOpProjector, NoOpEventBus, EventProjectionQueries>;
        let query_event_env: QueryEventEnv = TicketingEnvironment::with_projections(
            NextSystemClock,
            next_event_store.clone(),
            None::<NoOpProjector>,
            None::<NoOpEventBus>,
            "ticketing-events",
            event_projection_queries,
        );
        let query_event_handler = Arc::new(
            HandlerBuilder::new(EventBusinessLogic)
                .call_executor(NoOpCallExecutor)
                .query_fetcher(EventQueryFetcher)
                .environment(query_event_env)
                .build()
        );

        // ───────────────────────────────────────────────────────────────────────
        // Query-Enabled Inventory Handler
        // ───────────────────────────────────────────────────────────────────────
        let inventory_projection_queries = InventoryProjectionQueries::new(
            projections_pool.clone(),
        );
        type QueryInventoryEnv = TicketingEnvironment<NextSystemClock, NextPostgresEventStore, NoOpProjector, NoOpEventBus, InventoryProjectionQueries>;
        let query_inventory_env: QueryInventoryEnv = TicketingEnvironment::with_projections(
            NextSystemClock,
            next_event_store.clone(),
            None::<NoOpProjector>,
            None::<NoOpEventBus>,
            "ticketing-events",
            inventory_projection_queries,
        );
        let query_inventory_handler = Arc::new(
            HandlerBuilder::new(InventoryBusinessLogic)
                .call_executor(NoOpCallExecutor)
                .query_fetcher(InventoryQueryFetcher)
                .environment(query_inventory_env)
                .build()
        );

        // ───────────────────────────────────────────────────────────────────────
        // Query-Enabled Payment Handler
        // ───────────────────────────────────────────────────────────────────────
        let payment_projection_queries = PaymentProjectionQueries::new(
            projections_pool.clone(),
        );
        type QueryPaymentEnv = TicketingEnvironment<NextSystemClock, NextPostgresEventStore, NoOpProjector, NoOpEventBus, PaymentProjectionQueries>;
        let query_payment_env: QueryPaymentEnv = TicketingEnvironment::with_projections(
            NextSystemClock,
            next_event_store.clone(),
            None::<NoOpProjector>,
            None::<NoOpEventBus>,
            "ticketing-events",
            payment_projection_queries,
        );
        let query_payment_handler = Arc::new(
            HandlerBuilder::new(PaymentBusinessLogic)
                .call_executor(NoOpCallExecutor)
                .query_fetcher(PaymentQueryFetcher)
                .environment(query_payment_env)
                .build()
        );

        // ───────────────────────────────────────────────────────────────────────
        // Analytics Handler
        // ───────────────────────────────────────────────────────────────────────
        let analytics_projection_queries = AnalyticsProjectionQueries::new(
            projections_pool.clone(),
        );
        type AnalyticsEnv = TicketingEnvironment<NextSystemClock, NextPostgresEventStore, NoOpProjector, NoOpEventBus, AnalyticsProjectionQueries>;
        let analytics_env: AnalyticsEnv = TicketingEnvironment::with_projections(
            NextSystemClock,
            next_event_store.clone(),
            None::<NoOpProjector>,
            None::<NoOpEventBus>,
            "ticketing-events",
            analytics_projection_queries,
        );
        let analytics_handler = Arc::new(
            HandlerBuilder::new(AnalyticsBusinessLogic)
                .call_executor(NoOpCallExecutor)
                .query_fetcher(AnalyticsQueryFetcher)
                .environment(analytics_env)
                .build()
        );

        // ───────────────────────────────────────────────────────────────────────
        // Reservation Query Handler
        // ───────────────────────────────────────────────────────────────────────
        let reservation_projection_queries = ReservationProjectionQueries::new(
            projections_pool.clone(),
        );
        type ReservationQueryEnv = TicketingEnvironment<NextSystemClock, NextPostgresEventStore, NoOpProjector, NoOpEventBus, ReservationProjectionQueries>;
        let reservation_query_env: ReservationQueryEnv = TicketingEnvironment::with_projections(
            NextSystemClock,
            next_event_store.clone(),
            None::<NoOpProjector>,
            None::<NoOpEventBus>,
            "ticketing-events",
            reservation_projection_queries,
        );
        let reservation_query_handler = Arc::new(
            HandlerBuilder::new(ReservationQueryLogic)
                .call_executor(NoOpCallExecutor)
                .query_fetcher(ReservationQueryFetcher)
                .environment(reservation_query_env)
                .build()
        );

        // ───────────────────────────────────────────────────────────────────────
        // Reservation Saga Handler (orchestrates Inventory + Payment)
        // ───────────────────────────────────────────────────────────────────────
        // Use the query-enabled handlers (with real QueryFetcher) for the saga
        // so that commands get their `fetched` fields populated from projections.
        // The type-erased trait objects hide the QueryFetcher types from the saga.
        let saga_inventory_handler: Arc<dyn InventoryHandlerTrait> = query_inventory_handler.clone();
        let saga_payment_handler: Arc<dyn PaymentHandlerTrait> = query_payment_handler.clone();
        let reservation_saga_call_executor = ReservationSagaCallExecutor::new(
            saga_inventory_handler,
            saga_payment_handler,
            next_event_store.clone(),
        );

        // Create in-memory reservation saga projection state (shared between projector and query fetcher)
        let reservation_saga_projection_state: crate::next::InMemoryReservationSagaProjection =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));

        // Create reservation saga projector that updates both in-memory state AND PostgreSQL
        let reservation_saga_projector = ReservationSagaProjector::new(
            reservation_saga_projection_state.clone(),
            projections_pool.clone(),
        );

        // Create reservation saga query fetcher that reads from the in-memory state
        let reservation_saga_query_fetcher = ReservationSagaQueryFetcher::new(reservation_saga_projection_state.clone());

        // Create saga projection queries for the environment
        let reservation_saga_projection_queries = ReservationSagaProjectionQueries::new(reservation_saga_projection_state);

        type ReservationSagaEnv = TicketingEnvironment<NextSystemClock, NextPostgresEventStore, ReservationSagaProjector, NoOpEventBus, ReservationSagaProjectionQueries>;
        let saga_env: ReservationSagaEnv = TicketingEnvironment::with_projections(
            NextSystemClock,
            next_event_store.clone(),
            Some(reservation_saga_projector),
            None::<NoOpEventBus>,
            "ticketing-sagas",
            reservation_saga_projection_queries,
        );
        let saga_handler = Arc::new(
            HandlerBuilder::new(ReservationSagaLogic)
                .call_executor(reservation_saga_call_executor)
                .query_fetcher(reservation_saga_query_fetcher)
                .environment(saga_env)
                .build()
        );

        // ───────────────────────────────────────────────────────────────────────
        // Event-Inventory Saga Handler (orchestrates Event + Inventory creation)
        // ───────────────────────────────────────────────────────────────────────
        // Use the handlers with real QueryFetcher for the saga.
        // Note: For event creation, the event handler uses NoOpQueryFetcher since
        // Initialize doesn't need fetched data (fetched: None is valid).
        // The inventory handler uses the query-enabled version.
        let saga_event_handler: Arc<dyn EventHandlerTrait> = event_handler.clone();
        let saga_inventory_handler_for_event_saga: Arc<dyn InventoryHandlerTrait> = inventory_handler.clone();
        let event_inventory_saga_executor = EventInventorySagaCallExecutor::new(
            saga_event_handler,
            saga_inventory_handler_for_event_saga,
            next_event_store.clone(),
        );

        // Create in-memory saga projection state (shared between projector and query fetcher)
        let saga_projection_state: crate::next::InMemorySagaProjection =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));

        // Create saga projector that updates the in-memory state
        let saga_projector = EventInventorySagaProjector::new(saga_projection_state.clone());

        // Create saga query fetcher that reads from the in-memory state
        let saga_query_fetcher = SagaQueryFetcher::new(saga_projection_state.clone());

        // Create saga projection queries for the environment
        let saga_projection_queries = SagaProjectionQueries::new(saga_projection_state);

        type EventInventorySagaEnv = TicketingEnvironment<NextSystemClock, NextPostgresEventStore, EventInventorySagaProjector, NoOpEventBus, SagaProjectionQueries>;
        let event_inventory_saga_env: EventInventorySagaEnv = TicketingEnvironment::with_projections(
            NextSystemClock,
            next_event_store.clone(),
            Some(saga_projector),
            None::<NoOpEventBus>,
            "ticketing-event-inventory-sagas",
            saga_projection_queries,
        );
        let event_inventory_saga_handler = Arc::new(
            HandlerBuilder::new(EventInventorySagaLogic)
                .call_executor(event_inventory_saga_executor)
                .query_fetcher(saga_query_fetcher)
                .environment(event_inventory_saga_env)
                .build()
        );

        // ═══════════════════════════════════════════════════════════════════════
        // Build State Types
        // ═══════════════════════════════════════════════════════════════════════

        let auth_state = AuthAppState {
            auth_store,
            config: config.clone(),
        };

        let next_state = NextAppState {
            event_handler: event_handler.clone(),
            inventory_handler: inventory_handler.clone(),
            payment_handler: payment_handler.clone(),
        };

        let query_state = QueryAppState {
            event_handler: query_event_handler.clone(),
        };

        let full_query_state = FullQueryAppState {
            event_handler: event_handler.clone(),
            query_event_handler: query_event_handler.clone(),
            inventory_handler: inventory_handler.clone(),
            query_inventory_handler: query_inventory_handler.clone(),
            payment_handler: payment_handler.clone(),
            query_payment_handler: query_payment_handler.clone(),
        };

        let analytics_state = AnalyticsAppState {
            analytics_handler,
        };

        let reservation_query_state = ReservationQueryAppState {
            reservation_handler: reservation_query_handler,
        };

        let reservation_state = ReservationAppState {
            saga_handler,
        };

        let event_creation_state = EventCreationAppState {
            saga_handler: event_inventory_saga_handler,
        };

        // ═══════════════════════════════════════════════════════════════════════
        // Build HTTP Router
        // ═══════════════════════════════════════════════════════════════════════

        // Build API v2 routes (nested under /api/v2)
        let api_v2_routes = axum::Router::new()
            .merge(crate::next::http::event_creation_routes().with_state(event_creation_state))
            .merge(crate::next::http::events_v2_routes().with_state(full_query_state.clone()))
            .merge(crate::next::http::query_routes().with_state(query_state))
            .merge(crate::next::http::availability_routes().with_state(full_query_state.clone()))
            .merge(crate::next::http::payment_routes().with_state(full_query_state))
            .merge(crate::next::http::analytics_routes().with_state(analytics_state))
            .merge(crate::next::http::reservation_query_routes().with_state(reservation_query_state))
            .merge(crate::next::http::reservation_routes().with_state(reservation_state));

        let router = crate::next::http::health_routes()
            .nest("/api/v2", api_v2_routes)
            .nest("/auth", axum::Router::new()
                .route("/magic-link/request", axum::routing::post(crate::auth::handlers::send_magic_link))
                .route("/magic-link/verify", axum::routing::post(crate::auth::handlers::verify_magic_link))
                .with_state(auth_state));

        // Bind TCP listener
        let listener = tokio::net::TcpListener::bind(format!(
            "{}:{}",
            config.server.host, config.server.port
        ))
        .await?;

        // Combine all consumers
        let mut all_consumers = self.aggregate_consumers;
        all_consumers.extend(projection_system.consumers);

        // Create Application
        Ok(Application::new(
            listener,
            router,
            all_consumers,
            projection_system.managers,
            self.shutdown_tx,
            config,
        ))
    }
}

impl Default for ApplicationBuilder {
    fn default() -> Self {
        Self::new()
    }
}
