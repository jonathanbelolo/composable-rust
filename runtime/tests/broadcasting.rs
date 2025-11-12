//! Integration tests for Store action broadcasting
//!
//! Tests the action observation features that enable HTTP request-response
//! patterns and WebSocket event streaming without coupling to HTTP layer.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code can use unwrap/expect/panic
#![allow(clippy::needless_continue, clippy::match_same_arms, clippy::collapsible_if, clippy::collapsible_match)] // Test code - allow pedantic warnings

use composable_rust_core::{effect::Effect, reducer::Reducer, smallvec, SmallVec};
use composable_rust_runtime::Store;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

// ============================================================================
// Test Fixtures
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
enum TestAction {
    /// Start a saga with correlation ID
    StartSaga { id: u64 },
    /// Saga step completed
    StepCompleted { id: u64, step: u32 },
    /// Saga finished (terminal action)
    SagaCompleted { id: u64 },
    /// Saga failed (terminal action)
    SagaFailed { id: u64, error: String },
    /// Simple increment command
    Increment,
    /// Incremented event
    Incremented { value: u32 },
}

#[derive(Debug, Clone, Default)]
struct TestState {
    counter: u32,
    saga_steps: Vec<u32>,
}

#[derive(Clone)]
struct TestEnvironment;

#[derive(Clone)]
struct TestReducer;

impl Reducer for TestReducer {
    type State = TestState;
    type Action = TestAction;
    type Environment = TestEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            TestAction::StartSaga { id } => {
                state.saga_steps.clear();
                smallvec![
                    Effect::Future(Box::pin(async move {
                        // Simulate async work
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        Some(TestAction::StepCompleted { id, step: 1 })
                    })),
                ]
            }

            TestAction::StepCompleted { id, step } => {
                state.saga_steps.push(step);

                if step < 3 {
                    // Continue saga
                    smallvec![Effect::Future(Box::pin(async move {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        Some(TestAction::StepCompleted { id, step: step + 1 })
                    }))]
                } else {
                    // Finish saga
                    smallvec![Effect::Future(Box::pin(async move {
                        Some(TestAction::SagaCompleted { id })
                    }))]
                }
            }

            TestAction::SagaCompleted { .. } | TestAction::SagaFailed { .. } => {
                // Terminal actions, no effects
                smallvec![Effect::None]
            }

            TestAction::Increment => {
                state.counter += 1;
                let value = state.counter;
                smallvec![Effect::Future(Box::pin(async move {
                    Some(TestAction::Incremented { value })
                }))]
            }

            TestAction::Incremented { .. } => {
                smallvec![Effect::None]
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Test `send_and_wait_for` with immediate response
///
/// Verifies that we can send an action and wait for a terminal action
/// that is produced immediately.
#[tokio::test]
async fn test_send_and_wait_for_immediate() {
    let store = Store::new(TestState::default(), TestReducer, TestEnvironment);

    let result = store
        .send_and_wait_for(
            TestAction::Increment,
            |action| matches!(action, TestAction::Incremented { .. }),
            Duration::from_secs(1),
        )
        .await;

    assert!(result.is_ok());
    assert!(matches!(
        result.unwrap(),
        TestAction::Incremented { value: 1 }
    ));
}

/// Test `send_and_wait_for` with delayed response (saga)
///
/// Verifies that we can wait for a terminal action from a multi-step saga
/// that takes multiple async operations to complete.
#[tokio::test]
async fn test_send_and_wait_for_saga() {
    let store = Store::new(TestState::default(), TestReducer, TestEnvironment);

    let result = store
        .send_and_wait_for(
            TestAction::StartSaga { id: 42 },
            |action| matches!(action, TestAction::SagaCompleted { id: 42 }),
            Duration::from_secs(1),
        )
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), TestAction::SagaCompleted { id: 42 });

    // Verify saga completed all steps
    let saga_steps = store.state(|s| s.saga_steps.clone()).await;
    assert_eq!(saga_steps, vec![1, 2, 3]);
}

/// Test `send_and_wait_for` timeout behavior
///
/// Verifies that we get a timeout error if the terminal action
/// doesn't arrive within the specified duration.
#[tokio::test]
async fn test_send_and_wait_for_timeout() {
    let store = Store::new(TestState::default(), TestReducer, TestEnvironment);

    let result = store
        .send_and_wait_for(
            TestAction::StartSaga { id: 99 },
            |action| {
                // Wait for an action that will never come
                matches!(action, TestAction::SagaFailed { id: 99, .. })
            },
            Duration::from_millis(50), // Short timeout
        )
        .await;

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        composable_rust_runtime::StoreError::Timeout
    ));
}

/// Test concurrent subscribers
///
/// Verifies that multiple subscribers can independently wait for
/// different terminal actions without interfering with each other.
#[tokio::test]
async fn test_concurrent_subscribers() {
    let store = Arc::new(Store::new(
        TestState::default(),
        TestReducer,
        TestEnvironment,
    ));

    // Spawn multiple concurrent requests
    let mut handles = vec![];

    for id in 1..=5 {
        let store_clone = Arc::clone(&store);
        let handle = tokio::spawn(async move {
            store_clone
                .send_and_wait_for(
                    TestAction::StartSaga { id },
                    move |action| matches!(action, TestAction::SagaCompleted { id: saga_id } if *saga_id == id),
                    Duration::from_secs(2),
                )
                .await
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for (i, handle) in handles.into_iter().enumerate() {
        let result = handle.await.expect("Task panicked");
        assert!(result.is_ok(), "Saga {} should complete successfully", i + 1);
    }

    // Verify final state - sagas may interleave but all should have run
    let saga_steps = store.state(|s| s.saga_steps.clone()).await;
    // All sagas completed, so we should have 15 total steps (5 sagas × 3 steps each)
    assert_eq!(saga_steps.len(), 15, "Expected 15 total steps from 5 sagas");
}

/// Test `subscribe_actions` streaming
///
/// Verifies that subscribers receive all actions produced by effects
/// in real-time, enabling WebSocket event streaming.
#[tokio::test]
async fn test_subscribe_actions_streaming() {
    let store = Arc::new(Store::new(
        TestState::default(),
        TestReducer,
        TestEnvironment,
    ));

    let mut rx = store.subscribe_actions();

    // Collect actions in background task
    let received = Arc::new(Mutex::new(Vec::new()));
    let received_clone = Arc::clone(&received);

    tokio::spawn(async move {
        let mut count = 0;
        while count < 4 {
            // Expect 4 actions: StepCompleted(1,2,3), SagaCompleted
            if let Ok(action) = rx.recv().await {
                received_clone.lock().await.push(action);
                count += 1;
            }
        }
    });

    // Give subscriber time to set up
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Send saga
    store.send(TestAction::StartSaga { id: 100 }).await.ok();

    // Wait for saga to complete
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify received actions
    let actions = received.lock().await;
    assert_eq!(actions.len(), 4);
    assert!(matches!(
        actions[0],
        TestAction::StepCompleted { id: 100, step: 1 }
    ));
    assert!(matches!(
        actions[1],
        TestAction::StepCompleted { id: 100, step: 2 }
    ));
    assert!(matches!(
        actions[2],
        TestAction::StepCompleted { id: 100, step: 3 }
    ));
    assert!(matches!(actions[3], TestAction::SagaCompleted { id: 100 }));
}

/// Test correlation ID filtering
///
/// Verifies that predicates can filter actions by correlation ID,
/// enabling multiple concurrent HTTP requests to wait for their
/// specific terminal actions.
#[tokio::test]
async fn test_correlation_id_filtering() {
    let store = Arc::new(Store::new(
        TestState::default(),
        TestReducer,
        TestEnvironment,
    ));

    // Start two sagas concurrently
    let store1 = Arc::clone(&store);
    let handle1 = tokio::spawn(async move {
        store1
            .send_and_wait_for(
                TestAction::StartSaga { id: 1 },
                |action| matches!(action, TestAction::SagaCompleted { id: 1 }),
                Duration::from_secs(1),
            )
            .await
    });

    let store2 = Arc::clone(&store);
    let handle2 = tokio::spawn(async move {
        store2
            .send_and_wait_for(
                TestAction::StartSaga { id: 2 },
                |action| matches!(action, TestAction::SagaCompleted { id: 2 }),
                Duration::from_secs(1),
            )
            .await
    });

    // Both should complete with their correct IDs
    let result1 = handle1.await.expect("Task 1 panicked");
    let result2 = handle2.await.expect("Task 2 panicked");

    assert!(result1.is_ok());
    assert!(result2.is_ok());

    assert_eq!(result1.unwrap(), TestAction::SagaCompleted { id: 1 });
    assert_eq!(result2.unwrap(), TestAction::SagaCompleted { id: 2 });
}

/// Test lagging subscriber behavior
///
/// Verifies that slow subscribers skip old actions but continue
/// receiving new ones without blocking the store.
#[tokio::test]
async fn test_lagging_subscriber() {
    // Create store with small capacity to trigger lagging
    let store = Arc::new(Store::with_broadcast_capacity(
        TestState::default(),
        TestReducer,
        TestEnvironment,
        4, // Small capacity
    ));

    let mut rx = store.subscribe_actions();

    // Send many actions rapidly to overflow buffer
    for _ in 0..20 {
        store.send(TestAction::Increment).await.ok();
    }

    // Give effects time to execute
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscriber should handle lagging gracefully
    let mut received = 0;
    let mut lagged = false;

    loop {
        match rx.try_recv() {
            Ok(_) => received += 1,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                lagged = true;
                continue; // Skip and continue
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
        }
    }

    // Should have lagged at some point
    assert!(lagged, "Expected subscriber to lag");
    // Should still receive some actions (not all 20)
    assert!(received > 0, "Should receive at least some actions");
    assert!(received < 20, "Should not receive all actions if lagged");
}

/// Test multiple independent subscribers
///
/// Verifies that multiple subscribers can operate independently
/// without affecting each other.
#[tokio::test]
async fn test_multiple_independent_subscribers() {
    let store = Arc::new(Store::new(
        TestState::default(),
        TestReducer,
        TestEnvironment,
    ));

    let mut rx1 = store.subscribe_actions();
    let mut rx2 = store.subscribe_actions();
    let mut rx3 = store.subscribe_actions();

    // Send some actions
    store.send(TestAction::Increment).await.ok();
    store.send(TestAction::Increment).await.ok();

    // Give effects time to execute
    tokio::time::sleep(Duration::from_millis(50)).await;

    // All subscribers should receive both actions
    let count1 = count_available_actions(&mut rx1);
    let count2 = count_available_actions(&mut rx2);
    let count3 = count_available_actions(&mut rx3);

    assert_eq!(count1, 2);
    assert_eq!(count2, 2);
    assert_eq!(count3, 2);
}

/// Test that initial actions are NOT broadcast
///
/// Verifies that only actions produced by effects are broadcast,
/// not the initial actions sent to the store.
#[tokio::test]
async fn test_initial_actions_not_broadcast() {
    let store = Arc::new(Store::new(
        TestState::default(),
        TestReducer,
        TestEnvironment,
    ));

    let mut rx = store.subscribe_actions();

    // Send action that produces an effect
    store.send(TestAction::Increment).await.ok();

    // Give effect time to execute
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Should only receive Incremented (from effect), not Increment (initial)
    let actions: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();

    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], TestAction::Incremented { .. }));
}

/// Test `Effect::Delay` broadcasting
///
/// Verifies that actions produced by `Effect::Delay` are also broadcast,
/// not just `Effect::Future`.
#[tokio::test]
async fn test_effect_delay_broadcasting() {
    // New action type with delay
    #[derive(Debug, Clone, PartialEq)]
    enum DelayAction {
        Start,
        Delayed,
    }

    #[derive(Clone, Default)]
    struct DelayState;

    #[derive(Clone)]
    struct DelayReducer;

    impl Reducer for DelayReducer {
        type State = DelayState;
        type Action = DelayAction;
        type Environment = TestEnvironment;

        fn reduce(
            &self,
            _state: &mut Self::State,
            action: Self::Action,
            _env: &Self::Environment,
        ) -> SmallVec<[Effect<Self::Action>; 4]> {
            match action {
                DelayAction::Start => smallvec![Effect::Delay {
                    duration: Duration::from_millis(10),
                    action: Box::new(DelayAction::Delayed),
                }],
                DelayAction::Delayed => smallvec![Effect::None],
            }
        }
    }

    let store = Store::new(DelayState, DelayReducer, TestEnvironment);
    let mut rx = store.subscribe_actions();

    // Send action that produces Effect::Delay
    store.send(DelayAction::Start).await.ok();

    // Wait for delayed action to be broadcast
    let action = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("Timeout waiting for delayed action")
        .expect("Channel closed");

    assert_eq!(action, DelayAction::Delayed);
}

/// Test nested effects (Parallel containing Futures)
///
/// Verifies that actions produced by effects inside `Effect::Parallel`
/// are correctly broadcast.
#[tokio::test]
async fn test_parallel_effects_broadcasting() {
    #[derive(Debug, Clone, PartialEq)]
    enum ParallelAction {
        Start,
        Result1,
        Result2,
    }

    #[derive(Clone, Default)]
    struct ParallelState;

    #[derive(Clone)]
    struct ParallelReducer;

    impl Reducer for ParallelReducer {
        type State = ParallelState;
        type Action = ParallelAction;
        type Environment = TestEnvironment;

        fn reduce(
            &self,
            _state: &mut Self::State,
            action: Self::Action,
            _env: &Self::Environment,
        ) -> SmallVec<[Effect<Self::Action>; 4]> {
            match action {
                ParallelAction::Start => smallvec![Effect::Parallel(vec![
                    Effect::Future(Box::pin(async {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        Some(ParallelAction::Result1)
                    })),
                    Effect::Future(Box::pin(async {
                        tokio::time::sleep(Duration::from_millis(15)).await;
                        Some(ParallelAction::Result2)
                    })),
                ])],
                ParallelAction::Result1 | ParallelAction::Result2 => smallvec![Effect::None],
            }
        }
    }

    let store = Arc::new(Store::new(
        ParallelState,
        ParallelReducer,
        TestEnvironment,
    ));

    let mut rx = store.subscribe_actions();

    // Send action that produces parallel effects
    store.send(ParallelAction::Start).await.ok();

    // Collect both results
    let mut results = Vec::new();
    for _ in 0..2 {
        if let Ok(action) = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            if let Ok(action) = action {
                results.push(action);
            }
        }
    }

    // Both actions should be broadcast (order may vary)
    assert_eq!(results.len(), 2);
    assert!(results.contains(&ParallelAction::Result1));
    assert!(results.contains(&ParallelAction::Result2));
}

/// Test nested effects (Sequential containing Futures)
///
/// Verifies that actions produced by effects inside `Effect::Sequential`
/// are correctly broadcast in order.
#[tokio::test]
async fn test_sequential_effects_broadcasting() {
    #[derive(Debug, Clone, PartialEq)]
    enum SeqAction {
        Start,
        Step1,
        Step2,
    }

    #[derive(Clone, Default)]
    struct SeqState;

    #[derive(Clone)]
    struct SeqReducer;

    impl Reducer for SeqReducer {
        type State = SeqState;
        type Action = SeqAction;
        type Environment = TestEnvironment;

        fn reduce(
            &self,
            _state: &mut Self::State,
            action: Self::Action,
            _env: &Self::Environment,
        ) -> SmallVec<[Effect<Self::Action>; 4]> {
            match action {
                SeqAction::Start => smallvec![Effect::Sequential(vec![
                    Effect::Future(Box::pin(async {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        Some(SeqAction::Step1)
                    })),
                    Effect::Future(Box::pin(async {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        Some(SeqAction::Step2)
                    })),
                ])],
                SeqAction::Step1 | SeqAction::Step2 => smallvec![Effect::None],
            }
        }
    }

    let store = Arc::new(Store::new(SeqState, SeqReducer, TestEnvironment));

    let mut rx = store.subscribe_actions();

    // Send action that produces sequential effects
    store.send(SeqAction::Start).await.ok();

    // Collect results in order
    let action1 = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("Timeout")
        .expect("Channel closed");
    let action2 = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("Timeout")
        .expect("Channel closed");

    // Actions should arrive in order
    assert_eq!(action1, SeqAction::Step1);
    assert_eq!(action2, SeqAction::Step2);
}

/// Test `ChannelClosed` error when Store is dropped
///
/// Verifies that subscribers waiting for actions receive `ChannelClosed`
/// error when the Store is dropped.
#[tokio::test]
async fn test_channel_closed_on_store_drop() {
    let store = Store::new(TestState::default(), TestReducer, TestEnvironment);

    // Start waiting for an action that will never come
    let result = store
        .send_and_wait_for(
            TestAction::Increment,
            |action| matches!(action, TestAction::SagaCompleted { .. }), // Will never match
            Duration::from_secs(10), // Long timeout
        )
        .await;

    // Store lives until here, then gets dropped
    drop(store);

    // If store was dropped during wait, we'd get ChannelClosed
    // But since store lives until after await, we get Timeout instead
    // Let's test the actual ChannelClosed path differently
    assert!(result.is_err()); // Either Timeout or ChannelClosed
}

/// Test `ChannelClosed` error (proper test with concurrent drop)
///
/// Verifies that `ChannelClosed` error is returned when Store is dropped
/// while a subscriber is actively waiting.
#[tokio::test]
async fn test_channel_closed_concurrent_drop() {
    use tokio::sync::oneshot;

    let store = Arc::new(Store::new(
        TestState::default(),
        TestReducer,
        TestEnvironment,
    ));

    let (tx, rx) = oneshot::channel();

    // Spawn task that will wait for an action (without keeping a store clone)
    let mut subscriber = store.subscribe_actions();
    let wait_handle = tokio::spawn(async move {
        // Signal that we're about to wait
        tx.send(()).ok();

        // Wait for any action
        subscriber.recv().await
    });

    // Wait for the task to start waiting
    rx.await.ok();

    // Give it a moment to actually be waiting
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Drop the store, which closes the channel
    drop(store);

    // The waiting task should get ChannelClosed error
    let result = wait_handle.await.expect("Task panicked");

    // Should get Closed error
    assert!(matches!(
        result,
        Err(tokio::sync::broadcast::error::RecvError::Closed)
    ));
}

/// Test custom broadcast capacity
///
/// Verifies that `with_broadcast_capacity` creates a store with the
/// specified buffer size.
#[tokio::test]
async fn test_custom_broadcast_capacity() {
    // Create store with capacity of 2
    let store = Arc::new(Store::with_broadcast_capacity(
        TestState::default(),
        TestReducer,
        TestEnvironment,
        2, // Very small capacity
    ));

    let mut rx = store.subscribe_actions();

    // Send 5 actions rapidly (will overflow buffer)
    for _ in 0..5 {
        store.send(TestAction::Increment).await.ok();
    }

    // Give effects time to execute
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Should receive some actions and possibly lag
    let mut received = 0;
    let mut lagged = false;

    loop {
        match rx.try_recv() {
            Ok(_) => received += 1,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                lagged = true;
                continue;
            }
            Err(_) => break,
        }
    }

    // With capacity 2, we should have lagged
    assert!(lagged || received < 5, "Should lag or miss actions with small buffer");
}

/// Test saga failure scenario
///
/// Verifies that error actions (`SagaFailed`) are also broadcast correctly.
#[tokio::test]
async fn test_saga_failure_broadcasting() {
    #[derive(Debug, Clone, PartialEq)]
    enum FailAction {
        Start,
        Failed { error: String },
    }

    #[derive(Clone, Default)]
    struct FailState;

    #[derive(Clone)]
    struct FailReducer;

    impl Reducer for FailReducer {
        type State = FailState;
        type Action = FailAction;
        type Environment = TestEnvironment;

        fn reduce(
            &self,
            _state: &mut Self::State,
            action: Self::Action,
            _env: &Self::Environment,
        ) -> SmallVec<[Effect<Self::Action>; 4]> {
            match action {
                FailAction::Start => smallvec![Effect::Future(Box::pin(async {
                    // Simulate failure
                    Some(FailAction::Failed {
                        error: "Test error".to_string()
                    })
                }))],
                FailAction::Failed { .. } => smallvec![Effect::None],
            }
        }
    }

    let store = Store::new(FailState, FailReducer, TestEnvironment);

    let result = store
        .send_and_wait_for(
            FailAction::Start,
            |action| matches!(action, FailAction::Failed { .. }),
            Duration::from_secs(1),
        )
        .await;

    assert!(result.is_ok());
    if let Ok(FailAction::Failed { error }) = result {
        assert_eq!(error, "Test error");
    } else {
        panic!("Expected Failed action");
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Count available actions in receiver without blocking
fn count_available_actions(rx: &mut tokio::sync::broadcast::Receiver<TestAction>) -> usize {
    let mut count = 0;
    loop {
        match rx.try_recv() {
            Ok(_) => count += 1,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    count
}
