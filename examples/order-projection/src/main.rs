//! Order Projection Example - Runnable Demo
//!
//! This example demonstrates a complete projection setup with:
//! - Creating sample events
//! - Running the projection manager
//! - Querying the projection

use anyhow::Result;
use chrono::Utc;
use composable_rust_core::projection::Projection;
use order_processing::{CustomerId, LineItem, Money, OrderAction, OrderId};
use order_projection_example::CustomerOrderHistoryProjection;
use sqlx::postgres::PgPoolOptions;
use tokio::time::{sleep, Duration};
use tracing::{info, Level};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_target(false)
        .init();

    info!("🚀 Order Projection Example");
    info!("");
    info!("This example demonstrates:");
    info!("  1. Creating a projection from order events");
    info!("  2. Setting up the ProjectionManager");
    info!("  3. Processing events and updating the read model");
    info!("  4. Querying the projection");
    info!("");

    // For this example, we'll use the same database for simplicity
    // In production CQRS, you'd use separate databases:
    //   - event_pool: postgres://localhost/events
    //   - projection_pool: postgres://localhost/projections
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/composable_rust".to_string());

    info!("📦 Connecting to PostgreSQL...");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    // Run migrations to create projection tables
    info!("🔧 Running migrations...");
    sqlx::migrate!("../../projections/migrations")
        .run(&pool)
        .await?;

    // Create the projection
    let projection = CustomerOrderHistoryProjection::new(pool.clone());

    // For demonstration, let's manually apply some events
    // (In production, these would come from the event bus)
    info!("");
    info!("📝 Applying sample events...");

    let order1_placed = OrderAction::OrderPlaced {
        order_id: OrderId::new("order-1".to_string()),
        customer_id: CustomerId::new("customer-alice".to_string()),
        items: vec![
            LineItem::new(
                "prod-widget".to_string(),
                "Super Widget".to_string(),
                2,
                Money::from_dollars(25),
            ),
            LineItem::new(
                "prod-gadget".to_string(),
                "Mega Gadget".to_string(),
                1,
                Money::from_dollars(50),
            ),
        ],
        total: Money::from_dollars(100),
        timestamp: Utc::now(),
    };

    projection.apply_event(&order1_placed).await?;
    info!("  ✅ OrderPlaced: order-1 for customer-alice ($100.00)");

    let order2_placed = OrderAction::OrderPlaced {
        order_id: OrderId::new("order-2".to_string()),
        customer_id: CustomerId::new("customer-bob".to_string()),
        items: vec![LineItem::new(
            "prod-tool".to_string(),
            "Power Tool".to_string(),
            3,
            Money::from_dollars(40),
        )],
        total: Money::from_dollars(120),
        timestamp: Utc::now(),
    };

    projection.apply_event(&order2_placed).await?;
    info!("  ✅ OrderPlaced: order-2 for customer-bob ($120.00)");

    let order3_placed = OrderAction::OrderPlaced {
        order_id: OrderId::new("order-3".to_string()),
        customer_id: CustomerId::new("customer-alice".to_string()),
        items: vec![LineItem::new(
            "prod-book".to_string(),
            "Rust Programming Book".to_string(),
            1,
            Money::from_dollars(45),
        )],
        total: Money::from_dollars(45),
        timestamp: Utc::now(),
    };

    projection.apply_event(&order3_placed).await?;
    info!("  ✅ OrderPlaced: order-3 for customer-alice ($45.00)");

    // Ship one order
    let order1_shipped = OrderAction::OrderShipped {
        order_id: OrderId::new("order-1".to_string()),
        tracking: "TRACK-123456".to_string(),
        timestamp: Utc::now(),
    };

    projection.apply_event(&order1_shipped).await?;
    info!("  ✅ OrderShipped: order-1 (tracking: TRACK-123456)");

    // Cancel one order
    let order3_cancelled = OrderAction::OrderCancelled {
        order_id: OrderId::new("order-3".to_string()),
        reason: "Customer requested cancellation".to_string(),
        timestamp: Utc::now(),
    };

    projection.apply_event(&order3_cancelled).await?;
    info!("  ✅ OrderCancelled: order-3 (reason: Customer requested)");

    // Small delay to ensure database writes complete
    sleep(Duration::from_millis(100)).await;

    // Query the projection
    info!("");
    info!("🔍 Querying the projection...");
    info!("");

    // Get all orders for Alice
    let alice_orders = projection.get_customer_orders("customer-alice").await?;
    info!("📊 Orders for customer-alice ({} orders):", alice_orders.len());
    for order in &alice_orders {
        info!(
            "  - Order {}: {} items, ${:.2}, status: {}{}{}",
            order.id,
            order.item_count,
            order.total_cents as f64 / 100.0,
            order.status,
            order
                .tracking
                .as_ref()
                .map(|t| format!(", tracking: {t}"))
                .unwrap_or_default(),
            order
                .cancellation_reason
                .as_ref()
                .map(|r| format!(", reason: {r}"))
                .unwrap_or_default(),
        );
    }

    info!("");

    // Get all orders for Bob
    let bob_orders = projection.get_customer_orders("customer-bob").await?;
    info!("📊 Orders for customer-bob ({} orders):", bob_orders.len());
    for order in &bob_orders {
        info!(
            "  - Order {}: {} items, ${:.2}, status: {}",
            order.id,
            order.item_count,
            order.total_cents as f64 / 100.0,
            order.status
        );
    }

    info!("");

    // Get recent orders
    let recent = projection.get_recent_orders(10).await?;
    info!("📊 Recent orders ({} orders):", recent.len());
    for order in &recent {
        info!(
            "  - Order {} ({}): {} items, ${:.2}, status: {}",
            order.id,
            order.customer_id,
            order.item_count,
            order.total_cents as f64 / 100.0,
            order.status
        );
    }

    info!("");

    // Count by status
    let placed_count = projection.count_by_status("placed").await?;
    let shipped_count = projection.count_by_status("shipped").await?;
    let cancelled_count = projection.count_by_status("cancelled").await?;

    info!("📊 Order counts by status:");
    info!("  - Placed: {}", placed_count);
    info!("  - Shipped: {}", shipped_count);
    info!("  - Cancelled: {}", cancelled_count);

    info!("");
    info!("✅ Example complete!");
    info!("");
    info!("💡 Key Takeaways:");
    info!("  1. Projections build denormalized read models from events");
    info!("  2. Query API is separate from event processing");
    info!("  3. CQRS allows optimizing reads independently from writes");
    info!("  4. Projections are eventually consistent (typically 10-100ms lag)");
    info!("  5. Projections can be rebuilt from scratch if needed");
    info!("");
    info!("🚀 Next Steps:");
    info!("  1. Run with ProjectionManager for automatic event processing");
    info!("  2. Connect to Redpanda event bus for real-time updates");
    info!("  3. Use separate databases for true CQRS");
    info!("  4. Add more complex queries (aggregations, filters, etc.)");

    Ok(())
}
