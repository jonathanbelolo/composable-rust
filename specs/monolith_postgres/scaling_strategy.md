# Scaling Strategy for PostgreSQL Monolith

> **Extends**: `bounded_contexts.md`
>
> This document describes how to scale the PostgreSQL monolith architecture
> horizontally without abandoning the core design. The bounded contexts
> architecture was designed with this scaling path in mind.

---

## 1. Scaling Philosophy

### 1.1 The Monolith Advantage

Starting with a PostgreSQL monolith provides:
- **Simplicity**: Single database, ACID transactions, no distributed systems complexity
- **Consistency**: Cross-context operations are transactional
- **Performance**: PostgreSQL is remarkably capable (millions of rows, thousands of TPS)
- **Flexibility**: Schema-per-context is a "soft boundary" that can become "hard"

### 1.2 When to Scale

| Signal | Solution |
|--------|----------|
| Read latency increasing | Add read replicas |
| Write throughput saturating | Split hot contexts to separate databases |
| Connection limits reached | Connection pooling (PgBouncer) |
| Storage growing large | Archive old events, partition tables |
| Geographic latency | Regional deployments |

### 1.3 Scaling Stages

```
Stage 1: Single Database (Start Here)
┌─────────────────────────────────────────────┐
│                PostgreSQL                   │
│  ┌─────────┐ ┌─────────┐ ┌─────────┐       │
│  │  sales  │ │inventory│ │shipping │       │
│  │ schema  │ │ schema  │ │ schema  │       │
│  └─────────┘ └─────────┘ └─────────┘       │
│         integration.domain_events           │
└─────────────────────────────────────────────┘

Stage 2: Read Replicas
┌──────────────┐     ┌──────────────┐
│   Primary    │────▶│   Replica    │
│   (writes)   │     │   (reads)    │
└──────────────┘     └──────────────┘

Stage 3: Context Splitting
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│  Sales DB    │  │ Inventory DB │  │ Shipping DB  │
└──────────────┘  └──────────────┘  └──────────────┘
        │                │                │
        └────────────────┼────────────────┘
                         ▼
              ┌─────────────────────┐
              │  Integration Layer  │
              │ (Kafka/Redpanda)    │
              └─────────────────────┘

Stage 4: Per-Context Replicas
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│  Sales DB    │  │ Inventory DB │  │ Shipping DB  │
│  + 1 replica │  │  + 4 replicas│  │  + 2 replicas│
└──────────────┘  └──────────────┘  └──────────────┘
```

---

## 2. Stage 1: Optimizing the Monolith

Before adding infrastructure, optimize the existing database.

### 2.1 Connection Pooling

```
┌─────────────────┐
│   pg-gateway    │
│  (Rust layer)   │
└────────┬────────┘
         │ Many connections
         ▼
┌─────────────────┐
│   PgBouncer     │
│  (connection    │
│   pooling)      │
└────────┬────────┘
         │ Few connections
         ▼
┌─────────────────┐
│   PostgreSQL    │
└─────────────────┘
```

**PgBouncer configuration (pgbouncer.ini):**
```ini
[databases]
myapp = host=localhost port=5432 dbname=myapp

[pgbouncer]
listen_port = 6432
listen_addr = *
auth_type = md5
auth_file = /etc/pgbouncer/userlist.txt

; Transaction pooling for best efficiency
pool_mode = transaction

; Size limits
max_client_conn = 1000
default_pool_size = 20
min_pool_size = 5
reserve_pool_size = 5
```

**pg-gateway connection:**
```rust
use composable_rust_pg_gateway::{DbConfig, create_pool};

// Connect through PgBouncer
let config = DbConfig::with_url("postgres://user:pass@localhost:6432/myapp")
    .max_connections(100);  // Can be higher with PgBouncer
let pool = create_pool(&config).await?;
```

### 2.2 Indexing Strategy

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ESSENTIAL INDEXES
-- ═══════════════════════════════════════════════════════════════════════════

-- Global event log: Primary access patterns
CREATE INDEX idx_event_log_stream ON global.event_log (stream_id, version);
CREATE INDEX idx_event_log_context ON global.event_log (context, created_at);
CREATE INDEX idx_event_log_type ON global.event_log (event_type, created_at);

-- Context events: Stream loading
CREATE INDEX idx_sales_events_stream ON sales.events (stream_id, version);
CREATE INDEX idx_inventory_events_stream ON inventory.events (stream_id, version);

-- Projections: Common query patterns
CREATE INDEX idx_orders_customer ON sales.orders_projection (customer_id, created_at DESC);
CREATE INDEX idx_orders_status ON sales.orders_projection (status) WHERE status != 'completed';
CREATE INDEX idx_stock_product ON inventory.stock_projection (product_id, warehouse_id);

-- Integration: Consumer polling
CREATE INDEX idx_domain_events_pending ON integration.domain_events (id)
    WHERE processed_at IS NULL;
```

### 2.3 Table Partitioning

For high-volume event logs, partition by time:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- PARTITIONED EVENT LOG
-- ═══════════════════════════════════════════════════════════════════════════

-- Create partitioned table
CREATE TABLE global.event_log (
    id              BIGSERIAL,
    stream_id       TEXT NOT NULL,
    version         INTEGER NOT NULL,
    context         TEXT NOT NULL,
    event_type      TEXT NOT NULL,
    payload         JSONB NOT NULL,
    metadata        JSONB DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

-- IMPORTANT: Stream version uniqueness
-- PostgreSQL requires partition key in UNIQUE constraints, so we can't have
-- a simple UNIQUE (stream_id, version) across partitions. Options:
--
-- 1. Application-level: Use optimistic concurrency in _handle() functions
--    (already recommended in architecture.md)
--
-- 2. Streams table: Enforce uniqueness via a separate metadata table:
--
--    CREATE TABLE global.streams (
--        stream_id TEXT PRIMARY KEY,
--        current_version INTEGER NOT NULL DEFAULT 0,
--        context TEXT NOT NULL
--    );
--
--    Then in _handle(): UPDATE streams SET current_version = current_version + 1
--    WHERE stream_id = $1 AND current_version = $2 (expected version)
--
-- The streams table approach also provides O(1) version lookup.

-- Create monthly partitions
CREATE TABLE global.event_log_2025_01 PARTITION OF global.event_log
    FOR VALUES FROM ('2025-01-01') TO ('2025-02-01');
CREATE TABLE global.event_log_2025_02 PARTITION OF global.event_log
    FOR VALUES FROM ('2025-02-01') TO ('2025-03-01');
-- ... continue for future months

-- Automate partition creation (run monthly via cron/pg_cron)
CREATE OR REPLACE FUNCTION global.create_next_partition()
RETURNS void AS $$
DECLARE
    next_month DATE := date_trunc('month', now() + interval '1 month');
    partition_name TEXT := 'event_log_' || to_char(next_month, 'YYYY_MM');
    start_date DATE := next_month;
    end_date DATE := next_month + interval '1 month';
BEGIN
    EXECUTE format(
        'CREATE TABLE IF NOT EXISTS global.%I PARTITION OF global.event_log
         FOR VALUES FROM (%L) TO (%L)',
        partition_name, start_date, end_date
    );
END;
$$ LANGUAGE plpgsql;
```

### 2.4 Archiving Old Events

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ARCHIVE STRATEGY
-- ═══════════════════════════════════════════════════════════════════════════

-- Archive schema for cold storage
CREATE SCHEMA archive;

-- Move old partitions to archive (keeps data queryable but separate)
ALTER TABLE global.event_log_2024_01 SET SCHEMA archive;
ALTER TABLE global.event_log_2024_02 SET SCHEMA archive;

-- Or detach and compress
ALTER TABLE global.event_log DETACH PARTITION global.event_log_2024_01;
-- Export to S3/cold storage, then DROP

-- Projections don't need old events (they're already materialized)
-- Only keep recent events for replay capability
```

---

## 3. Stage 2: Read Replicas

### 3.1 Architecture

```
                    ┌─────────────────┐
                    │   pg-gateway    │
                    │  (Rust layer)   │
                    └────────┬────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
              ▼              ▼              ▼
        ┌──────────┐  ┌──────────┐  ┌──────────┐
        │ Primary  │  │ Replica  │  │ Replica  │
        │ (writes) │  │ (reads)  │  │ (reads)  │
        └──────────┘  └──────────┘  └──────────┘
              │              ▲              ▲
              │   Streaming  │   Streaming  │
              └──────────────┴──────────────┘
```

### 3.2 PostgreSQL Streaming Replication Setup

**Primary (postgresql.conf):**
```ini
# Replication settings
wal_level = replica
max_wal_senders = 10
max_replication_slots = 10
hot_standby = on

# Synchronous replication (optional, for consistency)
# synchronous_commit = on
# synchronous_standby_names = 'replica1'
```

**Primary (pg_hba.conf):**
```
# Allow replication connections
host replication replicator 10.0.0.0/8 scram-sha-256
```

**Create replication user:**
```sql
CREATE USER replicator WITH REPLICATION ENCRYPTED PASSWORD 'secure_password';
```

**Replica setup:**
```bash
# Stop PostgreSQL on replica
sudo systemctl stop postgresql

# Clear data directory
rm -rf /var/lib/postgresql/16/main/*

# Base backup from primary
pg_basebackup -h primary.example.com -D /var/lib/postgresql/16/main \
    -U replicator -P -R -X stream

# Start replica
sudo systemctl start postgresql
```

### 3.3 pg-gateway Read/Write Routing

```rust
// ═══════════════════════════════════════════════════════════════════════════
// CONNECTION POOLS FOR PRIMARY AND REPLICAS
// ═══════════════════════════════════════════════════════════════════════════

use composable_rust_pg_gateway::{DbConfig, create_pool};
use sqlx::PgPool;

#[derive(Clone)]
pub struct DatabasePools {
    /// Primary pool for writes
    primary: PgPool,
    /// Replica pool for reads (load-balanced across replicas)
    replica: PgPool,
}

/// Error type for database pool initialization.
#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    #[error("Missing environment variable: {0}")]
    MissingEnvVar(String),
    #[error("Database connection failed: {0}")]
    Connection(#[from] sqlx::Error),
}

impl DatabasePools {
    pub async fn from_env() -> Result<Self, PoolError> {
        let primary_url = std::env::var("DATABASE_PRIMARY_URL")
            .map_err(|_| PoolError::MissingEnvVar("DATABASE_PRIMARY_URL".into()))?;
        let replica_url = std::env::var("DATABASE_REPLICA_URL")
            .map_err(|_| PoolError::MissingEnvVar("DATABASE_REPLICA_URL".into()))?;

        let primary_config = DbConfig::with_url(primary_url)
            .max_connections(20);
        let primary = create_pool(&primary_config).await?;

        // Replica URL can be a load balancer or PgBouncer across replicas
        let replica_config = DbConfig::with_url(replica_url)
            .max_connections(50);
        let replica = create_pool(&replica_config).await?;

        Ok(Self { primary, replica })
    }

    /// Get pool for write operations
    pub fn writer(&self) -> &PgPool {
        &self.primary
    }

    /// Get pool for read operations
    pub fn reader(&self) -> &PgPool {
        &self.replica
    }
}
```

**Handler routing:**
```rust
// ═══════════════════════════════════════════════════════════════════════════
// ROUTE READS TO REPLICAS, WRITES TO PRIMARY
// ═══════════════════════════════════════════════════════════════════════════

/// Command handler - routes to PRIMARY
pub async fn handle_command(
    State(pools): State<DatabasePools>,
    identity: Identity,
    Json(command): Json<Command>,
) -> Result<Json<CommandResult>, ApiError> {
    // Writes ALWAYS go to primary
    let result = execute_with_identity(
        pools.writer(),  // PRIMARY
        &identity,
        "SELECT * FROM sales.order_handle($1)",
        &command,
    ).await?;

    Ok(Json(result))
}

/// Query handler - routes to REPLICA
pub async fn get_order(
    State(pools): State<DatabasePools>,
    identity: Identity,
    Path(order_id): Path<Uuid>,
) -> Result<Json<OrderView>, ApiError> {
    // Reads go to replica
    let order: OrderView = sqlx::query_as(
        "SELECT * FROM sales.orders_projection WHERE id = $1"
    )
    .bind(order_id)
    .fetch_optional(pools.reader())  // REPLICA
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(Json(order))
}

/// Read-your-writes pattern - use PRIMARY after write
pub async fn create_and_return_order(
    State(pools): State<DatabasePools>,
    identity: Identity,
    Json(command): Json<CreateOrder>,
) -> Result<Json<OrderView>, ApiError> {
    // Write to primary
    let result = execute_with_identity(
        pools.writer(),
        &identity,
        "SELECT * FROM sales.order_handle($1)",
        &command,
    ).await?;

    // Read from PRIMARY for consistency (not replica)
    // The projection may not have replicated yet
    let order: OrderView = sqlx::query_as(
        "SELECT * FROM sales.orders_projection WHERE id = $1"
    )
    .bind(result.order_id)
    .fetch_one(pools.writer())  // PRIMARY for read-your-writes
    .await?;

    Ok(Json(order))
}
```

### 3.4 Replication Lag Monitoring

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- MONITOR REPLICATION LAG
-- ═══════════════════════════════════════════════════════════════════════════

-- On primary: Check replication status
SELECT
    client_addr,
    state,
    sent_lsn,
    write_lsn,
    flush_lsn,
    replay_lsn,
    pg_wal_lsn_diff(sent_lsn, replay_lsn) AS lag_bytes,
    pg_wal_lsn_diff(sent_lsn, replay_lsn) / 1024 / 1024 AS lag_mb
FROM pg_stat_replication;

-- On replica: Check lag time
SELECT
    now() - pg_last_xact_replay_timestamp() AS replication_lag;
```

**Health check with lag:**
```rust
use axum::{extract::State, http::StatusCode, response::IntoResponse};

pub async fn health_check_with_replication(
    State(pools): State<DatabasePools>,
) -> impl IntoResponse {
    let primary_ok = check_database(pools.writer()).await;
    let replica_ok = check_database(pools.reader()).await;

    // Check replication lag on replica (convert interval to seconds in SQL)
    let lag_ok = if replica_ok {
        let lag_secs: Option<f64> = sqlx::query_scalar(
            "SELECT EXTRACT(EPOCH FROM (now() - pg_last_xact_replay_timestamp()))"
        )
        .fetch_one(pools.reader())
        .await
        .ok()
        .flatten();

        // Acceptable lag: under 10 seconds
        lag_secs.map(|secs| secs < 10.0).unwrap_or(false)
    } else {
        false
    };

    let status = if primary_ok && replica_ok && lag_ok {
        (StatusCode::OK, "healthy")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "unhealthy")
    };

    status
}
```

---

## 4. Stage 3: Bounded Context Splitting

When a single database is no longer sufficient, split contexts into separate databases.

### 4.1 When to Split

| Indicator | Threshold | Action |
|-----------|-----------|--------|
| Write throughput | >5,000 TPS sustained | Split hottest context |
| Table size | >500GB per context | Split that context |
| Lock contention | High wait times on specific schemas | Split contending contexts |
| Team scaling | Multiple teams, deployment independence | Split by team ownership |

### 4.2 Split Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│                            pg-gateway                                     │
│                         (Rust thin layer)                                │
└─────────────┬──────────────────┬──────────────────┬─────────────────────┘
              │                  │                  │
              ▼                  ▼                  ▼
       ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
       │   Sales DB   │  │ Inventory DB │  │ Shipping DB  │
       │              │  │              │  │              │
       │ ┌──────────┐ │  │ ┌──────────┐ │  │ ┌──────────┐ │
       │ │  sales   │ │  │ │inventory │ │  │ │shipping  │ │
       │ │  schema  │ │  │ │  schema  │ │  │ │  schema  │ │
       │ └──────────┘ │  │ └──────────┘ │  │ └──────────┘ │
       │              │  │              │  │              │
       │ event_log    │  │ event_log    │  │ event_log    │
       │ (per-context)│  │ (per-context)│  │ (per-context)│
       └──────┬───────┘  └──────┬───────┘  └──────┬───────┘
              │                 │                 │
              │    ┌────────────┼────────────┐    │
              │    │            ▼            │    │
              │    │  ┌──────────────────┐   │    │
              └────┼─▶│  Kafka/Redpanda  │◀──┼────┘
                   │  │ (Integration Bus)│   │
                   │  └──────────────────┘   │
                   │                         │
                   └─────────────────────────┘
```

### 4.3 What Changes

| Component | Before (Monolith) | After (Split) |
|-----------|-------------------|---------------|
| **Event Log** | `global.event_log` (single) | Per-context event log |
| **Integration** | `integration.domain_events` table | Kafka/Redpanda topics |
| **Cross-context comm** | LISTEN/NOTIFY | Kafka consumers |
| **Transactions** | ACID across contexts | Eventual consistency / Sagas |
| **Connection pools** | Single pool | Pool per context |

### 4.4 pg-gateway Multi-Database Configuration

```rust
// ═══════════════════════════════════════════════════════════════════════════
// MULTI-CONTEXT DATABASE POOLS
// ═══════════════════════════════════════════════════════════════════════════

use composable_rust_pg_gateway::{DbConfig, create_pool};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;

/// Holds connection pools for all bounded contexts.
#[derive(Clone)]
pub struct ContextPools {
    pools: Arc<HashMap<String, PgPool>>,
}

impl ContextPools {
    /// Create pools for all configured contexts.
    ///
    /// Reads environment variables: SALES_DATABASE_URL, INVENTORY_DATABASE_URL, etc.
    pub async fn from_env(contexts: &[&str]) -> Result<Self, PoolError> {
        let mut pools = HashMap::new();

        for context in contexts {
            let env_var = format!("{}_DATABASE_URL", context.to_uppercase());
            let url = std::env::var(&env_var)
                .map_err(|_| PoolError::MissingEnvVar(env_var))?;

            let config = DbConfig::with_url(url).max_connections(20);
            let pool = create_pool(&config).await?;
            pools.insert((*context).to_string(), pool);
        }

        Ok(Self { pools: Arc::new(pools) })
    }

    pub fn get(&self, context: &str) -> Option<&PgPool> {
        self.pools.get(context)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &PgPool)> {
        self.pools.iter()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TYPE-SAFE EXTRACTORS FOR SPECIFIC CONTEXTS
// ═══════════════════════════════════════════════════════════════════════════

use axum::extract::FromRef;

/// Type-safe wrapper for the sales database pool.
#[derive(Clone)]
pub struct SalesPool(pub PgPool);

/// Type-safe wrapper for the inventory database pool.
#[derive(Clone)]
pub struct InventoryPool(pub PgPool);

/// Type-safe wrapper for the shipping database pool.
#[derive(Clone)]
pub struct ShippingPool(pub PgPool);

/// Application state containing all context pools.
///
/// IMPORTANT: AppState must be initialized with all required contexts,
/// or the FromRef implementations will panic at runtime.
#[derive(Clone)]
pub struct AppState {
    pub context_pools: ContextPools,
    // ... other state fields
}

impl FromRef<AppState> for SalesPool {
    fn from_ref(state: &AppState) -> Self {
        // SAFETY: AppState initialization must ensure "sales" pool exists.
        // In production, validate this at startup rather than panicking here.
        let pool = state.context_pools
            .get("sales")
            .expect("sales pool not configured - check SALES_DATABASE_URL");
        Self(pool.clone())
    }
}

impl FromRef<AppState> for InventoryPool {
    fn from_ref(state: &AppState) -> Self {
        let pool = state.context_pools
            .get("inventory")
            .expect("inventory pool not configured - check INVENTORY_DATABASE_URL");
        Self(pool.clone())
    }
}

impl FromRef<AppState> for ShippingPool {
    fn from_ref(state: &AppState) -> Self {
        let pool = state.context_pools
            .get("shipping")
            .expect("shipping pool not configured - check SHIPPING_DATABASE_URL");
        Self(pool.clone())
    }
}
```

**Context-specific routers:**
```rust
// ═══════════════════════════════════════════════════════════════════════════
// CONTEXT-SPECIFIC ROUTING
// ═══════════════════════════════════════════════════════════════════════════

pub fn sales_router() -> Router<AppState> {
    Router::new()
        .route("/orders", post(create_order))
        .route("/orders/:id", get(get_order))
        .route("/orders/:id/commands", post(order_command))
}

pub fn inventory_router() -> Router<AppState> {
    Router::new()
        .route("/products", get(list_products))
        .route("/products/:id/stock", get(get_stock))
        .route("/reservations", post(create_reservation))
}

pub fn app_router() -> Router<AppState> {
    Router::new()
        .nest("/api/sales", sales_router())
        .nest("/api/inventory", inventory_router())
        .nest("/api/shipping", shipping_router())
}

// Handlers use context-specific extractors
async fn create_order(
    State(sales): State<SalesPool>,
    identity: Identity,
    Json(cmd): Json<CreateOrder>,
) -> Result<Json<OrderResult>, ApiError> {
    // Routes to sales database only
    execute_with_identity(&sales.0, &identity, "SELECT sales.order_handle($1)", &cmd).await
}
```

### 4.5 Integration Layer: Outbox to Kafka

Replace `integration.domain_events` with the transactional outbox pattern:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- INTEGRATION OUTBOX TABLE (in each context database)
-- For publishing domain events to Kafka when contexts are split
-- Note: This is separate from outbox.pending_tasks (used for side-effects)
-- ═══════════════════════════════════════════════════════════════════════════

CREATE SCHEMA IF NOT EXISTS outbox;

CREATE TABLE outbox.integration_events (
    id              BIGSERIAL PRIMARY KEY,
    aggregate_type  TEXT NOT NULL,      -- e.g., 'order', 'customer'
    aggregate_id    TEXT NOT NULL,      -- e.g., 'order-123'
    event_type      TEXT NOT NULL,      -- e.g., 'OrderCreated'
    payload         JSONB NOT NULL,     -- Full event data
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at    TIMESTAMPTZ         -- NULL until published to Kafka
);

CREATE INDEX idx_integration_events_unpublished
    ON outbox.integration_events (id)
    WHERE published_at IS NULL;

-- Trigger to populate outbox from domain events
CREATE OR REPLACE FUNCTION sales.publish_integration_event()
RETURNS TRIGGER AS $$
BEGIN
    -- Only publish events that other contexts care about
    IF NEW.event_type IN ('OrderCreated', 'OrderCancelled', 'OrderShipped') THEN
        INSERT INTO outbox.integration_events
            (aggregate_type, aggregate_id, event_type, payload)
        VALUES
            ('order', NEW.stream_id, NEW.event_type, NEW.payload);
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_publish_integration_event
    AFTER INSERT ON sales.event_log
    FOR EACH ROW EXECUTE FUNCTION sales.publish_integration_event();
```

**Outbox publisher (pg-gateway task):**
```rust
// ═══════════════════════════════════════════════════════════════════════════
// INTEGRATION EVENT PUBLISHER
// Polls outbox tables and publishes to Kafka
// ═══════════════════════════════════════════════════════════════════════════

use std::time::Duration;
use sqlx::PgPool;

#[derive(Debug, sqlx::FromRow)]
struct IntegrationEvent {
    id: i64,
    aggregate_type: String,
    aggregate_id: String,
    event_type: String,
    payload: serde_json::Value,
}

pub async fn run_integration_publisher(
    pools: ContextPools,
    kafka_producer: KafkaProducer,
) {
    loop {
        for (context, pool) in pools.iter() {
            if let Err(e) = publish_pending_events(context, pool, &kafka_producer).await {
                tracing::error!(context = %context, error = %e, "Failed to publish events");
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn publish_pending_events(
    context: &str,
    pool: &PgPool,
    kafka_producer: &KafkaProducer,
) -> Result<(), Box<dyn std::error::Error>> {
    // Fetch unpublished events with row-level locking
    let events: Vec<IntegrationEvent> = sqlx::query_as(
        "SELECT id, aggregate_type, aggregate_id, event_type, payload
         FROM outbox.integration_events
         WHERE published_at IS NULL
         ORDER BY id
         LIMIT 100
         FOR UPDATE SKIP LOCKED"
    )
    .fetch_all(pool)
    .await?;

    for event in events {
        // Publish to Kafka topic: {context}.{aggregate_type}
        let topic = format!("{}.{}", context, event.aggregate_type);
        kafka_producer.send(&topic, &event.payload).await?;

        // Mark as published (idempotent - Kafka may have duplicates)
        sqlx::query(
            "UPDATE outbox.integration_events SET published_at = now() WHERE id = $1"
        )
        .bind(event.id)
        .execute(pool)
        .await?;
    }

    Ok(())
}
```

### 4.6 Cross-Context Event Consumption

```rust
// ═══════════════════════════════════════════════════════════════════════════
// INTEGRATION EVENT CONSUMER
// Receives events from other contexts via Kafka
// ═══════════════════════════════════════════════════════════════════════════

pub async fn run_integration_consumer(
    inventory_pool: PgPool,
    kafka_consumer: KafkaConsumer,
) {
    // Inventory subscribes to sales events
    kafka_consumer.subscribe(&["sales.order"]).await;

    loop {
        if let Some(message) = kafka_consumer.poll().await {
            let event: IntegrationEvent = serde_json::from_slice(&message.payload)?;

            match event.event_type.as_str() {
                "OrderCreated" => {
                    // Reserve inventory for new order
                    sqlx::query("SELECT inventory.handle_order_created($1)")
                        .bind(&event.payload)
                        .execute(&inventory_pool)
                        .await?;
                }
                "OrderCancelled" => {
                    // Release reservation
                    sqlx::query("SELECT inventory.handle_order_cancelled($1)")
                        .bind(&event.payload)
                        .execute(&inventory_pool)
                        .await?;
                }
                _ => {}
            }

            kafka_consumer.commit(&message).await?;
        }
    }
}
```

---

## 5. Stage 4: Per-Context Replicas

### 5.1 Independent Scaling

Each context database can now scale independently:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                                                                             │
│   Sales Context (write-heavy)        Catalog Context (read-heavy)          │
│   ┌──────────────────────────┐       ┌──────────────────────────┐          │
│   │       Primary            │       │       Primary            │          │
│   │   (fast NVMe SSD)        │       │                          │          │
│   └────────────┬─────────────┘       └────────────┬─────────────┘          │
│                │                                  │                         │
│                ▼                     ┌────────────┼────────────┐            │
│   ┌──────────────────────────┐       │            │            │            │
│   │       Replica            │       ▼            ▼            ▼            │
│   │   (1 for DR only)        │   ┌────────┐  ┌────────┐  ┌────────┐        │
│   └──────────────────────────┘   │Replica1│  │Replica2│  │Replica3│        │
│                                  └────────┘  └────────┘  └────────┘        │
│                                                                             │
│   Analytics Context (batch)                                                 │
│   ┌──────────────────────────┐                                             │
│   │       Primary            │                                             │
│   │  (large HDD storage)     │◀── Receives events from all contexts       │
│   └────────────┬─────────────┘    via Kafka for cross-cutting analysis     │
│                │                                                            │
│                ▼                                                            │
│   ┌──────────────────────────┐                                             │
│   │    Replica (reporting)   │                                             │
│   └──────────────────────────┘                                             │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 5.2 Context-Specific Optimization

**Sales (Write-Heavy):**
```ini
# postgresql.conf for write-heavy workload
shared_buffers = 8GB
effective_cache_size = 24GB
maintenance_work_mem = 2GB
checkpoint_completion_target = 0.9
wal_buffers = 64MB
default_statistics_target = 100
random_page_cost = 1.1  # SSD
effective_io_concurrency = 200  # SSD
max_worker_processes = 8
max_parallel_workers_per_gather = 4
```

**Catalog (Read-Heavy):**
```ini
# postgresql.conf for read-heavy workload
shared_buffers = 16GB  # More buffer for caching
effective_cache_size = 48GB
work_mem = 256MB  # Larger for complex queries
random_page_cost = 1.1
effective_io_concurrency = 200
max_parallel_workers_per_gather = 8  # More parallelism for reads
```

### 5.3 Load Balancer Configuration

**HAProxy for read replicas:**
```haproxy
# haproxy.cfg

frontend pg_frontend
    bind *:5432
    default_backend pg_replicas

backend pg_replicas
    balance roundrobin
    option httpchk GET /health
    server replica1 10.0.1.1:5432 check port 8080
    server replica2 10.0.1.2:5432 check port 8080
    server replica3 10.0.1.3:5432 check port 8080

backend pg_primary
    server primary 10.0.0.1:5432 check
```

---

## 6. Event Log Splitting Options

### 6.1 Option A: Per-Context Event Logs (Recommended)

Each database has its own event log. This is the natural evolution:

```sql
-- Sales database
CREATE TABLE sales.event_log (
    id              BIGSERIAL PRIMARY KEY,
    stream_id       TEXT NOT NULL,
    version         INTEGER NOT NULL,
    event_type      TEXT NOT NULL,
    payload         JSONB NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (stream_id, version)
);

-- Inventory database (same structure)
CREATE TABLE inventory.event_log (...);

-- Shipping database (same structure)
CREATE TABLE shipping.event_log (...);
```

**Pros:**
- Simple, each context is self-contained
- No cross-database coordination
- Context can be moved, scaled, backed up independently

**Cons:**
- No global ordering across contexts
- Full system replay requires aggregating from all contexts

### 6.2 Option B: Dedicated Global Log Database

Keep a separate database as the canonical event log:

```
┌───────────────────────────────────────────────────────────────────────────┐
│                        Global Event Log Database                          │
│                                                                           │
│  global.event_log                                                         │
│  ├── id: BIGSERIAL (global ordering)                                      │
│  ├── context: TEXT                                                        │
│  ├── stream_id: TEXT                                                      │
│  ├── version: INTEGER                                                     │
│  ├── event_type: TEXT                                                     │
│  ├── payload: JSONB                                                       │
│  └── created_at: TIMESTAMPTZ                                              │
│                                                                           │
└───────────────────────────────────────────────────────────────────────────┘
                                    │
                    CDC / Logical Replication
                                    │
          ┌─────────────────────────┼─────────────────────────┐
          ▼                         ▼                         ▼
   ┌──────────────┐         ┌──────────────┐         ┌──────────────┐
   │   Sales DB   │         │ Inventory DB │         │ Shipping DB  │
   │              │         │              │         │              │
   │ sales.events │         │inventory.evts│         │shipping.evts │
   │ (filtered    │         │ (filtered    │         │ (filtered    │
   │  replica)    │         │  replica)    │         │  replica)    │
   └──────────────┘         └──────────────┘         └──────────────┘
```

**Implementation with logical replication:**
```sql
-- On global log database: Create publication
CREATE PUBLICATION sales_events
    FOR TABLE global.event_log
    WHERE (context = 'sales');

CREATE PUBLICATION inventory_events
    FOR TABLE global.event_log
    WHERE (context = 'inventory');

-- On sales database: Subscribe
CREATE SUBSCRIPTION sales_sub
    CONNECTION 'host=global-log.example.com dbname=events'
    PUBLICATION sales_events;
```

**Pros:**
- Global ordering preserved
- Single source of truth for compliance/audit
- Full system replay from one location

**Cons:**
- Additional infrastructure (global log database)
- Write amplification (global log + context database)
- Potential bottleneck at global log

### 6.3 Option C: Kafka as Global Log

Use Kafka/Redpanda as the distributed event log:

```
┌───────────────────────────────────────────────────────────────────────────┐
│                         Kafka / Redpanda                                  │
│                                                                           │
│  Topics:                                                                  │
│  ├── sales.orders (partitioned by order_id)                              │
│  ├── sales.customers                                                      │
│  ├── inventory.stock                                                      │
│  ├── inventory.reservations                                               │
│  ├── shipping.shipments                                                   │
│  └── integration.domain-events (cross-context)                           │
│                                                                           │
└───────────────────────────────────────────────────────────────────────────┘
                                    │
                    Kafka Connect / Custom consumers
                                    │
          ┌─────────────────────────┼─────────────────────────┐
          ▼                         ▼                         ▼
   ┌──────────────┐         ┌──────────────┐         ┌──────────────┐
   │   Sales DB   │         │ Inventory DB │         │ Shipping DB  │
   │  (projection │         │  (projection │         │  (projection │
   │   storage)   │         │   storage)   │         │   storage)   │
   └──────────────┘         └──────────────┘         └──────────────┘
```

**Pros:**
- Infinite retention, replay capability
- Built-in partitioning for parallelism
- Natural fit for event-driven architecture

**Cons:**
- Largest architectural change
- PostgreSQL becomes projection-only (not source of truth)
- Requires Kafka expertise

---

## 7. Migration Strategy

### 7.1 Zero-Downtime Migration Path

```
Phase 1: Dual-Write
┌─────────────┐     ┌─────────────┐
│  Monolith   │────▶│  New Sales  │
│  (primary)  │     │     DB      │
└─────────────┘     └─────────────┘
      │                    │
      └────── Both receive writes (shadow)

Phase 2: Read Migration
┌─────────────┐     ┌─────────────┐
│  Monolith   │     │  New Sales  │
│  (primary)  │     │ (reads here)│
└─────────────┘     └─────────────┘
      │                    ▲
      └────── Writes ──────┘

Phase 3: Write Migration
┌─────────────┐     ┌─────────────┐
│  Monolith   │     │  New Sales  │
│  (shadow)   │◀────│  (primary)  │
└─────────────┘     └─────────────┘

Phase 4: Cutover
                    ┌─────────────┐
                    │  New Sales  │
                    │     DB      │
                    └─────────────┘
```

### 7.2 Data Migration Script

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- MIGRATE SALES CONTEXT TO NEW DATABASE
-- ═══════════════════════════════════════════════════════════════════════════

-- Step 1: Create foreign data wrapper (on new database)
CREATE EXTENSION postgres_fdw;

CREATE SERVER monolith_server
    FOREIGN DATA WRAPPER postgres_fdw
    OPTIONS (host 'monolith.example.com', dbname 'app', port '5432');

CREATE USER MAPPING FOR CURRENT_USER
    SERVER monolith_server
    OPTIONS (user 'migrator', password 'secret');

-- Step 2: Import sales schema
IMPORT FOREIGN SCHEMA sales
    FROM SERVER monolith_server
    INTO sales_import;

-- Step 3: Copy events
INSERT INTO sales.event_log
SELECT * FROM sales_import.events;

-- Step 4: Copy projections
INSERT INTO sales.orders_projection
SELECT * FROM sales_import.orders_projection;

-- Step 5: Verify counts match
SELECT
    (SELECT COUNT(*) FROM sales.event_log) AS new_count,
    (SELECT COUNT(*) FROM sales_import.events) AS old_count;
```

---

## 8. Monitoring Distributed System

### 8.1 Metrics to Track

```rust
// ═══════════════════════════════════════════════════════════════════════════
// KEY METRICS FOR DISTRIBUTED POSTGRESQL
// ═══════════════════════════════════════════════════════════════════════════

/// Per-context metrics
struct ContextMetrics {
    /// Write latency (p50, p95, p99)
    write_latency_ms: Histogram,

    /// Read latency
    read_latency_ms: Histogram,

    /// Events per second
    events_per_second: Counter,

    /// Replication lag (for replicas)
    replication_lag_ms: Gauge,

    /// Connection pool utilization
    pool_utilization_percent: Gauge,

    /// Outbox queue depth
    outbox_pending_count: Gauge,
}

/// Cross-context metrics
struct IntegrationMetrics {
    /// Kafka consumer lag
    consumer_lag_messages: Gauge,

    /// Integration event processing time
    integration_latency_ms: Histogram,

    /// Failed event deliveries
    delivery_failures: Counter,
}
```

### 8.2 Health Check Aggregation

```rust
// ═══════════════════════════════════════════════════════════════════════════
// AGGREGATE HEALTH ACROSS ALL CONTEXTS
// ═══════════════════════════════════════════════════════════════════════════

use axum::{extract::State, Json};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct SystemHealth {
    pub status: HealthStatus,
    pub contexts: HashMap<String, ContextHealth>,
    pub integration: IntegrationHealth,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextHealth {
    pub primary: bool,
    pub replication_lag_ms: Option<u64>,
    pub outbox_depth: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegrationHealth {
    pub kafka: bool,
}

/// Combined state for health checks.
/// Note: Axum allows only one State extractor per handler.
#[derive(Clone)]
pub struct HealthState {
    pub context_pools: ContextPools,
    pub kafka: KafkaClient,
}

impl FromRef<AppState> for HealthState {
    fn from_ref(state: &AppState) -> Self {
        Self {
            context_pools: state.context_pools.clone(),
            kafka: state.kafka.clone(),
        }
    }
}

pub async fn system_health_check(
    State(health): State<HealthState>,
) -> Json<SystemHealth> {
    let mut contexts = HashMap::new();

    for (name, pool) in health.context_pools.iter() {
        let primary_ok = check_database(pool).await;
        let lag = get_replication_lag(pool).await;
        let outbox = get_outbox_depth(pool).await;

        contexts.insert(name.clone(), ContextHealth {
            primary: primary_ok,
            replication_lag_ms: lag,
            outbox_depth: outbox,
        });
    }

    let kafka_ok = health.kafka.health_check().await;
    let all_contexts_ok = contexts.values().all(|c| c.primary);

    Json(SystemHealth {
        status: match (all_contexts_ok, kafka_ok) {
            (true, true) => HealthStatus::Healthy,
            (true, false) | (false, true) => HealthStatus::Degraded,
            (false, false) => HealthStatus::Unhealthy,
        },
        contexts,
        integration: IntegrationHealth { kafka: kafka_ok },
    })
}

// Helper functions (implementations depend on your setup)
async fn check_database(pool: &PgPool) -> bool {
    sqlx::query("SELECT 1").fetch_one(pool).await.is_ok()
}

async fn get_replication_lag(pool: &PgPool) -> Option<u64> {
    sqlx::query_scalar::<_, f64>(
        "SELECT EXTRACT(EPOCH FROM (now() - pg_last_xact_replay_timestamp())) * 1000"
    )
    .fetch_one(pool)
    .await
    .ok()
    .map(|ms| ms as u64)
}

async fn get_outbox_depth(pool: &PgPool) -> u64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM outbox.integration_events WHERE published_at IS NULL"
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0) as u64
}
```

---

## 9. Summary: Scaling Decision Matrix

| Need | Solution | Complexity | Downtime |
|------|----------|------------|----------|
| **More read capacity** | Add read replicas | Low | Zero |
| **Better read latency** | Add replicas in region | Low | Zero |
| **More connections** | PgBouncer | Low | Minutes |
| **More write capacity** | Split hot context | Medium | Hours (with planning) |
| **Storage growth** | Partition + archive | Low | Zero |
| **Team independence** | Split by ownership | Medium | Hours |
| **Global distribution** | Regional deployments | High | Days |

### 9.1 What Stays the Same

Regardless of scaling stage, these remain constant:

- **Business logic in PostgreSQL functions** (IMMUTABLE process, VOLATILE handle)
- **Event sourcing pattern** (append-only events, derived projections)
- **pg-gateway as thin shell** (protocol translation, identity propagation)
- **Error code mapping** (P0001→400, P0401→401, etc.)
- **Identity context** (`SET LOCAL app.user_id`, etc.)

### 9.2 The Key Insight

The bounded contexts architecture uses **schemas as soft boundaries**. When you need to scale:

1. Schemas become databases (soft → hard boundary)
2. `integration.domain_events` becomes Kafka topics
3. LISTEN/NOTIFY becomes Kafka consumers
4. Cross-context transactions become sagas

**The business logic doesn't change.** Only the infrastructure around it evolves.
