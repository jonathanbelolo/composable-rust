# Database Setup Guide

This guide covers setting up PostgreSQL for event sourcing with Composable Rust.

## Overview

Composable Rust uses PostgreSQL as its event store for production deployments. The framework provides:

- **PostgresEventStore**: Production-ready event store implementation
- **Migrations**: SQL schema for events and snapshots tables
- **Optimistic Concurrency**: Version-based conflict detection
- **Snapshot Support**: Performance optimization for long event streams

## Prerequisites

- PostgreSQL 12+ (PostgreSQL 16 recommended)
- Rust 1.85.0+ (Edition 2024)
- sqlx-cli for running migrations

## Local Development Setup

### 1. Install PostgreSQL

**macOS (Homebrew):**
```bash
brew install postgresql@16
brew services start postgresql@16
```

**Ubuntu/Debian:**
```bash
sudo apt-get install postgresql-16
sudo systemctl start postgresql
```

**Docker:**
```bash
docker run -d \
  --name composable-rust-postgres \
  -e POSTGRES_PASSWORD=postgres \
  -e POSTGRES_USER=postgres \
  -e POSTGRES_DB=composable_rust \
  -p 5432:5432 \
  postgres:16
```

### 2. Create Database

```bash
# Connect to PostgreSQL
psql -U postgres

# Create database
CREATE DATABASE composable_rust;

# Exit psql
\q
```

### 3. Install sqlx-cli

```bash
cargo install sqlx-cli --no-default-features --features postgres
```

### 4. Set Database URL

Create a `.env` file in your project root:

```bash
DATABASE_URL=postgres://postgres:postgres@localhost/composable_rust
```

### 5. Run Migrations

```bash
sqlx migrate run --source migrations
```

This creates two tables:
- `events`: Immutable append-only event log
- `snapshots`: Aggregate state snapshots for performance

### 6. Verify Setup

```bash
psql -U postgres -d composable_rust -c "\dt"
```

You should see:
```
           List of relations
 Schema |   Name    | Type  |  Owner
--------+-----------+-------+----------
 public | events    | table | postgres
 public | snapshots | table | postgres
```

## Database Schema

### Events Table

```sql
CREATE TABLE events (
    stream_id TEXT NOT NULL,           -- Aggregate ID (e.g., "order-123")
    version BIGINT NOT NULL,            -- Event version (for optimistic concurrency)
    event_type TEXT NOT NULL,           -- Event type name for deserialization
    event_data BYTEA NOT NULL,          -- Bincode-serialized event payload
    metadata JSONB,                     -- Optional metadata (correlation IDs, etc.)
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (stream_id, version)
);

CREATE INDEX idx_events_created ON events(created_at);
CREATE INDEX idx_events_type ON events(event_type);
```

**Key Design Decisions:**

- **PRIMARY KEY (stream_id, version)**: Enforces optimistic concurrency at database level
- **BYTEA for event_data**: Stores bincode-serialized events (5-10x faster than JSON)
- **JSONB for metadata**: Human-readable metadata for debugging/auditing
- **Indexes**: Support common queries (time-based, event-type filtering)

### Snapshots Table

```sql
CREATE TABLE snapshots (
    stream_id TEXT PRIMARY KEY,         -- Aggregate ID (same as events.stream_id)
    version BIGINT NOT NULL,            -- Event version at snapshot
    state_data BYTEA NOT NULL,          -- Bincode-serialized aggregate state
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

**Snapshot Strategy:**

- One snapshot per stream (latest only)
- UPSERT pattern: `ON CONFLICT DO UPDATE`
- Typical threshold: Create snapshot every 100 events
- Load snapshot + replay events since snapshot for fast state reconstruction

## Using PostgresEventStore in Your Application

### Basic Usage

```rust
use composable_rust_postgres::PostgresEventStore;
use composable_rust_core::event_store::EventStore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to database
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    let event_store = PostgresEventStore::new(&database_url).await?;

    // Use event store with your aggregates
    // ...

    Ok(())
}
```

### Custom Connection Pool

```rust
use sqlx::postgres::PgPoolOptions;
use composable_rust_postgres::PostgresEventStore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create custom pool
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&std::env::var("DATABASE_URL")?)
        .await?;

    let event_store = PostgresEventStore::from_pool(pool);

    Ok(())
}
```

### With Dependency Injection

```rust
use composable_rust_postgres::PostgresEventStore;
use std::sync::Arc;

struct MyEnvironment {
    event_store: Arc<dyn composable_rust_core::event_store::EventStore>,
    // ... other dependencies
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_store = Arc::new(
        PostgresEventStore::new(&std::env::var("DATABASE_URL")?).await?
    );

    let env = MyEnvironment {
        event_store,
        // ...
    };

    Ok(())
}
```

## Connection Strings

### Local Development
```
postgres://postgres:postgres@localhost/composable_rust
```

### Production (AWS RDS)
```
postgres://username:password@mydb.123456789012.us-east-1.rds.amazonaws.com:5432/composable_rust
```

### Production (Heroku)
```
postgres://user:password@ec2-xxx.compute-1.amazonaws.com:5432/database?sslmode=require
```

### Connection String Options

```
postgres://user:password@host:5432/database?sslmode=require&connect_timeout=10
```

Common options:
- `sslmode=require`: Enforce SSL connection
- `connect_timeout=10`: Connection timeout in seconds
- `application_name=my-app`: Identify connections in pg_stat_activity

## Testing

### Integration Tests with Testcontainers

Integration tests use testcontainers to spin up temporary PostgreSQL instances:

```bash
# Requires Docker running
cargo test -p composable-rust-postgres --test integration_tests
```

**Note**: Docker must be running for these tests to pass.

### Unit Tests (No Database Required)

Use `InMemoryEventStore` for fast unit tests:

```rust
use composable_rust_testing::mocks::InMemoryEventStore;

#[tokio::test]
async fn test_my_aggregate() {
    let event_store = InMemoryEventStore::new();
    // Test your reducer logic...
}
```

## Production Configuration

### Connection Pooling

Recommended settings for production:

```rust
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

let pool = PgPoolOptions::new()
    .max_connections(20)           // Adjust based on load
    .min_connections(5)             // Keep warm connections
    .acquire_timeout(Duration::from_secs(30))
    .idle_timeout(Duration::from_secs(600))
    .max_lifetime(Duration::from_secs(1800))
    .connect(&database_url)
    .await?;
```

### Database Tuning

**PostgreSQL Configuration** (`postgresql.conf`):

```ini
# Memory
shared_buffers = 256MB          # 25% of RAM
effective_cache_size = 1GB      # 50-75% of RAM
work_mem = 16MB                  # Per-operation memory

# Write Performance
wal_buffers = 16MB
checkpoint_completion_target = 0.9
max_wal_size = 1GB

# Connections
max_connections = 100
```

### Monitoring Queries

**Check Event Store Size:**
```sql
SELECT
    pg_size_pretty(pg_total_relation_size('events')) as events_size,
    pg_size_pretty(pg_total_relation_size('snapshots')) as snapshots_size;
```

**Check Event Counts by Stream:**
```sql
SELECT stream_id, COUNT(*) as event_count
FROM events
GROUP BY stream_id
ORDER BY event_count DESC
LIMIT 10;
```

**Check Snapshot Coverage:**
```sql
SELECT
    e.stream_id,
    COUNT(*) as total_events,
    s.version as snapshot_version,
    COUNT(*) - COALESCE(s.version, 0) as events_since_snapshot
FROM events e
LEFT JOIN snapshots s ON e.stream_id = s.stream_id
GROUP BY e.stream_id, s.version
HAVING COUNT(*) - COALESCE(s.version, 0) > 100
ORDER BY events_since_snapshot DESC;
```

## Backup and Restore

### Backup

```bash
# Full database backup
pg_dump -U postgres composable_rust > backup.sql

# Events table only
pg_dump -U postgres -t events composable_rust > events_backup.sql

# Compressed backup
pg_dump -U postgres composable_rust | gzip > backup.sql.gz
```

### Restore

```bash
# From SQL file
psql -U postgres composable_rust < backup.sql

# From compressed backup
gunzip -c backup.sql.gz | psql -U postgres composable_rust
```

### Continuous Backup (WAL Archiving)

For production, enable PostgreSQL WAL archiving for point-in-time recovery:

```ini
# postgresql.conf
wal_level = replica
archive_mode = on
archive_command = 'cp %p /path/to/archive/%f'
```

## Troubleshooting

### Connection Issues

**Error: "password authentication failed"**
```bash
# Check pg_hba.conf allows password authentication
sudo vim /etc/postgresql/16/main/pg_hba.conf
# Change "peer" to "md5" for local connections
```

**Error: "too many connections"**
```sql
-- Check current connections
SELECT count(*) FROM pg_stat_activity;

-- Increase max_connections in postgresql.conf
ALTER SYSTEM SET max_connections = 200;
SELECT pg_reload_conf();
```

### Migration Issues

**Error: "relation already exists"**
```bash
# Drop tables and re-run migrations
sqlx migrate revert --source migrations
sqlx migrate run --source migrations
```

### Performance Issues

**Slow event loading:**
```sql
-- Analyze tables
ANALYZE events;
ANALYZE snapshots;

-- Check query plans
EXPLAIN ANALYZE
SELECT * FROM events
WHERE stream_id = 'order-123'
ORDER BY version;
```

**Create missing indexes:**
```sql
-- If you frequently query by event type
CREATE INDEX IF NOT EXISTS idx_events_stream_type
ON events(stream_id, event_type);
```

## Next Steps

- [Getting Started Guide](./getting-started.md) - Learn the basics
- [Order Processing Example](../examples/order-processing/) - See it in action
- [Performance Tuning](./performance.md) - Optimize for production

## Strategic Notes

**Why PostgreSQL?**

1. **Vendor Independence**: Open source, ubiquitous, zero lock-in
2. **Cost Control**: Free infrastructure, no per-event pricing
3. **Full Control**: Optimize schema and queries for exact needs
4. **Client Flexibility**: Every client can use standard Postgres (managed or self-hosted)
5. **AI Agent Compatibility**: Standard SQL that AI agents can optimize

**Why NOT EventStoreDB/Kurrent?**

- Vendor lock-in risk with proprietary licenses
- If deployed to 100s of clients, all are hostage to one vendor
- Migration nightmare with years of event history across all clients
- With Postgres: clients choose their infrastructure, can swap vendors
