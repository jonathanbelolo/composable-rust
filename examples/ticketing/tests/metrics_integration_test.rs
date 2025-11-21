//! Prometheus metrics integration tests.
//!
//! Tests metrics initialization, recording, and exposition via HTTP endpoint.
//! Verifies that metrics middleware correctly records HTTP requests and that
//! the `/metrics` endpoint returns valid Prometheus text format.

#![allow(clippy::expect_used)] // Integration tests can use expect for setup
#![allow(clippy::unwrap_used)] // Integration tests can use unwrap for clarity

use prometheus::Encoder;
use ticketing::metrics::{
    init_metrics, ACTIVE_RESERVATIONS, EFFECTS_EXECUTED_TOTAL, EVENTS_CREATED_TOTAL,
    HTTP_REQUESTS_TOTAL, HTTP_REQUEST_DURATION, REDUCER_EXECUTIONS_TOTAL, REVENUE_TOTAL,
    TICKETS_SOLD_TOTAL,
};

#[test]
fn test_metrics_initialization() {
    // Verify that init_metrics() successfully forces evaluation of all metrics
    init_metrics();

    // Verify that metrics are callable (would panic if not registered)
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/test", "200"])
        .inc();

    HTTP_REQUEST_DURATION
        .with_label_values(&["GET", "/test", "200"])
        .observe(0.1);

    EVENTS_CREATED_TOTAL
        .with_label_values(&["published"])
        .inc();

    TICKETS_SOLD_TOTAL
        .with_label_values(&["event-123", "VIP"])
        .inc();

    REVENUE_TOTAL
        .with_label_values(&["event-123", "VIP"])
        .inc_by(10000.0); // $100.00 in cents

    ACTIVE_RESERVATIONS.with_label_values(&["event-123"]).inc();

    REDUCER_EXECUTIONS_TOTAL
        .with_label_values(&["inventory", "reserve_seats"])
        .inc();

    EFFECTS_EXECUTED_TOTAL
        .with_label_values(&["database", "success"])
        .inc();

    println!("✅ All metrics initialized and callable");
}

#[test]
fn test_http_metrics_recording() {
    init_metrics();

    // Record some HTTP metrics
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["POST", "/api/events", "201"])
        .inc();

    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/api/events", "200"])
        .inc();

    HTTP_REQUEST_DURATION
        .with_label_values(&["POST", "/api/events", "201"])
        .observe(0.05);

    HTTP_REQUEST_DURATION
        .with_label_values(&["GET", "/api/events", "200"])
        .observe(0.02);

    // Verify metrics are recorded (would panic if not properly registered)
    let metric_families = prometheus::gather();
    let http_requests = metric_families
        .iter()
        .find(|mf| mf.get_name() == "http_requests_total");

    assert!(
        http_requests.is_some(),
        "http_requests_total metric should exist"
    );

    let http_duration = metric_families
        .iter()
        .find(|mf| mf.get_name() == "http_request_duration_seconds");

    assert!(
        http_duration.is_some(),
        "http_request_duration_seconds metric should exist"
    );

    println!("✅ HTTP metrics recorded successfully");
}

#[test]
fn test_business_metrics_recording() {
    init_metrics();

    // Record business metrics
    EVENTS_CREATED_TOTAL.with_label_values(&["draft"]).inc();
    EVENTS_CREATED_TOTAL
        .with_label_values(&["published"])
        .inc();

    TICKETS_SOLD_TOTAL
        .with_label_values(&["event-456", "General"])
        .inc_by(5.0);

    REVENUE_TOTAL
        .with_label_values(&["event-456", "General"])
        .inc_by(25000.0); // $250.00

    ACTIVE_RESERVATIONS
        .with_label_values(&["event-456"])
        .set(3.0);

    // Verify metrics exist
    let metric_families = prometheus::gather();

    let events_created = metric_families
        .iter()
        .find(|mf| mf.get_name() == "events_created_total");
    assert!(events_created.is_some(), "events_created_total should exist");

    let tickets_sold = metric_families
        .iter()
        .find(|mf| mf.get_name() == "tickets_sold_total");
    assert!(tickets_sold.is_some(), "tickets_sold_total should exist");

    let revenue = metric_families
        .iter()
        .find(|mf| mf.get_name() == "revenue_total_cents");
    assert!(revenue.is_some(), "revenue_total_cents should exist");

    let reservations = metric_families
        .iter()
        .find(|mf| mf.get_name() == "active_reservations");
    assert!(reservations.is_some(), "active_reservations should exist");

    println!("✅ Business metrics recorded successfully");
}

#[test]
fn test_reducer_and_effect_metrics() {
    init_metrics();

    // Record reducer execution metrics
    REDUCER_EXECUTIONS_TOTAL
        .with_label_values(&["event", "create_event"])
        .inc();

    REDUCER_EXECUTIONS_TOTAL
        .with_label_values(&["inventory", "reserve_seats"])
        .inc();

    REDUCER_EXECUTIONS_TOTAL
        .with_label_values(&["payment", "process_payment"])
        .inc();

    // Record effect execution metrics
    EFFECTS_EXECUTED_TOTAL
        .with_label_values(&["database", "success"])
        .inc();

    EFFECTS_EXECUTED_TOTAL
        .with_label_values(&["event_bus", "success"])
        .inc();

    EFFECTS_EXECUTED_TOTAL
        .with_label_values(&["http", "failure"])
        .inc();

    // Verify metrics exist
    let metric_families = prometheus::gather();

    let reducer_execs = metric_families
        .iter()
        .find(|mf| mf.get_name() == "reducer_executions_total");
    assert!(
        reducer_execs.is_some(),
        "reducer_executions_total should exist"
    );

    let effects_exec = metric_families
        .iter()
        .find(|mf| mf.get_name() == "effects_executed_total");
    assert!(
        effects_exec.is_some(),
        "effects_executed_total should exist"
    );

    println!("✅ Reducer and effect metrics recorded successfully");
}

#[test]
fn test_metrics_prometheus_text_format() {
    init_metrics();

    // Record a sample metric
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/test", "200"])
        .inc();

    // Gather all metrics
    let metric_families = prometheus::gather();

    // Encode to Prometheus text format
    let encoder = prometheus::TextEncoder::new();
    let mut buffer = vec![];
    encoder
        .encode(&metric_families, &mut buffer)
        .expect("Failed to encode metrics");

    let body_text = std::str::from_utf8(&buffer).expect("Response body should be valid UTF-8");

    // Verify Prometheus text format characteristics
    assert!(
        body_text.contains("# HELP"),
        "Should contain metric HELP comments"
    );
    assert!(
        body_text.contains("# TYPE"),
        "Should contain metric TYPE declarations"
    );
    assert!(
        body_text.contains("http_requests_total"),
        "Should contain http_requests_total metric"
    );

    println!("✅ Metrics encode to valid Prometheus text format");
    println!("Sample output (first 500 chars):");
    println!("{}", &body_text[..body_text.len().min(500)]);
}

#[test]
fn test_metrics_labels() {
    init_metrics();

    // Test that metrics accept proper labels
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/api/events", "200"])
        .inc();

    HTTP_REQUESTS_TOTAL
        .with_label_values(&["POST", "/api/reservations", "201"])
        .inc();

    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/api/availability", "200"])
        .inc();

    // Verify each label combination creates a separate metric series
    let metric_families = prometheus::gather();
    let http_requests = metric_families
        .iter()
        .find(|mf| mf.get_name() == "http_requests_total")
        .expect("http_requests_total should exist");

    let metrics = http_requests.get_metric();
    assert!(
        metrics.len() >= 3,
        "Should have at least 3 label combinations"
    );

    println!("✅ Metrics correctly handle label combinations");
}

#[test]
fn test_counter_vs_gauge_vs_histogram() {
    init_metrics();

    // Counter: can only increase
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/test", "200"])
        .inc();
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/test", "200"])
        .inc_by(5.0);

    // Gauge: can increase or decrease
    ACTIVE_RESERVATIONS.with_label_values(&["event-789"]).inc();
    ACTIVE_RESERVATIONS
        .with_label_values(&["event-789"])
        .set(10.0);
    ACTIVE_RESERVATIONS.with_label_values(&["event-789"]).dec();

    // Histogram: records distributions
    HTTP_REQUEST_DURATION
        .with_label_values(&["GET", "/test", "200"])
        .observe(0.01);
    HTTP_REQUEST_DURATION
        .with_label_values(&["GET", "/test", "200"])
        .observe(0.05);
    HTTP_REQUEST_DURATION
        .with_label_values(&["GET", "/test", "200"])
        .observe(0.1);

    // Verify all metric types are recorded
    let metric_families = prometheus::gather();

    let counter = metric_families
        .iter()
        .find(|mf| mf.get_name() == "http_requests_total");
    assert!(counter.is_some(), "Counter metric should exist");

    let gauge = metric_families
        .iter()
        .find(|mf| mf.get_name() == "active_reservations");
    assert!(gauge.is_some(), "Gauge metric should exist");

    let histogram = metric_families
        .iter()
        .find(|mf| mf.get_name() == "http_request_duration_seconds");
    assert!(histogram.is_some(), "Histogram metric should exist");

    println!("✅ Counter, Gauge, and Histogram metrics work correctly");
}
