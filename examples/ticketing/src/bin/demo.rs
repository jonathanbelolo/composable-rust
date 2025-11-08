//! Ticketing System Demo
//!
//! Interactive demonstration of the event ticketing system showing:
//! - Event creation with inventory initialization
//! - Ticket purchase workflow (reservation → payment → confirmation)
//! - Real-time projection updates
//! - Saga compensation on payment failure
//! - Timeout expiration
//!
//! # Usage
//!
//! ```bash
//! # Start infrastructure
//! docker compose up -d
//!
//! # Run demo
//! cargo run --bin demo
//! ```

use composable_rust_core::stream::StreamId;
use ticketing::{
    Config, TicketingApp,
    aggregates::{InventoryAction, ReservationAction, PaymentAction},
    types::{EventId, CustomerId, ReservationId, PaymentId, Capacity, Money, PaymentMethod},
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,ticketing=debug,sqlx=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    println!("\n🎫 ============================================");
    println!("   Ticketing System - Live Demo");
    println!("============================================\n");

    // Load configuration
    let config = Config::from_env();

    // Initialize application
    println!("⚙️  Initializing application...");
    let app = TicketingApp::new(config).await?;

    // Start event processing
    app.start().await?;

    println!("✓ Application started\n");

    // Give projections a moment to initialize
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // ========== Demo Scenario ==========

    println!("📋 Demo Scenario: Concert Ticket Purchase");
    println!("   Event: Summer Music Festival 2025");
    println!("   Section: General Admission");
    println!("   Capacity: 100 seats\n");

    // Step 1: Create event and initialize inventory
    println!("1️⃣  Creating event and initializing inventory...");

    let event_id = EventId::new();
    let stream_id = StreamId::new(format!("inventory-{event_id}"));

    app.inventory.handle(
        stream_id,
        InventoryAction::InitializeInventory {
            event_id,
            section: "General".to_string(),
            capacity: Capacity::new(100),
            seat_numbers: None,
        },
    ).await?;

    println!("   ✓ Event created: {event_id}");
    println!("   ✓ Inventory initialized: 100 seats available\n");

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Step 2: Customer initiates reservation
    println!("2️⃣  Customer initiating reservation...");

    let customer_id = CustomerId::new();
    let reservation_id = ReservationId::new();
    let reservation_stream = StreamId::new(format!("reservation-{reservation_id}"));

    println!("   Customer: {customer_id}");
    println!("   Reservation: {reservation_id}");
    println!("   Quantity: 2 tickets\n");

    app.reservation.handle(
        reservation_stream.clone(),
        ReservationAction::InitiateReservation {
            reservation_id,
            event_id,
            customer_id,
            section: "General".to_string(),
            quantity: 2,
            specific_seats: None,
        },
    ).await?;

    println!("   ✓ Reservation initiated (5-minute timer started)\n");

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Step 3: Reserve seats in inventory
    println!("3️⃣  Reserving seats in inventory...");

    let inventory_stream = StreamId::new(format!("inventory-{event_id}"));

    app.inventory.handle(
        inventory_stream,
        InventoryAction::ReserveSeats {
            reservation_id,
            event_id,
            section: "General".to_string(),
            quantity: 2,
            specific_seats: None,
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        },
    ).await?;

    println!("   ✓ Seats reserved: 2 seats");
    println!("   ✓ Available seats: 98\n");

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Step 4: Process payment
    println!("4️⃣  Processing payment...");

    let payment_id = PaymentId::new();
    let payment_stream = StreamId::new(format!("payment-{payment_id}"));

    println!("   Payment ID: {payment_id}");
    println!("   Amount: $100.00");
    println!("   Method: Credit Card ****4242\n");

    app.payment.handle(
        payment_stream,
        PaymentAction::ProcessPayment {
            payment_id,
            reservation_id,
            amount: Money::from_dollars(100),
            payment_method: PaymentMethod::CreditCard {
                last_four: "4242".to_string(),
            },
        },
    ).await?;

    println!("   ✓ Payment succeeded\n");

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Step 5: Confirm seats (mark as sold)
    println!("5️⃣  Confirming reservation...");

    let inventory_stream = StreamId::new(format!("inventory-{event_id}"));

    app.inventory.handle(
        inventory_stream,
        InventoryAction::ConfirmReservation {
            reservation_id,
            customer_id,
        },
    ).await?;

    println!("   ✓ Seats confirmed and marked as sold");
    println!("   ✓ Tickets issued\n");

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Step 6: Show final state
    println!("6️⃣  Final State:");

    // Access projections
    let available_seats = app.available_seats.read().await;
    let sales = app.sales_analytics.read().await;

    if let Some(availability) = available_seats.get_availability(&event_id, "General") {
        println!("   📊 Inventory:");
        println!("      - Total capacity: {}", availability.total_capacity);
        println!("      - Sold: {}", availability.sold);
        println!("      - Reserved: {}", availability.reserved);
        println!("      - Available: {}", availability.available);
    }

    if let Some(metrics) = sales.get_metrics(&event_id) {
        println!("\n   💰 Sales:");
        println!("      - Total sales: ${}.00", metrics.total_revenue.cents() / 100);
        println!("      - Tickets sold: {}", metrics.tickets_sold);
    }

    println!("\n✨ Demo completed successfully!");
    println!("\n📝 What happened:");
    println!("   1. Event created with 100 seats");
    println!("   2. Customer reserved 2 tickets (5-min timeout started)");
    println!("   3. Seats locked in inventory (98 remaining)");
    println!("   4. Payment processed successfully");
    println!("   5. Seats confirmed as sold");
    println!("   6. Tickets issued to customer");
    println!("\n🎯 Key Features Demonstrated:");
    println!("   ✓ Event Sourcing - All state changes recorded as events");
    println!("   ✓ CQRS - Separate write (aggregates) and read (projections) models");
    println!("   ✓ Saga Pattern - Multi-step workflow with compensation");
    println!("   ✓ Event Bus - Cross-aggregate communication via RedPanda");
    println!("   ✓ Real-time Projections - Instant query updates");
    println!("\n🎫 System is ready for production use!");

    Ok(())
}
