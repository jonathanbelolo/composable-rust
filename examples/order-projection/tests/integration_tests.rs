//! Integration tests for Order Projection
//!
//! These tests use testcontainers to spin up a real PostgreSQL database,
//! ensuring the projection works correctly end-to-end.
//!
//! # Requirements
//!
//! Docker must be running to execute these tests.

#![allow(clippy::expect_used)] // Test code uses expect for clear failure messages

use chrono::Utc;
use composable_rust_core::projection::Projection;
use order_processing::{CustomerId, LineItem, Money, OrderAction, OrderId};
use order_projection_example::CustomerOrderHistoryProjection;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;

/// Helper to set up PostgreSQL testcontainer and run migrations
async fn setup_test_db() -> (ContainerAsync<Postgres>, sqlx::PgPool) {
    // Start Postgres container
    let container = Postgres::default()
        .start()
        .await
        .expect("Failed to start postgres container");

    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("Failed to get postgres port");

    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    // Wait for postgres to be ready with retry logic
    let mut retries = 0;
    let max_retries = 60;
    let pool = loop {
        if let Ok(pool) = PgPoolOptions::new()
            .max_connections(5)
            .connect(&connection_string)
            .await
        {
            // Verify with a simple query
            if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                break pool;
            }
        }

        retries += 1;
        if retries >= max_retries {
            panic!("Postgres container failed to become ready after {max_retries} attempts");
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    };

    // Run migrations from projections crate
    sqlx::migrate!("../../projections/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Small delay to ensure migrations are fully applied
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    (container, pool)
}

#[tokio::test]
async fn test_apply_order_placed_event() {
    let (_docker, pool) = setup_test_db().await;
    let projection = CustomerOrderHistoryProjection::new(pool.clone());

    let event = OrderAction::OrderPlaced {
        order_id: OrderId::new("order-1".to_string()),
        customer_id: CustomerId::new("customer-alice".to_string()),
        items: vec![LineItem::new(
            "prod-1".to_string(),
            "Widget".to_string(),
            2,
            Money::from_dollars(25),
        )],
        total: Money::from_dollars(50),
        timestamp: Utc::now(),
    };

    // Apply event
    projection
        .apply_event(&event)
        .await
        .expect("Failed to apply OrderPlaced event");

    // Verify projection was updated
    let order = projection
        .get_order("order-1")
        .await
        .expect("Failed to query order")
        .expect("Order not found");

    assert_eq!(order.id, "order-1");
    assert_eq!(order.customer_id, "customer-alice");
    assert_eq!(order.item_count, 1); // 1 line item
    assert_eq!(order.total_cents, 5000); // $50.00
    assert_eq!(order.status, "placed");
    assert!(order.tracking.is_none());
    assert!(order.cancellation_reason.is_none());
}

#[tokio::test]
async fn test_apply_order_shipped_event() {
    let (_docker, pool) = setup_test_db().await;
    let projection = CustomerOrderHistoryProjection::new(pool.clone());

    // First place an order
    let placed_event = OrderAction::OrderPlaced {
        order_id: OrderId::new("order-2".to_string()),
        customer_id: CustomerId::new("customer-bob".to_string()),
        items: vec![LineItem::new(
            "prod-2".to_string(),
            "Gadget".to_string(),
            3,
            Money::from_dollars(30),
        )],
        total: Money::from_dollars(90),
        timestamp: Utc::now(),
    };

    projection
        .apply_event(&placed_event)
        .await
        .expect("Failed to apply OrderPlaced");

    // Then ship it
    let shipped_event = OrderAction::OrderShipped {
        order_id: OrderId::new("order-2".to_string()),
        tracking: "TRACK-12345".to_string(),
        timestamp: Utc::now(),
    };

    projection
        .apply_event(&shipped_event)
        .await
        .expect("Failed to apply OrderShipped");

    // Verify projection was updated
    let order = projection
        .get_order("order-2")
        .await
        .expect("Failed to query order")
        .expect("Order not found");

    assert_eq!(order.status, "shipped");
    assert_eq!(order.tracking, Some("TRACK-12345".to_string()));
    assert!(order.cancellation_reason.is_none());
}

#[tokio::test]
async fn test_apply_order_cancelled_event() {
    let (_docker, pool) = setup_test_db().await;
    let projection = CustomerOrderHistoryProjection::new(pool.clone());

    // First place an order
    let placed_event = OrderAction::OrderPlaced {
        order_id: OrderId::new("order-3".to_string()),
        customer_id: CustomerId::new("customer-charlie".to_string()),
        items: vec![LineItem::new(
            "prod-3".to_string(),
            "Tool".to_string(),
            1,
            Money::from_dollars(100),
        )],
        total: Money::from_dollars(100),
        timestamp: Utc::now(),
    };

    projection
        .apply_event(&placed_event)
        .await
        .expect("Failed to apply OrderPlaced");

    // Then cancel it
    let cancelled_event = OrderAction::OrderCancelled {
        order_id: OrderId::new("order-3".to_string()),
        reason: "Customer requested".to_string(),
        timestamp: Utc::now(),
    };

    projection
        .apply_event(&cancelled_event)
        .await
        .expect("Failed to apply OrderCancelled");

    // Verify projection was updated
    let order = projection
        .get_order("order-3")
        .await
        .expect("Failed to query order")
        .expect("Order not found");

    assert_eq!(order.status, "cancelled");
    assert_eq!(
        order.cancellation_reason,
        Some("Customer requested".to_string())
    );
    assert!(order.tracking.is_none());
}

#[tokio::test]
async fn test_get_customer_orders() {
    let (_docker, pool) = setup_test_db().await;
    let projection = CustomerOrderHistoryProjection::new(pool.clone());

    // Create multiple orders for the same customer
    for i in 1..=3 {
        let event = OrderAction::OrderPlaced {
            order_id: OrderId::new(format!("order-{i}")),
            customer_id: CustomerId::new("customer-david".to_string()),
            items: vec![LineItem::new(
                format!("prod-{i}"),
                format!("Product {i}"),
                i,
                Money::from_dollars(10),
            )],
            total: Money::from_dollars(10 * i64::from(i)),
            timestamp: Utc::now(),
        };

        projection
            .apply_event(&event)
            .await
            .expect("Failed to apply event");
    }

    // Query customer orders
    let orders = projection
        .get_customer_orders("customer-david")
        .await
        .expect("Failed to query customer orders");

    assert_eq!(orders.len(), 3);

    // Verify all orders belong to the same customer
    for order in &orders {
        assert_eq!(order.customer_id, "customer-david");
        assert_eq!(order.status, "placed");
    }
}

#[tokio::test]
async fn test_get_recent_orders() {
    let (_docker, pool) = setup_test_db().await;
    let projection = CustomerOrderHistoryProjection::new(pool.clone());

    // Create orders for different customers
    for i in 1..=5 {
        let event = OrderAction::OrderPlaced {
            order_id: OrderId::new(format!("recent-order-{i}")),
            customer_id: CustomerId::new(format!("customer-{i}")),
            items: vec![LineItem::new(
                "prod-1".to_string(),
                "Product".to_string(),
                1,
                Money::from_dollars(20),
            )],
            total: Money::from_dollars(20),
            timestamp: Utc::now(),
        };

        projection
            .apply_event(&event)
            .await
            .expect("Failed to apply event");
    }

    // Query recent orders
    let orders = projection
        .get_recent_orders(3)
        .await
        .expect("Failed to query recent orders");

    // Should return at most 3 orders
    assert!(orders.len() <= 3);
}

#[tokio::test]
async fn test_count_by_status() {
    let (_docker, pool) = setup_test_db().await;
    let projection = CustomerOrderHistoryProjection::new(pool.clone());

    // Create orders with different statuses
    // 2 placed orders
    for i in 1..=2 {
        let event = OrderAction::OrderPlaced {
            order_id: OrderId::new(format!("count-order-{i}")),
            customer_id: CustomerId::new(format!("customer-{i}")),
            items: vec![LineItem::new(
                "prod-1".to_string(),
                "Product".to_string(),
                1,
                Money::from_dollars(10),
            )],
            total: Money::from_dollars(10),
            timestamp: Utc::now(),
        };

        projection
            .apply_event(&event)
            .await
            .expect("Failed to apply OrderPlaced");
    }

    // Ship one order
    let shipped_event = OrderAction::OrderShipped {
        order_id: OrderId::new("count-order-1".to_string()),
        tracking: "TRACK-001".to_string(),
        timestamp: Utc::now(),
    };

    projection
        .apply_event(&shipped_event)
        .await
        .expect("Failed to apply OrderShipped");

    // Cancel one order
    let cancelled_event = OrderAction::OrderCancelled {
        order_id: OrderId::new("count-order-2".to_string()),
        reason: "Test cancellation".to_string(),
        timestamp: Utc::now(),
    };

    projection
        .apply_event(&cancelled_event)
        .await
        .expect("Failed to apply OrderCancelled");

    // Count by status
    let placed_count = projection
        .count_by_status("placed")
        .await
        .expect("Failed to count placed orders");

    let shipped_count = projection
        .count_by_status("shipped")
        .await
        .expect("Failed to count shipped orders");

    let cancelled_count = projection
        .count_by_status("cancelled")
        .await
        .expect("Failed to count cancelled orders");

    assert_eq!(placed_count, 0); // All orders have been updated
    assert_eq!(shipped_count, 1);
    assert_eq!(cancelled_count, 1);
}

#[tokio::test]
async fn test_rebuild_projection() {
    let (_docker, pool) = setup_test_db().await;
    let projection = CustomerOrderHistoryProjection::new(pool.clone());

    // Create some orders
    for i in 1..=3 {
        let event = OrderAction::OrderPlaced {
            order_id: OrderId::new(format!("rebuild-order-{i}")),
            customer_id: CustomerId::new("rebuild-customer".to_string()),
            items: vec![LineItem::new(
                "prod-1".to_string(),
                "Product".to_string(),
                1,
                Money::from_dollars(10),
            )],
            total: Money::from_dollars(10),
            timestamp: Utc::now(),
        };

        projection
            .apply_event(&event)
            .await
            .expect("Failed to apply event");
    }

    // Verify orders exist
    let orders_before = projection
        .get_customer_orders("rebuild-customer")
        .await
        .expect("Failed to query orders");
    assert_eq!(orders_before.len(), 3);

    // Rebuild projection (clears all data)
    projection
        .rebuild()
        .await
        .expect("Failed to rebuild projection");

    // Verify all orders are gone
    let orders_after = projection
        .get_customer_orders("rebuild-customer")
        .await
        .expect("Failed to query orders");
    assert_eq!(orders_after.len(), 0);
}

#[tokio::test]
async fn test_idempotency_same_event_twice() {
    let (_docker, pool) = setup_test_db().await;
    let projection = CustomerOrderHistoryProjection::new(pool.clone());

    let event = OrderAction::OrderPlaced {
        order_id: OrderId::new("idempotent-order".to_string()),
        customer_id: CustomerId::new("idempotent-customer".to_string()),
        items: vec![LineItem::new(
            "prod-1".to_string(),
            "Product".to_string(),
            1,
            Money::from_dollars(10),
        )],
        total: Money::from_dollars(10),
        timestamp: Utc::now(),
    };

    // Apply event first time
    projection
        .apply_event(&event)
        .await
        .expect("Failed to apply event first time");

    // Apply same event again (should be idempotent due to ON CONFLICT)
    projection
        .apply_event(&event)
        .await
        .expect("Failed to apply event second time");

    // Verify only one order exists
    let orders = projection
        .get_customer_orders("idempotent-customer")
        .await
        .expect("Failed to query orders");

    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].id, "idempotent-order");
}

#[tokio::test]
async fn test_projection_name() {
    let (_docker, pool) = setup_test_db().await;
    let projection = CustomerOrderHistoryProjection::new(pool);

    assert_eq!(projection.name(), "customer_order_history");
}

#[tokio::test]
async fn test_ignore_commands() {
    let (_docker, pool) = setup_test_db().await;
    let projection = CustomerOrderHistoryProjection::new(pool.clone());

    // Commands should be ignored by projections
    let command = OrderAction::PlaceOrder {
        order_id: OrderId::new("command-order".to_string()),
        customer_id: CustomerId::new("command-customer".to_string()),
        items: vec![],
    };

    // Should not fail, just ignore
    projection
        .apply_event(&command)
        .await
        .expect("Commands should be ignored without error");

    // Verify no order was created
    let order = projection
        .get_order("command-order")
        .await
        .expect("Failed to query");
    assert!(order.is_none());
}
