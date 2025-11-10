# composable-rust-postgres

**PostgreSQL event store implementation for Composable Rust.**

## Overview

Production-ready PostgreSQL implementation of the `EventStore` trait with migrations, batch operations, and connection pooling.

## Installation

```toml
[dependencies]
composable-rust-postgres = { path = "../postgres" }
sqlx = { version = "0.8", features = ["runtime-tokio-native-tls", "postgres"] }
```

## Quick Start

```rust
use composable_rust_postgres::PostgresEventStore;
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create connection pool
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect("postgresql://user:pass@localhost/db").await?;

    // Run migrations
    let event_store = PostgresEventStore::new(pool).await?;

    // Use with Store
    let environment = OrderEnvironment {
        event_store,
        clock: SystemClock::new(),
    };

    Ok(())
}
```

## Features

- ✅ **Event persistence** - Append-only event log
- ✅ **Optimistic concurrency** - Version-based conflict detection
- ✅ **Batch operations** - Efficient bulk appends
- ✅ **Migrations** - sqlx::migrate!() integration
- ✅ **Connection pooling** - Production-ready with sqlx
- ✅ **Transaction support** - ACID guarantees

## Database Schema

```sql
CREATE TABLE events (
    id BIGSERIAL PRIMARY KEY,
    stream_id TEXT NOT NULL,
    version BIGINT NOT NULL,
    event_type TEXT NOT NULL,
    event_data BYTEA NOT NULL,
    metadata JSONB,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(stream_id, version)
);

CREATE INDEX idx_events_stream_id ON events(stream_id);
CREATE INDEX idx_events_timestamp ON events(timestamp);
```

## API

### `PostgresEventStore::new()`

```rust
pub async fn new(pool: PgPool) -> Result<Self>
```

Creates event store and runs migrations.

### `append_events()`

```rust
async fn append_events(
    &self,
    stream_id: &StreamId,
    events: Vec<SerializedEvent>,
    expected_version: Option<Version>,
) -> Result<Version>
```

Appends events with optimistic concurrency control.

### `append_batch()`

```rust
async fn append_batch(
    &self,
    batches: Vec<EventBatch>,
) -> Result<Vec<Version>>
```

Efficient batch append (10-100x faster for bulk operations).

## Configuration

### Connection Pool

```rust
let pool = PgPoolOptions::new()
    .max_connections(20)
    .min_connections(5)
    .max_lifetime(Duration::from_secs(30 * 60))
    .idle_timeout(Duration::from_secs(10 * 60))
    .connect(&database_url).await?;
```

### Environment Variables

```bash
DATABASE_URL=postgresql://user:password@localhost:5432/myapp
DATABASE_MAX_CONNECTIONS=20
DATABASE_MIN_CONNECTIONS=5
```

## Further Reading

- [Production Database Guide](../docs/production-database.md) - Migrations, backups, monitoring
- [Database Setup](../docs/database-setup.md) - Local development setup
- [EventStore Trait](../core/README.md#event_store---eventstore-trait) - Core abstraction

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
