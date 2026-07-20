//! Replay payment events to rebuild projections.
//!
//! This tool reads historical payment events from the event store and
//! projects them to the payments read model. This is useful for:
//!
//! - Recovering payment history that was not projected due to missing projector
//! - Rebuilding projections after schema changes
//! - Testing projection logic against real events
//!
//! # Usage
//!
//! ```bash
//! cargo run --bin replay-payments
//! ```

use composable_rust_next::{Projector, SerializedEvent, Version};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use ticketing::next::projector::PaymentProjector;

/// Database connection URLs (matching main application).
const EVENTS_DATABASE_URL: &str = "postgres://postgres:postgres@localhost:5436/ticketing_events";
const PROJECTIONS_DATABASE_URL: &str =
    "postgres://postgres:postgres@localhost:5433/ticketing_projections";

/// Payment event types we want to replay.
const PAYMENT_EVENT_TYPES: &[&str] = &[
    "PaymentProcessed",
    "PaymentSucceeded",
    "PaymentFailed",
    "PaymentRefunded",
];

/// Row from event store with `stream_id` for logging.
struct EventRow {
    stream_id: String,
    event: SerializedEvent,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    tracing::info!("Starting payment event replay...");

    // Connect to events database
    let events_pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(EVENTS_DATABASE_URL)
        .await?;

    tracing::info!("Connected to events database");

    // Connect to projections database
    let projections_pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(PROJECTIONS_DATABASE_URL)
        .await?;

    tracing::info!("Connected to projections database");

    // Check initial state
    let initial_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payments")
        .fetch_one(&projections_pool)
        .await
        .unwrap_or(0);

    tracing::info!(initial_count, "Current payments in projection table");

    // Create projector
    let projector = PaymentProjector::new(projections_pool.clone());

    // Query all payment events ordered by created_at to maintain causality
    let event_rows = query_payment_events(&events_pool).await?;

    tracing::info!("Found {} payment events to replay", event_rows.len());

    if event_rows.is_empty() {
        tracing::info!("No payment events found. Nothing to replay.");
        return Ok(());
    }

    // Project each event
    let mut success_count = 0;
    let mut error_count = 0;

    for event_row in &event_rows {
        match projector
            .project(std::slice::from_ref(&event_row.event))
            .await
        {
            Ok(()) => {
                success_count += 1;
                tracing::debug!(
                    stream_id = %event_row.stream_id,
                    event_type = %event_row.event.event_type,
                    "Projected event"
                );
            },
            Err(e) => {
                error_count += 1;
                tracing::error!(
                    stream_id = %event_row.stream_id,
                    event_type = %event_row.event.event_type,
                    error = %e,
                    "Failed to project event"
                );
            },
        }
    }

    tracing::info!(
        success = success_count,
        errors = error_count,
        total = event_rows.len(),
        "Payment event replay complete"
    );

    // Verify final count
    let final_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payments")
        .fetch_one(&projections_pool)
        .await
        .unwrap_or(0);

    tracing::info!(
        before = initial_count,
        after = final_count,
        added = final_count - initial_count,
        "Payments table now contains {} records",
        final_count
    );

    Ok(())
}

/// Query all payment events from the event store.
async fn query_payment_events(pool: &PgPool) -> Result<Vec<EventRow>, sqlx::Error> {
    // Build query for all payment event types
    let query = r"
        SELECT
            stream_id,
            version,
            event_type,
            event_data
        FROM events
        WHERE event_type = ANY($1)
        ORDER BY created_at ASC, stream_id ASC, version ASC
    ";

    let rows = sqlx::query(query)
        .bind(PAYMENT_EVENT_TYPES)
        .fetch_all(pool)
        .await?;

    #[allow(clippy::cast_sign_loss)]
    let events: Vec<EventRow> = rows
        .into_iter()
        .map(|row| {
            let version_i64: i64 = row.get("version");
            EventRow {
                stream_id: row.get("stream_id"),
                event: SerializedEvent {
                    event_type: row.get("event_type"),
                    payload: row.get("event_data"),
                    metadata: None,
                    version: Some(Version::new(version_i64 as u64)),
                    stream_id: Some(row.get("stream_id")),
                },
            }
        })
        .collect();

    Ok(events)
}
