# Projections in Composable Rust

A comprehensive guide to implementing CQRS read models using the projection system.

## Table of Contents

1. [Overview](#overview)
2. [Core Concepts](#core-concepts)
3. [Getting Started](#getting-started)
4. [Architecture](#architecture)
5. [Usage Patterns](#usage-patterns)
6. [Testing](#testing)
7. [Best Practices](#best-practices)
8. [Advanced Topics](#advanced-topics)
9. [Examples](#examples)

---

## Overview

### What are Projections?

Projections are **denormalized read models** built from event streams. They implement the **read side** of Command Query Responsibility Segregation (CQRS), providing optimized views of your data for specific query patterns.

**Key Benefits:**

- **Query Optimization**: Design read models specifically for your queries
- **Separation of Concerns**: Write model (events) separate from read model (projections)
- **Scalability**: Scale reads independently from writes
- **Multiple Views**: Build multiple projections from the same event stream
- **Eventual Consistency**: Accept eventual consistency for better performance

### When to Use Projections

✅ **Good Use Cases:**
- Complex queries across aggregate boundaries
- Reporting and analytics
- Read-heavy workloads
- Multiple consumers with different query needs
- Caching and denormalization

❌ **Not Ideal For:**
- Simple CRUD applications
- Immediate consistency requirements
- Low query volume

---

## Core Concepts

### The Projection Trait

The `Projection` trait defines how events are transformed into read models:

```rust
pub trait Projection: Send + Sync {
    /// The event type this projection processes
    type Event: for<'de> Deserialize<'de> + Send;

    /// Unique name for this projection (for checkpointing)
    fn name(&self) -> &str;

    /// Apply an event to update the projection
    async fn apply_event(&self, event: &Self::Event) -> Result<()>;

    /// Rebuild the projection from scratch (clears all data)
    async fn rebuild(&self) -> Result<()>;
}
```

### ProjectionStore

Storage backend for projection data:

```rust
pub trait ProjectionStore: Send + Sync {
    async fn save(&self, key: &str, data: &[u8]) -> Result<()>;
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    async fn delete(&self, key: &str) -> Result<()>;
    async fn exists(&self, key: &str) -> Result<bool>;
}
```

**Implementations:**
- `PostgresProjectionStore`: Relational database storage
- `InMemoryProjectionStore`: Fast in-memory storage for testing

### ProjectionCheckpoint

Tracks projection progress through the event stream:

```rust
pub trait ProjectionCheckpoint: Send + Sync {
    fn save_position(
        &self,
        projection_name: &str,
        position: EventPosition,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;

    fn load_position(
        &self,
        projection_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<EventPosition>>> + Send + '_>>;
}
```

**Purpose**: Enables projection resumption after restart or failure.

---

## Getting Started

### Basic Projection Example

Here's a simple projection that tracks user login counts:

```rust
use composable_rust_core::projection::{Projection, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
enum UserEvent {
    UserLoggedIn { user_id: String, timestamp: DateTime<Utc> },
    UserLoggedOut { user_id: String, timestamp: DateTime<Utc> },
}

pub struct LoginCountProjection {
    store: Arc<dyn ProjectionStore>,
}

impl Projection for LoginCountProjection {
    type Event = UserEvent;

    fn name(&self) -> &str {
        "login_count"
    }

    async fn apply_event(&self, event: &Self::Event) -> Result<()> {
        match event {
            UserEvent::UserLoggedIn { user_id, .. } => {
                // Get current count
                let key = format!("login_count:{user_id}");
                let count = self.get_count(&key).await?;

                // Increment and save
                let new_count = count + 1;
                self.store.save(&key, &new_count.to_le_bytes()).await?;
            }
            UserEvent::UserLoggedOut { .. } => {
                // Ignore logout events for this projection
            }
        }
        Ok(())
    }

    async fn rebuild(&self) -> Result<()> {
        // Clear all login counts
        // In practice, you'd iterate and delete all keys
        Ok(())
    }
}

impl LoginCountProjection {
    async fn get_count(&self, key: &str) -> Result<u64> {
        match self.store.get(key).await? {
            Some(bytes) => {
                let arr: [u8; 8] = bytes.try_into()
                    .map_err(|_| ProjectionError::Serialization("Invalid count".into()))?;
                Ok(u64::from_le_bytes(arr))
            }
            None => Ok(0),
        }
    }

    pub async fn get_login_count(&self, user_id: &str) -> Result<u64> {
        let key = format!("login_count:{user_id}");
        self.get_count(&key).await
    }
}
```

### Using ProjectionManager

The `ProjectionManager` orchestrates projection updates:

```rust
use composable_rust_projections::ProjectionManager;

// Create manager
let manager = ProjectionManager::new(
    event_store,
    checkpoint_store,
    projection_store,
);

// Register projection
manager.register(LoginCountProjection::new(store)).await?;

// Start processing events
manager.start().await?;

// Query the projection
let projection = manager.get::<LoginCountProjection>("login_count").await?;
let count = projection.get_login_count("user-123").await?;
```

---

## Architecture

### Projection Pipeline

```
Event Store → Event Stream → Projection → ProjectionStore
                    ↓
              Checkpoint Tracker
```

1. **Event Store**: Source of truth (event log)
2. **Event Stream**: Real-time or catch-up event delivery
3. **Projection**: Transforms events into read model
4. **ProjectionStore**: Persists projection data
5. **Checkpoint Tracker**: Tracks progress for resumption

### Separate Database Strategy

For true CQRS separation, use different databases:

```rust
// Write side: Event store in PostgreSQL
let event_pool = PgPool::connect("postgres://localhost/events").await?;
let event_store = PostgresEventStore::new(event_pool);

// Read side: Projections in separate PostgreSQL instance
let projection_pool = PgPool::connect("postgres://read-replica/projections").await?;
let projection_store = PostgresProjectionStore::new(projection_pool);
```

**Benefits:**
- Independent scaling
- Different optimization strategies
- Isolated failures
- Technology flexibility

---

## Usage Patterns

### Pattern 1: Generic Key-Value Projection

Use `ProjectionStore` for simple projections:

```rust
// Store serialized data
let data = serde_json::to_vec(&order_summary)?;
store.save("order:123", &data).await?;

// Retrieve and deserialize
let bytes = store.get("order:123").await?;
let summary: OrderSummary = serde_json::from_slice(&bytes)?;
```

**When to use:**
- Simple data structures
- Low query complexity
- Prototyping

### Pattern 2: Custom Table Projection

Use custom database tables for complex queries:

```sql
CREATE TABLE order_projections (
    id TEXT PRIMARY KEY,
    customer_id TEXT NOT NULL,
    item_count INTEGER NOT NULL,
    total_cents BIGINT NOT NULL,
    status TEXT NOT NULL,
    placed_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_order_projections_customer ON order_projections(customer_id);
CREATE INDEX idx_order_projections_status ON order_projections(status);
CREATE INDEX idx_order_projections_placed_at ON order_projections(placed_at DESC);
```

```rust
impl Projection for OrderProjection {
    async fn apply_event(&self, event: &OrderEvent) -> Result<()> {
        match event {
            OrderEvent::OrderPlaced { order_id, customer_id, items, total, timestamp } => {
                sqlx::query(
                    "INSERT INTO order_projections
                     (id, customer_id, item_count, total_cents, status, placed_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     ON CONFLICT (id) DO UPDATE
                     SET customer_id = EXCLUDED.customer_id,
                         item_count = EXCLUDED.item_count,
                         total_cents = EXCLUDED.total_cents,
                         updated_at = now()"
                )
                .bind(order_id)
                .bind(customer_id)
                .bind(items.len() as i32)
                .bind(total.cents())
                .bind("placed")
                .bind(timestamp)
                .bind(Utc::now())
                .execute(&self.pool)
                .await?;
            }
            // ... other events
        }
        Ok(())
    }
}

// Query methods
impl OrderProjection {
    pub async fn get_customer_orders(&self, customer_id: &str) -> Result<Vec<OrderSummary>> {
        sqlx::query_as(
            "SELECT * FROM order_projections
             WHERE customer_id = $1
             ORDER BY placed_at DESC
             LIMIT 100"
        )
        .bind(customer_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn count_by_status(&self, status: &str) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM order_projections WHERE status = $1"
        )
        .bind(status)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }
}
```

**When to use:**
- Complex filtering and sorting
- Aggregations
- Joins across related data
- High query volume

### Pattern 3: Materialized View Projection

Use database materialized views for complex analytics:

```sql
CREATE MATERIALIZED VIEW order_statistics AS
SELECT
    customer_id,
    COUNT(*) as total_orders,
    SUM(total_cents) as total_spent,
    AVG(total_cents) as avg_order_value,
    MAX(placed_at) as last_order_date
FROM order_projections
GROUP BY customer_id;

CREATE INDEX idx_order_statistics_customer ON order_statistics(customer_id);
```

```rust
impl Projection for OrderStatisticsProjection {
    async fn apply_event(&self, event: &OrderEvent) -> Result<()> {
        // Update base table
        self.base_projection.apply_event(event).await?;

        // Refresh materialized view periodically
        if self.should_refresh() {
            sqlx::query("REFRESH MATERIALIZED VIEW CONCURRENTLY order_statistics")
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }
}
```

**When to use:**
- Heavy analytics queries
- Infrequent updates acceptable
- Complex aggregations

### Pattern 4: Multiple Projections from Same Events

Build different views for different use cases:

```rust
// Projection 1: Customer order history (for customer portal)
struct CustomerOrderHistoryProjection {
    // Optimized for: "Show me all orders for customer X"
}

// Projection 2: Order fulfillment (for warehouse)
struct OrderFulfillmentProjection {
    // Optimized for: "Show me all orders to ship today"
}

// Projection 3: Sales analytics (for business intelligence)
struct SalesAnalyticsProjection {
    // Optimized for: "Show me revenue trends by day/week/month"
}

// All consuming the same OrderEvent stream
```

---

## Testing

### Unit Testing with InMemoryProjectionStore

```rust
use composable_rust_testing::{InMemoryProjectionStore, ProjectionTestHarness};

#[tokio::test]
async fn test_login_count_projection() {
    let store = Arc::new(InMemoryProjectionStore::new());
    let projection = LoginCountProjection::new(store.clone());
    let mut harness = ProjectionTestHarness::new(projection, store);

    // Given: User logs in twice
    harness
        .given_events(vec![
            UserEvent::UserLoggedIn {
                user_id: "user-1".to_string(),
                timestamp: Utc::now(),
            },
            UserEvent::UserLoggedIn {
                user_id: "user-1".to_string(),
                timestamp: Utc::now(),
            },
        ])
        .await
        .unwrap();

    // Then: Login count should be 2
    let count = harness.projection().get_login_count("user-1").await.unwrap();
    assert_eq!(count, 2);
}
```

### Integration Testing with testcontainers

```rust
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;

async fn setup_test_db() -> (ContainerAsync<Postgres>, sqlx::PgPool) {
    let container = Postgres::default().start().await
        .expect("Failed to start postgres container");

    let port = container.get_host_port_ipv4(5432).await.expect("Failed to get port");
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&connection_string)
        .await
        .expect("Failed to connect");

    sqlx::migrate!("./migrations").run(&pool).await.expect("Migrations failed");

    (container, pool)
}

#[tokio::test]
async fn test_order_projection_integration() {
    let (_docker, pool) = setup_test_db().await;
    let projection = OrderProjection::new(pool.clone());

    // Test with real database
    projection.apply_event(&order_placed_event).await.unwrap();

    let orders = projection.get_customer_orders("customer-1").await.unwrap();
    assert_eq!(orders.len(), 1);
}
```

### Testing Idempotency

Projections should handle duplicate events gracefully:

```rust
#[tokio::test]
async fn test_projection_idempotency() {
    let store = Arc::new(InMemoryProjectionStore::new());
    let projection = OrderProjection::new(store.clone());

    let event = OrderEvent::OrderPlaced { /* ... */ };

    // Apply event twice
    projection.apply_event(&event).await.unwrap();
    projection.apply_event(&event).await.unwrap();

    // Should only create one order (use ON CONFLICT in SQL)
    let orders = projection.get_all_orders().await.unwrap();
    assert_eq!(orders.len(), 1);
}
```

---

## Best Practices

### 1. Idempotency

Always make projections idempotent using `ON CONFLICT`:

```sql
INSERT INTO order_projections (id, ...) VALUES ($1, ...)
ON CONFLICT (id) DO UPDATE SET updated_at = now();
```

### 2. Explicit Field Mapping

Don't use `SELECT *` - explicitly list fields for schema evolution:

```rust
sqlx::query_as::<_, OrderSummary>(
    "SELECT id, customer_id, item_count, total_cents, status,
            placed_at, updated_at, tracking, cancellation_reason
     FROM order_projections
     WHERE id = $1"
)
```

### 3. Checkpoint Management

Always checkpoint after successful event processing:

```rust
async fn process_events(&self) -> Result<()> {
    let checkpoint = self.load_checkpoint().await?;
    let events = self.event_store.load_events_after(checkpoint).await?;

    for event in events {
        self.projection.apply_event(&event).await?;

        // Checkpoint after each event
        let position = EventPosition::new(event.offset, event.timestamp);
        self.checkpoint_store.save_position(self.projection.name(), position).await?;
    }
    Ok(())
}
```

### 4. Error Handling

Use retries with exponential backoff for transient errors:

```rust
async fn apply_event_with_retry(&self, event: &Event) -> Result<()> {
    let mut retries = 0;
    let max_retries = 3;

    loop {
        match self.projection.apply_event(event).await {
            Ok(()) => return Ok(()),
            Err(e) if is_transient_error(&e) && retries < max_retries => {
                retries += 1;
                let delay = Duration::from_millis(100 * 2_u64.pow(retries));
                tokio::time::sleep(delay).await;
            }
            Err(e) => return Err(e),
        }
    }
}
```

### 5. Projection Versioning

Include version info for schema migrations:

```rust
const PROJECTION_VERSION: u32 = 2;

impl Projection for OrderProjectionV2 {
    fn name(&self) -> &str {
        "order_projection_v2"  // Different name = separate checkpoint
    }

    async fn rebuild(&self) -> Result<()> {
        // Rebuild from events with new schema
        Ok(())
    }
}
```

### 6. Separate Query Interface

Separate projection updates from queries:

```rust
// Update interface (internal)
impl Projection for OrderProjection {
    async fn apply_event(&self, event: &OrderEvent) -> Result<()> { /* ... */ }
}

// Query interface (public)
impl OrderProjection {
    pub async fn get_customer_orders(&self, customer_id: &str) -> Result<Vec<OrderSummary>> { /* ... */ }
    pub async fn get_order(&self, order_id: &str) -> Result<Option<OrderSummary>> { /* ... */ }
    pub async fn count_by_status(&self, status: &str) -> Result<i64> { /* ... */ }
}
```

---

## Advanced Topics

### Projection Rebuilding

Rebuild projections from scratch when schema changes:

```rust
async fn rebuild_projection(&self) -> Result<()> {
    // 1. Clear existing data
    self.projection.rebuild().await?;

    // 2. Reset checkpoint
    let start_position = EventPosition::new(0, DateTime::UNIX_EPOCH);
    self.checkpoint_store.save_position(self.projection.name(), start_position).await?;

    // 3. Replay all events
    let events = self.event_store.load_all_events().await?;
    for event in events {
        self.projection.apply_event(&event).await?;
    }

    Ok(())
}
```

### Snapshot Strategy

Use snapshots for faster rebuilds:

```rust
struct SnapshotStrategy {
    snapshot_interval: u64,  // Snapshot every N events
}

impl SnapshotStrategy {
    async fn should_snapshot(&self, offset: u64) -> bool {
        offset % self.snapshot_interval == 0
    }

    async fn save_snapshot(&self, projection_name: &str, data: &[u8]) -> Result<()> {
        self.snapshot_store.save(&format!("snapshot:{projection_name}"), data).await
    }

    async fn load_snapshot(&self, projection_name: &str) -> Result<Option<Vec<u8>>> {
        self.snapshot_store.get(&format!("snapshot:{projection_name}")).await
    }
}
```

### Multi-Projection Coordination

Run multiple projections efficiently:

```rust
async fn run_projections_parallel(&self) -> Result<()> {
    let handles: Vec<_> = self.projections.iter().map(|proj| {
        let proj = proj.clone();
        tokio::spawn(async move {
            proj.process_events().await
        })
    }).collect();

    // Wait for all projections
    for handle in handles {
        handle.await??;
    }
    Ok(())
}
```

### Projection Monitoring

Track projection health:

```rust
#[derive(Debug)]
struct ProjectionMetrics {
    name: String,
    last_processed_offset: u64,
    last_processed_time: DateTime<Utc>,
    events_processed: u64,
    errors: u64,
    lag: Duration,  // Time behind event store
}

impl ProjectionMonitor {
    async fn check_health(&self) -> Vec<ProjectionMetrics> {
        let mut metrics = Vec::new();

        for projection in &self.projections {
            let checkpoint = self.load_checkpoint(projection.name()).await?;
            let latest_event = self.event_store.get_latest_event().await?;

            let lag = latest_event.timestamp - checkpoint.timestamp;

            metrics.push(ProjectionMetrics {
                name: projection.name().to_string(),
                last_processed_offset: checkpoint.offset,
                last_processed_time: checkpoint.timestamp,
                lag,
                // ... other metrics
            });
        }

        metrics
    }
}
```

---

## Examples

### Example 1: User Profile Projection

Aggregate user data from multiple event types:

```rust
#[derive(Serialize, Deserialize)]
struct UserProfile {
    user_id: String,
    email: String,
    display_name: String,
    created_at: DateTime<Utc>,
    last_login: DateTime<Utc>,
    login_count: u64,
    is_verified: bool,
}

impl Projection for UserProfileProjection {
    type Event = UserEvent;

    async fn apply_event(&self, event: &UserEvent) -> Result<()> {
        match event {
            UserEvent::UserRegistered { user_id, email, display_name, timestamp } => {
                let profile = UserProfile {
                    user_id: user_id.clone(),
                    email: email.clone(),
                    display_name: display_name.clone(),
                    created_at: *timestamp,
                    last_login: *timestamp,
                    login_count: 0,
                    is_verified: false,
                };
                self.save_profile(&profile).await?;
            }

            UserEvent::UserLoggedIn { user_id, timestamp } => {
                if let Some(mut profile) = self.load_profile(user_id).await? {
                    profile.last_login = *timestamp;
                    profile.login_count += 1;
                    self.save_profile(&profile).await?;
                }
            }

            UserEvent::EmailVerified { user_id, .. } => {
                if let Some(mut profile) = self.load_profile(user_id).await? {
                    profile.is_verified = true;
                    self.save_profile(&profile).await?;
                }
            }

            _ => {} // Ignore other events
        }
        Ok(())
    }
}
```

### Example 2: Real-Time Dashboard Projection

Track system metrics in real-time:

```rust
struct DashboardProjection {
    metrics: Arc<RwLock<DashboardMetrics>>,
}

#[derive(Default)]
struct DashboardMetrics {
    total_orders: u64,
    total_revenue: i64,
    orders_today: u64,
    orders_this_hour: u64,
    active_users: HashSet<String>,
}

impl Projection for DashboardProjection {
    type Event = SystemEvent;

    async fn apply_event(&self, event: &SystemEvent) -> Result<()> {
        let mut metrics = self.metrics.write().unwrap();

        match event {
            SystemEvent::OrderPlaced { total, timestamp, .. } => {
                metrics.total_orders += 1;
                metrics.total_revenue += total.cents();

                if is_today(*timestamp) {
                    metrics.orders_today += 1;
                }

                if is_this_hour(*timestamp) {
                    metrics.orders_this_hour += 1;
                }
            }

            SystemEvent::UserActivity { user_id, .. } => {
                metrics.active_users.insert(user_id.clone());
            }

            _ => {}
        }
        Ok(())
    }
}

impl DashboardProjection {
    pub fn get_metrics(&self) -> DashboardMetrics {
        self.metrics.read().unwrap().clone()
    }
}
```

### Example 3: Search Index Projection

Build search indexes from events:

```rust
struct SearchIndexProjection {
    search_engine: Arc<dyn SearchEngine>,
}

#[derive(Serialize)]
struct ProductSearchDocument {
    id: String,
    name: String,
    description: String,
    category: String,
    price: i64,
    tags: Vec<String>,
}

impl Projection for SearchIndexProjection {
    type Event = ProductEvent;

    async fn apply_event(&self, event: &ProductEvent) -> Result<()> {
        match event {
            ProductEvent::ProductCreated { product_id, name, description, category, price, .. } => {
                let doc = ProductSearchDocument {
                    id: product_id.clone(),
                    name: name.clone(),
                    description: description.clone(),
                    category: category.clone(),
                    price: price.cents(),
                    tags: vec![],
                };
                self.search_engine.index_document(&doc).await?;
            }

            ProductEvent::ProductUpdated { product_id, name, description, .. } => {
                self.search_engine.update_document(product_id, |doc| {
                    if let Some(n) = name {
                        doc.name = n.clone();
                    }
                    if let Some(d) = description {
                        doc.description = d.clone();
                    }
                }).await?;
            }

            ProductEvent::ProductDeleted { product_id, .. } => {
                self.search_engine.delete_document(product_id).await?;
            }

            _ => {}
        }
        Ok(())
    }
}
```

---

## Performance Considerations

### Batching

Process events in batches for better throughput:

```rust
const BATCH_SIZE: usize = 100;

async fn process_events_batched(&self) -> Result<()> {
    let mut events = Vec::with_capacity(BATCH_SIZE);

    loop {
        events.clear();

        // Collect batch
        for _ in 0..BATCH_SIZE {
            if let Some(event) = self.event_stream.next().await {
                events.push(event);
            } else {
                break;
            }
        }

        if events.is_empty() {
            break;
        }

        // Process batch in transaction
        let mut tx = self.pool.begin().await?;

        for event in &events {
            self.apply_event_in_tx(&mut tx, event).await?;
        }

        // Checkpoint last event
        if let Some(last_event) = events.last() {
            self.save_checkpoint_in_tx(&mut tx, last_event).await?;
        }

        tx.commit().await?;
    }

    Ok(())
}
```

### Caching

Cache frequently accessed projection data:

```rust
struct CachedProjection {
    projection: OrderProjection,
    cache: Arc<RwLock<HashMap<String, CachedOrder>>>,
    ttl: Duration,
}

impl CachedProjection {
    pub async fn get_order(&self, order_id: &str) -> Result<Option<OrderSummary>> {
        // Check cache
        {
            let cache = self.cache.read().unwrap();
            if let Some(cached) = cache.get(order_id) {
                if cached.expires_at > Utc::now() {
                    return Ok(Some(cached.order.clone()));
                }
            }
        }

        // Cache miss - load from projection
        if let Some(order) = self.projection.get_order(order_id).await? {
            let mut cache = self.cache.write().unwrap();
            cache.insert(order_id.to_string(), CachedOrder {
                order: order.clone(),
                expires_at: Utc::now() + self.ttl,
            });
            Ok(Some(order))
        } else {
            Ok(None)
        }
    }
}
```

---

## Troubleshooting

### Projection Lag

**Symptom**: Projection falls behind event store.

**Causes**:
- Slow database queries
- Complex event processing logic
- Insufficient resources

**Solutions**:
1. Add database indexes
2. Batch event processing
3. Optimize apply_event logic
4. Scale horizontally (partition projections)

### Missing Events

**Symptom**: Projection data inconsistent with event store.

**Causes**:
- Checkpoint not saved after event processing
- Event deserialization errors
- Database transaction failures

**Solutions**:
1. Always checkpoint in same transaction as projection update
2. Add comprehensive error logging
3. Implement dead letter queue for failed events

### Projection Corruption

**Symptom**: Invalid data in projection.

**Causes**:
- Non-idempotent event handling
- Missing event handlers
- Schema mismatches

**Solutions**:
1. Make all event handlers idempotent
2. Handle all event types (even if ignored)
3. Rebuild projection from scratch
4. Add validation checks

---

## Related Documentation

- [Event Sourcing Guide](./event-sourcing.md)
- [CQRS Architecture](./cqrs.md)
- [Testing Guide](./testing.md)
- [PostgreSQL Setup](./postgres-setup.md)

---

## Further Reading

- [CQRS Journey by Microsoft](https://docs.microsoft.com/en-us/previous-versions/msp-n-p/jj554200(v=pandp.10))
- [Event Sourcing Pattern](https://martinfowler.com/eaaDev/EventSourcing.html)
- [Projections in Event Store](https://www.eventstore.com/blog/projections-1-theory)
