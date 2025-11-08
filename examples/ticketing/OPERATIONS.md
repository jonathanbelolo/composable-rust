# Operations Guide - Ticketing System

Complete operational playbook for managing the event ticketing system.

## 📋 Table of Contents

1. [First Time Setup](#first-time-setup)
2. [Daily Operations](#daily-operations)
3. [Data Management](#data-management)
4. [Monitoring](#monitoring)
5. [Troubleshooting](#troubleshooting)

---

## 🚀 First Time Setup

### Prerequisites Check
```bash
# 1. Verify Docker is running
docker --version
docker info

# 2. Verify Rust toolchain
rustc --version  # Should be 1.85.0 or newer

# 3. Clone/navigate to project
cd examples/ticketing
```

### Bootstrap Process

The bootstrap script handles everything:
```bash
./scripts/bootstrap.sh
```

**What it does:**
1. ✅ Checks Docker is running
2. ✅ Starts PostgreSQL container
3. ✅ Starts RedPanda 3-broker cluster
4. ✅ Starts RedPanda Console (web UI)
5. ✅ Waits for services to be healthy
6. ✅ Creates `ticketing` database
7. ✅ Migrations run automatically on first app start

**Expected output:**
```
🎫 Bootstrapping Ticketing System...

1️⃣  Checking Docker...
   ✓ Docker is running

2️⃣  Starting infrastructure (PostgreSQL + RedPanda)...
   ⏳ Waiting for PostgreSQL to be ready...
   ✓ PostgreSQL is ready
   ⏳ Waiting for RedPanda to be ready...
   ✓ RedPanda is ready

3️⃣  Creating database...
   ✓ Database 'ticketing' ready

4️⃣  Migrations will run automatically on first app start

5️⃣  Infrastructure Status:
   [Container status table]

✅ Bootstrap complete!
```

**First run:**
```bash
# Option 1: Run demo (recommended)
cargo run --bin demo

# Option 2: Run server
cargo run --bin server
```

---

## 🔄 Daily Operations

### Start Services
```bash
# If already bootstrapped, just start containers
docker compose up -d

# Check status
./scripts/status.sh
```

### Stop Services
```bash
# Stop containers (preserves data)
docker compose down

# OR use cleanup script (interactive)
./scripts/cleanup.sh
```

### View Status
```bash
./scripts/status.sh
```

### View Logs
```bash
# All services
docker compose logs -f

# Specific service
docker compose logs -f postgres
docker compose logs -f redpanda
docker compose logs -f console

# Application logs
RUST_LOG=debug cargo run --bin server
```

---

## 🗄️ Data Management

### Reset Data (Developers)

**Scenario:** You want to clear all events and start fresh, but keep containers running.

```bash
./scripts/reset.sh
```

**Confirmation prompt:**
```
⚠️  WARNING: This will delete ALL data!
   - All events in PostgreSQL
   - All snapshots
   - All RedPanda topics and messages

Continue? (y/N)
```

**What it does:**
1. Drops and recreates PostgreSQL `ticketing` database
2. Deletes all RedPanda topics
3. Leaves containers running and healthy
4. Ready for fresh `cargo run --bin demo`

### Backup Data

**PostgreSQL Backup:**
```bash
# Dump to file
docker compose exec postgres pg_dump -U postgres ticketing > backup_$(date +%Y%m%d).sql

# Restore from file
docker compose exec -T postgres psql -U postgres ticketing < backup_20250107.sql
```

**Event Store Backup (Critical):**
```bash
# Export events table
docker compose exec postgres pg_dump -U postgres -t events ticketing > events_backup.sql

# Export snapshots
docker compose exec postgres pg_dump -U postgres -t snapshots ticketing > snapshots_backup.sql
```

### Query Events Directly

```bash
# Connect to database
docker compose exec postgres psql -U postgres -d ticketing
```

**Useful queries:**
```sql
-- Count events
SELECT COUNT(*) FROM events;

-- Events by type
SELECT event_type, COUNT(*)
FROM events
GROUP BY event_type
ORDER BY count DESC;

-- Recent events
SELECT stream_id, version, event_type, created_at
FROM events
ORDER BY created_at DESC
LIMIT 10;

-- Events for specific stream
SELECT version, event_type, created_at
FROM events
WHERE stream_id = 'inventory-abc123'
ORDER BY version;

-- Events in time range
SELECT event_type, COUNT(*)
FROM events
WHERE created_at > NOW() - INTERVAL '1 hour'
GROUP BY event_type;

-- Exit
\q
```

---

## 📊 Monitoring

### Infrastructure Status

```bash
./scripts/status.sh
```

**What it shows:**
- Docker container status
- PostgreSQL health and database state
- Event and snapshot table counts
- RedPanda cluster health
- Topic list
- RedPanda Console availability

### RedPanda Console (Web UI)

**URL:** http://localhost:8080

**Features:**
- 📊 Topic list and configuration
- 📨 Live message streaming
- 🔍 Message search and inspection
- 📈 Consumer lag monitoring
- ⚙️ Cluster configuration
- 📊 Throughput metrics

### PostgreSQL Monitoring

```bash
# Connection count
docker compose exec postgres psql -U postgres -d ticketing -c \
  "SELECT count(*) FROM pg_stat_activity WHERE datname='ticketing';"

# Database size
docker compose exec postgres psql -U postgres -d ticketing -c \
  "SELECT pg_size_pretty(pg_database_size('ticketing'));"

# Table sizes
docker compose exec postgres psql -U postgres -d ticketing -c \
  "SELECT
     schemaname,
     tablename,
     pg_size_pretty(pg_total_relation_size(schemaname||'.'||tablename)) AS size
   FROM pg_tables
   WHERE schemaname = 'public'
   ORDER BY pg_total_relation_size(schemaname||'.'||tablename) DESC;"

# Index usage
docker compose exec postgres psql -U postgres -d ticketing -c \
  "SELECT
     schemaname,
     tablename,
     indexname,
     idx_scan
   FROM pg_stat_user_indexes
   ORDER BY idx_scan DESC;"
```

### RedPanda Monitoring

```bash
# Cluster info
docker compose exec redpanda-1 rpk cluster info

# Topic list
docker compose exec redpanda-1 rpk topic list

# Topic details
docker compose exec redpanda-1 rpk topic describe ticketing-inventory-events

# Consumer groups
docker compose exec redpanda-1 rpk group list

# Consumer lag
docker compose exec redpanda-1 rpk group describe ticketing-projections
```

---

## 🔧 Troubleshooting

### Problem: Bootstrap fails with "Docker not running"

**Symptom:**
```
❌ Docker is not running. Please start Docker Desktop.
```

**Solution:**
1. Open Docker Desktop application
2. Wait for Docker to fully start (whale icon stops animating)
3. Retry: `./scripts/bootstrap.sh`

---

### Problem: Port conflict (5432, 9092, 8080)

**Symptom:**
```
Error: Port 5432 is already allocated
```

**Solution:**
```bash
# Find what's using the port
lsof -i :5432    # PostgreSQL
lsof -i :9092    # RedPanda
lsof -i :8080    # Console

# Option 1: Stop the conflicting service
# (e.g., local PostgreSQL installation)

# Option 2: Change port in docker-compose.yml
# Edit: "5433:5432" instead of "5432:5432"
# Also update DATABASE_URL in .env
```

---

### Problem: PostgreSQL won't start

**Symptom:**
```
⏳ Waiting for PostgreSQL to be ready...
❌ PostgreSQL failed to start
```

**Diagnosis:**
```bash
# Check logs
docker compose logs postgres

# Check container status
docker compose ps postgres
```

**Common fixes:**
```bash
# Fix 1: Remove corrupted volumes
docker compose down -v
./scripts/bootstrap.sh

# Fix 2: Check disk space
df -h

# Fix 3: Reset Docker Desktop
# Docker Desktop → Troubleshoot → Reset to factory defaults
```

---

### Problem: RedPanda won't start

**Symptom:**
```
console_1  | Error: Cannot connect to Kafka brokers
```

**Diagnosis:**
```bash
# Check RedPanda logs
docker compose logs redpanda

# Check if port is listening
nc -zv localhost 9092
```

**Common fixes:**
```bash
# Reset RedPanda data
docker compose down
docker volume rm ticketing_redpanda_data || true
docker compose up -d

# Check health
docker compose exec redpanda rpk cluster health
```

---

### Problem: Database connection refused from app

**Symptom:**
```
Database error: connection refused
```

**Diagnosis:**
```bash
# 1. Check PostgreSQL is running
docker compose ps postgres

# 2. Check if database exists
docker compose exec postgres psql -U postgres -l | grep ticketing

# 3. Test connection
docker compose exec postgres psql -U postgres -d ticketing -c "SELECT 1;"

# 4. Check .env file
cat .env | grep DATABASE_URL
```

**Solution:**
```bash
# Ensure DATABASE_URL matches docker-compose.yml
# Default: postgres://postgres:postgres@localhost:5432/ticketing

# If database doesn't exist:
docker compose exec postgres psql -U postgres -c "CREATE DATABASE ticketing;"
```

---

### Problem: Migrations not running

**Symptom:**
```
table "events" does not exist
```

**Solution:**
```bash
# Migrations run automatically on app start
# They are located at: ../../migrations/ (from examples/ticketing/)

# Verify migrations exist:
ls ../../migrations/

# Expected:
# 001_create_events_table.sql
# 002_create_snapshots_table.sql

# Run app (triggers migrations):
cargo run --bin server

# Manual migration (if needed):
cd ../../migrations
sqlx migrate run --database-url postgres://postgres:postgres@localhost:5432/ticketing
```

---

### Problem: Events not appearing in projections

**Symptom:**
- Server logs show events published
- But projection queries return empty/stale data

**Diagnosis:**
```bash
# 1. Check events are in database
docker compose exec postgres psql -U postgres -d ticketing -c "SELECT COUNT(*) FROM events;"

# 2. Check RedPanda topics
docker compose exec redpanda rpk topic list | grep ticketing

# 3. Check if projections are subscribing
# Look for "Subscribed to event bus" in server logs
```

**Common causes:**
1. **App not started:** Projections only update when `cargo run --bin server` is running
2. **Topic mismatch:** Check topic names in .env match docker-compose.yml
3. **Subscription error:** Check server logs for event bus errors

---

### Problem: "Too many open files" error

**Symptom:**
```
Error: Too many open files (os error 24)
```

**Solution (macOS/Linux):**
```bash
# Check current limit
ulimit -n

# Increase limit temporarily
ulimit -n 10240

# Permanent fix (macOS):
sudo launchctl limit maxfiles 65536 200000

# Permanent fix (Linux):
# Edit /etc/security/limits.conf
* soft nofile 65536
* hard nofile 65536
```

---

### Problem: Slow startup or high CPU

**Symptom:**
- Docker containers use 100% CPU
- Startup takes > 2 minutes

**Common causes:**
1. **Resource limits:** Docker Desktop → Settings → Resources → Increase CPU/Memory
2. **Too many containers:** `docker ps -a | wc -l` (clean up old containers)
3. **Disk I/O:** Use SSD, or increase Docker disk image size

**Solutions:**
```bash
# Clean up unused Docker resources
docker system prune -a

# Restart Docker Desktop

# Allocate more resources:
# Docker Desktop → Settings → Resources
# Recommended: 4 CPUs, 8GB RAM
```

---

## 🧪 Testing the Full Stack

### End-to-End Test Checklist

```bash
# 1. Clean slate
./scripts/cleanup.sh  # Yes to remove volumes
./scripts/bootstrap.sh

# 2. Verify status
./scripts/status.sh
# Expected: All green checkmarks

# 3. Run demo
cargo run --bin demo
# Expected: Full workflow completes successfully

# 4. Verify data persisted
docker compose exec postgres psql -U postgres -d ticketing -c \
  "SELECT event_type, COUNT(*) FROM events GROUP BY event_type;"
# Expected: Multiple event types with counts

# 5. Check RedPanda Console
open http://localhost:8080
# Expected: See topics with messages

# 6. Test server
# Terminal 1:
cargo run --bin server

# Terminal 2:
./scripts/status.sh
# Expected: Events count increases as server processes

# 7. Test reset
./scripts/reset.sh
./scripts/status.sh
# Expected: Event count = 0

# 8. Cleanup
./scripts/cleanup.sh
```

---

## 📝 Quick Reference

### Scripts
```bash
./scripts/bootstrap.sh   # Fresh start (first time)
./scripts/status.sh      # Show current state
./scripts/reset.sh       # Clear data, keep containers
./scripts/cleanup.sh     # Stop and optionally remove
```

### Docker Compose
```bash
docker compose up -d          # Start services
docker compose down           # Stop services
docker compose down -v        # Stop + remove volumes
docker compose ps             # Container status
docker compose logs -f [svc]  # Follow logs
docker compose restart [svc]  # Restart service
```

### Database
```bash
# Connect
docker compose exec postgres psql -U postgres -d ticketing

# Quick query
docker compose exec postgres psql -U postgres -d ticketing -c "SELECT COUNT(*) FROM events;"

# Backup
docker compose exec postgres pg_dump -U postgres ticketing > backup.sql

# Restore
docker compose exec -T postgres psql -U postgres ticketing < backup.sql
```

### RedPanda
```bash
# Topics
docker compose exec redpanda rpk topic list

# Describe topic
docker compose exec redpanda rpk topic describe ticketing-inventory-events

# Consume messages
docker compose exec redpanda rpk topic consume ticketing-inventory-events

# Consumer groups
docker compose exec redpanda rpk group list
```

---

## 🎯 Best Practices

### Development Workflow
1. **Start of day:** `docker compose up -d && ./scripts/status.sh`
2. **Between tests:** `./scripts/reset.sh` (keeps containers warm)
3. **End of day:** `docker compose down` (or leave running)

### Before Committing Code
1. `./scripts/cleanup.sh` (Yes to remove volumes)
2. `./scripts/bootstrap.sh`
3. `cargo run --bin demo`
4. Verify full workflow completes

### Production Deployment
- Use managed PostgreSQL (AWS RDS, Google Cloud SQL)
- Use managed Kafka/RedPanda (Confluent Cloud, RedPanda Cloud)
- Add connection pooling
- Add retry logic with exponential backoff
- Add circuit breakers
- Monitor with Prometheus/Grafana
- Set up alerts for event store lag

---

For more help, see QUICKSTART.md or open an issue!
