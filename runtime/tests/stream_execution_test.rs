//! Integration tests for Effect::Stream execution in Store runtime
//!
//! Tests validate that streams are correctly executed, items are fed back
//! to reducers, and metrics are properly tracked.

use composable_rust_core::{effect::Effect, reducer::Reducer, SmallVec};
use composable_rust_runtime::Store;
use futures::stream;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
struct StreamState {
    items_received: Vec<String>,
}

impl Default for StreamState {
    fn default() -> Self {
        Self {
            items_received: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum StreamAction {
    StartStream { items: Vec<String> },
    StreamItem { text: String },
    StreamComplete,
    GetItems,
}

#[derive(Clone)]
struct StreamReducer;

impl Reducer for StreamReducer {
    type State = StreamState;
    type Action = StreamAction;
    type Environment = ();

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            StreamAction::StartStream { items } => {
                // Create a stream that yields each item as an action
                let stream_effect = Effect::Stream(Box::pin(stream::iter(
                    items
                        .into_iter()
                        .map(|text| StreamAction::StreamItem { text })
                        .chain(std::iter::once(StreamAction::StreamComplete)),
                )));

                SmallVec::from_vec(vec![stream_effect])
            }
            StreamAction::StreamItem { text } => {
                // Accumulate items
                state.items_received.push(text);
                SmallVec::from_vec(vec![Effect::None])
            }
            StreamAction::StreamComplete => {
                // Stream finished
                SmallVec::from_vec(vec![Effect::None])
            }
            StreamAction::GetItems => {
                // No-op action for tests
                SmallVec::from_vec(vec![Effect::None])
            }
        }
    }
}

#[tokio::test]
async fn test_stream_basic_execution() {
    let store = Store::new(StreamState::default(), StreamReducer, ());

    // Start a stream with 3 items
    store
        .send(StreamAction::StartStream {
            items: vec!["item1".to_string(), "item2".to_string(), "item3".to_string()],
        })
        .await
        .unwrap();

    // Give the stream time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify all items were received
    let items = store.state(|s| s.items_received.clone()).await;
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], "item1");
    assert_eq!(items[1], "item2");
    assert_eq!(items[2], "item3");
}

#[tokio::test]
async fn test_stream_empty() {
    let store = Store::new(StreamState::default(), StreamReducer, ());

    // Start a stream with no items
    store
        .send(StreamAction::StartStream { items: vec![] })
        .await
        .unwrap();

    // Give the stream time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Verify no items (just the complete action)
    let items = store.state(|s| s.items_received.clone()).await;
    assert_eq!(items.len(), 0);
}

#[tokio::test]
async fn test_stream_large_volume() {
    let store = Store::new(StreamState::default(), StreamReducer, ());

    // Create 100 items
    let items: Vec<String> = (0..100).map(|i| format!("item{}", i)).collect();

    store
        .send(StreamAction::StartStream {
            items: items.clone(),
        })
        .await
        .unwrap();

    // Give the stream time to process all items
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Verify all items were received
    let received = store.state(|s| s.items_received.clone()).await;
    assert_eq!(received.len(), 100);
    assert_eq!(received, items);
}

// Test async stream with delays
#[derive(Clone, Debug, PartialEq)]
struct AsyncStreamState {
    items_received: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
enum AsyncStreamAction {
    StartAsyncStream { count: u32 },
    Item { value: u32 },
}

#[derive(Clone)]
struct AsyncStreamReducer;

impl Reducer for AsyncStreamReducer {
    type State = AsyncStreamState;
    type Action = AsyncStreamAction;
    type Environment = ();

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            AsyncStreamAction::StartAsyncStream { count } => {
                // Create async stream with delays
                let stream_effect = Effect::Stream(Box::pin(stream::unfold(0, move |i| async move {
                    if i < count {
                        // Small delay between items
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                        Some((AsyncStreamAction::Item { value: i }, i + 1))
                    } else {
                        None
                    }
                })));

                SmallVec::from_vec(vec![stream_effect])
            }
            AsyncStreamAction::Item { value } => {
                state.items_received.push(value);
                SmallVec::from_vec(vec![Effect::None])
            }
        }
    }
}

#[tokio::test]
async fn test_async_stream_with_delays() {
    let store = Store::new(
        AsyncStreamState {
            items_received: Vec::new(),
        },
        AsyncStreamReducer,
        (),
    );

    let start = std::time::Instant::now();

    // Start async stream with 5 items (should take ~50ms)
    store
        .send(AsyncStreamAction::StartAsyncStream { count: 5 })
        .await
        .unwrap();

    // Wait for completion
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

    let elapsed = start.elapsed();

    // Verify all items received
    let items = store.state(|s| s.items_received.clone()).await;
    assert_eq!(items.len(), 5);
    assert_eq!(items, vec![0, 1, 2, 3, 4]);

    // Verify timing (should take at least 50ms due to delays)
    assert!(
        elapsed.as_millis() >= 50,
        "Stream should take at least 50ms with delays"
    );
}

// Test multiple concurrent streams
#[derive(Clone, Debug, PartialEq)]
struct ConcurrentStreamState {
    stream_a_items: Vec<String>,
    stream_b_items: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum ConcurrentStreamAction {
    StartBothStreams,
    StreamAItem { text: String },
    StreamBItem { text: String },
}

#[derive(Clone)]
struct ConcurrentStreamReducer;

impl Reducer for ConcurrentStreamReducer {
    type State = ConcurrentStreamState;
    type Action = ConcurrentStreamAction;
    type Environment = ();

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            ConcurrentStreamAction::StartBothStreams => {
                // Create two streams in parallel
                let stream_a = Effect::Stream(Box::pin(stream::iter(
                    vec!["a1", "a2", "a3"]
                        .into_iter()
                        .map(|s| ConcurrentStreamAction::StreamAItem { text: s.to_string() }),
                )));

                let stream_b = Effect::Stream(Box::pin(stream::iter(
                    vec!["b1", "b2", "b3"]
                        .into_iter()
                        .map(|s| ConcurrentStreamAction::StreamBItem { text: s.to_string() }),
                )));

                SmallVec::from_vec(vec![Effect::Parallel(vec![stream_a, stream_b])])
            }
            ConcurrentStreamAction::StreamAItem { text } => {
                state.stream_a_items.push(text);
                SmallVec::from_vec(vec![Effect::None])
            }
            ConcurrentStreamAction::StreamBItem { text } => {
                state.stream_b_items.push(text);
                SmallVec::from_vec(vec![Effect::None])
            }
        }
    }
}

#[tokio::test]
async fn test_concurrent_streams() {
    let store = Store::new(
        ConcurrentStreamState {
            stream_a_items: Vec::new(),
            stream_b_items: Vec::new(),
        },
        ConcurrentStreamReducer,
        (),
    );

    // Start both streams concurrently
    store
        .send(ConcurrentStreamAction::StartBothStreams)
        .await
        .unwrap();

    // Wait for both to complete
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify both streams completed
    let (stream_a, stream_b) = store
        .state(|s| (s.stream_a_items.clone(), s.stream_b_items.clone()))
        .await;

    assert_eq!(stream_a.len(), 3);
    assert_eq!(stream_a, vec!["a1", "a2", "a3"]);

    assert_eq!(stream_b.len(), 3);
    assert_eq!(stream_b, vec!["b1", "b2", "b3"]);
}

// Test stream in sequential effect
#[derive(Clone, Debug, PartialEq)]
struct SequentialStreamState {
    phase: String,
    items: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum SequentialStreamAction {
    StartSequence,
    PhaseChanged { phase: String },
    Item { text: String },
}

#[derive(Clone)]
struct SequentialStreamReducer;

impl Reducer for SequentialStreamReducer {
    type State = SequentialStreamState;
    type Action = SequentialStreamAction;
    type Environment = ();

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            SequentialStreamAction::StartSequence => {
                // Sequential: phase change, then stream
                let phase_effect = Effect::Future(Box::pin(async {
                    Some(SequentialStreamAction::PhaseChanged {
                        phase: "streaming".to_string(),
                    })
                }));

                let stream_effect = Effect::Stream(Box::pin(stream::iter(
                    vec!["s1", "s2"]
                        .into_iter()
                        .map(|s| SequentialStreamAction::Item { text: s.to_string() }),
                )));

                SmallVec::from_vec(vec![Effect::Sequential(vec![phase_effect, stream_effect])])
            }
            SequentialStreamAction::PhaseChanged { phase } => {
                state.phase = phase;
                SmallVec::from_vec(vec![Effect::None])
            }
            SequentialStreamAction::Item { text } => {
                state.items.push(text);
                SmallVec::from_vec(vec![Effect::None])
            }
        }
    }
}

#[tokio::test]
async fn test_stream_in_sequential() {
    let store = Store::new(
        SequentialStreamState {
            phase: "initial".to_string(),
            items: Vec::new(),
        },
        SequentialStreamReducer,
        (),
    );

    store
        .send(SequentialStreamAction::StartSequence)
        .await
        .unwrap();

    // Wait for sequence to complete
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify phase changed before stream items
    let (phase, items) = store.state(|s| (s.phase.clone(), s.items.clone())).await;

    assert_eq!(phase, "streaming");
    assert_eq!(items, vec!["s1", "s2"]);
}

// Test backpressure by ensuring actions are processed sequentially
#[derive(Clone, Debug)]
struct BackpressureState {
    processing_log: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone, Debug, PartialEq)]
enum BackpressureAction {
    StartStream,
    Item { id: u32 },
}

#[derive(Clone)]
struct BackpressureReducer;

impl Reducer for BackpressureReducer {
    type State = BackpressureState;
    type Action = BackpressureAction;
    type Environment = ();

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            BackpressureAction::StartStream => {
                let stream_effect =
                    Effect::Stream(Box::pin(stream::iter((0..10).map(|id| BackpressureAction::Item { id }))));

                SmallVec::from_vec(vec![stream_effect])
            }
            BackpressureAction::Item { id } => {
                // Simulate slow processing
                let log = state.processing_log.clone();
                let effect = Effect::Future(Box::pin(async move {
                    tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
                    log.lock().unwrap().push(format!("processed_{}", id));
                    None
                }));

                SmallVec::from_vec(vec![effect])
            }
        }
    }
}

#[tokio::test]
async fn test_stream_backpressure() {
    let processing_log = Arc::new(Mutex::new(Vec::new()));
    let store = Store::new(
        BackpressureState {
            processing_log: processing_log.clone(),
        },
        BackpressureReducer,
        (),
    );

    store.send(BackpressureAction::StartStream).await.unwrap();

    // Wait for all items to process (10 items * 5ms = ~50ms + overhead)
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Verify all items were processed
    let log = processing_log.lock().unwrap().clone();
    assert_eq!(log.len(), 10);

    // Verify sequential order (backpressure working)
    for i in 0..10 {
        assert_eq!(log[i], format!("processed_{}", i));
    }
}
