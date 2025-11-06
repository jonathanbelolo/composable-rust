# Production Database Setup Guide

This guide covers production deployment of the PostgreSQL event store, including migrations, connection pooling, backup/restore procedures, and monitoring.

---

## Table of Contents

1. [Database Migrations](#database-migrations)
2. [Connection Pooling](#connection-pooling)
3. [Backup and Restore](#backup-and-restore)
4. [Performance Tuning](#performance-tuning)
5. [Monitoring](#monitoring)
6. [Disaster Recovery](#disaster-recovery)

---

## Database Migrations

### Overview

The framework uses `sqlx::migrate!()` to manage database schema migrations. Migrations are:

- **Versioned**: Each migration has a sequential number (001, 002, etc.)
- **Idempotent**: Safe to run multiple times
- **Transactional**: Each migration runs in a transaction (rollback on failure)
- **Embedded**: Compiled into the binary at build time

### Running Migrations

#### Option 1: Using the Helper Function

For initialization scripts and deployment automation:

```rust
use composable_rust_postgres::run_migrations;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Run migrations during application startup
    run_migrations("postgres://user:pass@localhost/mydb").await?;

    println!("Database ready!");
    Ok(())
}
```

#### Option 2: Using the EventStore Method

When you already have an event store instance:

```rust
use composable_rust_postgres::PostgresEventStore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = PostgresEventStore::new("postgres://user:pass@localhost/mydb").await?;

    // Run migrations
    store.run_migrations().await?;

    // Now use the store
    Ok(())
}
```

#### Option 3: Using sqlx CLI (Development)

For local development, you can use the sqlx command-line tool:

```bash
# Install sqlx-cli
cargo install sqlx-cli --no-default-features --features postgres

# Run migrations
sqlx migrate run --database-url postgres://localhost/mydb

# Revert last migration (if needed)
sqlx migrate revert --database-url postgres://localhost/mydb
```

### Creating New Migrations

1. Create a new SQL file in `migrations/` with the next sequential number:

```bash
migrations/003_add_user_context.sql
```

2. Write idempotent SQL using `IF NOT EXISTS`:

```sql
-- Add user_context column to events table
ALTER TABLE events
ADD COLUMN IF NOT EXISTS user_context JSONB;

-- Add index
CREATE INDEX IF NOT EXISTS idx_events_user_context
ON events USING GIN (user_context);
```

3. Test locally:

```bash
sqlx migrate run --database-url postgres://localhost/testdb
```

4. The migration will be automatically embedded on next build.

### Migration Best Practices

- ✅ **Always use `IF NOT EXISTS`** for idempotency
- ✅ **One logical change per migration** (easier to debug/rollback)
- ✅ **Test migrations on a copy of production data**
- ✅ **Use transactions** (migrations are transactional by default)
- ✅ **Document breaking changes** in migration comments
- ❌ **Never edit existing migrations** (create a new migration instead)
- ❌ **Never delete migrations** (breaks version tracking)

---

## Connection Pooling

### Default Configuration

The `PostgresEventStore::new()` method creates a connection pool with conservative defaults:

```rust
PgPoolOptions::new()
    .max_connections(5)      // Maximum 5 connections
    .connect(database_url)
    .await
```

### Custom Pool Configuration

For production, customize the pool based on your workload:

```rust
use composable_rust_postgres::PostgresEventStore;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = PgPoolOptions::new()
        // Connection limits
        .max_connections(20)                  // Max concurrent connections
        .min_connections(5)                   // Keep 5 connections warm

        // Timeouts
        .acquire_timeout(Duration::from_secs(10))  // Wait up to 10s for connection
        .idle_timeout(Duration::from_secs(600))    // Close idle connections after 10min
        .max_lifetime(Duration::from_secs(1800))   // Recycle connections after 30min

        // Health checks
        .test_before_acquire(true)            // Test connections before use

        .connect("postgres://user:pass@localhost/mydb")
        .await?;

    let store = PostgresEventStore::from_pool(pool);

    Ok(())
}
```

### Sizing Guidelines

#### Calculate Required Connections

**Formula**: `max_connections = (expected_concurrent_requests × connection_time) / request_time + buffer`

**Example**:
- Expected load: 1000 requests/sec
- Average request time: 50ms
- Average connection hold time: 10ms
- Calculation: `(1000 × 0.010) / 0.050 + 5 = 25 connections`

#### General Recommendations

| Environment | Max Connections | Notes |
|-------------|-----------------|-------|
| **Development** | 5 | Minimal overhead |
| **Staging** | 10-20 | Simulate production |
| **Production (Low)** | 20-50 | < 100 req/sec |
| **Production (Medium)** | 50-100 | 100-1000 req/sec |
| **Production (High)** | 100-200 | > 1000 req/sec |

#### PostgreSQL Limits

PostgreSQL has a global connection limit (default 100). Configure in `postgresql.conf`:

```ini
max_connections = 200
```

**Important**: Reserve connections for:
- Background tasks (10-20)
- Monitoring/admin tools (5-10)
- Connection poolers like PgBouncer (if used)

### Connection Pool Monitoring

Monitor pool health using sqlx metrics:

```rust
use sqlx::postgres::PgPoolOptions;

let pool = PgPoolOptions::new()
    .max_connections(20)
    .connect("postgres://localhost/mydb")
    .await?;

// Check pool status
let size = pool.size();           // Current number of connections
let idle = pool.num_idle();       // Idle connections available

println!("Pool: {size} connections ({idle} idle)");
```

**Metrics to monitor**:
- `pool.size()`: Should stay below max_connections
- `pool.num_idle()`: Should be > 0 (connections available)
- Connection acquire time: Should be < 100ms
- Connection errors: Should be near 0

---

## Backup and Restore

### Backup Strategies

#### 1. Full Database Backup (pg_dump)

**Use case**: Complete database snapshot

```bash
# Backup entire database
pg_dump -h localhost -U postgres -d mydb \
    --format=custom \
    --file=backup_$(date +%Y%m%d_%H%M%S).dump

# Backup with compression
pg_dump -h localhost -U postgres -d mydb \
    --format=custom \
    --compress=9 \
    --file=backup_$(date +%Y%m%d_%H%M%S).dump.gz
```

**Advantages**:
- Complete snapshot
- Includes all tables, indexes, constraints
- Can restore to empty database

**Disadvantages**:
- Larger file size
- Longer restore time
- Includes non-event data (if any)

#### 2. Events Table Only Backup

**Use case**: Event sourcing systems where events are the source of truth

```bash
# Backup only events table
pg_dump -h localhost -U postgres -d mydb \
    --table=events \
    --format=custom \
    --file=events_backup_$(date +%Y%m%d_%H%M%S).dump

# Backup events + snapshots
pg_dump -h localhost -U postgres -d mydb \
    --table=events --table=snapshots \
    --format=custom \
    --file=events_snapshots_backup_$(date +%Y%m%d_%H%M%S).dump
```

**Advantages**:
- Smaller file size
- Faster backup/restore
- Focused on event data

**Disadvantages**:
- Doesn't include read models or projections
- Must rebuild snapshots after restore

#### 3. Continuous Archiving (WAL)

**Use case**: Point-in-time recovery (PITR)

```bash
# Enable WAL archiving in postgresql.conf
wal_level = replica
archive_mode = on
archive_command = 'cp %p /var/lib/postgresql/wal_archive/%f'

# Take base backup
pg_basebackup -h localhost -U postgres \
    --format=tar \
    --gzip \
    --pgdata=/var/lib/postgresql/base_backup
```

**Advantages**:
- Continuous protection
- Point-in-time recovery
- Minimal downtime

**Disadvantages**:
- Complex setup
- Requires disk space for WAL archives
- Recovery requires base backup + WAL replay

### Restore Procedures

#### Restore from pg_dump

```bash
# Restore full database (drops existing database)
pg_restore -h localhost -U postgres \
    --clean \
    --create \
    --dbname=postgres \
    backup_20250106_143022.dump

# Restore to existing database
pg_restore -h localhost -U postgres \
    --dbname=mydb \
    --no-owner \
    --no-acl \
    backup_20250106_143022.dump
```

#### Restore Events Table Only

```bash
# Drop existing events (if needed)
psql -h localhost -U postgres -d mydb -c "TRUNCATE TABLE events CASCADE;"

# Restore events
pg_restore -h localhost -U postgres \
    --dbname=mydb \
    --table=events \
    events_backup_20250106_143022.dump
```

#### Point-in-Time Recovery (PITR)

```bash
# 1. Stop PostgreSQL
systemctl stop postgresql

# 2. Restore base backup
rm -rf /var/lib/postgresql/data
tar -xzf base_backup.tar.gz -C /var/lib/postgresql/data

# 3. Create recovery.conf
cat > /var/lib/postgresql/data/recovery.conf <<EOF
restore_command = 'cp /var/lib/postgresql/wal_archive/%f %p'
recovery_target_time = '2025-01-06 14:30:00'
EOF

# 4. Start PostgreSQL (will replay WAL to target time)
systemctl start postgresql

# 5. Promote to normal operation once recovered
psql -c "SELECT pg_wal_replay_resume();"
```

### Backup Automation

#### Daily Backup Script

```bash
#!/bin/bash
# daily-backup.sh

set -e

BACKUP_DIR="/backups/postgres"
DATE=$(date +%Y%m%d_%H%M%S)
DB_NAME="mydb"
RETENTION_DAYS=30

# Create backup
pg_dump -h localhost -U postgres -d $DB_NAME \
    --format=custom \
    --compress=9 \
    --file=$BACKUP_DIR/backup_$DATE.dump

# Upload to S3 (optional)
aws s3 cp $BACKUP_DIR/backup_$DATE.dump \
    s3://my-backups/postgres/backup_$DATE.dump

# Delete old backups
find $BACKUP_DIR -name "backup_*.dump" -mtime +$RETENTION_DAYS -delete

echo "Backup completed: backup_$DATE.dump"
```

#### Cron Schedule

```cron
# Daily backup at 2 AM
0 2 * * * /usr/local/bin/daily-backup.sh >> /var/log/postgres-backup.log 2>&1

# Weekly full backup on Sunday at 3 AM
0 3 * * 0 /usr/local/bin/weekly-full-backup.sh >> /var/log/postgres-backup.log 2>&1
```

### Testing Backups

**Critical**: Regularly test your backup restoration process!

```bash
# Test restore procedure (monthly)
# 1. Create test database
createdb -h localhost -U postgres mydb_test

# 2. Restore backup
pg_restore -h localhost -U postgres \
    --dbname=mydb_test \
    backup_latest.dump

# 3. Verify data
psql -h localhost -U postgres -d mydb_test -c \
    "SELECT COUNT(*) FROM events;"

# 4. Drop test database
dropdb -h localhost -U postgres mydb_test
```

---

## Performance Tuning

### PostgreSQL Configuration

Optimize `postgresql.conf` for event sourcing workloads:

```ini
# Memory
shared_buffers = 4GB                  # 25% of RAM
effective_cache_size = 12GB           # 75% of RAM
work_mem = 64MB                       # Per-operation memory

# Write performance
wal_buffers = 16MB
checkpoint_completion_target = 0.9
max_wal_size = 4GB
min_wal_size = 1GB

# Query performance
random_page_cost = 1.1               # For SSDs (default 4.0 is for HDDs)
effective_io_concurrency = 200       # For SSDs

# Logging
log_min_duration_statement = 1000    # Log queries > 1s
log_checkpoints = on
log_connections = on
log_disconnections = on
```

### Index Optimization

The default schema includes these indexes:

```sql
-- Primary key (automatic index)
PRIMARY KEY (stream_id, version)

-- Query by creation time
CREATE INDEX idx_events_created ON events(created_at);

-- Query by event type
CREATE INDEX idx_events_type ON events(event_type);
```

**Add custom indexes** based on your query patterns:

```sql
-- Query events by metadata fields
CREATE INDEX idx_events_user_id
ON events ((metadata->>'user_id'));

-- Query events by correlation ID
CREATE INDEX idx_events_correlation
ON events ((metadata->>'correlation_id'));

-- Partial index for recent events
CREATE INDEX idx_events_recent
ON events(created_at)
WHERE created_at > NOW() - INTERVAL '30 days';
```

### Table Maintenance

```sql
-- Vacuum regularly (reclaim space, update statistics)
VACUUM ANALYZE events;

-- Full vacuum (locks table, use during maintenance window)
VACUUM FULL events;

-- Reindex if indexes are bloated
REINDEX TABLE events;
```

**Automate with autovacuum** (`postgresql.conf`):

```ini
autovacuum = on
autovacuum_vacuum_scale_factor = 0.1
autovacuum_analyze_scale_factor = 0.05
```

---

## Monitoring

### Key Metrics

Monitor these PostgreSQL metrics:

```sql
-- Active connections
SELECT count(*) FROM pg_stat_activity WHERE state = 'active';

-- Database size
SELECT pg_size_pretty(pg_database_size('mydb'));

-- Table sizes
SELECT
    schemaname,
    tablename,
    pg_size_pretty(pg_total_relation_size(schemaname||'.'||tablename)) AS size
FROM pg_tables
WHERE schemaname NOT IN ('pg_catalog', 'information_schema')
ORDER BY pg_total_relation_size(schemaname||'.'||tablename) DESC;

-- Index usage
SELECT
    schemaname,
    tablename,
    indexname,
    idx_scan,
    idx_tup_read,
    idx_tup_fetch
FROM pg_stat_user_indexes
ORDER BY idx_scan DESC;

-- Slow queries (requires log_min_duration_statement > 0)
SELECT
    query,
    calls,
    total_exec_time,
    mean_exec_time,
    max_exec_time
FROM pg_stat_statements
ORDER BY mean_exec_time DESC
LIMIT 10;
```

### Health Check Queries

```rust
use composable_rust_postgres::PostgresEventStore;

async fn health_check(store: &PostgresEventStore) -> Result<(), Box<dyn std::error::Error>> {
    let pool = store.pool();

    // Simple connectivity check
    sqlx::query("SELECT 1")
        .execute(pool)
        .await?;

    // Check events table exists
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events")
        .fetch_one(pool)
        .await?;

    println!("Health check passed: {} events in store", row.0);
    Ok(())
}
```

### Alerting Thresholds

Set up alerts for:

- **Connection pool exhaustion**: `pool.num_idle() == 0` for > 1 minute
- **High query latency**: p99 > 1 second
- **Disk space**: < 20% free
- **Replication lag**: > 60 seconds (if using streaming replication)
- **Failed backups**: Last successful backup > 24 hours ago

---

## Disaster Recovery

### Recovery Time Objective (RTO)

**Target**: < 4 hours to full recovery

**Steps**:
1. Provision new PostgreSQL instance (30 min)
2. Restore from backup (1-2 hours, depending on size)
3. Verify data integrity (30 min)
4. Reconfigure applications (30 min)

### Recovery Point Objective (RPO)

**Target**: < 1 hour of data loss

**Strategy**:
- **Tier 1** (No data loss): Continuous WAL archiving + PITR
- **Tier 2** (< 1 hour): Hourly backups
- **Tier 3** (< 24 hours): Daily backups

### Runbook: Database Failure

1. **Assess the situation**:
   - Check PostgreSQL logs: `/var/log/postgresql/`
   - Check disk space: `df -h`
   - Check system resources: `top`, `iostat`

2. **Attempt quick recovery**:
   ```bash
   # Restart PostgreSQL
   systemctl restart postgresql

   # Check status
   systemctl status postgresql
   psql -c "SELECT 1"
   ```

3. **If restart fails, restore from backup**:
   ```bash
   # Stop application
   systemctl stop myapp

   # Restore latest backup
   pg_restore -h localhost -U postgres \
       --clean --create --dbname=postgres \
       /backups/postgres/backup_latest.dump

   # Run migrations (if needed)
   ./target/release/myapp migrate

   # Start application
   systemctl start myapp
   ```

4. **Verify recovery**:
   ```bash
   # Check event count
   psql -c "SELECT COUNT(*) FROM events;"

   # Check application health
   curl http://localhost:8080/health
   ```

5. **Post-mortem**:
   - Document what happened
   - Update runbook with learnings
   - Implement preventive measures

---

## Summary

### Checklist: Production Readiness

- [ ] Migrations automated in deployment pipeline
- [ ] Connection pool sized appropriately for load
- [ ] Daily automated backups configured
- [ ] Backup restoration tested monthly
- [ ] Monitoring and alerting set up
- [ ] Performance tuning applied (`postgresql.conf`)
- [ ] Disaster recovery runbook documented and tested
- [ ] Database access properly secured (SSL, firewall, auth)
- [ ] High availability configured (if needed: replication, failover)

### Quick Reference

```bash
# Run migrations
cargo run --release --bin myapp -- migrate

# Backup database
pg_dump -Fc mydb > backup.dump

# Restore database
pg_restore -d mydb backup.dump

# Check pool status
psql -c "SELECT count(*) FROM pg_stat_activity;"

# Monitor slow queries
psql -c "SELECT query, mean_exec_time FROM pg_stat_statements ORDER BY mean_exec_time DESC LIMIT 5;"
```

---

**Next Steps**: See `observability.md` for comprehensive monitoring and tracing setup.
