//! Order Processing HTTP API server.
//!
//! Demonstrates HTTP request-response pattern with `send_and_wait_for()`.
//!
//! # Usage
//!
//! Run with in-memory event store:
//! ```bash
//! cargo run --bin order-processing --features http
//! ```
//!
//! Run with PostgreSQL event store:
//! ```bash
//! DATABASE_URL=postgres://user:pass@localhost/db \
//!   cargo run --bin order-processing --features http,postgres
//! ```
//!
//! # API Endpoints
//!
//! - `POST /api/v1/orders` - Place a new order
//! - `GET /api/v1/orders/:id` - Get order details
//! - `POST /api/v1/orders/:id/cancel` - Cancel an order
//! - `POST /api/v1/orders/:id/ship` - Ship an order
//! - `GET /health` - Health check
//!
//! # Example Requests
//!
//! ```bash
//! # Place an order
//! curl -X POST http://localhost:3000/api/v1/orders \
//!   -H "Content-Type: application/json" \
//!   -d '{
//!     "customer_id": "cust-123",
//!     "items": [
//!       {
//!         "product_id": "prod-1",
//!         "name": "Widget",
//!         "quantity": 2,
//!         "unit_price_cents": 1000
//!       }
//!     ]
//!   }'
//!
//! # Get order status
//! curl http://localhost:3000/api/v1/orders/order-abc123
//!
//! # Ship order
//! curl -X POST http://localhost:3000/api/v1/orders/order-abc123/ship \
//!   -H "Content-Type: application/json" \
//!   -d '{"tracking": "TRACK123"}'
//! ```

#![cfg(feature = "http")]

use axum::Router;
use composable_rust_core::environment::SystemClock;
use composable_rust_runtime::Store;
use composable_rust_testing::mocks::InMemoryEventStore;
use composable_rust_web::handlers::health::health_check;
use order_processing::{OrderEnvironment, OrderReducer, OrderState};
use std::sync::Arc;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("=== Order Processing HTTP API Server ===\n");

    // Create event store (in-memory or PostgreSQL based on feature flag)
    let event_store: Arc<dyn composable_rust_core::event_store::EventStore> = {
        #[cfg(feature = "postgres")]
        {
            if let Ok(database_url) = std::env::var("DATABASE_URL") {
                info!("Using PostgreSQL event store: {}", database_url);
                Arc::new(
                    composable_rust_postgres::PostgresEventStore::new(&database_url)
                        .await
                        .expect("Failed to connect to PostgreSQL"),
                )
            } else {
                info!("DATABASE_URL not set, using in-memory event store");
                Arc::new(InMemoryEventStore::new())
            }
        }
        #[cfg(not(feature = "postgres"))]
        {
            info!("Using in-memory event store");
            info!("(Compile with --features postgres for PostgreSQL persistence)\n");
            Arc::new(InMemoryEventStore::new())
        }
    };

    // Create environment with event store and clock
    let clock: Arc<dyn composable_rust_core::environment::Clock> = Arc::new(SystemClock);
    let env = OrderEnvironment::new(Arc::clone(&event_store), Arc::clone(&clock));

    // Create store
    let store = Arc::new(Store::new(
        OrderState::new(),
        OrderReducer::new(),
        env,
    ));

    info!("Store created with order-processing reducer");

    // Build router
    let app = Router::new()
        .route("/health", axum::routing::get(health_check))
        .nest("/api/v1", order_processing::router::order_router(Arc::clone(&store)));

    // Start server
    let addr = "0.0.0.0:3000";
    let listener = tokio::net::TcpListener::bind(addr).await?;

    info!("Server listening on http://{}", addr);
    info!("\nAPI Endpoints:");
    info!("  POST   /api/v1/orders           - Place order");
    info!("  GET    /api/v1/orders/:id       - Get order");
    info!("  POST   /api/v1/orders/:id/cancel - Cancel order");
    info!("  POST   /api/v1/orders/:id/ship  - Ship order");
    info!("  GET    /health                  - Health check\n");

    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(not(feature = "http"))]
fn main() {
    eprintln!("This binary requires the 'http' feature");
    eprintln!("Run with: cargo run --bin order-processing --features http");
    std::process::exit(1);
}
