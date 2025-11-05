//! Integration tests for Counter with Store
//!
//! These tests demonstrate the full end-to-end flow of the
//! Composable Rust architecture.

use composable_rust_runtime::Store;
use composable_rust_testing::test_clock;
use counter::{CounterAction, CounterEnvironment, CounterReducer, CounterState};

#[tokio::test]
async fn test_counter_with_store() {
    let env = CounterEnvironment::new(test_clock());
    let store = Store::new(CounterState::default(), CounterReducer::new(), env);

    // Initial state
    let count = store.state(|s| s.count).await;
    assert_eq!(count, 0);

    // Increment
    let _ = store.send(CounterAction::Increment).await;
    let count = store.state(|s| s.count).await;
    assert_eq!(count, 1);

    // Increment again
    let _ = store.send(CounterAction::Increment).await;
    let count = store.state(|s| s.count).await;
    assert_eq!(count, 2);

    // Decrement
    let _ = store.send(CounterAction::Decrement).await;
    let count = store.state(|s| s.count).await;
    assert_eq!(count, 1);

    // Reset
    let _ = store.send(CounterAction::Reset).await;
    let count = store.state(|s| s.count).await;
    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_concurrent_increments() {
    let env = CounterEnvironment::new(test_clock());
    let store = Store::new(CounterState::default(), CounterReducer::new(), env);

    // Send multiple increments concurrently
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let store = store.clone();
            tokio::spawn(async move {
                let _ = store.send(CounterAction::Increment).await;
            })
        })
        .collect();

    // Wait for all to complete
    #[allow(clippy::panic)]
    for handle in handles {
        if let Err(e) = handle.await {
            panic!("concurrent increment task panicked: {e}");
        }
    }

    // Verify final count
    let count = store.state(|s| s.count).await;
    assert_eq!(count, 10);
}

#[tokio::test]
async fn test_state_isolation() {
    let env1 = CounterEnvironment::new(test_clock());
    let env2 = CounterEnvironment::new(test_clock());

    let store1 = Store::new(CounterState::default(), CounterReducer::new(), env1);
    let store2 = Store::new(CounterState::default(), CounterReducer::new(), env2);

    // Modify store1
    let _ = store1.send(CounterAction::Increment).await;
    let _ = store1.send(CounterAction::Increment).await;

    // Modify store2
    let _ = store2.send(CounterAction::Increment).await;

    // Verify isolation
    let count1 = store1.state(|s| s.count).await;
    let count2 = store2.state(|s| s.count).await;

    assert_eq!(count1, 2);
    assert_eq!(count2, 1);
}

#[tokio::test]
async fn test_negative_count() {
    let env = CounterEnvironment::new(test_clock());
    let store = Store::new(CounterState::default(), CounterReducer::new(), env);

    // Decrement below zero
    let _ = store.send(CounterAction::Decrement).await;
    let _ = store.send(CounterAction::Decrement).await;
    let _ = store.send(CounterAction::Decrement).await;

    let count = store.state(|s| s.count).await;
    assert_eq!(count, -3);
}

#[tokio::test]
async fn test_large_counts() {
    let env = CounterEnvironment::new(test_clock());
    let store = Store::new(
        CounterState {
            count: i64::MAX - 5,
        },
        CounterReducer::new(),
        env,
    );

    // Increment multiple times
    for _ in 0..3 {
        let _ = store.send(CounterAction::Increment).await;
    }

    let count = store.state(|s| s.count).await;
    assert_eq!(count, i64::MAX - 2);

    // Reset works even with large numbers
    let _ = store.send(CounterAction::Reset).await;
    let count = store.state(|s| s.count).await;
    assert_eq!(count, 0);
}
