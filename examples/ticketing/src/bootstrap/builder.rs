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
use crate::projections::query_adapters::{PostgresInventoryQuery, PostgresPaymentQuery, PostgresReservationQuery};
use crate::projections::{
    PostgresAvailableSeatsProjection, PostgresEventsProjection, PostgresReservationsProjection,
    ProjectionCompletionTracker,
};
use crate::runtime::{Application, EventConsumer};
use crate::server::{build_router, AppState};
use composable_rust_core::environment::SystemClock;
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
        let inventory_query = Arc::new(PostgresInventoryQuery::new(available_seats_projection.clone()));
        let payment_query = Arc::new(PostgresPaymentQuery::new());
        let reservation_query = Arc::new(PostgresReservationQuery::new(reservations_projection.clone()));

        // Create projection completion tracker
        let projection_completion_tracker = Arc::new(
            ProjectionCompletionTracker::new(resources.event_bus.clone())
                .await
                .map_err(|e| format!("Failed to create projection completion tracker: {e}"))?,
        );

        // Build AppState
        let app_state = AppState::new(
            config.clone(),
            auth_store,
            Arc::new(SystemClock),
            resources.event_store.clone(),
            resources.event_bus.clone(),
            inventory_query,
            payment_query,
            reservation_query,
            events_projection,
            reservations_projection,
            available_seats_projection,
            projection_system.sales_analytics,
            projection_system.customer_history,
            projection_system.reservation_ownership,
            projection_system.payment_ownership,
            projection_completion_tracker,
        );

        // Build HTTP router
        let router = build_router(app_state);

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
