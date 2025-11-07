//! Metrics Demo - Prometheus Exporter Example
//!
//! This example demonstrates how to expose Composable Rust metrics to Prometheus.
//!
//! # Running the Example
//!
//! ```bash
//! cargo run --example metrics-demo
//! ```
//!
//! Then visit:
//! - Metrics endpoint: <http://localhost:9000/metrics>
//!
//! # Prometheus Configuration
//!
//! Add to your `prometheus.yml`:
//!
//! ```yaml
//! scrape_configs:
//!   - job_name: 'composable-rust-metrics-demo'
//!     static_configs:
//!       - targets: ['localhost:9000']
//! ```

#![allow(missing_docs)]
#![allow(clippy::expect_used)] // Examples can use expect

use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use composable_rust_core::{smallvec, SmallVec};
use composable_rust_runtime::{RetryPolicy, Store, StoreConfig};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Simple counter state
#[derive(Clone, Debug)]
struct CounterState {
    count: i32,
}

// Actions
#[derive(Clone, Debug)]
enum CounterAction {
    Increment,
    Decrement,
    ProduceDelayedIncrement,
}

// Empty environment
#[derive(Clone)]
struct CounterEnv;

// Reducer
#[derive(Clone)]
struct CounterReducer;

impl Reducer for CounterReducer {
    type State = CounterState;
    type Action = CounterAction;
    type Environment = CounterEnv;

    #[allow(clippy::cognitive_complexity)] // Demo example
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            CounterAction::Increment => {
                state.count += 1;
                tracing::info!(count = state.count, "Incremented");
                smallvec![Effect::None]
            }
            CounterAction::Decrement => {
                state.count -= 1;
                tracing::info!(count = state.count, "Decremented");
                smallvec![Effect::None]
            }
            CounterAction::ProduceDelayedIncrement => {
                tracing::info!("Scheduling delayed increment");
                smallvec![Effect::Delay {
                    duration: Duration::from_millis(100),
                    action: Box::new(CounterAction::Increment),
                }]
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Initialize tracing subscriber
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,composable_rust=debug,metrics_demo=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting Metrics Demo");

    // 2. Install Prometheus exporter
    let builder = PrometheusBuilder::new();
    builder
        .with_http_listener(([0, 0, 0, 0], 9000))
        .install()
        .expect("Failed to install Prometheus exporter");

    tracing::info!("✓ Prometheus metrics available at http://localhost:9000/metrics");

    // 3. Create store with configuration
    let config = StoreConfig::default()
        .with_dlq_max_size(100)
        .with_retry_policy(RetryPolicy::default())
        .with_shutdown_timeout(Duration::from_secs(10));

    let store = Store::with_config(
        CounterState { count: 0 },
        CounterReducer,
        CounterEnv,
        config,
    );

    tracing::info!("✓ Store initialized");

    // 4. Check initial health
    let health = store.health();
    tracing::info!(?health, "Initial health check");

    // 5. Generate some traffic to produce metrics
    tracing::info!("Generating sample traffic...");

    for i in 0..10 {
        let _handle = store.send(CounterAction::Increment).await?;
        tokio::time::sleep(Duration::from_millis(50)).await;

        if i % 3 == 0 {
            let _handle = store.send(CounterAction::ProduceDelayedIncrement).await?;
        }
    }

    for _ in 0..5 {
        let _handle = store.send(CounterAction::Decrement).await?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    tracing::info!("✓ Generated sample traffic");

    // 6. Display final state
    let count = store.state(|s| s.count).await;
    tracing::info!(count, "Final counter value");

    // 7. Check final health
    let health = store.health();
    tracing::info!(?health, "Final health check");

    // 8. Keep running to allow metric scraping
    tracing::info!("");
    tracing::info!("========================================");
    tracing::info!("Metrics server running!");
    tracing::info!("Visit: http://localhost:9000/metrics");
    tracing::info!("Press Ctrl+C to exit");
    tracing::info!("========================================");

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;

    tracing::info!("Shutdown signal received, stopping...");

    // 9. Graceful shutdown
    store.shutdown(Duration::from_secs(5)).await?;

    tracing::info!("✓ Clean shutdown complete");

    Ok(())
}
