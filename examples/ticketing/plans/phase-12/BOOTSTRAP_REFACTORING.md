# Phase 12.5: Bootstrap Refactoring for DSL Generation

**Goal**: Refactor `main.rs` bootstrap code into a clean, composable, declarative structure suitable for DSL code generation.

**Current Status**: 814 lines of monolithic bootstrap code with significant duplication

**Target**: ~50 line main.rs with modular, reusable bootstrap components

**Estimated Effort**: 8-12 hours

**Priority**: Foundation for Phase 13 (DSL-based application generation)

---

## Executive Summary

The current `main.rs` contains 814 lines of bootstrap code with 436 lines of duplicated consumer logic across 4 nearly-identical event consumers. This makes the code:
- Hard to maintain (changes must be repeated 4 times)
- Hard to test (monolithic functions)
- Hard to generate from DSL (no clear patterns)
- Contains dead code (unused circuit breakers)

This refactoring will:
1. Extract a generic `EventConsumer` (reduces 436 lines → ~60 lines)
2. Create modular bootstrap components (resources, aggregates, projections)
3. Implement builder pattern for declarative application construction
4. Separate concerns (setup, wiring, lifecycle management)
5. Fix bugs (remove dead code, eliminate duplication)

---

## Current State Analysis

### File Structure

```
src/
├── main.rs              (814 lines - MONOLITHIC)
├── aggregates/          (Domain logic - GOOD)
├── projections/         (Read models - GOOD)
├── api/                 (HTTP handlers - GOOD)
├── server/              (AppState, routes - GOOD)
└── auth/                (Auth setup - GOOD)
```

### main.rs Breakdown

| Section | Lines | Purpose | Issues |
|---------|-------|---------|--------|
| `main()` | 37-307 (270 lines) | Bootstrap everything | Too large, sequential dependencies |
| `spawn_aggregate_consumers()` | 309-556 (247 lines) | Inventory + Payment consumers | 95% duplicated code |
| `spawn_analytics_consumers()` | 558-780 (222 lines) | Sales + Customer consumers | 95% duplicated code |
| `shutdown_signal()` | 782-814 (32 lines) | Graceful shutdown | Good, keep as-is |

**Total duplicated consumer code**: ~436 lines (4 consumers × ~109 lines each)

### Detailed Issues

#### Issue 1: Duplicate Consumer Boilerplate

All 4 consumers follow identical patterns:

```rust
// Pattern repeated 4 times with slight variations:
loop {
    tokio::select! {
        _ = shutdown.recv() => break,
        subscribe_result = bus.subscribe(topics) => {
            match subscribe_result {
                Ok(mut stream) => {
                    loop {
                        tokio::select! {
                            _ = shutdown.recv() => return,
                            event_result = stream.next() => {
                                match event_result {
                                    Some(Ok(serialized_event)) => {
                                        match bincode::deserialize::<TicketingEvent>(&serialized_event.data) {
                                            Ok(event) => {
                                                // ONLY PART THAT DIFFERS (5-15 lines)
                                                handle_event(event).await;
                                            }
                                            Err(e) => warn!("Deserialize error"),
                                        }
                                    }
                                    Some(Err(e)) => error!("Stream error"),
                                    None => break, // Reconnect
                                }
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                Err(e) => {
                    error!("Subscribe failed, retry in 5s");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }
}
```

**Analysis**: Only the "handle_event" portion (5-15 lines) differs between consumers. The subscribe-process-reconnect loop (94-104 lines) is identical.

#### Issue 2: Dead Code (Unused Circuit Breakers)

**Location**: main.rs:203-228

```rust
// Lines 203-228: Circuit breakers created but NEVER USED
let event_bus_breaker = Arc::new(CircuitBreaker::new(...));           // ❌ Never used
let payment_gateway_breaker = Arc::new(CircuitBreaker::new(...));     // ❌ Shadows first definition

// Lines 115-123: First payment_gateway_breaker (ACTUALLY USED)
let payment_gateway_breaker = Arc::new(CircuitBreaker::new(...));     // ✅ Used at line 136
```

**Bug**: Second batch of circuit breakers (lines 203-228) creates unused variables. This is dead code that should be removed.

#### Issue 3: No Separation of Concerns

The 270-line `main()` function handles:
1. Environment setup (dotenv, tracing)
2. Configuration loading
3. Database connection + migrations (2 databases)
4. Event store setup
5. Event bus setup
6. Aggregate queries initialization
7. Payment gateway setup
8. Circuit breaker setup (duplicate)
9. Consumer spawning (aggregate + analytics)
10. Auth setup
11. Projection setup
12. AppState construction
13. HTTP server setup
14. Graceful shutdown coordination

**Problem**: Sequential dependencies make testing impossible. Cannot test consumer setup without database setup. Cannot mock resources for testing.

#### Issue 4: Hard-Coded Configuration

Circuit breakers, retry delays, and topic names are hard-coded:

```rust
// Line 117-121: Hard-coded configuration
CircuitBreakerConfig::builder()
    .failure_threshold(5)
    .timeout(Duration::from_secs(30))
    .success_threshold(2)
    .build()

// Line 430: Hard-coded retry delay
tokio::time::sleep(Duration::from_secs(5)).await;
```

**Problem**: Cannot adjust without recompiling. Should be in Config.

#### Issue 5: Per-Message Store Pattern Buried in Code

**Location**: main.rs:388-406, 496-514

Critical pattern for event sourcing:

```rust
// Create fresh store per message (per-request pattern)
let inventory_env = InventoryEnvironment::new(
    inventory_clock.clone(),
    inventory_event_store.clone(),
    inventory_event_bus.clone(),
    StreamId::new("inventory"),
    inventory_query_clone.clone(),
);
let inventory_store = Store::new(
    InventoryState::new(),
    InventoryReducer::new(),
    inventory_env,
);

// Dispatch action to fresh store
if let Err(e) = inventory_store.send(action).await {
    error!(error = %e, "Failed to dispatch action to inventory store");
}
// Store dropped here - memory freed
```

**Problem**: This pattern is repeated identically for Inventory and Payment aggregates. Should be abstracted.

---

## Target Architecture

### New Module Structure

```
src/
├── main.rs                          (~50 lines - entry point only)
│
├── bootstrap/                       (Application bootstrap - NEW)
│   ├── mod.rs                       (Public API)
│   ├── builder.rs                   (ApplicationBuilder)
│   ├── resources.rs                 (Database, EventBus, Clock setup)
│   ├── aggregates.rs                (Aggregate consumer registration)
│   ├── projections.rs               (Projection consumer registration)
│   └── auth.rs                      (Auth store setup)
│
├── runtime/                         (Generic runtime components - NEW)
│   ├── mod.rs                       (Public API)
│   ├── consumer.rs                  (Generic EventConsumer)
│   ├── handlers.rs                  (Concrete EventHandler implementations)
│   └── lifecycle.rs                 (Application lifecycle + shutdown)
│
├── aggregates/                      (Existing - unchanged)
├── projections/                     (Existing - unchanged)
├── api/                             (Existing - unchanged)
├── server/                          (Existing - unchanged)
└── auth/                            (Existing - unchanged)
```

### Component Responsibilities

#### 1. `runtime/consumer.rs` - Generic Event Consumer

**Purpose**: Eliminate 436 lines of duplication

```rust
/// Generic event bus consumer that handles:
/// - Subscribe with retry loop
/// - Event deserialization
/// - Handler invocation
/// - Error handling and logging
/// - Reconnection on stream end
/// - Graceful shutdown coordination
pub struct EventConsumer {
    name: String,
    topics: Vec<String>,
    event_bus: Arc<dyn EventBus>,
    handler: Arc<dyn EventHandler>,
    shutdown: broadcast::Receiver<()>,
}

impl EventConsumer {
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        // Single implementation replaces all 4 consumers
    }
}
```

**Lines of Code**: ~60 lines (replaces 436 lines)

#### 2. `runtime/handlers.rs` - Event Handler Trait

**Purpose**: Pluggable event processing logic

```rust
/// Handler for processing deserialized events.
#[async_trait]
pub trait EventHandler: Send + Sync + 'static {
    /// Handle a deserialized ticketing event.
    async fn handle(&self, event: TicketingEvent) -> Result<(), Box<dyn std::error::Error>>;
}

/// Handler for aggregate consumers (creates Store per message)
pub struct InventoryHandler {
    clock: Arc<SystemClock>,
    event_store: Arc<PostgresEventStore>,
    event_bus: Arc<dyn EventBus>,
    query: Arc<PostgresInventoryQuery>,
}

#[async_trait]
impl EventHandler for InventoryHandler {
    async fn handle(&self, event: TicketingEvent) -> Result<(), Box<dyn std::error::Error>> {
        if let TicketingEvent::Inventory(action) = event {
            let env = InventoryEnvironment::new(
                self.clock.clone(),
                self.event_store.clone(),
                self.event_bus.clone(),
                StreamId::new("inventory"),
                self.query.clone(),
            );
            let store = Store::new(InventoryState::new(), InventoryReducer::new(), env);
            store.send(action).await?;
        }
        Ok(())
    }
}

/// Similar implementations for: PaymentHandler, SalesAnalyticsHandler, CustomerHistoryHandler
```

**Lines of Code**: ~200 lines total (4 handlers × ~50 lines each)

#### 3. `bootstrap/resources.rs` - Resource Manager

**Purpose**: Centralize infrastructure setup

```rust
/// Manages all infrastructure resources.
pub struct ResourceManager {
    pub config: Arc<Config>,
    pub clock: Arc<SystemClock>,
    pub event_store: Arc<PostgresEventStore>,
    pub event_bus: Arc<dyn EventBus>,
    pub projections_pool: Arc<sqlx::PgPool>,
    pub auth_pool: Arc<sqlx::PgPool>,
    pub payment_gateway: Arc<dyn PaymentGateway>,
    pub payment_gateway_breaker: Arc<CircuitBreaker>,
}

impl ResourceManager {
    /// Initialize all resources from config.
    ///
    /// Handles:
    /// - Database connections + migrations (3 databases)
    /// - Event store setup
    /// - Event bus connection
    /// - Payment gateway + circuit breaker
    pub async fn from_config(config: Arc<Config>) -> Result<Self, Box<dyn std::error::Error>> {
        // All the setup logic from main() lines 65-124
    }
}
```

**Lines of Code**: ~100 lines

#### 4. `bootstrap/aggregates.rs` - Aggregate Registration

**Purpose**: Create aggregate consumers

```rust
/// Register all aggregate consumers.
pub fn register_aggregate_consumers(
    resources: &ResourceManager,
    shutdown: broadcast::Receiver<()>,
) -> Vec<EventConsumer> {
    vec![
        create_inventory_consumer(resources, shutdown.resubscribe()),
        create_payment_consumer(resources, shutdown.resubscribe()),
    ]
}

fn create_inventory_consumer(
    resources: &ResourceManager,
    shutdown: broadcast::Receiver<()>,
) -> EventConsumer {
    let handler = Arc::new(InventoryHandler {
        clock: resources.clock.clone(),
        event_store: resources.event_store.clone(),
        event_bus: resources.event_bus.clone(),
        query: create_inventory_query(resources),
    });

    EventConsumer {
        name: "inventory".to_string(),
        topics: vec![resources.config.redpanda.inventory_topic.clone()],
        event_bus: resources.event_bus.clone(),
        handler,
        shutdown,
    }
}

// Similar for: create_payment_consumer
```

**Lines of Code**: ~80 lines (2 aggregates × ~40 lines each)

#### 5. `bootstrap/projections.rs` - Projection Registration

**Purpose**: Create projection consumers + managers

```rust
/// Register all projection consumers and managers.
pub async fn register_projections(
    resources: &ResourceManager,
    shutdown: broadcast::Receiver<()>,
) -> Result<ProjectionSystem, Box<dyn std::error::Error>> {
    // Create projection managers (PostgreSQL-backed projections)
    let managers = setup_projection_managers(
        resources.config.as_ref(),
        resources.event_bus.clone(),
        resources.event_bus.clone(),
    ).await?;

    // Create in-memory projections
    let sales_analytics = Arc::new(RwLock::new(SalesAnalyticsProjection::new()));
    let customer_history = Arc::new(RwLock::new(CustomerHistoryProjection::new()));

    // Create ownership indices
    let reservation_ownership = Arc::new(RwLock::new(HashMap::new()));
    let payment_ownership = Arc::new(RwLock::new(HashMap::new()));

    // Create projection consumers
    let consumers = vec![
        create_sales_analytics_consumer(
            resources,
            sales_analytics.clone(),
            reservation_ownership.clone(),
            payment_ownership.clone(),
            shutdown.resubscribe(),
        ),
        create_customer_history_consumer(
            resources,
            customer_history.clone(),
            reservation_ownership.clone(),
            shutdown.resubscribe(),
        ),
    ];

    Ok(ProjectionSystem {
        managers,
        sales_analytics,
        customer_history,
        reservation_ownership,
        payment_ownership,
        consumers,
    })
}

pub struct ProjectionSystem {
    pub managers: ProjectionManagers,
    pub sales_analytics: Arc<RwLock<SalesAnalyticsProjection>>,
    pub customer_history: Arc<RwLock<CustomerHistoryProjection>>,
    pub reservation_ownership: Arc<RwLock<HashMap<ReservationId, CustomerId>>>,
    pub payment_ownership: Arc<RwLock<HashMap<PaymentId, ReservationId>>>,
    pub consumers: Vec<EventConsumer>,
}
```

**Lines of Code**: ~150 lines

#### 6. `bootstrap/builder.rs` - Application Builder

**Purpose**: Declarative application construction

```rust
/// Builder for constructing the application.
pub struct ApplicationBuilder {
    config: Option<Arc<Config>>,
    resources: Option<ResourceManager>,
    projection_system: Option<ProjectionSystem>,
    auth_store: Option<Arc<TicketingAuthStore>>,
    consumers: Vec<EventConsumer>,
    shutdown_tx: Option<broadcast::Sender<()>>,
}

impl ApplicationBuilder {
    pub fn new() -> Self {
        Self {
            config: None,
            resources: None,
            projection_system: None,
            auth_store: None,
            consumers: Vec::new(),
            shutdown_tx: None,
        }
    }

    pub fn with_config(mut self, config: Config) -> Self {
        self.config = Some(Arc::new(config));
        self
    }

    pub fn with_tracing(self) -> Result<Self, Box<dyn std::error::Error>> {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "ticketing=info,tower_http=debug".into()),
            )
            .with(tracing_subscriber::fmt::layer())
            .init();
        Ok(self)
    }

    pub async fn with_resources(mut self) -> Result<Self, Box<dyn std::error::Error>> {
        let config = self.config.as_ref()
            .ok_or("Config must be set before resources")?;

        self.resources = Some(ResourceManager::from_config(config.clone()).await?);
        Ok(self)
    }

    pub async fn with_aggregates(mut self) -> Result<Self, Box<dyn std::error::Error>> {
        let resources = self.resources.as_ref()
            .ok_or("Resources must be set before aggregates")?;

        let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        let aggregate_consumers = register_aggregate_consumers(resources, shutdown_rx);
        self.consumers.extend(aggregate_consumers);

        Ok(self)
    }

    pub async fn with_projections(mut self) -> Result<Self, Box<dyn std::error::Error>> {
        let resources = self.resources.as_ref()
            .ok_or("Resources must be set before projections")?;

        let shutdown_rx = self.shutdown_tx.as_ref()
            .ok_or("Shutdown coordinator must be initialized")?
            .subscribe();

        let projection_system = register_projections(resources, shutdown_rx).await?;
        self.consumers.extend(projection_system.consumers.clone());
        self.projection_system = Some(projection_system);

        Ok(self)
    }

    pub async fn with_auth(mut self) -> Result<Self, Box<dyn std::error::Error>> {
        let config = self.config.as_ref()
            .ok_or("Config must be set before auth")?;

        let resources = self.resources.as_ref()
            .ok_or("Resources must be set before auth")?;

        let auth_store = build_auth_store(config.as_ref(), resources.auth_pool.as_ref().clone()).await?;
        self.auth_store = Some(auth_store);

        Ok(self)
    }

    pub async fn build(self) -> Result<Application, Box<dyn std::error::Error>> {
        let config = self.config.ok_or("Config not set")?;
        let resources = self.resources.ok_or("Resources not set")?;
        let projection_system = self.projection_system.ok_or("Projections not set")?;
        let auth_store = self.auth_store.ok_or("Auth not set")?;
        let shutdown_tx = self.shutdown_tx.ok_or("Shutdown coordinator not set")?;

        // Create projection queries for AppState
        let available_seats_projection = Arc::new(
            PostgresAvailableSeatsProjection::new(resources.projections_pool.clone())
        );

        let inventory_query = Arc::new(PostgresInventoryQuery::new(available_seats_projection.clone()));
        let payment_query = Arc::new(PostgresPaymentQuery::new());
        let reservation_query = Arc::new(PostgresReservationQuery::new());

        // Create projection completion tracker
        let projection_completion_tracker = Arc::new(
            ProjectionCompletionTracker::new(resources.event_bus.clone())
                .await
                .map_err(|e| format!("Failed to create projection completion tracker: {e}"))?
        );

        // Build AppState
        let state = AppState::new(
            config.clone(),
            auth_store,
            resources.clock.clone(),
            resources.event_store.clone(),
            resources.event_bus.clone(),
            inventory_query,
            payment_query,
            reservation_query,
            available_seats_projection,
            projection_system.sales_analytics,
            projection_system.customer_history,
            projection_system.reservation_ownership,
            projection_system.payment_ownership,
            projection_completion_tracker,
        );

        // Build router
        let app = build_router(state);

        // Create TCP listener
        let addr = format!("{}:{}", config.server.host, config.server.port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;

        Ok(Application {
            listener,
            app,
            consumers: self.consumers,
            projection_managers: projection_system.managers,
            shutdown_tx,
            config,
        })
    }
}
```

**Lines of Code**: ~150 lines

#### 7. `runtime/lifecycle.rs` - Application Lifecycle

**Purpose**: Run application + graceful shutdown

```rust
/// Running application with all background tasks.
pub struct Application {
    listener: tokio::net::TcpListener,
    app: axum::Router,
    consumers: Vec<EventConsumer>,
    projection_managers: ProjectionManagers,
    shutdown_tx: broadcast::Sender<()>,
    config: Arc<Config>,
}

impl Application {
    /// Run the application until shutdown signal received.
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        info!(address = %format!("{}:{}", self.config.server.host, self.config.server.port),
              "Starting HTTP server");

        // Start all consumers
        let consumer_handles: Vec<_> = self.consumers
            .into_iter()
            .map(|consumer| consumer.spawn())
            .collect();

        // Start projection managers
        let projection_handles = self.projection_managers.start_all();

        // Run HTTP server with graceful shutdown
        axum::serve(self.listener, self.app)
            .with_graceful_shutdown(shutdown_signal())
            .await?;

        info!("HTTP server stopped, initiating graceful shutdown...");

        // Send shutdown signal to all background tasks
        let _ = self.shutdown_tx.send(());

        // Wait for all tasks to complete
        Self::await_shutdown(consumer_handles, projection_handles).await;

        info!("Graceful shutdown complete");
        Ok(())
    }

    async fn await_shutdown(
        consumer_handles: Vec<tokio::task::JoinHandle<()>>,
        projection_handles: Vec<tokio::task::JoinHandle<()>>,
    ) {
        let timeout = Duration::from_secs(10);

        for (idx, handle) in consumer_handles.into_iter().enumerate() {
            match tokio::time::timeout(timeout, handle).await {
                Ok(Ok(())) => info!(consumer = idx, "Consumer stopped gracefully"),
                Ok(Err(e)) => warn!(consumer = idx, error = %e, "Consumer task failed"),
                Err(_) => warn!(consumer = idx, "Consumer shutdown timed out"),
            }
        }

        for (idx, handle) in projection_handles.into_iter().enumerate() {
            match tokio::time::timeout(timeout, handle).await {
                Ok(Ok(())) => info!(projection = idx, "Projection manager stopped gracefully"),
                Ok(Err(e)) => warn!(projection = idx, error = %e, "Projection manager task failed"),
                Err(_) => warn!(projection = idx, "Projection manager shutdown timed out"),
            }
        }
    }
}

/// Graceful shutdown signal handler.
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {
            info!("Received Ctrl+C signal, shutting down gracefully...");
        },
        () = terminate => {
            info!("Received SIGTERM signal, shutting down gracefully...");
        },
    }
}
```

**Lines of Code**: ~100 lines

#### 8. `main.rs` - Entry Point

**Purpose**: Minimal, declarative application startup

```rust
//! Ticketing system HTTP server.
//!
//! Event-sourced ticketing platform with CQRS, sagas, and real-time updates.

use ticketing::bootstrap::ApplicationBuilder;
use ticketing::config::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenvy::dotenv().ok();

    // Build and run application
    ApplicationBuilder::new()
        .with_config(Config::from_env())
        .with_tracing()?
        .with_resources().await?
        .with_aggregates().await?
        .with_projections().await?
        .with_auth().await?
        .build().await?
        .run().await
}
```

**Lines of Code**: ~20 lines (down from 814 lines!)

---

## Implementation Plan

### Step 1: Create Generic EventConsumer (High Impact)

**Priority**: HIGH (eliminates 436 lines of duplication)

**Files to Create**:
- `src/runtime/mod.rs`
- `src/runtime/consumer.rs`
- `src/runtime/handlers.rs`

**Tasks**:

#### Task 1.1: Create EventHandler Trait
**File**: `src/runtime/handlers.rs`

```rust
use async_trait::async_trait;
use crate::projections::TicketingEvent;

#[async_trait]
pub trait EventHandler: Send + Sync + 'static {
    async fn handle(&self, event: TicketingEvent) -> Result<(), Box<dyn std::error::Error>>;
}
```

#### Task 1.2: Implement Generic EventConsumer
**File**: `src/runtime/consumer.rs`

```rust
pub struct EventConsumer {
    name: String,
    topics: Vec<String>,
    event_bus: Arc<dyn EventBus>,
    handler: Arc<dyn EventHandler>,
    shutdown: broadcast::Receiver<()>,
}

impl EventConsumer {
    pub fn new(
        name: impl Into<String>,
        topics: Vec<String>,
        event_bus: Arc<dyn EventBus>,
        handler: Arc<dyn EventHandler>,
        shutdown: broadcast::Receiver<()>,
    ) -> Self {
        Self {
            name: name.into(),
            topics,
            event_bus,
            handler,
            shutdown,
        }
    }

    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        // Extract subscribe-process-reconnect loop from main.rs
        // (lines 356-440 as template)
    }
}
```

#### Task 1.3: Implement InventoryHandler
**File**: `src/runtime/handlers.rs`

```rust
pub struct InventoryHandler {
    clock: Arc<SystemClock>,
    event_store: Arc<PostgresEventStore>,
    event_bus: Arc<dyn EventBus>,
    query: Arc<PostgresInventoryQuery>,
}

#[async_trait]
impl EventHandler for InventoryHandler {
    async fn handle(&self, event: TicketingEvent) -> Result<(), Box<dyn std::error::Error>> {
        if let TicketingEvent::Inventory(action) = event {
            // Extract store creation logic from main.rs lines 388-406
        }
        Ok(())
    }
}
```

#### Task 1.4: Implement PaymentHandler
**File**: `src/runtime/handlers.rs`

Similar to InventoryHandler, extract from lines 496-514.

#### Task 1.5: Implement SalesAnalyticsHandler
**File**: `src/runtime/handlers.rs`

Extract from lines 617-660 (includes ownership tracking).

#### Task 1.6: Implement CustomerHistoryHandler
**File**: `src/runtime/handlers.rs`

Extract from lines 720-751 (includes ownership tracking).

**Testing**:
1. Add unit tests for each handler
2. Add integration test using InMemoryEventBus
3. Verify identical behavior to original code

**Success Criteria**:
- All 4 handlers implemented
- Generic consumer replaces 436 lines with ~60 lines
- Integration tests pass
- Original tests still pass (no behavior change)

**Estimated Time**: 3-4 hours

---

### Step 2: Extract Resource Manager

**Priority**: MEDIUM (enables further refactoring)

**Files to Create**:
- `src/bootstrap/mod.rs`
- `src/bootstrap/resources.rs`

**Tasks**:

#### Task 2.1: Create ResourceManager Struct
**File**: `src/bootstrap/resources.rs`

```rust
pub struct ResourceManager {
    pub config: Arc<Config>,
    pub clock: Arc<SystemClock>,
    pub event_store: Arc<PostgresEventStore>,
    pub event_bus: Arc<dyn EventBus>,
    pub projections_pool: Arc<sqlx::PgPool>,
    pub auth_pool: Arc<sqlx::PgPool>,
    pub payment_gateway: Arc<dyn PaymentGateway>,
    pub payment_gateway_breaker: Arc<CircuitBreaker>,
}
```

#### Task 2.2: Implement ResourceManager::from_config()
**File**: `src/bootstrap/resources.rs`

Extract setup logic from main.rs:
- Lines 65-77: Event store + migrations
- Lines 79-88: Projections database + migrations
- Lines 90-98: Event bus setup
- Lines 100-110: Clock + queries
- Lines 112-124: Payment gateway + circuit breaker
- Lines 141-145: Auth database

**Testing**:
1. Add integration test that creates ResourceManager
2. Verify all resources initialized correctly
3. Verify migrations run

**Success Criteria**:
- ResourceManager successfully creates all resources
- Integration test passes
- Main function reduced by ~60 lines

**Estimated Time**: 1-2 hours

---

### Step 3: Extract Aggregate Registration

**Priority**: MEDIUM

**Files to Create**:
- `src/bootstrap/aggregates.rs`

**Tasks**:

#### Task 3.1: Create Aggregate Consumer Factory Functions
**File**: `src/bootstrap/aggregates.rs`

```rust
pub fn register_aggregate_consumers(
    resources: &ResourceManager,
    shutdown: broadcast::Receiver<()>,
) -> Vec<EventConsumer> {
    vec![
        create_inventory_consumer(resources, shutdown.resubscribe()),
        create_payment_consumer(resources, shutdown.resubscribe()),
    ]
}

fn create_inventory_consumer(
    resources: &ResourceManager,
    shutdown: broadcast::Receiver<()>,
) -> EventConsumer {
    // Create handler
    // Create EventConsumer
    // Return
}
```

#### Task 3.2: Update main.rs to Use Registration
**File**: `src/main.rs`

Replace spawn_aggregate_consumers() with:
```rust
let aggregate_consumers = register_aggregate_consumers(&resources, shutdown_rx);
let aggregate_handles: Vec<_> = aggregate_consumers
    .into_iter()
    .map(|consumer| consumer.spawn())
    .collect();
```

**Testing**:
- Verify aggregate consumers still work
- Integration test with real event bus

**Success Criteria**:
- Aggregate consumers work identically to before
- spawn_aggregate_consumers() removed (247 lines deleted)
- Main function reduced by ~10 lines

**Estimated Time**: 1 hour

---

### Step 4: Extract Projection Registration

**Priority**: MEDIUM

**Files to Create**:
- `src/bootstrap/projections.rs`

**Tasks**:

#### Task 4.1: Create ProjectionSystem Struct
**File**: `src/bootstrap/projections.rs`

```rust
pub struct ProjectionSystem {
    pub managers: ProjectionManagers,
    pub sales_analytics: Arc<RwLock<SalesAnalyticsProjection>>,
    pub customer_history: Arc<RwLock<CustomerHistoryProjection>>,
    pub reservation_ownership: Arc<RwLock<HashMap<ReservationId, CustomerId>>>,
    pub payment_ownership: Arc<RwLock<HashMap<PaymentId, ReservationId>>>,
    pub consumers: Vec<EventConsumer>,
}
```

#### Task 4.2: Implement register_projections()
**File**: `src/bootstrap/projections.rs`

Extract from main.rs:
- Lines 152-172: Projection managers setup
- Lines 182-189: In-memory projections + ownership indices
- Lines 192-201: Analytics consumer spawning

#### Task 4.3: Update main.rs to Use Registration
**File**: `src/main.rs`

Replace projection setup with:
```rust
let projection_system = register_projections(&resources, shutdown_rx).await?;
```

**Testing**:
- Verify projections consume events correctly
- Verify ownership tracking works

**Success Criteria**:
- Projections work identically to before
- spawn_analytics_consumers() removed (222 lines deleted)
- Main function reduced by ~40 lines

**Estimated Time**: 2 hours

---

### Step 5: Create Application Builder

**Priority**: MEDIUM (foundation for DSL)

**Files to Create**:
- `src/bootstrap/builder.rs`
- `src/bootstrap/auth.rs`

**Tasks**:

#### Task 5.1: Implement ApplicationBuilder
**File**: `src/bootstrap/builder.rs`

Create builder with methods:
- `new()`
- `with_config()`
- `with_tracing()`
- `with_resources()`
- `with_aggregates()`
- `with_projections()`
- `with_auth()`
- `build()`

#### Task 5.2: Extract Auth Setup
**File**: `src/bootstrap/auth.rs`

Move auth setup logic (lines 141-150) into separate module.

#### Task 5.3: Update main.rs to Use Builder
**File**: `src/main.rs`

Replace entire main() with:
```rust
ApplicationBuilder::new()
    .with_config(Config::from_env())
    .with_tracing()?
    .with_resources().await?
    .with_aggregates().await?
    .with_projections().await?
    .with_auth().await?
    .build().await?
    .run().await
```

**Testing**:
- Full integration test using builder
- Verify identical behavior

**Success Criteria**:
- Builder successfully constructs application
- Main function reduced to ~20 lines
- All integration tests pass

**Estimated Time**: 2-3 hours

---

### Step 6: Create Application Lifecycle Manager

**Priority**: LOW (polish)

**Files to Create**:
- `src/runtime/lifecycle.rs`

**Tasks**:

#### Task 6.1: Implement Application Struct
**File**: `src/runtime/lifecycle.rs`

```rust
pub struct Application {
    listener: tokio::net::TcpListener,
    app: axum::Router,
    consumers: Vec<EventConsumer>,
    projection_managers: ProjectionManagers,
    shutdown_tx: broadcast::Sender<()>,
    config: Arc<Config>,
}

impl Application {
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        // Start consumers
        // Start projection managers
        // Run HTTP server
        // Coordinate graceful shutdown
    }
}
```

#### Task 6.2: Extract Shutdown Logic
**File**: `src/runtime/lifecycle.rs`

Move graceful shutdown logic (lines 264-306) into Application.

**Testing**:
- Test graceful shutdown with SIGTERM
- Test graceful shutdown with Ctrl+C
- Verify all tasks stop within timeout

**Success Criteria**:
- Application.run() handles full lifecycle
- Graceful shutdown works correctly
- Main function is just builder chain

**Estimated Time**: 1-2 hours

---

### Step 7: Remove Dead Code

**Priority**: LOW (cleanup)

**Files to Modify**:
- `src/main.rs` (if not already done by previous steps)

**Tasks**:

#### Task 7.1: Remove Unused Circuit Breakers
**File**: `src/bootstrap/resources.rs`

Remove duplicate circuit breaker creation (current lines 203-228 in main.rs).

#### Task 7.2: Run Clippy and Fix Warnings

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

**Success Criteria**:
- No dead code warnings
- No unused variable warnings
- Clippy passes with no errors

**Estimated Time**: 30 minutes

---

### Step 8: Documentation and Testing

**Priority**: MEDIUM

**Tasks**:

#### Task 8.1: Add Module Documentation
Add comprehensive docs to:
- `src/bootstrap/mod.rs`
- `src/runtime/mod.rs`
- Each new module

#### Task 8.2: Add Integration Tests
Create `tests/bootstrap_test.rs`:
```rust
#[tokio::test]
async fn test_application_builder() {
    // Test full application construction
}

#[tokio::test]
async fn test_graceful_shutdown() {
    // Test shutdown coordination
}
```

#### Task 8.3: Update CLAUDE.md
Document new architecture in project instructions.

**Success Criteria**:
- All modules documented
- Integration tests pass
- CLAUDE.md reflects new structure

**Estimated Time**: 1-2 hours

---

## Success Criteria

### Quantitative Metrics

| Metric | Before | After | Improvement |
|--------|--------|-------|-------------|
| main.rs lines | 814 | ~50 | 94% reduction |
| Duplicated consumer code | 436 lines | 0 lines | 100% elimination |
| Number of modules | 7 | 11 | Better organization |
| Testable components | 2 | 8 | 4× improvement |
| Dead code (unused vars) | 2 circuit breakers | 0 | Bug fixed |

### Qualitative Goals

1. **DSL-Ready Structure**: Builder pattern with declarative API
2. **Modular Components**: Each module has single responsibility
3. **Testable**: Can test each component in isolation
4. **Maintainable**: Changes localized to specific modules
5. **No Behavior Changes**: All existing tests pass

### Validation Tests

1. **All existing integration tests pass** (6 tests)
2. **All existing unit tests pass**
3. **New integration test for builder pattern**
4. **Clippy passes with no warnings**
5. **Documentation builds with no warnings**

---

## Risk Assessment

### Low Risk
- Step 1 (Generic consumer): Pure extraction, easy to test
- Step 2 (Resource manager): Simple grouping of existing code
- Step 7 (Dead code removal): Just cleanup

### Medium Risk
- Step 3-4 (Aggregate/Projection registration): Must preserve exact behavior
- Step 5 (Builder): Must handle async carefully

### High Risk
- Step 6 (Lifecycle manager): Graceful shutdown is subtle

### Mitigation Strategy

1. **Test after each step**: Don't proceed until tests pass
2. **Keep original code**: Comment out old code before deleting
3. **Integration tests**: Run full test suite after each step
4. **Incremental commits**: Commit after each successful step

---

## Dependencies

### Technical Dependencies
- No new crates required (all existing dependencies)
- Requires `async-trait` crate (already in use)

### Blocked By
- None (can start immediately)

### Blocks
- Phase 13: DSL implementation (requires this refactoring)

---

## Future Extensions

After this refactoring, the following become easy:

1. **DSL Code Generation**: Generate builder chain from declarative config
2. **Testing Utilities**: Mock ResourceManager for tests
3. **Additional Aggregates**: Just add new handler + registration function
4. **Configuration Hot-Reload**: Restart consumers without full restart
5. **Metrics**: Add metrics to EventConsumer (all consumers get it)
6. **Distributed Tracing**: Add tracing spans to EventConsumer

---

## Appendix A: Line-by-Line Mapping

### Current main.rs → Target Structure

| Current Lines | Target Module | Target Function |
|---------------|---------------|-----------------|
| 37-48 | bootstrap/builder.rs | `ApplicationBuilder::with_tracing()` |
| 52-63 | bootstrap/builder.rs | `ApplicationBuilder::new()` |
| 65-77 | bootstrap/resources.rs | `ResourceManager::from_config()` |
| 79-88 | bootstrap/resources.rs | `ResourceManager::from_config()` |
| 90-98 | bootstrap/resources.rs | `ResourceManager::from_config()` |
| 100-110 | bootstrap/resources.rs | `ResourceManager::from_config()` |
| 112-124 | bootstrap/resources.rs | `ResourceManager::from_config()` |
| 126-139 | bootstrap/aggregates.rs | `register_aggregate_consumers()` |
| 141-150 | bootstrap/auth.rs | `setup_auth_store()` |
| 152-172 | bootstrap/projections.rs | `register_projections()` |
| 174-189 | bootstrap/projections.rs | `register_projections()` |
| 192-201 | bootstrap/projections.rs | `register_projections()` |
| 203-228 | **DELETED** | (Dead code) |
| 230-246 | bootstrap/builder.rs | `ApplicationBuilder::build()` |
| 248-262 | runtime/lifecycle.rs | `Application::run()` |
| 264-306 | runtime/lifecycle.rs | `Application::run()` |
| 309-556 | runtime/consumer.rs + runtime/handlers.rs | `EventConsumer` + `InventoryHandler` + `PaymentHandler` |
| 558-780 | runtime/consumer.rs + runtime/handlers.rs | `EventConsumer` + `SalesAnalyticsHandler` + `CustomerHistoryHandler` |
| 782-814 | runtime/lifecycle.rs | `shutdown_signal()` |

---

## Appendix B: Example DSL Target

After this refactoring, we can support DSL like:

```yaml
# ticketing.composable.yaml
application:
  name: ticketing
  version: 1.0.0

resources:
  event_store:
    postgres:
      url: env:DATABASE_URL
      migrations: ./migrations_events

  projections_store:
    postgres:
      url: env:PROJECTION_DATABASE_URL
      migrations: ./migrations_projections

  event_bus:
    redpanda:
      brokers: env:REDPANDA_BROKERS
      consumer_group: env:REDPANDA_CONSUMER_GROUP

aggregates:
  - name: inventory
    state: InventoryState
    reducer: InventoryReducer
    topics: [inventory]
    role: child  # Receives commands from event bus
    query: PostgresInventoryQuery

  - name: payment
    state: PaymentState
    reducer: PaymentReducer
    topics: [payment]
    role: child
    query: PostgresPaymentQuery
    circuit_breaker:
      failure_threshold: 5
      timeout: 30s
      success_threshold: 2

  - name: reservation
    state: ReservationState
    reducer: ReservationReducer
    topics: [reservation]
    role: coordinator  # Publishes commands to event bus
    query: PostgresReservationQuery

projections:
  - name: available_seats
    type: AvailableSeatsProjection
    topics: [inventory, reservation]
    storage: postgres

  - name: sales_analytics
    type: SalesAnalyticsProjection
    topics: [reservation, payment]
    storage: memory

  - name: customer_history
    type: CustomerHistoryProjection
    topics: [reservation]
    storage: memory

http:
  host: 0.0.0.0
  port: 8080
  routes:
    - path: /events
      handlers: [create_event, list_events, get_event, update_event, delete_event]
    - path: /reservations
      handlers: [create_reservation, get_reservation, list_reservations, cancel_reservation]
```

**Generated Code**:
```rust
// Generated by composable-rust-dsl v1.0.0
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    ApplicationBuilder::new()
        .with_config(Config::from_env())
        .with_tracing()?
        .with_resources().await?
        .with_aggregates().await?
        .with_projections().await?
        .with_auth().await?
        .build().await?
        .run().await
}
```

The builder handles all the complexity, and the DSL just needs to generate configuration and wire up the types.

---

## Summary

This refactoring transforms 814 lines of monolithic bootstrap code into a clean, modular, testable architecture suitable for DSL generation. The key wins:

1. **436 lines of duplication eliminated** via generic EventConsumer
2. **Dead code removed** (unused circuit breakers)
3. **Builder pattern** enables declarative application construction
4. **Separation of concerns** (resources, aggregates, projections, lifecycle)
5. **Testability** (each component testable in isolation)
6. **Maintainability** (changes localized to specific modules)

The resulting structure is a foundation for Phase 13's DSL-based application generation, where the entire bootstrap can be specified declaratively and compiled to this modular architecture.
