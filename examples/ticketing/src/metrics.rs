//! Prometheus metrics for the ticketing system.
//!
//! This module provides comprehensive metrics covering:
//! - HTTP request metrics (counter, duration)
//! - Business metrics (tickets sold, revenue, events created)
//! - Reducer execution metrics
//! - Effect execution metrics
//! - Database metrics (query duration, connection pool)
//! - DLQ metrics

use std::sync::LazyLock;
use prometheus::{
    register_counter_vec, register_gauge_vec, register_histogram_vec, CounterVec, GaugeVec,
    HistogramVec,
};

// ============================================================================
// HTTP Request Metrics
// ============================================================================

/// HTTP request counter (total requests by method, path, status).
#[allow(clippy::expect_used)] // Metric registration failure is unrecoverable
pub static HTTP_REQUESTS_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "http_requests_total",
        "Total HTTP requests",
        &["method", "path", "status"]
    )
    .expect("Failed to register http_requests_total metric")
});

/// HTTP request duration histogram (request latency by method, path, status).
#[allow(clippy::expect_used)] // Metric registration failure is unrecoverable
pub static HTTP_REQUEST_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "http_request_duration_seconds",
        "HTTP request duration in seconds",
        &["method", "path", "status"]
    )
    .expect("Failed to register http_request_duration_seconds metric")
});

// ============================================================================
// Business Metrics
// ============================================================================

/// Events created counter (total events created by status).
#[allow(clippy::expect_used)] // Metric registration failure is unrecoverable
pub static EVENTS_CREATED_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "events_created_total",
        "Total events created",
        &["status"]
    )
    .expect("Failed to register events_created_total metric")
});

/// Tickets sold counter (total tickets sold by `event_id` and tier).
#[allow(clippy::expect_used)] // Metric registration failure is unrecoverable
pub static TICKETS_SOLD_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "tickets_sold_total",
        "Total tickets sold",
        &["event_id", "tier"]
    )
    .expect("Failed to register tickets_sold_total metric")
});

/// Revenue counter (total revenue in cents by `event_id` and tier).
#[allow(clippy::expect_used)] // Metric registration failure is unrecoverable
pub static REVENUE_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "revenue_total_cents",
        "Total revenue in cents",
        &["event_id", "tier"]
    )
    .expect("Failed to register revenue_total_cents metric")
});

/// Active reservations gauge (current active reservations by `event_id`).
#[allow(clippy::expect_used)] // Metric registration failure is unrecoverable
pub static ACTIVE_RESERVATIONS: LazyLock<GaugeVec> = LazyLock::new(|| {
    register_gauge_vec!(
        "active_reservations",
        "Current active reservations",
        &["event_id"]
    )
    .expect("Failed to register active_reservations metric")
});

// ============================================================================
// Reducer Metrics
// ============================================================================

/// Reducer executions counter (total reducer executions by aggregate and `action_type`).
#[allow(clippy::expect_used)] // Metric registration failure is unrecoverable
pub static REDUCER_EXECUTIONS_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "reducer_executions_total",
        "Total reducer executions",
        &["aggregate", "action_type"]
    )
    .expect("Failed to register reducer_executions_total metric")
});

/// Reducer duration histogram (reducer execution duration by aggregate and `action_type`).
#[allow(clippy::expect_used)] // Metric registration failure is unrecoverable
pub static REDUCER_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "reducer_duration_seconds",
        "Reducer execution duration",
        &["aggregate", "action_type"]
    )
    .expect("Failed to register reducer_duration_seconds metric")
});

// ============================================================================
// Effect Metrics
// ============================================================================

/// Effects executed counter (total effects executed by `effect_type` and status).
#[allow(clippy::expect_used)] // Metric registration failure is unrecoverable
pub static EFFECTS_EXECUTED_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "effects_executed_total",
        "Total effects executed",
        &["effect_type", "status"]
    )
    .expect("Failed to register effects_executed_total metric")
});

/// Effect duration histogram (effect execution duration by `effect_type`).
#[allow(clippy::expect_used)] // Metric registration failure is unrecoverable
pub static EFFECT_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "effect_duration_seconds",
        "Effect execution duration",
        &["effect_type"]
    )
    .expect("Failed to register effect_duration_seconds metric")
});

// ============================================================================
// Database Metrics
// ============================================================================

/// Database query duration histogram (query duration by `query_type`).
#[allow(clippy::expect_used)] // Metric registration failure is unrecoverable
pub static DB_QUERY_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "db_query_duration_seconds",
        "Database query duration",
        &["query_type"]
    )
    .expect("Failed to register db_query_duration_seconds metric")
});

/// Database connection pool size gauge (pool size by database).
#[allow(clippy::expect_used)] // Metric registration failure is unrecoverable
pub static DB_CONNECTION_POOL_SIZE: LazyLock<GaugeVec> = LazyLock::new(|| {
    register_gauge_vec!(
        "db_connection_pool_size",
        "Database connection pool size",
        &["database"]
    )
    .expect("Failed to register db_connection_pool_size metric")
});

/// Database connection pool idle connections gauge (idle connections by database).
#[allow(clippy::expect_used)] // Metric registration failure is unrecoverable
pub static DB_CONNECTION_POOL_IDLE: LazyLock<GaugeVec> = LazyLock::new(|| {
    register_gauge_vec!(
        "db_connection_pool_idle",
        "Database connection pool idle connections",
        &["database"]
    )
    .expect("Failed to register db_connection_pool_idle metric")
});

// ============================================================================
// DLQ Metrics
// ============================================================================

/// DLQ entries gauge (total DLQ entries by status).
#[allow(clippy::expect_used)] // Metric registration failure is unrecoverable
pub static DLQ_ENTRIES_TOTAL: LazyLock<GaugeVec> = LazyLock::new(|| {
    register_gauge_vec!(
        "dlq_entries_total",
        "Total DLQ entries",
        &["status"]
    )
    .expect("Failed to register dlq_entries_total metric")
});

/// Initialize all metrics by forcing lazy evaluation.
///
/// This ensures all metrics are registered at startup.
pub fn init_metrics() {
    // Force lazy evaluation of all metrics
    let _ = &*HTTP_REQUESTS_TOTAL;
    let _ = &*HTTP_REQUEST_DURATION;
    let _ = &*EVENTS_CREATED_TOTAL;
    let _ = &*TICKETS_SOLD_TOTAL;
    let _ = &*REVENUE_TOTAL;
    let _ = &*ACTIVE_RESERVATIONS;
    let _ = &*REDUCER_EXECUTIONS_TOTAL;
    let _ = &*REDUCER_DURATION;
    let _ = &*EFFECTS_EXECUTED_TOTAL;
    let _ = &*EFFECT_DURATION;
    let _ = &*DB_QUERY_DURATION;
    let _ = &*DB_CONNECTION_POOL_SIZE;
    let _ = &*DB_CONNECTION_POOL_IDLE;
    let _ = &*DLQ_ENTRIES_TOTAL;

    tracing::info!("Prometheus metrics initialized");
}
