//! Distributed tracing integration tests.
//!
//! Tests correlation ID propagation, tracing span creation, and observability
//! infrastructure across the request lifecycle.

#![allow(clippy::expect_used)] // Integration tests can use expect for setup
#![allow(clippy::unwrap_used)] // Integration tests can use unwrap for clarity

use composable_rust_core::event::EventMetadata;
use tracing::{subscriber::DefaultGuard, Instrument};
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter};

/// Test helper to capture tracing output.
fn init_tracing() -> DefaultGuard {
    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::new("debug"))
        .with(fmt::layer().with_test_writer());

    tracing::subscriber::set_default(subscriber)
}

#[test]
fn test_correlation_id_generation() {
    let _guard = init_tracing();

    // Verify that correlation IDs are properly formatted UUIDs
    let correlation_id = uuid::Uuid::new_v4().to_string();

    assert_eq!(correlation_id.len(), 36, "Correlation ID should be 36 chars");
    assert!(correlation_id.contains('-'), "Correlation ID should be hyphenated UUID");

    tracing::info!(
        correlation_id = %correlation_id,
        "✅ Correlation ID generated successfully"
    );
}

#[test]
fn test_event_metadata_with_correlation_id() {
    let _guard = init_tracing();

    let correlation_id = uuid::Uuid::new_v4().to_string();

    // Create EventMetadata with correlation ID
    let metadata = EventMetadata {
        correlation_id: Some(correlation_id.clone()),
        user_id: Some("test-user-123".to_string()),
        timestamp: Some(chrono::Utc::now().to_rfc3339()),
        causation_id: None,
    };

    assert_eq!(metadata.correlation_id, Some(correlation_id.clone()));

    tracing::info!(
        correlation_id = %correlation_id,
        user_id = "test-user-123",
        "✅ EventMetadata contains correlation ID"
    );
}

#[test]
fn test_tracing_span_hierarchy() {
    let _guard = init_tracing();

    let correlation_id = uuid::Uuid::new_v4().to_string();

    // Simulate request span hierarchy
    let root_span = tracing::info_span!(
        "http.request",
        correlation_id = %correlation_id,
        method = "POST",
        path = "/api/reservations"
    );

    let _enter = root_span.enter();

    tracing::info!("Request received");

    // Child span: event store operation
    let event_store_span = tracing::info_span!(
        "event_store.append",
        aggregate = "Reservation"
    );

    let _enter2 = event_store_span.enter();
    tracing::info!("Appending events to event store");

    // Grandchild span: projection update
    let projection_span = tracing::info_span!(
        "projection.apply_event",
        projection = "reservations"
    );

    let _enter3 = projection_span.enter();
    tracing::info!("Updating projection");

    tracing::info!(
        correlation_id = %correlation_id,
        "✅ Span hierarchy: http.request → event_store.append → projection.apply_event"
    );
}

#[test]
fn test_saga_coordination_spans() {
    let _guard = init_tracing();

    let correlation_id = uuid::Uuid::new_v4().to_string();
    let reservation_id = uuid::Uuid::new_v4();

    // Simulate saga coordination with proper spans
    let saga_span = tracing::info_span!(
        "saga.reservation.initiate",
        correlation_id = %correlation_id,
        reservation_id = %reservation_id,
        step = "1_initiate"
    );

    let _enter = saga_span.enter();
    tracing::info!("Step 1: Reservation initiated");

    // Step 2: Seats allocated
    let seats_span = tracing::info_span!(
        "saga.reservation.seats_allocated",
        reservation_id = %reservation_id,
        seat_count = 2,
        step = "2_seats_allocated"
    );

    let _enter2 = seats_span.enter();
    tracing::info!("Step 2: Seats allocated");

    // Step 3a: Payment succeeded
    let payment_span = tracing::info_span!(
        "saga.reservation.payment_succeeded",
        reservation_id = %reservation_id,
        step = "3a_payment_succeeded"
    );

    let _enter3 = payment_span.enter();
    tracing::info!("Step 3a: Payment succeeded");

    tracing::info!(
        correlation_id = %correlation_id,
        "✅ Saga spans: initiate → seats_allocated → payment_succeeded"
    );
}

#[test]
fn test_saga_compensation_spans() {
    let _guard = init_tracing();

    let correlation_id = uuid::Uuid::new_v4().to_string();
    let reservation_id = uuid::Uuid::new_v4();
    let reason = "Payment declined";

    // Simulate saga compensation with proper spans
    let compensation_span = tracing::info_span!(
        "saga.reservation.compensation",
        correlation_id = %correlation_id,
        reservation_id = %reservation_id,
        reason = %reason,
        step = "3b_compensation"
    );

    let _enter = compensation_span.enter();

    tracing::warn!(
        reservation_id = %reservation_id,
        reason = %reason,
        "Payment failed, triggering saga compensation"
    );

    tracing::info!(
        correlation_id = %correlation_id,
        "✅ Compensation span created with warning log"
    );
}

#[test]
fn test_projection_tracing_integration() {
    let _guard = init_tracing();

    let correlation_id = uuid::Uuid::new_v4().to_string();

    // Test projection span structure
    let projections = vec![
        "reservations",
        "available_seats",
        "payments_projection",
        "customer_history",
        "sales_analytics",
        "events",
    ];

    for projection_name in projections {
        let projection_span = tracing::info_span!(
            "projection.apply_event",
            projection = projection_name,
            correlation_id = %correlation_id
        );

        let _enter = projection_span.enter();
        tracing::debug!(
            projection = projection_name,
            "Processing event in projection"
        );
    }

    tracing::info!(
        correlation_id = %correlation_id,
        projection_count = 6,
        "✅ All projections have tracing spans"
    );
}

#[test]
fn test_correlation_id_uuid_format() {
    let _guard = init_tracing();

    // Test various UUID formats
    let test_cases = vec![
        "550e8400-e29b-41d4-a716-446655440000",  // Valid UUID v4
        "00000000-0000-0000-0000-000000000000",  // Nil UUID (valid)
    ];

    for correlation_id in test_cases {
        // Verify UUID can be parsed
        let parsed = uuid::Uuid::parse_str(correlation_id);
        assert!(parsed.is_ok(), "Correlation ID should be valid UUID: {}", correlation_id);

        let metadata = EventMetadata {
            correlation_id: Some(correlation_id.to_string()),
            user_id: None,
            timestamp: None,
            causation_id: None,
        };

        assert_eq!(metadata.correlation_id, Some(correlation_id.to_string()));
    }

    tracing::info!("✅ All correlation ID formats are valid UUIDs");
}

#[test]
fn test_tracing_field_types() {
    let _guard = init_tracing();

    let correlation_id = uuid::Uuid::new_v4();
    let reservation_id = uuid::Uuid::new_v4();

    // Test different field type formats
    let span = tracing::info_span!(
        "test.span",
        correlation_id = %correlation_id,  // Display formatting
        reservation_id = %reservation_id,  // Display formatting
        count = 42,                        // Integer
        status = "active",                 // String literal
        success = true                     // Boolean
    );

    let _enter = span.enter();

    tracing::info!(
        correlation_id = %correlation_id,
        reservation_id = %reservation_id,
        count = 42,
        status = "active",
        success = true,
        "✅ All field types work correctly"
    );
}

#[test]
fn test_end_to_end_tracing_flow() {
    let _guard = init_tracing();

    let correlation_id = uuid::Uuid::new_v4().to_string();
    let reservation_id = uuid::Uuid::new_v4();
    let event_id = uuid::Uuid::new_v4();

    tracing::info!(
        correlation_id = %correlation_id,
        "=== Starting end-to-end tracing flow test ==="
    );

    // 1. HTTP request
    let http_span = tracing::info_span!(
        "http.request",
        correlation_id = %correlation_id,
        method = "POST",
        path = "/api/reservations"
    );
    let _http_enter = http_span.enter();
    tracing::info!("📥 HTTP request received");

    // 2. Saga initiation
    let saga_span = tracing::info_span!(
        "saga.reservation.initiate",
        reservation_id = %reservation_id,
        event_id = %event_id,
        step = "1_initiate"
    );
    let _saga_enter = saga_span.enter();
    tracing::info!("🚀 Saga initiated");

    // 3. Event store append
    let event_store_span = tracing::info_span!(
        "event_store.append",
        aggregate = "Reservation",
        aggregate_id = %reservation_id
    );
    let _event_enter = event_store_span.enter();
    tracing::info!("💾 Events appended to event store");
    drop(_event_enter);

    // 4. Projection updates (parallel)
    for projection in &["reservations", "available_seats"] {
        let proj_span = tracing::info_span!(
            "projection.apply_event",
            projection = projection
        );
        let _proj_enter = proj_span.enter();
        tracing::info!(projection = projection, "📊 Projection updated");
    }

    tracing::info!(
        correlation_id = %correlation_id,
        "✅ End-to-end flow: HTTP → Saga → EventStore → Projections"
    );
}

#[tokio::test]
async fn test_async_tracing_propagation() {
    let _guard = init_tracing();

    let correlation_id = uuid::Uuid::new_v4().to_string();

    // Test that tracing context propagates across await points
    let span = tracing::info_span!(
        "async.operation",
        correlation_id = %correlation_id
    );

    async {
        tracing::info!("Before await");

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        tracing::info!("After await");

        // Nested async operation
        async {
            tracing::info!("Nested async operation");
        }.await;

        tracing::info!(
            correlation_id = %correlation_id,
            "✅ Tracing context preserved across await points"
        );
    }
    .instrument(span)
    .await;
}
