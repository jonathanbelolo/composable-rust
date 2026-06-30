//! Declarative application builder API.
//!
//! Provides a fluent builder interface for constructing and running the ticketing application.
//! This API is designed to be **framework-level reusable** across different applications.
//!
//! # Design Philosophy
//!
//! The builder follows a **declarative, step-by-step** initialization pattern:
//! 1. Configure (config, tracing)
//! 2. Initialize infrastructure (databases, auth)
//! 3. Build HTTP server (routes, state)
//! 4. Run application
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
//!     .with_auth().await?
//!     .build().await?
//!     .run().await?;
//! ```

use crate::auth::setup::{build_auth_store, TicketingAuthStore};
use crate::bootstrap::ResourceManager;
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
    projector::{EventInventorySagaProjector, EventProjector, InventoryProjector, PaymentProjector, PgAtomicPersist},
    TicketingEnvironment, NoOpEventBus, NoOpProjector,
};
use crate::server::routes::AuthAppState;
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
/// 2. **Infrastructure**: Initialize databases, auth
/// 3. **HTTP Server**: Build router and bind listener
/// 4. **Runtime**: Create application ready to run
pub struct ApplicationBuilder {
    /// Application configuration
    config: Option<Arc<Config>>,

    /// Infrastructure resources (databases, etc.)
    resources: Option<ResourceManager>,

    /// Authentication store for session management
    auth_store: Option<Arc<TicketingAuthStore>>,

    /// Shutdown signal broadcaster
    shutdown_tx: broadcast::Sender<()>,
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
    #[must_use]
    pub fn new() -> Self {
        let (shutdown_tx, _) = broadcast::channel(16);

        Self {
            config: None,
            resources: None,
            auth_store: None,
            shutdown_tx,
        }
    }

    /// Set application configuration.
    ///
    /// This should be called first, as other steps depend on the configuration.
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
    /// # Errors
    ///
    /// Returns error if tracing subscriber cannot be initialized (e.g., already initialized).
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
    /// 3. Creates system clock
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Config not set (call `with_config` first)
    /// - Database connection fails
    /// - Database migrations fail
    pub async fn with_resources(mut self) -> Result<Self, Box<dyn std::error::Error>> {
        let config = self
            .config
            .as_ref()
            .ok_or("Config must be set before initializing resources")?;

        let resources = ResourceManager::from_config(config.as_ref()).await?;
        self.resources = Some(resources);

        Ok(self)
    }

    /// Setup authentication store.
    ///
    /// Initializes the authentication framework with:
    /// - Magic link authentication
    /// - Session management
    /// - Email sender configuration
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Config not set
    /// - Resources not initialized
    /// - Auth store initialization fails
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
    /// 1. Creates all handler instances using next-generation architecture
    /// 2. Builds AppState with all dependencies
    /// 3. Builds HTTP router with all routes
    /// 4. Binds TCP listener
    /// 5. Returns Application ready to run
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Any required component not initialized
    /// - HTTP server cannot bind to address
    /// - Router construction fails
    pub async fn build(self) -> Result<Application, Box<dyn std::error::Error>> {
        // Validate all components initialized
        let config = self.config.ok_or("Config must be set")?;
        let resources = self.resources.ok_or("Resources must be initialized")?;
        let auth_store = self.auth_store.ok_or("Auth must be initialized")?;

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
        let event_env: TicketingEnvironment<NextSystemClock, NextPostgresEventStore, EventProjector, NoOpEventBus> = TicketingEnvironment::new(
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
        let inventory_env: TicketingEnvironment<NextSystemClock, NextPostgresEventStore, InventoryProjector, NoOpEventBus> = TicketingEnvironment::new(
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
        let payment_env: TicketingEnvironment<NextSystemClock, NextPostgresEventStore, PaymentProjector, NoOpEventBus> = TicketingEnvironment::new(
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
        let query_event_env: TicketingEnvironment<NextSystemClock, NextPostgresEventStore, NoOpProjector, NoOpEventBus, EventProjectionQueries> = TicketingEnvironment::with_projections(
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
        // Full Event Handler (Query + Projection)
        // ───────────────────────────────────────────────────────────────────────
        // This handler has BOTH query fetcher (for validation) AND projector (for writes).
        // Used for operations like Publish/Cancel that need to validate and update.
        let full_event_projection_queries = EventProjectionQueries::new(
            projections_pool.clone(),
        );
        let full_event_env: TicketingEnvironment<NextSystemClock, NextPostgresEventStore, EventProjector, NoOpEventBus, EventProjectionQueries> = TicketingEnvironment::with_projections(
            NextSystemClock,
            next_event_store.clone(),
            Some(EventProjector::new(projections_pool.clone())),
            None::<NoOpEventBus>,
            "ticketing-events",
            full_event_projection_queries,
        );
        let full_event_handler = Arc::new(
            HandlerBuilder::new(EventBusinessLogic)
                .call_executor(NoOpCallExecutor)
                .query_fetcher(EventQueryFetcher)
                .environment(full_event_env)
                .build()
        );

        // ───────────────────────────────────────────────────────────────────────
        // Query-Enabled Inventory Handler
        // ───────────────────────────────────────────────────────────────────────
        let inventory_projection_queries = InventoryProjectionQueries::new(
            projections_pool.clone(),
        );
        let query_inventory_env: TicketingEnvironment<NextSystemClock, NextPostgresEventStore, NoOpProjector, NoOpEventBus, InventoryProjectionQueries> = TicketingEnvironment::with_projections(
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
        // Full Inventory Handler (Query + Projection)
        // ───────────────────────────────────────────────────────────────────────
        // This handler has BOTH query fetcher (for validation) AND projector (for writes).
        // Used by the Reservation Saga to validate availability AND update inventory.
        let full_inventory_projection_queries = InventoryProjectionQueries::new(
            projections_pool.clone(),
        );
        let full_inventory_env: TicketingEnvironment<NextSystemClock, NextPostgresEventStore, InventoryProjector, NoOpEventBus, InventoryProjectionQueries> = TicketingEnvironment::with_projections(
            NextSystemClock,
            next_event_store.clone(),
            Some(InventoryProjector::new(projections_pool.clone())),
            None::<NoOpEventBus>,
            "ticketing-inventory",
            full_inventory_projection_queries,
        );
        let full_inventory_handler = Arc::new(
            HandlerBuilder::new(InventoryBusinessLogic)
                .call_executor(NoOpCallExecutor)
                .query_fetcher(InventoryQueryFetcher)
                .environment(full_inventory_env)
                .build()
        );

        // ───────────────────────────────────────────────────────────────────────
        // Query-Enabled Payment Handler (with Projector for saga use)
        // ───────────────────────────────────────────────────────────────────────
        // NOTE: The saga uses this handler for payment processing, so it MUST have
        // the PaymentProjector attached to project payment events to the read model.
        let payment_projection_queries = PaymentProjectionQueries::new(
            projections_pool.clone(),
        );
        let query_payment_projector = PaymentProjector::new(projections_pool.clone());
        let query_payment_env: TicketingEnvironment<NextSystemClock, NextPostgresEventStore, PaymentProjector, NoOpEventBus, PaymentProjectionQueries> = TicketingEnvironment::with_projections(
            NextSystemClock,
            next_event_store.clone(),
            Some(query_payment_projector),
            None::<NoOpEventBus>,
            "ticketing-payments",
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
        let analytics_env: TicketingEnvironment<NextSystemClock, NextPostgresEventStore, NoOpProjector, NoOpEventBus, AnalyticsProjectionQueries> = TicketingEnvironment::with_projections(
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
        let reservation_query_env: TicketingEnvironment<NextSystemClock, NextPostgresEventStore, NoOpProjector, NoOpEventBus, ReservationProjectionQueries> = TicketingEnvironment::with_projections(
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
        // NOTE: We use full_inventory_handler here because the saga needs BOTH:
        // 1. Query fetcher to validate seat availability before reserving
        // 2. Projector to update the inventory table when seats are reserved/confirmed
        let saga_inventory_handler: Arc<dyn InventoryHandlerTrait> = full_inventory_handler.clone();
        let saga_payment_handler: Arc<dyn PaymentHandlerTrait> = query_payment_handler.clone();
        let reservation_saga_call_executor = ReservationSagaCallExecutor::new(
            saga_inventory_handler,
            saga_payment_handler,
            next_event_store.clone(),
        );

        // The saga's durable state lives in the SAME database as the event log, so
        // its events and `saga_state` commit in one transaction (no in-memory map).
        // The fetcher reads `saga_state` (restart-safe, OCC); the projection happens
        // transactionally via `atomic_persist`, so the env's Projector is a no-op.
        let reservation_saga_query_fetcher = ReservationSagaQueryFetcher::new();
        let reservation_saga_projection_queries =
            ReservationSagaProjectionQueries::new(next_event_store.pool().clone());
        let reservation_saga_atomic_persist =
            Arc::new(PgAtomicPersist::new(next_event_store.clone()));

        let saga_env: TicketingEnvironment<
            NextSystemClock,
            NextPostgresEventStore,
            NoOpProjector,
            NoOpEventBus,
            ReservationSagaProjectionQueries,
        > = TicketingEnvironment::with_projections(
            NextSystemClock,
            next_event_store.clone(),
            None::<NoOpProjector>,
            None::<NoOpEventBus>,
            "ticketing-sagas",
            reservation_saga_projection_queries,
        )
        .with_atomic_persist(reservation_saga_atomic_persist);

        let saga_handler = Arc::new(
            HandlerBuilder::new(ReservationSagaLogic)
                .call_executor(reservation_saga_call_executor)
                .query_fetcher(reservation_saga_query_fetcher)
                .environment(saga_env)
                .build(),
        );

        // ───────────────────────────────────────────────────────────────────────
        // Event-Inventory Saga Handler (orchestrates Event + Inventory creation)
        // ───────────────────────────────────────────────────────────────────────
        let saga_event_handler: Arc<dyn EventHandlerTrait> = event_handler.clone();
        let saga_inventory_handler_for_event_saga: Arc<dyn InventoryHandlerTrait> = inventory_handler.clone();
        let event_inventory_saga_executor = EventInventorySagaCallExecutor::new(
            saga_event_handler,
            saga_inventory_handler_for_event_saga,
            next_event_store.clone(),
        );

        // Create in-memory saga projection state
        let saga_projection_state: crate::next::InMemorySagaProjection =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));

        // Create saga projector
        let saga_projector = EventInventorySagaProjector::new(saga_projection_state.clone());

        // Create saga query fetcher
        let saga_query_fetcher = SagaQueryFetcher::new(saga_projection_state.clone());

        // Create saga projection queries
        let saga_projection_queries = SagaProjectionQueries::new(saga_projection_state);

        let event_inventory_saga_env: TicketingEnvironment<NextSystemClock, NextPostgresEventStore, EventInventorySagaProjector, NoOpEventBus, SagaProjectionQueries> = TicketingEnvironment::with_projections(
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
        let _ = next_state; // Keep for potential future use

        let query_state = QueryAppState {
            event_handler: query_event_handler.clone(),
        };

        let full_query_state = FullQueryAppState {
            event_handler: event_handler.clone(),
            query_event_handler: query_event_handler.clone(),
            full_event_handler: full_event_handler.clone(),
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

        // Create Application
        Ok(Application::new(
            listener,
            router,
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

/// A running application instance.
pub struct Application {
    listener: tokio::net::TcpListener,
    router: axum::Router,
    shutdown_tx: broadcast::Sender<()>,
    config: Arc<Config>,
}

impl Application {
    /// Create a new application.
    fn new(
        listener: tokio::net::TcpListener,
        router: axum::Router,
        shutdown_tx: broadcast::Sender<()>,
        config: Arc<Config>,
    ) -> Self {
        Self {
            listener,
            router,
            shutdown_tx,
            config,
        }
    }

    /// Run the application.
    ///
    /// This starts the HTTP server and waits for shutdown signals.
    ///
    /// # Errors
    ///
    /// Returns error if the server fails to start.
    ///
    /// # Panics
    ///
    /// Panics if the CTRL+C signal handler cannot be installed (platform-specific failure).
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        tracing::info!(
            "Starting ticketing server at http://{}:{}",
            self.config.server.host,
            self.config.server.port
        );

        // Create shutdown signal handler
        let shutdown_tx = self.shutdown_tx.clone();
        let shutdown_signal = async move {
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to install CTRL+C signal handler");
            tracing::info!("Shutdown signal received, initiating graceful shutdown...");
            let _ = shutdown_tx.send(());
        };

        // Run the server with graceful shutdown
        axum::serve(self.listener, self.router)
            .with_graceful_shutdown(shutdown_signal)
            .await?;

        tracing::info!("Server shut down successfully");
        Ok(())
    }
}
