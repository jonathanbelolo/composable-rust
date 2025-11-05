//! Counter example binary
//!
//! Demonstrates the Composable Rust architecture with a simple counter.

use composable_rust_runtime::Store;
use composable_rust_testing::test_clock;
use counter::{CounterAction, CounterEnvironment, CounterReducer, CounterState};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "counter=debug,composable_rust_runtime=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    println!("=== Counter Example: Composable Rust Architecture ===\n");

    // Create environment with test clock
    let env = CounterEnvironment::new(test_clock());

    // Create store with initial state, reducer, and environment
    let store = Store::new(CounterState::default(), CounterReducer::new(), env);

    // Initial state
    let count = store.state(|s| s.count).await;
    println!("Initial count: {count}");

    // Increment
    println!("\n>>> Sending: Increment");
    let _ = store.send(CounterAction::Increment).await;
    let count = store.state(|s| s.count).await;
    println!("Count after Increment: {count}");

    // Increment again
    println!("\n>>> Sending: Increment");
    let _ = store.send(CounterAction::Increment).await;
    let count = store.state(|s| s.count).await;
    println!("Count after Increment: {count}");

    // Increment once more
    println!("\n>>> Sending: Increment");
    let _ = store.send(CounterAction::Increment).await;
    let count = store.state(|s| s.count).await;
    println!("Count after Increment: {count}");

    // Decrement
    println!("\n>>> Sending: Decrement");
    let _ = store.send(CounterAction::Decrement).await;
    let count = store.state(|s| s.count).await;
    println!("Count after Decrement: {count}");

    // Reset
    println!("\n>>> Sending: Reset");
    let _ = store.send(CounterAction::Reset).await;
    let count = store.state(|s| s.count).await;
    println!("Count after Reset: {count}");

    println!("\n=== Architecture Demonstration Complete ===");
    println!("\nKey concepts demonstrated:");
    println!("  • State: CounterState (domain data)");
    println!("  • Action: CounterAction (events)");
    println!("  • Reducer: Pure function (state, action) → (new state, effects)");
    println!("  • Store: Runtime that coordinates everything");
    println!("  • Environment: Injected dependencies (Clock)");
    println!("  • Effects: Side effect descriptions (none for pure counter)");
    println!("\nThis is Phase 1: Pure state machine with no side effects.");
    println!("Later phases will add persistence, event sourcing, and sagas.");
}
