//! Order Processing example demonstrating event sourcing.
//!
//! This example shows:
//! 1. Placing an order with validation
//! 2. Shipping an order
//! 3. State reconstruction from events (simulating process restart)
//!
//! # Usage
//!
//! Run with in-memory event store (default):
//! ```bash
//! cargo run --bin order-processing
//! ```
//!
//! Run with `PostgreSQL` event store:
//! ```bash
//! DATABASE_URL=postgres://user:pass@localhost/db cargo run --bin order-processing --features postgres
//! ```

#![allow(clippy::expect_used)] // Example code demonstrates error handling with expect
#![allow(clippy::too_many_lines)] // Example main function demonstrates complete workflow

use composable_rust_core::environment::SystemClock;
use composable_rust_core::stream::StreamId;
use composable_rust_runtime::Store;
use composable_rust_testing::mocks::InMemoryEventStore;
use order_processing::{
    CustomerId, LineItem, Money, OrderAction, OrderEnvironment, OrderId, OrderReducer, OrderState,
    OrderStatus,
};
use std::sync::Arc;
use tracing::{Level, info};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("=== Order Processing Example: Event Sourcing Demo ===\n");

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
                info!("DATABASE_URL not set, falling back to in-memory event store");
                Arc::new(InMemoryEventStore::new())
            }
        }
        #[cfg(not(feature = "postgres"))]
        {
            info!("Using in-memory event store (compile with --features postgres for PostgreSQL)");
            Arc::new(InMemoryEventStore::new())
        }
    };

    let clock: Arc<dyn composable_rust_core::environment::Clock> = Arc::new(SystemClock);
    let env = OrderEnvironment::new(Arc::clone(&event_store), Arc::clone(&clock));

    // ========== Part 1: Place an Order ==========
    info!("Part 1: Placing a new order...");

    let order_id = OrderId::new("order-12345".to_string());
    let customer_id = CustomerId::new("customer-001".to_string());

    let store = Store::new(OrderState::new(), OrderReducer::new(), env.clone());

    // Create order with two items
    let items = vec![
        LineItem::new(
            "prod-widget-a".to_string(),
            "Premium Widget A".to_string(),
            2,
            Money::from_dollars(25),
        ),
        LineItem::new(
            "prod-widget-b".to_string(),
            "Deluxe Widget B".to_string(),
            1,
            Money::from_dollars(50),
        ),
    ];

    info!("  Order ID: {}", order_id);
    info!("  Customer: {}", customer_id);
    info!("  Items:");
    for item in &items {
        info!(
            "    - {} x{} @ {} each = {}",
            item.name,
            item.quantity,
            item.unit_price,
            item.total()
        );
    }

    // Send PlaceOrder command
    let handle = store
        .send(OrderAction::PlaceOrder {
            order_id: order_id.clone(),
            customer_id: customer_id.clone(),
            items: items.clone(),
        })
        .await;

    // Wait for effects to complete
    handle?.wait().await;

    // Read state after placing order
    let state_after_place = store.state(Clone::clone).await;
    info!(
        "\n  Order placed successfully! Status: {}, Total: {}",
        state_after_place.status, state_after_place.total
    );

    // ========== Part 2: Ship the Order ==========
    info!("\nPart 2: Shipping the order...");

    let tracking = "TRACK-ABC123XYZ".to_string();
    info!("  Tracking number: {}", tracking);

    let handle = store
        .send(OrderAction::ShipOrder {
            order_id: order_id.clone(),
            tracking: tracking.clone(),
        })
        .await;

    handle?.wait().await;

    let state_after_ship = store.state(Clone::clone).await;
    info!("  Order shipped! Status: {}", state_after_ship.status);

    // ========== Part 3: State Reconstruction from Events ==========
    info!("\nPart 3: Simulating process restart - reconstructing state from events...");

    // Create a new store with fresh state (simulating app restart)
    let new_store = Store::new(OrderState::new(), OrderReducer::new(), env.clone());

    info!("  New store created with empty state");

    // Load events from event store
    let stream_id = StreamId::new(format!("order-{}", order_id.as_str()));
    info!("  Loading events from stream: {}", stream_id);

    // Load all events from the event store
    let serialized_events = event_store
        .load_events(stream_id.clone(), None)
        .await
        .expect("Failed to load events");

    info!("  Found {} events to replay", serialized_events.len());

    // Deserialize and replay events through the reducer
    for (idx, serialized_event) in serialized_events.iter().enumerate() {
        info!(
            "  Replaying event {}/{}: {}",
            idx + 1,
            serialized_events.len(),
            serialized_event.event_type
        );

        // Deserialize the event
        let event =
            OrderAction::from_serialized(serialized_event).expect("Failed to deserialize event");

        // Send event through store (will apply via reducer)
        let handle = new_store.send(event).await;
        handle?.wait().await;
    }

    // Now state should be reconstructed
    let final_state = new_store.state(Clone::clone).await;
    info!(
        "\n  Reconstructed state: Status={}, Items={}, Total={}, Version={}",
        final_state.status,
        final_state.items.len(),
        final_state.total,
        final_state
            .version
            .map_or("None".to_string(), |v| v.value().to_string())
    );

    // Verify reconstruction worked
    assert_eq!(final_state.status, OrderStatus::Shipped);
    assert_eq!(final_state.items.len(), 2);
    assert_eq!(final_state.total, Money::from_dollars(100));
    assert_eq!(
        final_state.version,
        Some(composable_rust_core::stream::Version::new(2)),
        "Version should be 2 after replaying 2 events"
    );

    info!(
        "✓ State successfully reconstructed from {} events!",
        serialized_events.len()
    );

    // ========== Part 4: Demonstrate Validation ==========
    info!("\nPart 4: Demonstrating command validation...");

    // Try to cancel an already-shipped order (should fail validation)
    info!("  Attempting to cancel an already-shipped order...");

    let handle = new_store
        .send(OrderAction::CancelOrder {
            order_id: order_id.clone(),
            reason: "Customer changed mind".to_string(),
        })
        .await;

    handle?.wait().await;

    let state_after_invalid_cancel = new_store.state(Clone::clone).await;
    info!(
        "  Validation prevented cancellation. Status remains: {}",
        state_after_invalid_cancel.status
    );
    if let Some(error) = &state_after_invalid_cancel.last_error {
        info!("  Error tracked in state: {}", error);
    }

    // ========== Summary ==========
    info!("\n=== Summary ===");
    info!("✓ Order was successfully placed with 2 items");
    info!("✓ Order was shipped with tracking number");
    info!("✓ State can be reconstructed from events (event sourcing)");
    info!("✓ Business rules prevent invalid state transitions");
    info!("\nThis example demonstrates:");
    info!("  - Command/Event pattern");
    info!("  - Event persistence to EventStore");
    info!("  - State reconstruction from events");
    info!("  - Business logic validation in reducers");

    Ok(())
}
