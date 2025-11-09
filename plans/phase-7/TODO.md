# Phase 7: Axum Web Framework Integration - TODO

**Goal**: Add action broadcasting to Store and integrate Axum for HTTP request-response and WebSocket streaming.

**Status**: 🚧 In Progress

**Duration**: 10 days (5 sprints)

---

## Overview

Phase 7 adds **action observation** to the Store, enabling HTTP request-response patterns and real-time event streaming while maintaining functional purity. The HTTP layer observes domain actions and maps them to responses without coupling business logic to HTTP.

**Key Innovation**: `store.send_and_wait_for(action, predicate, timeout)` allows HTTP handlers to wait for terminal actions from sagas.

---

## Sprint 7.1: Store Action Broadcasting (2 days)

**Goal**: Add action broadcasting infrastructure to the Store

### Tasks

#### 1. Add broadcast channel to Store (3 hours)

**File**: `runtime/src/lib.rs`

- [ ] Add `action_broadcast` field to Store struct:
  ```rust
  pub struct Store<S, A, E, R> {
      // ... existing fields
      action_broadcast: broadcast::Sender<A>,
  }
  ```

- [ ] Update `Store::new()` to create broadcast channel:
  ```rust
  pub fn new(initial_state: S, reducer: R, environment: E) -> Self {
      let (action_broadcast, _) = broadcast::channel(16);  // Default capacity
      Self {
          // ... existing fields
          action_broadcast,
      }
  }
  ```

- [ ] Add `Store::with_broadcast_capacity()` constructor for custom capacity

- [ ] Update `Clone` impl if needed (broadcast::Sender is already Clone)

#### 2. Broadcast actions from effect execution (2 hours)

**File**: `runtime/src/lib.rs` (around line 1820)

- [ ] Modify `Effect::Future` execution in `execute_effect_internal`:
  ```rust
  if let Some(action) = fut.await {
      // NEW: Broadcast to observers
      let _ = self.action_broadcast.send(action.clone());

      // Existing: Auto-feedback
      let _ = store.send(action).await;
  }
  ```

- [ ] Modify `Effect::Delay` execution similarly

- [ ] Consider: Should we also broadcast the initial action from `send()`?
  - Decision: No, only broadcast actions produced by effects (result actions)

#### 3. Implement `send_and_wait_for` method (4 hours)

**File**: `runtime/src/lib.rs`

- [ ] Add method to Store:
  ```rust
  pub async fn send_and_wait_for<F>(
      &self,
      action: A,
      predicate: F,
      timeout: Duration,
  ) -> Result<A, StoreError>
  where
      F: Fn(&A) -> bool,
  {
      // 1. Subscribe BEFORE sending (avoid race)
      let mut rx = self.action_broadcast.subscribe();

      // 2. Send initial action
      self.send(action).await?;

      // 3. Wait for matching action
      tokio::time::timeout(timeout, async {
          loop {
              match rx.recv().await {
                  Ok(action) if predicate(&action) => return Ok(action),
                  Ok(_) => continue,
                  Err(broadcast::error::RecvError::Lagged(skipped)) => {
                      tracing::warn!(skipped, "Observer lagged");
                      continue;
                  }
                  Err(broadcast::error::RecvError::Closed) => {
                      return Err(StoreError::ChannelClosed);
                  }
              }
          }
      })
      .await
      .map_err(|_| StoreError::Timeout)?
  }
  ```

- [ ] Add comprehensive documentation with examples

#### 4. Implement `subscribe_actions` method (1 hour)

**File**: `runtime/src/lib.rs`

- [ ] Add method:
  ```rust
  pub fn subscribe_actions(&self) -> broadcast::Receiver<A> {
      self.action_broadcast.subscribe()
  }
  ```

- [ ] Document use cases (WebSockets, SSE, metrics)

#### 5. Add new error variants (30 min)

**File**: `runtime/src/lib.rs`

- [ ] Add to `StoreError` enum:
  ```rust
  pub enum StoreError {
      // ... existing variants

      /// Timeout waiting for terminal action
      #[error("Timeout waiting for action")]
      Timeout,

      /// Action broadcast channel closed
      #[error("Action broadcast channel closed")]
      ChannelClosed,
  }
  ```

#### 6. Write tests (4 hours)

**File**: `runtime/tests/broadcasting.rs` (new file)

- [ ] Test `send_and_wait_for` with immediate response
- [ ] Test `send_and_wait_for` with delayed response (saga)
- [ ] Test timeout behavior
- [ ] Test concurrent subscribers
- [ ] Test lagging subscriber behavior
- [ ] Test `subscribe_actions` streaming
- [ ] Test correlation ID filtering
- [ ] Benchmark broadcasting overhead

**Success Criteria**:
- ✅ Store broadcasts all actions produced by effects
- ✅ `send_and_wait_for` correctly waits for matching actions
- ✅ Timeout handling works
- ✅ Multiple concurrent subscribers work
- ✅ All tests passing
- ✅ Performance: <200ns overhead per action

---

## Sprint 7.2: Web Crate Foundation (1 day)

**Goal**: Create generic web utilities for ANY domain

### Tasks

#### 1. Create web crate structure (1 hour)

- [ ] Create `web/` directory
- [ ] Create `web/Cargo.toml`:
  ```toml
  [package]
  name = "composable-rust-web"
  version.workspace = true
  edition.workspace = true

  [dependencies]
  composable-rust-core = { path = "../core" }
  composable-rust-runtime = { path = "../runtime" }

  axum = "0.7"
  tokio = { workspace = true }
  tower = "0.5"
  tower-http = { version = "0.6", features = ["cors", "trace", "compression-gzip"] }
  serde = { workspace = true }
  serde_json = "1"
  http = "1"
  hyper = "1"
  tracing = { workspace = true }
  uuid = { version = "1", features = ["serde", "v4"] }

  [dev-dependencies]
  composable-rust-testing = { path = "../testing" }
  axum-test = "16"
  ```

- [ ] Add to workspace `Cargo.toml`
- [ ] Create module structure

#### 2. Implement AppError (2 hours)

**File**: `web/src/error.rs`

- [ ] Create generic error type:
  ```rust
  pub struct AppError {
      status: StatusCode,
      message: String,
      code: String,
      source: Option<anyhow::Error>,
  }
  ```

- [ ] Implement constructor helpers:
  - `bad_request(msg)`
  - `unauthorized(msg)`
  - `forbidden(msg)`
  - `not_found(resource, id)`
  - `conflict(msg)`
  - `validation(msg)`
  - `timeout(msg)`
  - `internal(msg)`
  - `unavailable(msg)`

- [ ] Implement `IntoResponse`:
  - Log 5xx errors
  - Return JSON body with code + message

- [ ] Implement `From<anyhow::Error>`
- [ ] Write tests

#### 3. Create generic extractors (2 hours)

**File**: `web/src/extractors.rs`

- [ ] `CorrelationId` extractor (from header or generate):
  ```rust
  pub struct CorrelationId(pub Uuid);

  #[async_trait]
  impl<S> FromRequestParts<S> for CorrelationId {
      // Extract from X-Correlation-ID header or generate new
  }
  ```

- [ ] `ClientIp` extractor (from X-Forwarded-For or connection):
  ```rust
  pub struct ClientIp(pub IpAddr);
  ```

- [ ] `UserAgent` extractor:
  ```rust
  pub struct UserAgent(pub String);
  ```

- [ ] Write tests for each extractor

#### 4. Health check endpoint (1 hour)

**File**: `web/src/handlers/health.rs`

- [ ] Generic health check that works with any Store:
  ```rust
  pub async fn health_check<S, A, E, R>(
      State(store): State<Arc<Store<S, A, E, R>>>,
  ) -> impl IntoResponse
  where
      R: Reducer<State = S, Action = A, Environment = E>,
  {
      let health = store.health();
      let status = match health.status {
          HealthStatus::Healthy => StatusCode::OK,
          HealthStatus::Degraded => StatusCode::OK,
          HealthStatus::Unhealthy => StatusCode::SERVICE_UNAVAILABLE,
      };
      (status, Json(health))
  }
  ```

- [ ] Test health endpoint

#### 5. Documentation (1 hour)

- [ ] Create `web/README.md` explaining:
  - Purpose of web crate (generic utilities)
  - Domain handlers go in domain crates
  - Examples of usage

- [ ] Add module-level docs

**Success Criteria**:
- ✅ Web crate compiles
- ✅ AppError works and has good ergonomics
- ✅ Extractors work
- ✅ Health check endpoint works
- ✅ All tests passing
- ✅ Documentation clear

---

## Sprint 7.3: Auth HTTP Handlers (3 days)

**Goal**: Implement auth-specific HTTP handlers using `send_and_wait_for`

**Note**: These go in `auth` crate, NOT `web` crate

### Tasks

#### 1. Add correlation IDs to AuthAction (2 hours)

**File**: `auth/src/actions.rs`

- [ ] Add `correlation_id: Uuid` to all command actions:
  - `SendMagicLink`
  - `VerifyMagicLink`
  - `InitiateOAuth`
  - `CompleteOAuth`
  - `RegisterPasskey`
  - `AuthenticatePasskey`

- [ ] Add `correlation_id` to corresponding event actions:
  - `MagicLinkSent`
  - `MagicLinkFailed`
  - `SessionCreated`
  - etc.

- [ ] Update reducers to propagate correlation IDs

#### 2. Magic Link handlers (4 hours)

**File**: `auth/src/handlers/magic_link.rs` (new file)

- [ ] `send_magic_link` handler:
  - Extract email from request
  - Generate correlation_id
  - Call `store.send_and_wait_for()` with 10s timeout
  - Wait for `MagicLinkSent` or `MagicLinkFailed`
  - Map to HTTP response

- [ ] `verify_magic_link` handler:
  - Extract token from query params
  - Call `send_and_wait_for()` with 5s timeout
  - Wait for `SessionCreated` or `InvalidToken`
  - Set session cookie
  - Return session info or error

- [ ] Write integration tests

#### 3. OAuth handlers (6 hours)

**File**: `auth/src/handlers/oauth.rs` (new file)

- [ ] `oauth_authorize` handler:
  - Extract provider from path
  - Generate correlation_id
  - Send `InitiateOAuth` action
  - Wait for `OAuthUrlGenerated`
  - Redirect to OAuth provider

- [ ] `oauth_callback` handler:
  - Extract code and state from query
  - Send `CompleteOAuth` action
  - Wait for `SessionCreated` or `OAuthFailed`
  - Set session cookie
  - Redirect to app

- [ ] Write integration tests
- [ ] Test error cases (invalid provider, failed OAuth)

#### 4. Passkey handlers (8 hours)

**File**: `auth/src/handlers/passkey.rs` (new file)

- [ ] `register_begin` handler:
  - Generate correlation_id
  - Send `BeginPasskeyRegistration`
  - Wait for `PasskeyOptionsGenerated`
  - Return WebAuthn challenge

- [ ] `register_complete` handler:
  - Extract credential from request
  - Send `CompletePasskeyRegistration`
  - Wait for `PasskeyRegistered` or `RegistrationFailed`
  - Return success or error

- [ ] `login_begin` handler:
  - Send `BeginPasskeyAuth`
  - Wait for `PasskeyOptionsGenerated`
  - Return challenge

- [ ] `login_complete` handler:
  - Extract assertion from request
  - Send `CompletePasskeyAuth`
  - Wait for `SessionCreated` or `AuthFailed`
  - Set session cookie

- [ ] Write integration tests for full flow
- [ ] Test error cases

#### 5. Session handlers (3 hours)

**File**: `auth/src/handlers/session.rs` (new file)

- [ ] `get_session` handler (read from projection):
  - Extract session ID from cookie
  - Query session projection
  - Return session info

- [ ] `refresh_session` handler:
  - Extract session ID
  - Send `RefreshSession`
  - Wait for `SessionRefreshed`
  - Return new session

- [ ] `logout` handler:
  - Extract session ID
  - Send `Logout` action (fire-and-forget)
  - Clear cookie
  - Return success

- [ ] Write tests

#### 6. Router composition (1 hour)

**File**: `auth/src/router.rs` (new file)

- [ ] Create `auth_router()` function:
  ```rust
  pub fn auth_router<S>(
      store: Arc<Store<...>>,
  ) -> Router<S> {
      Router::new()
          .route("/magic-link/send", post(handlers::send_magic_link))
          .route("/magic-link/verify", get(handlers::verify_magic_link))
          .route("/oauth/:provider/authorize", get(handlers::oauth_authorize))
          .route("/oauth/:provider/callback", get(handlers::oauth_callback))
          .route("/passkey/register/begin", post(handlers::register_begin))
          .route("/passkey/register/complete", post(handlers::register_complete))
          .route("/passkey/login/begin", post(handlers::login_begin))
          .route("/passkey/login/complete", post(handlers::login_complete))
          .route("/session/me", get(handlers::get_session))
          .route("/session/refresh", post(handlers::refresh_session))
          .route("/session/logout", post(handlers::logout))
          .with_state(store)
  }
  ```

**Success Criteria**:
- ✅ All auth endpoints implemented
- ✅ Correlation IDs working
- ✅ `send_and_wait_for` used correctly
- ✅ Timeout handling works
- ✅ Integration tests passing
- ✅ Error cases handled

---

## Sprint 7.4: WebSocket Streaming (2 days)

**Goal**: Implement WebSocket endpoint for real-time action streaming

### Tasks

#### 1. WebSocket endpoint (3 hours)

**File**: `web/src/handlers/websocket.rs` (new file)

- [ ] Implement basic WebSocket handler:
  ```rust
  pub async fn websocket_handler<S, A, E, R>(
      ws: WebSocketUpgrade,
      State(store): State<Arc<Store<S, A, E, R>>>,
  ) -> impl IntoResponse
  where
      A: Serialize + Clone + Send + 'static,
      R: Reducer<State = S, Action = A, Environment = E>,
  {
      ws.on_upgrade(move |socket| handle_socket(socket, store))
  }

  async fn handle_socket<S, A, E, R>(
      socket: WebSocket,
      store: Arc<Store<S, A, E, R>>,
  ) {
      let (mut sender, _receiver) = socket.split();
      let mut rx = store.subscribe_actions();

      while let Ok(action) = rx.recv().await {
          let json = serde_json::to_string(&action).unwrap();
          if sender.send(Message::Text(json)).await.is_err() {
              break;  // Client disconnected
          }
      }
  }
  ```

- [ ] Add to router
- [ ] Test basic WebSocket connection

#### 2. Filtered streaming (3 hours)

**File**: `auth/src/handlers/websocket.rs` (new file)

- [ ] Auth-specific WebSocket with filtering:
  ```rust
  pub async fn user_events_websocket(
      ws: WebSocketUpgrade,
      State(store): State<Arc<Store<...>>>,
      SessionGuard(session): SessionGuard,
  ) -> impl IntoResponse {
      let user_id = session.user_id;
      ws.on_upgrade(move |socket| {
          stream_user_events(socket, store, user_id)
      })
  }

  async fn stream_user_events(
      socket: WebSocket,
      store: Arc<Store<...>>,
      user_id: UserId,
  ) {
      let (mut sender, _) = socket.split();
      let mut rx = store.subscribe_actions();

      while let Ok(action) = rx.recv().await {
          // Filter: only send actions for this user
          if action.user_id() == Some(&user_id) {
              let json = serde_json::to_string(&action).unwrap();
              let _ = sender.send(Message::Text(json)).await;
          }
      }
  }
  ```

- [ ] Implement `user_id()` helper on `AuthAction`
- [ ] Test filtering

#### 3. Lagging handling (2 hours)

- [ ] Handle `RecvError::Lagged` gracefully:
  - Log warning
  - Optionally send "gap" message to client
  - Continue streaming

- [ ] Test lagging behavior (slow client)

#### 4. Reconnection handling (2 hours)

- [ ] Add heartbeat/ping-pong
- [ ] Handle client reconnection
- [ ] Test reconnection

#### 5. Integration tests (2 hours)

**File**: `auth/tests/websocket_tests.rs` (new file)

- [ ] Test WebSocket connection
- [ ] Test receiving actions
- [ ] Test filtering by user
- [ ] Test client disconnect
- [ ] Test multiple concurrent clients

**Success Criteria**:
- ✅ WebSocket endpoint works
- ✅ Actions stream in real-time
- ✅ Filtering works
- ✅ Lagging handled gracefully
- ✅ Tests passing

---

## Sprint 7.5: Example Application (2 days)

**Goal**: Complete reference app demonstrating all patterns

### Tasks

#### 1. Example app structure (2 hours)

**File**: `examples/auth-server/` (new directory)

- [ ] Create binary crate
- [ ] Add dependencies
- [ ] Create main.rs with server setup

#### 2. Server composition (3 hours)

**File**: `examples/auth-server/src/main.rs`

- [ ] Set up Store with real dependencies:
  - PostgreSQL event store
  - Redis session store
  - Email sender
  - OAuth clients

- [ ] Compose Axum router:
  ```rust
  let app = Router::new()
      .route("/health", get(web::health_check))
      .nest("/api/v1/auth", auth::router(auth_store))
      .nest("/ws", websocket_router(auth_store))
      .layer(TraceLayer::new_for_http())
      .layer(CorsLayer::permissive());
  ```

- [ ] Run server

#### 3. Docker Compose setup (2 hours)

**File**: `examples/auth-server/docker-compose.yml`

- [ ] Add PostgreSQL
- [ ] Add Redis
- [ ] Add example app service
- [ ] Add env file with config

#### 4. Example client (4 hours)

**File**: `examples/auth-server/client/` (simple HTML/JS)

- [ ] Magic link flow
- [ ] OAuth flow
- [ ] Passkey flow
- [ ] WebSocket event display

#### 5. Load testing (2 hours)

**File**: `examples/auth-server/loadtest/` (k6 or similar)

- [ ] Write load test script
- [ ] Test concurrent requests
- [ ] Measure latency
- [ ] Verify no action broadcasting overhead

#### 6. Documentation (3 hours)

**File**: `examples/auth-server/README.md`

- [ ] Explain architecture
- [ ] How to run
- [ ] How to test
- [ ] Performance metrics

**Success Criteria**:
- ✅ Example app runs
- ✅ All auth flows work end-to-end
- ✅ WebSocket streams work
- ✅ Load tests pass
- ✅ Documentation complete

---

## Final Checklist

Before marking Phase 7 complete:

- [ ] All sprints completed
- [ ] All tests passing (unit, integration, load)
- [ ] Documentation complete:
  - [ ] SPEC.md updated
  - [ ] TODO.md updated
  - [ ] API docs generated
  - [ ] Example README
- [ ] Performance benchmarks run:
  - [ ] Action broadcasting overhead < 200ns
  - [ ] HTTP latency acceptable
  - [ ] WebSocket throughput acceptable
- [ ] Code review completed
- [ ] No clippy warnings
- [ ] Git commit with comprehensive message
- [ ] Update main project README

---

## Success Metrics

- ✅ Store can broadcast actions to multiple observers
- ✅ HTTP handlers can wait for terminal actions with `send_and_wait_for`
- ✅ WebSocket clients can stream actions in real-time
- ✅ Timeout handling works correctly
- ✅ Zero coupling between domain logic and HTTP
- ✅ Correlation IDs enable concurrent request handling
- ✅ Performance: <200ns overhead per action
- ✅ Example app demonstrates all patterns
- ✅ All tests passing

---

## Notes

- The web crate is GENERIC - works with any domain
- Auth handlers go in auth crate, NOT web crate
- This pattern applies to other frameworks (Actix, Rocket, etc.)
- WebSocket is optional but demonstrates the power of action broadcasting
