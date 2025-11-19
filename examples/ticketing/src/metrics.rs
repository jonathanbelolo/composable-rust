//! Business metrics for the ticketing system.
//!
//! This module provides Prometheus metrics for tracking business operations:
//! - Reservations (created, completed, cancelled, expired)
//! - Payments (processed, succeeded, failed, refunded, revenue)
//! - Tickets (sold, reserved, available)
//! - Events (created, published)
//!
//! # Exported Metrics
//!
//! ## Counters
//! - `ticketing_reservations_total{status}` - Total reservations by status
//! - `ticketing_payments_total{status}` - Total payments by status
//! - `ticketing_payment_revenue_cents_total` - Total revenue in cents
//! - `ticketing_tickets_sold_total` - Total tickets sold
//! - `ticketing_events_created_total` - Total events created
//!
//! ## Gauges
//! - `ticketing_active_reservations` - Current active reservations
//! - `ticketing_tickets_available` - Current available tickets
//!
//! ## Histograms
//! - `ticketing_reservation_duration_seconds` - Time from creation to completion
//! - `ticketing_payment_duration_seconds` - Payment processing time

use metrics::{describe_counter, describe_gauge, describe_histogram};

/// Initialize and register all business metrics descriptions.
///
/// This should be called once at application startup, before any metrics are recorded.
pub fn register_business_metrics() {
    // Reservation metrics
    describe_counter!(
        "ticketing_reservations_total",
        "Total number of reservations by status (created, completed, cancelled, expired)"
    );
    describe_gauge!(
        "ticketing_active_reservations",
        "Current number of active reservations (pending payment)"
    );
    describe_histogram!(
        "ticketing_reservation_duration_seconds",
        "Time taken from reservation creation to completion"
    );

    // Payment metrics
    describe_counter!(
        "ticketing_payments_total",
        "Total number of payments by status (processed, succeeded, failed, refunded)"
    );
    describe_counter!(
        "ticketing_payment_revenue_cents_total",
        "Total revenue from successful payments in cents"
    );
    describe_counter!(
        "ticketing_payment_refunds_cents_total",
        "Total refunds issued in cents"
    );
    describe_histogram!(
        "ticketing_payment_duration_seconds",
        "Time taken to process a payment"
    );

    // Ticket metrics
    describe_counter!(
        "ticketing_tickets_sold_total",
        "Total number of tickets sold"
    );
    describe_gauge!(
        "ticketing_tickets_available",
        "Current number of available tickets"
    );

    // Event metrics
    describe_counter!(
        "ticketing_events_created_total",
        "Total number of events created"
    );

    tracing::info!("Business metrics registered");
}

// ============================================================================
// Metric Recording Functions
// ============================================================================

/// Record a reservation created event.
///
/// # Arguments
///
/// * `quantity` - Number of tickets in the reservation
pub fn record_reservation_created(quantity: u32) {
    metrics::counter!("ticketing_reservations_total", "status" => "created").increment(1);
    metrics::gauge!("ticketing_active_reservations").increment(1.0);
    tracing::debug!(quantity, "Recorded reservation_created metric");
}

/// Record a reservation completed event.
///
/// # Arguments
///
/// * `quantity` - Number of tickets in the reservation
/// * `duration_secs` - Time from creation to completion in seconds
pub fn record_reservation_completed(quantity: u32, duration_secs: f64) {
    metrics::counter!("ticketing_reservations_total", "status" => "completed").increment(1);
    metrics::gauge!("ticketing_active_reservations").decrement(1.0);
    metrics::histogram!("ticketing_reservation_duration_seconds").record(duration_secs);
    metrics::counter!("ticketing_tickets_sold_total").increment(u64::from(quantity));
    tracing::debug!(quantity, duration_secs, "Recorded reservation_completed metric");
}

/// Record a reservation cancelled event.
pub fn record_reservation_cancelled() {
    metrics::counter!("ticketing_reservations_total", "status" => "cancelled").increment(1);
    metrics::gauge!("ticketing_active_reservations").decrement(1.0);
    tracing::debug!("Recorded reservation_cancelled metric");
}

/// Record a reservation expired event.
pub fn record_reservation_expired() {
    metrics::counter!("ticketing_reservations_total", "status" => "expired").increment(1);
    metrics::gauge!("ticketing_active_reservations").decrement(1.0);
    tracing::debug!("Recorded reservation_expired metric");
}

/// Record a payment processed event.
pub fn record_payment_processed() {
    metrics::counter!("ticketing_payments_total", "status" => "processed").increment(1);
    tracing::debug!("Recorded payment_processed metric");
}

/// Record a payment succeeded event.
///
/// # Arguments
///
/// * `amount_cents` - Payment amount in cents
/// * `duration_secs` - Time taken to process payment in seconds
pub fn record_payment_succeeded(amount_cents: u64, duration_secs: f64) {
    metrics::counter!("ticketing_payments_total", "status" => "succeeded").increment(1);
    metrics::counter!("ticketing_payment_revenue_cents_total").increment(amount_cents);
    metrics::histogram!("ticketing_payment_duration_seconds").record(duration_secs);
    tracing::debug!(amount_cents, duration_secs, "Recorded payment_succeeded metric");
}

/// Record a payment failed event.
///
/// # Arguments
///
/// * `reason` - Failure reason (e.g., "gateway_error", "insufficient_funds")
pub fn record_payment_failed(reason: String) {
    tracing::debug!(?reason, "Recorded payment_failed metric");
    metrics::counter!("ticketing_payments_total", "status" => "failed", "reason" => reason).increment(1);
}

/// Record a payment refunded event.
///
/// # Arguments
///
/// * `amount_cents` - Refund amount in cents
pub fn record_payment_refunded(amount_cents: u64) {
    metrics::counter!("ticketing_payments_total", "status" => "refunded").increment(1);
    metrics::counter!("ticketing_payment_refunds_cents_total").increment(amount_cents);
    tracing::debug!(amount_cents, "Recorded payment_refunded metric");
}

/// Record an event created.
pub fn record_event_created() {
    metrics::counter!("ticketing_events_created_total").increment(1);
    tracing::debug!("Recorded event_created metric");
}

/// Update available tickets gauge for an event.
///
/// # Arguments
///
/// * `event_id` - Event ID as string
/// * `available` - Current number of available tickets
pub fn update_tickets_available(event_id: &str, available: i64) {
    metrics::gauge!("ticketing_tickets_available", "event_id" => event_id.to_owned())
        .set(available as f64);
    tracing::debug!(event_id, available, "Updated tickets_available metric");
}
