//! Ticketing system HTTP server.
//!
//! Event-sourced ticketing platform with CQRS, sagas, and real-time updates.

use ticketing::{
    aggregates::{
        inventory::{InventoryEnvironment, InventoryReducer},
        payment::{PaymentEnvironment, PaymentReducer},
        reservation::{ReservationEnvironment, ReservationReducer},
    },
    auth::setup::build_auth_store,
    config::Config,
    projections::{
        query_adapters::{PostgresInventoryQuery, PostgresPaymentQuery, PostgresReservationQuery},
        setup_projection_managers, CustomerHistoryProjection, Projection, PostgresAvailableSeatsProjection,
        SalesAnalyticsProjection, TicketingEvent,
    },
    server::{build_router, AppState},
    types::{InventoryState, PaymentState, ReservationState},
};
use composable_rust_core::environment::SystemClock;
use composable_rust_core::event_bus::EventBus;
use composable_rust_core::stream::StreamId;
use composable_rust_postgres::PostgresEventStore;
use composable_rust_redpanda::RedpandaEventBus;
use composable_rust_runtime::Store;
use futures::StreamExt;
use std::sync::{Arc, RwLock};
use tokio::signal;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ticketing=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Ticketing System HTTP Server");

    // Load configuration
    let config = Config::from_env();
    info!(
        postgres_url = %config.postgres.url,
        projections_url = %config.projections.url,
        redpanda_brokers = %config.redpanda.brokers,
        "Configuration loaded"
    );

    // Setup event store (write side)
    info!("Connecting to event store database...");
    let event_store = Arc::new(PostgresEventStore::new(&config.postgres.url).await?);
    info!("Event store connected");

    // Setup event bus
    info!("Connecting to Redpanda event bus...");
    let event_bus: Arc<dyn composable_rust_core::event_bus::EventBus> = Arc::new(
        RedpandaEventBus::builder()
            .brokers(&config.redpanda.brokers)
            .consumer_group(&config.redpanda.consumer_group)
            .build()?,
    );
    info!("Event bus connected");

    // Setup aggregate stores (Composable Rust architecture)
    info!("Initializing aggregate stores...");
    let clock = Arc::new(SystemClock);

    // Create projection queries for aggregates to load state on-demand
    let inventory_query = Arc::new(PostgresInventoryQuery::new(Arc::new(PostgresAvailableSeatsProjection::new(Arc::new(event_store.pool().clone())))));
    let payment_query = Arc::new(PostgresPaymentQuery::new());
    let reservation_query = Arc::new(PostgresReservationQuery::new());

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
    // Note: Following TCA pattern - ReservationState holds child STATE, not stores
    let reservation_env = ReservationEnvironment::new(
        clock.clone(),
        event_store.clone(),
        event_bus.clone(),
        StreamId::new("reservation"),
        reservation_query,
    );
    let reservation_state = ReservationState::new(); // No stores passed!
    let reservation = Arc::new(Store::new(
        reservation_state,
        ReservationReducer::new(),
        reservation_env,
    ));
    info!("Aggregate stores initialized");

    // Setup PostgreSQL pool for auth
    info!("Connecting to auth database...");
    let auth_pg_pool = sqlx::PgPool::connect(&config.postgres.url).await?;

    // Setup authentication store
    info!("Initializing authentication store...");
    let auth_store = build_auth_store(&config, auth_pg_pool).await?;
    info!("Authentication store initialized");

    // Setup projections (read side)
    info!("Setting up projection managers...");
    let projection_managers = setup_projection_managers(&config, event_bus.clone()).await?;
    info!("Projection managers configured");

    // Start projection managers in background
    info!("Starting projection ETL services...");
    let projection_handles = projection_managers.start_all();
    info!(
        projection_count = projection_handles.len(),
        "Projection managers started"
    );

    // Get available seats projection for queries
    let available_seats_projection = Arc::new(
        ticketing::projections::PostgresAvailableSeatsProjection::new(
            Arc::new(
                sqlx::PgPool::connect(&config.projections.url).await?
            )
        )
    );

    // Create in-memory analytics projections
    info!("Initializing in-memory analytics projections...");
    let sales_analytics_projection = Arc::new(RwLock::new(SalesAnalyticsProjection::new()));
    let customer_history_projection = Arc::new(RwLock::new(CustomerHistoryProjection::new()));

    // Create security ownership indices
    info!("Initializing ownership tracking indices...");
    let reservation_ownership = Arc::new(RwLock::new(std::collections::HashMap::new()));
    let payment_ownership = Arc::new(RwLock::new(std::collections::HashMap::new()));

    // Subscribe to events for analytics projections and ownership tracking
    info!("Starting analytics projection event consumers...");
    spawn_analytics_consumers(
        event_bus.clone(),
        sales_analytics_projection.clone(),
        customer_history_projection.clone(),
        reservation_ownership.clone(),
        payment_ownership.clone(),
        &config.redpanda,
    );

    // Build application state
    let state = AppState::new(
        auth_store,
        inventory,
        payment,
        reservation,
        event_store,
        event_bus,
        available_seats_projection,
        sales_analytics_projection,
        customer_history_projection,
        reservation_ownership,
        payment_ownership,
    );

    // Build router
    let app = build_router(state);

    // Create server address
    let addr = format!("{}:{}", config.server.host, config.server.port);
    info!(address = %addr, "Starting HTTP server");

    // Create TCP listener
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Server listening on {}", addr);

    // Run server with graceful shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Server stopped");
    Ok(())
}

/// Spawn background tasks to consume events and update analytics projections.
///
/// Creates two consumer tasks:
/// 1. Sales analytics consumer (reservations + payments)
/// 2. Customer history consumer (reservations)
///
/// Also updates ownership indices for security filtering:
/// - `reservation_ownership`: ReservationId → CustomerId
/// - `payment_ownership`: PaymentId → ReservationId
fn spawn_analytics_consumers(
    event_bus: Arc<dyn EventBus>,
    sales_projection: Arc<RwLock<SalesAnalyticsProjection>>,
    customer_projection: Arc<RwLock<CustomerHistoryProjection>>,
    reservation_ownership: Arc<RwLock<std::collections::HashMap<ticketing::types::ReservationId, ticketing::types::CustomerId>>>,
    payment_ownership: Arc<RwLock<std::collections::HashMap<ticketing::types::PaymentId, ticketing::types::ReservationId>>>,
    redpanda_config: &ticketing::config::RedpandaConfig,
) {
    // Spawn sales analytics consumer (also tracks ownership)
    let sales_bus = event_bus.clone();
    let sales_proj = sales_projection.clone();
    let reservation_ownership_sales = reservation_ownership.clone();
    let payment_ownership_sales = payment_ownership.clone();
    let reservation_topic = redpanda_config.reservation_topic.clone();
    let payment_topic = redpanda_config.payment_topic.clone();

    tokio::spawn(async move {
        info!("Sales analytics projection consumer started");

        // Subscribe to reservation and payment topics
        let topics = &[reservation_topic.as_str(), payment_topic.as_str()];

        loop {
            // Try to subscribe to events
            match sales_bus.subscribe(topics).await {
                Ok(mut stream) => {
                    info!("Sales projection subscribed to event topics");

                    // Process events from stream
                    while let Some(result) = stream.next().await {
                        match result {
                            Ok(serialized_event) => {
                                // Deserialize event from data field
                                match bincode::deserialize::<TicketingEvent>(&serialized_event.data) {
                                    Ok(event) => {
                                        // Track ownership for security
                                        use ticketing::aggregates::{PaymentAction, ReservationAction};
                                        match &event {
                                            TicketingEvent::Reservation(ReservationAction::ReservationInitiated {
                                                reservation_id, customer_id, ..
                                            }) => {
                                                if let Ok(mut index) = reservation_ownership_sales.write() {
                                                    index.insert(*reservation_id, *customer_id);
                                                    info!(reservation_id = %reservation_id.as_uuid(), customer_id = %customer_id.as_uuid(), "Tracked reservation ownership");
                                                }
                                            }
                                            TicketingEvent::Payment(PaymentAction::PaymentProcessed {
                                                payment_id, reservation_id, ..
                                            }) => {
                                                if let Ok(mut index) = payment_ownership_sales.write() {
                                                    index.insert(*payment_id, *reservation_id);
                                                    info!(payment_id = %payment_id.as_uuid(), reservation_id = %reservation_id.as_uuid(), "Tracked payment ownership");
                                                }
                                            }
                                            _ => {}
                                        }

                                        // Update projection
                                        if let Ok(mut projection) = sales_proj.write() {
                                            if let Err(e) = projection.handle_event(&event) {
                                                error!(error = %e, "Failed to handle event in sales projection");
                                            }
                                        } else {
                                            warn!("Failed to acquire write lock on sales projection");
                                        }
                                    }
                                    Err(e) => {
                                        warn!(error = %e, "Failed to deserialize event");
                                    }
                                }
                            }
                            Err(e) => {
                                error!(error = %e, "Error receiving event from stream");
                            }
                        }
                    }

                    // Stream ended, reconnect
                    warn!("Event stream ended, reconnecting in 5s");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
                Err(e) => {
                    error!(error = %e, "Failed to subscribe to event bus, retrying in 5s");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }
    });

    // Spawn customer history consumer (also tracks reservation ownership)
    let customer_bus = event_bus.clone();
    let customer_proj = customer_projection;
    let reservation_ownership_customer = reservation_ownership;
    let customer_topic = redpanda_config.reservation_topic.clone();

    tokio::spawn(async move {
        info!("Customer history projection consumer started");

        let topics = &[customer_topic.as_str()];

        loop {
            // Try to subscribe to events
            match customer_bus.subscribe(topics).await {
                Ok(mut stream) => {
                    info!("Customer projection subscribed to reservation topic");

                    // Process events from stream
                    while let Some(result) = stream.next().await {
                        match result {
                            Ok(serialized_event) => {
                                // Deserialize event from data field
                                match bincode::deserialize::<TicketingEvent>(&serialized_event.data) {
                                    Ok(event) => {
                                        // Track ownership for security (backup tracking)
                                        use ticketing::aggregates::ReservationAction;
                                        if let TicketingEvent::Reservation(ReservationAction::ReservationInitiated {
                                            reservation_id, customer_id, ..
                                        }) = &event {
                                            if let Ok(mut index) = reservation_ownership_customer.write() {
                                                index.insert(*reservation_id, *customer_id);
                                            }
                                        }

                                        // Update projection
                                        if let Ok(mut projection) = customer_proj.write() {
                                            if let Err(e) = projection.handle_event(&event) {
                                                error!(error = %e, "Failed to handle event in customer projection");
                                            }
                                        } else {
                                            warn!("Failed to acquire write lock on customer projection");
                                        }
                                    }
                                    Err(e) => {
                                        warn!(error = %e, "Failed to deserialize event");
                                    }
                                }
                            }
                            Err(e) => {
                                error!(error = %e, "Error receiving event from stream");
                            }
                        }
                    }

                    // Stream ended, reconnect
                    warn!("Event stream ended, reconnecting in 5s");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
                Err(e) => {
                    error!(error = %e, "Failed to subscribe to event bus, retrying in 5s");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }
    });

    info!("Analytics projection consumers spawned");
}

/// Graceful shutdown signal handler.
///
/// Waits for:
/// - Ctrl+C (SIGINT)
/// - SIGTERM (in production environments)
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {
            info!("Received Ctrl+C signal, shutting down gracefully...");
        },
        () = terminate => {
            info!("Received SIGTERM signal, shutting down gracefully...");
        },
    }
}
