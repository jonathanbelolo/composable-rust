# Effect::Stream Design

## Motivation

Streaming is **fundamental** to modern applications, not an optimization:

1. **LLM APIs** (Claude, GPT) stream token-by-token for UX
2. **WebSocket** (already in `web/`) streams real-time updates
3. **Server-Sent Events** stream notifications
4. **Database cursors** stream large result sets
5. **File uploads/downloads** stream data chunks

We already have WebSocket in our HTTP module. The Effect enum should reflect this reality.

## Current State

```rust
pub enum Effect<Action> {
    None,
    Parallel(Vec<Effect<Action>>),
    Sequential(Vec<Effect<Action>>),
    Delay { duration: Duration, action: Box<Action> },
    Future(Pin<Box<dyn Future<Output = Option<Action>> + Send>>),
    EventStore(EventStoreOperation<Action>),
    PublishEvent(EventBusOperation<Action>),
}
```

**Gap**: No way to represent streams that yield multiple actions over time.

## Proposed: Effect::Stream

### Core Type

```rust
use futures::stream::Stream;

pub enum Effect<Action> {
    None,
    Parallel(Vec<Effect<Action>>),
    Sequential(Vec<Effect<Action>>),
    Delay { duration: Duration, action: Box<Action> },

    /// Single-shot async computation
    Future(Pin<Box<dyn Future<Output = Option<Action>> + Send>>),

    /// Stream of actions over time (NEW)
    Stream(Pin<Box<dyn Stream<Item = Action> + Send>>),

    EventStore(EventStoreOperation<Action>),
    PublishEvent(EventBusOperation<Action>),
}
```

### Semantics

- **Future**: Yields 0 or 1 action, then completes
- **Stream**: Yields 0..N actions over time, then completes

```rust
// Future: one-shot
Effect::Future(Box::pin(async {
    Some(Action::Completed)  // Single action, then done
}))

// Stream: multiple actions
Effect::Stream(Box::pin(stream! {
    yield Action::Chunk1;
    yield Action::Chunk2;
    yield Action::Chunk3;
    // Stream ends
}))
```

### Comparison with Other Approaches

| Approach | Pros | Cons |
|----------|------|------|
| **Current (accumulate in Future)** | Simple, no enum change | Can't yield intermediate actions, bad UX |
| **Callback-based** | Flexible | Complex lifetimes, hard to test |
| **Channel-based** | Familiar pattern | Requires runtime coordination, leaks abstraction |
| **Effect::Stream** | Natural, composable, testable | Requires Stream trait in API |

**Verdict**: Effect::Stream is the right abstraction.

## Use Cases

### 1. Claude Streaming Responses

```rust
impl AgentEnvironment for ProductionEnvironment {
    fn call_claude_streaming(&self, request: MessagesRequest) -> Effect<AgentAction> {
        let client = self.client.clone();

        Effect::Stream(Box::pin(stream! {
            let mut response_stream = client.messages_stream(request).await?;

            while let Some(chunk) = response_stream.next().await {
                match chunk? {
                    StreamEvent::ContentBlockDelta { delta, .. } => {
                        yield AgentAction::StreamChunk {
                            content: delta.text,
                        };
                    }
                    StreamEvent::MessageStop => {
                        yield AgentAction::StreamComplete;
                    }
                    _ => {}
                }
            }
        }))
    }
}

// Reducer handles chunks as they arrive
impl Reducer for AgentReducer {
    fn reduce(&self, state: &mut AgentState, action: AgentAction, env: &Env)
        -> SmallVec<[Effect<AgentAction>; 4]>
    {
        match action {
            AgentAction::UserMessage { content } => {
                let request = MessagesRequest {
                    messages: vec![Message::user(content)],
                    stream: true,
                    // ...
                };

                smallvec![env.call_claude_streaming(request)]
            }

            AgentAction::StreamChunk { content } => {
                // Accumulate chunks in state
                state.current_response.push_str(&content);

                // Could emit WebSocket updates here
                smallvec![
                    Effect::None  // Or Effect::WebSocketSend if updating client
                ]
            }

            AgentAction::StreamComplete => {
                // Finalize response
                state.messages.push(Message::assistant(
                    state.current_response.clone()
                ));
                state.current_response.clear();

                smallvec![Effect::None]
            }

            _ => smallvec![Effect::None],
        }
    }
}
```

**Benefit**: UI can show tokens as they arrive, not just final response.

### 2. WebSocket Connection (Already Have This!)

```rust
// WebSocket effect that streams incoming messages
impl WebEnvironment {
    fn websocket_connect(&self, url: String) -> Effect<AppAction> {
        Effect::Stream(Box::pin(stream! {
            let ws_stream = connect_async(&url).await?;
            let (_, mut read) = ws_stream.split();

            while let Some(msg) = read.next().await {
                match msg? {
                    Message::Text(text) => {
                        yield AppAction::WebSocketMessage { text };
                    }
                    Message::Close(_) => {
                        yield AppAction::WebSocketClosed;
                        break;
                    }
                    _ => {}
                }
            }
        }))
    }
}

// Reducer handles WebSocket messages as they arrive
impl Reducer for AppReducer {
    fn reduce(&self, state: &mut AppState, action: AppAction, env: &Env)
        -> SmallVec<[Effect<AppAction>; 4]>
    {
        match action {
            AppAction::ConnectWebSocket { url } => {
                smallvec![env.websocket_connect(url)]
            }

            AppAction::WebSocketMessage { text } => {
                // Process incoming message
                state.messages.push(text);
                smallvec![Effect::None]
            }

            AppAction::WebSocketClosed => {
                // Handle disconnection
                state.connected = false;
                smallvec![env.reconnect_websocket()]
            }

            _ => smallvec![Effect::None],
        }
    }
}
```

**This aligns perfectly with our existing WebSocket code!**

### 3. Server-Sent Events

```rust
impl NotificationEnvironment {
    fn subscribe_to_events(&self, topic: String) -> Effect<NotificationAction> {
        let event_source = self.event_source.clone();

        Effect::Stream(Box::pin(stream! {
            let mut stream = event_source.subscribe(&topic).await?;

            while let Some(event) = stream.next().await {
                yield NotificationAction::EventReceived {
                    event_type: event.event_type,
                    data: event.data,
                };
            }
        }))
    }
}
```

### 4. Database Cursor (Large Result Sets)

```rust
impl QueryEnvironment {
    fn query_large_dataset(&self, query: String) -> Effect<QueryAction> {
        let db = self.database.clone();

        Effect::Stream(Box::pin(stream! {
            let mut cursor = db.query_cursor(&query).await?;

            while let Some(row) = cursor.next().await {
                yield QueryAction::RowFetched { row: row? };
            }

            yield QueryAction::QueryComplete;
        }))
    }
}

// Reducer processes rows incrementally
impl Reducer for QueryReducer {
    fn reduce(&self, state: &mut QueryState, action: QueryAction, env: &Env)
        -> SmallVec<[Effect<QueryAction>; 4]>
    {
        match action {
            QueryAction::RowFetched { row } => {
                // Process row incrementally
                state.results.push(row);

                // Publish progress update
                smallvec![Effect::PublishEvent(EventBusOperation::Publish {
                    topic: "query-progress".to_string(),
                    data: state.results.len(),
                    // ...
                })]
            }

            QueryAction::QueryComplete => {
                // Finalize results
                state.status = QueryStatus::Complete;
                smallvec![Effect::None]
            }

            _ => smallvec![Effect::None],
        }
    }
}
```

### 5. Multi-Agent Streaming (Orchestrator Pattern)

```rust
// Orchestrator spawns workers that stream results
impl Reducer for OrchestratorReducer {
    fn reduce(&self, state: &mut OrchestratorState, action: OrchestratorAction, env: &Env)
        -> SmallVec<[Effect<OrchestratorAction>; 4]>
    {
        match action {
            OrchestratorAction::TaskReceived { description } => {
                // Decompose into subtasks
                let subtasks = self.decompose_task(&description);

                // Spawn workers that stream progress
                let effects: SmallVec<_> = subtasks.into_iter()
                    .map(|subtask| env.spawn_worker_streaming(subtask))
                    .collect();

                smallvec![Effect::Parallel(effects.into_vec())]
            }

            OrchestratorAction::WorkerProgress { worker_id, progress } => {
                // Track worker progress
                state.worker_progress.insert(worker_id, progress);

                // Emit progress update
                smallvec![Effect::Stream(Box::pin(stream! {
                    yield OrchestratorAction::ProgressUpdate {
                        total_progress: self.calculate_total_progress(state),
                    };
                }))]
            }

            _ => smallvec![Effect::None],
        }
    }
}
```

## Implementation Plan

### Phase 1: Add Effect::Stream Variant

**File**: `core/src/lib.rs` (effect module)

```rust
use futures::stream::Stream;

pub enum Effect<Action> {
    None,
    Parallel(Vec<Effect<Action>>),
    Sequential(Vec<Effect<Action>>),
    Delay { duration: Duration, action: Box<Action> },
    Future(Pin<Box<dyn Future<Output = Option<Action>> + Send>>),

    /// Stream of actions over time
    ///
    /// Unlike `Future` which yields 0 or 1 action, `Stream` yields 0..N actions
    /// over time. Each item from the stream becomes a separate action fed back
    /// into the reducer.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use futures::stream;
    ///
    /// Effect::Stream(Box::pin(stream::iter(vec![
    ///     AgentAction::Chunk { text: "Hello".to_string() },
    ///     AgentAction::Chunk { text: " world".to_string() },
    ///     AgentAction::Complete,
    /// ])))
    /// ```
    Stream(Pin<Box<dyn Stream<Item = Action> + Send>>),

    EventStore(EventStoreOperation<Action>),
    PublishEvent(EventBusOperation<Action>),
}
```

**Dependencies**: Add `futures` to `core/Cargo.toml`:

```toml
[dependencies]
futures = "0.3"
```

### Phase 2: Update Debug Impl

```rust
impl<Action> std::fmt::Debug for Effect<Action>
where
    Action: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Effect::None => write!(f, "Effect::None"),
            Effect::Parallel(effects) => {
                f.debug_tuple("Effect::Parallel").field(effects).finish()
            },
            Effect::Sequential(effects) => {
                f.debug_tuple("Effect::Sequential").field(effects).finish()
            },
            Effect::Delay { duration, action } => f
                .debug_struct("Effect::Delay")
                .field("duration", duration)
                .field("action", action)
                .finish(),
            Effect::Future(_) => write!(f, "Effect::Future(<future>)"),
            Effect::Stream(_) => write!(f, "Effect::Stream(<stream>)"),  // NEW
            Effect::EventStore(op) => {
                // ... existing EventStore debug
            },
            Effect::PublishEvent(op) => {
                // ... existing PublishEvent debug
            },
        }
    }
}
```

### Phase 3: Update map() Implementation

```rust
impl<Action> Effect<Action> {
    pub fn map<B, F>(self, f: F) -> Effect<B>
    where
        F: Fn(Action) -> B + Send + Sync + 'static + Clone,
        Action: 'static,
        B: Send + 'static,
    {
        match self {
            Effect::None => Effect::None,
            Effect::Parallel(effects) => {
                let mapped = effects.into_iter()
                    .map(|e| e.map(f.clone()))
                    .collect();
                Effect::Parallel(mapped)
            },
            Effect::Sequential(effects) => {
                let mapped = effects.into_iter()
                    .map(|e| e.map(f.clone()))
                    .collect();
                Effect::Sequential(mapped)
            },
            Effect::Delay { duration, action } => Effect::Delay {
                duration,
                action: Box::new(f(*action)),
            },
            Effect::Future(fut) => {
                Effect::Future(Box::pin(async move { fut.await.map(f) }))
            },
            Effect::Stream(stream) => {
                Effect::Stream(Box::pin(stream.map(f)))  // NEW
            },
            Effect::EventStore(op) => Effect::EventStore(map_event_store_operation(op, f)),
            Effect::PublishEvent(op) => Effect::PublishEvent(map_event_bus_operation(op, f)),
        }
    }
}
```

**Note**: Stream already has `map()` method from `StreamExt` trait, so this is ergonomic!

### Phase 4: Update Runtime to Execute Streams

**File**: `runtime/src/lib.rs`

```rust
use futures::StreamExt;

impl<S, A, E> Store<S, A, E>
where
    S: Clone + Send + Sync + 'static,
    A: Send + 'static,
    E: Send + Sync + 'static,
{
    async fn execute_effect(&mut self, effect: Effect<A>) {
        match effect {
            Effect::None => {},

            Effect::Future(fut) => {
                if let Some(action) = fut.await {
                    self.send(action).await;
                }
            },

            Effect::Stream(mut stream) => {
                // Execute stream, feeding each item back as an action
                while let Some(action) = stream.next().await {
                    self.send(action).await;
                }
            },

            Effect::Parallel(effects) => {
                let futures: Vec<_> = effects.into_iter()
                    .map(|effect| {
                        let mut store_clone = self.clone();
                        async move {
                            store_clone.execute_effect(effect).await;
                        }
                    })
                    .collect();

                futures::future::join_all(futures).await;
            },

            Effect::Sequential(effects) => {
                for effect in effects {
                    self.execute_effect(effect).await;
                }
            },

            // ... other variants
        }
    }
}
```

**Key**: Each item from the stream becomes a separate `send(action)`, feeding back into the reducer loop.

### Phase 5: Add Tests

**File**: `core/src/lib.rs` (tests module)

```rust
#[tokio::test]
async fn test_effect_stream() {
    use futures::stream;

    let effect: Effect<TestAction> = Effect::Stream(Box::pin(stream::iter(vec![
        TestAction::Action1,
        TestAction::Action2,
        TestAction::Action3,
    ])));

    match effect {
        Effect::Stream(mut s) => {
            let items: Vec<_> = s.collect().await;
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], TestAction::Action1);
            assert_eq!(items[1], TestAction::Action2);
            assert_eq!(items[2], TestAction::Action3);
        }
        _ => panic!("Expected Stream effect"),
    }
}

#[tokio::test]
async fn test_effect_stream_map() {
    use futures::stream;

    let effect: Effect<TestAction> = Effect::Stream(Box::pin(stream::iter(vec![
        TestAction::Action1,
        TestAction::Action2,
    ])));

    let mapped: Effect<MappedAction> = effect.map(|a| MappedAction::Mapped(a));

    match mapped {
        Effect::Stream(mut s) => {
            let items: Vec<_> = s.collect().await;
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], MappedAction::Mapped(TestAction::Action1));
            assert_eq!(items[1], MappedAction::Mapped(TestAction::Action2));
        }
        _ => panic!("Expected Stream effect"),
    }
}

#[tokio::test]
async fn test_effect_stream_async() {
    use futures::stream;
    use tokio::time::{sleep, Duration};

    let effect: Effect<TestAction> = Effect::Stream(Box::pin(stream::unfold(0, |count| async move {
        if count < 3 {
            sleep(Duration::from_millis(10)).await;
            Some((TestAction::Action1, count + 1))
        } else {
            None
        }
    })));

    match effect {
        Effect::Stream(mut s) => {
            let start = std::time::Instant::now();
            let items: Vec<_> = s.collect().await;
            let elapsed = start.elapsed();

            assert_eq!(items.len(), 3);
            assert!(elapsed.as_millis() >= 30, "Should take ~30ms for 3 items");
        }
        _ => panic!("Expected Stream effect"),
    }
}

#[tokio::test]
async fn test_stream_in_parallel() {
    use futures::stream;

    let effect: Effect<TestAction> = Effect::Parallel(vec![
        Effect::Stream(Box::pin(stream::iter(vec![TestAction::Action1]))),
        Effect::Stream(Box::pin(stream::iter(vec![TestAction::Action2]))),
    ]);

    // Verify structure
    match effect {
        Effect::Parallel(effects) => {
            assert_eq!(effects.len(), 2);
            for effect in effects {
                assert!(matches!(effect, Effect::Stream(_)));
            }
        }
        _ => panic!("Expected Parallel effect"),
    }
}
```

### Phase 6: Integration Tests with Store

**File**: `runtime/tests/stream_tests.rs`

```rust
use composable_rust_core::effect::Effect;
use composable_rust_runtime::Store;
use futures::stream;

#[derive(Clone)]
struct StreamState {
    items: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum StreamAction {
    StartStreaming,
    Item { text: String },
    Complete,
}

struct StreamReducer;

impl Reducer for StreamReducer {
    type State = StreamState;
    type Action = StreamAction;
    type Environment = StreamEnvironment;

    fn reduce(&self, state: &mut Self::State, action: Self::Action, env: &Self::Environment)
        -> SmallVec<[Effect<Self::Action>; 4]>
    {
        match action {
            StreamAction::StartStreaming => {
                smallvec![env.create_stream()]
            }

            StreamAction::Item { text } => {
                state.items.push(text);
                smallvec![Effect::None]
            }

            StreamAction::Complete => {
                smallvec![Effect::None]
            }
        }
    }
}

struct StreamEnvironment;

impl StreamEnvironment {
    fn create_stream(&self) -> Effect<StreamAction> {
        Effect::Stream(Box::pin(stream::iter(vec![
            StreamAction::Item { text: "chunk1".to_string() },
            StreamAction::Item { text: "chunk2".to_string() },
            StreamAction::Item { text: "chunk3".to_string() },
            StreamAction::Complete,
        ])))
    }
}

#[tokio::test]
async fn test_store_executes_stream() {
    let state = StreamState { items: vec![] };
    let reducer = StreamReducer;
    let env = StreamEnvironment;

    let mut store = Store::new(reducer, state, env);

    // Start streaming
    store.send(StreamAction::StartStreaming).await;

    // Give time for stream to complete
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify all items were received
    let final_state = store.state();
    assert_eq!(final_state.items.len(), 3);
    assert_eq!(final_state.items[0], "chunk1");
    assert_eq!(final_state.items[1], "chunk2");
    assert_eq!(final_state.items[2], "chunk3");
}
```

## Design Decisions

### 1. Why Not Result<Action, Error>?

**Question**: Should streams yield `Result<Action, Error>` instead of `Action`?

**Answer**: No. Errors should be **actions**.

```rust
// ❌ BAD: Errors in stream type
Effect::Stream(Pin<Box<dyn Stream<Item = Result<Action, Error>> + Send>>)

// ✅ GOOD: Errors are actions
enum AgentAction {
    StreamChunk { text: String },
    StreamError { error: String },
    StreamComplete,
}

Effect::Stream(Box::pin(stream! {
    match fetch_data().await {
        Ok(data) => yield AgentAction::StreamChunk { text: data },
        Err(e) => yield AgentAction::StreamError { error: e.to_string() },
    }
})))
```

**Rationale**:
- Errors are domain events (reducer decides how to handle)
- Keeps Effect type simple
- Aligns with "actions as inputs" philosophy

### 2. When to Use Stream vs Future?

| Use Case | Effect Type |
|----------|-------------|
| Single API call | `Effect::Future` |
| LLM streaming response | `Effect::Stream` |
| Database query (small) | `Effect::Future` |
| Database cursor (large) | `Effect::Stream` |
| WebSocket message | `Effect::Stream` |
| HTTP request | `Effect::Future` |
| Server-Sent Events | `Effect::Stream` |

**Rule**: Use Stream when you need **intermediate feedback** before completion.

### 3. Cancellation?

**Question**: How to cancel a running stream?

**Answer**: Add `Effect::Cancellable` in future phase:

```rust
Effect::Cancellable {
    id: EffectId,
    effect: Box<Effect<Action>>,
}

// Reducer can emit:
AgentAction::CancelStream { id: EffectId }

// Runtime tracks cancellation tokens
```

**For Phase 8**: Not critical. Streams naturally end when source completes.

### 4. Backpressure?

**Question**: What if reducer can't keep up with stream?

**Answer**: Rust's Stream trait handles this via `poll_next()`. If reducer is slow, stream naturally backs off.

**For Phase 8**: Not a concern. LLM APIs rate-limit themselves.

## Documentation Updates

### User Guide: Streaming Effects

```markdown
# Streaming Effects

Use `Effect::Stream` when you need to process multiple values over time:

## Example: LLM Streaming Response

```rust
impl AgentEnvironment for ProductionEnvironment {
    fn call_claude_streaming(&self, request: MessagesRequest) -> Effect<AgentAction> {
        let client = self.client.clone();

        Effect::Stream(Box::pin(async_stream::stream! {
            let mut stream = client.messages_stream(request).await?;

            while let Some(chunk) = stream.next().await {
                yield AgentAction::StreamChunk {
                    content: chunk?.delta.text,
                };
            }

            yield AgentAction::StreamComplete;
        }))
    }
}
```

## When to Use

- **LLM streaming**: Show tokens as they arrive
- **WebSocket**: Handle incoming messages
- **Large datasets**: Process rows incrementally
- **Real-time updates**: Stream notifications

## Testing

Mock streams with `futures::stream::iter()`:

```rust
#[tokio::test]
async fn test_streaming() {
    let mock_env = MockEnvironment {
        stream: stream::iter(vec![
            Action::Chunk { text: "Hello" },
            Action::Chunk { text: " world" },
            Action::Complete,
        ]),
    };

    // Test reducer with mock stream
}
```
```

## Migration Path

### Phase 1 (Immediate)
1. Add `Effect::Stream` variant to `core/src/lib.rs`
2. Update `Debug` impl
3. Update `map()` method
4. Add unit tests

**Impact**: Core architecture only, no API breakage

### Phase 2 (Phase 8.1-8.2)
5. Update `runtime/` to execute streams
6. Add integration tests
7. Document streaming patterns

**Impact**: Runtime only, existing code still works

### Phase 3 (Phase 8.3+)
8. Use streams in agent patterns
9. WebSocket integration examples
10. Performance benchmarks

**Impact**: Agent patterns leverage streaming

## Success Metrics

- [ ] `Effect::Stream` variant compiles
- [ ] All existing tests pass (no breakage)
- [ ] Stream map() works correctly
- [ ] Runtime executes streams (integration test)
- [ ] LLM streaming example works end-to-end
- [ ] WebSocket streaming example works
- [ ] Documentation complete

## Conclusion

**Effect::Stream is essential, not optional.**

With WebSocket already in our architecture and LLM streaming being fundamental, we need Stream as a first-class effect variant **now**, not later.

The implementation is straightforward:
1. Add variant to enum (10 lines)
2. Update Debug (2 lines)
3. Update map (2 lines)
4. Update runtime executor (5 lines)
5. Add tests (50 lines)

**Total effort**: ~2 hours to implement, ~1 day to document and test thoroughly.

**Recommendation**: Add Effect::Stream before Phase 8.1 begins. It's a foundational capability.
