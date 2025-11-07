# Order Projection Example

This example demonstrates the **projection system** for building **read models** from events, showing the query side of CQRS.

## What You'll Learn

1. **Projection Trait**: How to implement a projection that processes events
2. **Event Processing**: Handling different event types to build a denormalized view
3. **Query API**: Separating query methods from projection updates
4. **CQRS Separation**: Using separate databases for events and projections
5. **Checkpoint Resumption**: Resuming from last processed event after restart
6. **Rebuild**: Dropping and rebuilding projections from scratch

## Architecture

```
Write Side (Event Store)          Read Side (Projections)
┌─────────────────────┐          ┌─────────────────────┐
│  PostgreSQL DB #1   │          │  PostgreSQL DB #2   │
│                     │          │                     │
│  events table       │          │  order_projections  │
│  snapshots          │   →→→    │  (denormalized)     │
│  (normalized)       │  Events  │  (optimized for     │
└─────────────────────┘          │   queries)          │
                                 └─────────────────────┘
```

## Key Concepts

### CQRS (Command Query Responsibility Segregation)

- **Write Side**: Event store with normalized data, optimized for writes
- **Read Side**: Projections with denormalized data, optimized for queries
- **Separation**: Different databases for different workloads
- **Eventually Consistent**: Projections lag behind events (typically 10-100ms)

### Projection Lifecycle

1. **Subscribe**: Projection manager subscribes to event bus topic
2. **Process**: Each event updates the projection's read model
3. **Checkpoint**: Progress is saved periodically (every N events)
4. **Resume**: On restart, projection resumes from last checkpoint
5. **Rebuild**: Can drop and rebuild projection from all events

## Running the Example

### Prerequisites

- PostgreSQL running locally (or use connection string)
- Database created: `composable_rust`

```bash
# Create database
createdb composable_rust

# Or specify custom connection
export DATABASE_URL="postgres://user:pass@localhost/mydb"
```

### Run the Example

```bash
cd examples/order-projection
cargo run
```

### Expected Output

```
🚀 Order Projection Example

📦 Connecting to PostgreSQL...
🔧 Running migrations...
📝 Applying sample events...
  ✅ OrderPlaced: order-1 for customer-alice ($100.00)
  ✅ OrderPlaced: order-2 for customer-bob ($120.00)
  ✅ OrderPlaced: order-3 for customer-alice ($45.00)
  ✅ OrderShipped: order-1 (tracking: TRACK-123456)
  ✅ OrderCancelled: order-3 (reason: Customer requested)

🔍 Querying the projection...

📊 Orders for customer-alice (2 orders):
  - Order order-1: 2 items, $100.00, status: shipped, tracking: TRACK-123456
  - Order order-3: 1 items, $45.00, status: cancelled, reason: Customer requested cancellation

📊 Orders for customer-bob (1 orders):
  - Order order-2: 3 items, $120.00, status: placed

📊 Recent orders (3 orders):
  - Order order-3 (customer-alice): 1 items, $45.00, status: cancelled
  - Order order-2 (customer-bob): 3 items, $120.00, status: placed
  - Order order-1 (customer-alice): 2 items, $100.00, status: shipped

📊 Order counts by status:
  - Placed: 1
  - Shipped: 1
  - Cancelled: 1

✅ Example complete!
```

## Code Walkthrough

### 1. Projection Implementation

```rust
pub struct CustomerOrderHistoryProjection {
    pool: PgPool,
}

impl Projection for CustomerOrderHistoryProjection {
    type Event = OrderAction;

    fn name(&self) -> &str {
        "customer_order_history"
    }

    async fn apply_event(&self, event: &Self::Event) -> Result<()> {
        match event {
            OrderAction::OrderPlaced { order_id, customer_id, items, total, timestamp } => {
                // Insert new order into projection
                sqlx::query("INSERT INTO order_projections (...) VALUES (...)")
                    .execute(&self.pool)
                    .await?;
                Ok(())
            }
            OrderAction::OrderShipped { order_id, tracking, .. } => {
                // Update order status
                sqlx::query("UPDATE order_projections SET status = 'shipped', tracking = $2 WHERE id = $1")
                    .execute(&self.pool)
                    .await?;
                Ok(())
            }
            // ... handle other events
        }
    }
}
```

### 2. Query API (Separate from Projection Updates)

```rust
impl CustomerOrderHistoryProjection {
    pub async fn get_customer_orders(&self, customer_id: &str) -> Result<Vec<OrderSummary>> {
        sqlx::query_as("SELECT * FROM order_projections WHERE customer_id = $1 ORDER BY placed_at DESC")
            .fetch_all(&self.pool)
            .await
    }

    pub async fn count_by_status(&self, status: &str) -> Result<i64> {
        sqlx::query_as("SELECT COUNT(*) FROM order_projections WHERE status = $1")
            .fetch_one(&self.pool)
            .await
    }
}
```

### 3. Denormalized Schema (Optimized for Queries)

```sql
CREATE TABLE order_projections (
    id TEXT PRIMARY KEY,
    customer_id TEXT NOT NULL,
    item_count INTEGER NOT NULL,
    total_cents BIGINT NOT NULL,
    status TEXT NOT NULL,
    placed_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    tracking TEXT,
    cancellation_reason TEXT
);

-- Indexes for common query patterns
CREATE INDEX idx_order_projections_customer ON order_projections(customer_id);
CREATE INDEX idx_order_projections_status ON order_projections(status);
CREATE INDEX idx_order_projections_placed_at ON order_projections(placed_at DESC);
```

## CQRS Patterns

### Pattern 1: Same Database (Simple)

For development or small systems:

```rust
// Same database for events and projections
let pool = PgPool::connect("postgres://localhost/composable_rust").await?;

let event_store = PostgresEventStore::new(pool.clone());
let projection = CustomerOrderHistoryProjection::new(pool.clone());
```

### Pattern 2: Separate Databases (Production CQRS)

For production systems with different workloads:

```rust
// Separate databases for write and read models
let event_pool = PgPool::connect("postgres://localhost/events").await?;
let projection_pool = PgPool::connect("postgres://localhost/projections").await?;

let event_store = PostgresEventStore::new(event_pool);
let projection = CustomerOrderHistoryProjection::new(projection_pool);
```

**Benefits**:
- **Scalability**: Scale reads and writes independently
- **Performance**: Optimize each database for its workload
- **Isolation**: Queries don't impact event writes
- **Flexibility**: Use different database types (Postgres for projections, Redis for caching)

## Projection Manager

In production, use `ProjectionManager` to automatically process events from the event bus:

```rust
use composable_rust_projections::manager::ProjectionManager;

// Create projection manager
let (manager, shutdown) = ProjectionManager::new(
    projection,
    event_bus,
    checkpoint,
    "order-events",          // Topic
    "order-projection-group", // Consumer group
);

// Start processing events (blocks until shutdown)
manager.start().await?;
```

The manager:
- Subscribes to event bus topics
- Processes events continuously
- Saves checkpoints periodically
- Resumes from last checkpoint on restart
- Handles errors gracefully

## Advanced Queries

The example shows basic queries, but you can build much more complex ones:

```rust
// Search orders by date range
pub async fn get_orders_by_date_range(
    &self,
    customer_id: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<OrderSummary>> {
    sqlx::query_as(
        "SELECT * FROM order_projections
         WHERE customer_id = $1 AND placed_at BETWEEN $2 AND $3
         ORDER BY placed_at DESC"
    )
    .fetch_all(&self.pool)
    .await
}

// Aggregate queries
pub async fn get_customer_stats(&self, customer_id: &str) -> Result<CustomerStats> {
    sqlx::query_as(
        "SELECT
            COUNT(*) as order_count,
            SUM(total_cents) as lifetime_value,
            MAX(placed_at) as last_order_date
         FROM order_projections
         WHERE customer_id = $1"
    )
    .fetch_one(&self.pool)
    .await
}

// Full-text search (if using JSONB)
pub async fn search_orders(&self, search_term: &str) -> Result<Vec<OrderSummary>> {
    sqlx::query_as(
        "SELECT * FROM order_projections
         WHERE data @> $1::jsonb
         ORDER BY placed_at DESC"
    )
    .fetch_all(&self.pool)
    .await
}
```

## Rebuilding Projections

If the projection schema changes or data gets corrupted, rebuild from scratch:

```rust
// 1. Drop projection data
projection.rebuild().await?;

// 2. Reset checkpoint
checkpoint.save_position("customer_order_history", EventPosition::beginning()).await?;

// 3. Replay all events
manager.start().await?;  // Will process from beginning
```

## Testing Projections

Use in-memory stores for fast, deterministic tests:

```rust
use composable_rust_testing::InMemoryProjectionStore;

#[tokio::test]
async fn test_order_projection() {
    let store = Arc::new(InMemoryProjectionStore::new());
    let projection = CustomerOrderProjection::new(store.clone());

    // Apply events
    projection.apply_event(&order_placed_event).await?;
    projection.apply_event(&order_shipped_event).await?;

    // Query and assert
    let orders = projection.get_customer_orders("customer-1").await?;
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].status, "shipped");
}
```

## Performance Considerations

### Indexes

Create indexes for common query patterns:

```sql
-- Customer lookup (most common)
CREATE INDEX idx_customer ON order_projections(customer_id);

-- Date range queries
CREATE INDEX idx_placed_at ON order_projections(placed_at DESC);

-- Status filtering
CREATE INDEX idx_status ON order_projections(status);

-- Full-text search (if using JSONB)
CREATE INDEX idx_data_gin ON order_projections USING gin(data);
```

### Checkpoint Interval

Balance between resumption granularity and I/O:

```rust
// More frequent = faster resumption, more I/O
let manager = manager.with_checkpoint_interval(50);   // Every 50 events

// Less frequent = less I/O, slower resumption
let manager = manager.with_checkpoint_interval(1000); // Every 1000 events
```

### Projection Lag

Monitor how far behind the projection is:

```rust
// Compare event stream position with projection checkpoint
let event_store_version = event_store.get_current_version().await?;
let projection_version = checkpoint.load_position("customer_order_history").await?;

let lag = event_store_version - projection_version.offset;
println!("Projection lag: {} events", lag);
```

## Next Steps

1. **Connect to Event Bus**: Use Redpanda for real-time event streaming
2. **Add More Projections**: Create projections for different query patterns
3. **Separate Databases**: Use separate databases for true CQRS
4. **Add Caching**: Layer Redis on top for hot data
5. **Monitor Lag**: Track projection lag and alert if too high
6. **Add Metrics**: Export projection metrics to Prometheus

## Related Examples

- `order-processing`: Event sourcing (write side)
- `checkout-saga`: Saga pattern (coordination)
- `metrics-demo`: Observability

## Further Reading

- [CQRS Pattern](https://martinfowler.com/bliki/CQRS.html)
- [Event Sourcing](https://martinfowler.com/eaaDev/EventSourcing.html)
- [Read Models](https://docs.microsoft.com/en-us/azure/architecture/patterns/cqrs#read-models)
