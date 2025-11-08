# 🎫 Ticketing System - Quick Start Guide

This guide will have you running the complete event-sourced ticketing system in **under 5 minutes**.

## Prerequisites

- **Docker Desktop** running
- **Rust 1.85+** installed
- **15 minutes** for the demo walkthrough (or 2 minutes just to start the server)

## 🚀 Option 1: Run the Demo (Recommended First Time)

This is the **fastest way** to see everything working:

```bash
# 1. Bootstrap the infrastructure (PostgreSQL + RedPanda)
./scripts/bootstrap.sh

# 2. Run the interactive demo
cargo run --bin demo

# 3. Watch the magic happen! 🎭
#    - Event creation
#    - Inventory initialization
#    - Ticket reservation
#    - Payment processing
#    - Real-time projections
```

**What you'll see:**
```
🎫 ============================================
   Ticketing System - Live Demo
============================================

📋 Demo Scenario: Concert Ticket Purchase
   Event: Summer Music Festival 2025
   Section: General Admission
   Capacity: 100 seats

1️⃣  Creating event and initializing inventory...
   ✓ Event created: abc123...
   ✓ Inventory initialized: 100 seats available

2️⃣  Customer initiating reservation...
   ✓ Reservation initiated (5-minute timer started)

... (full workflow with real-time updates)

✨ Demo completed successfully!
```

## 🖥️ Option 2: Run the Server (Production Mode)

For long-running server process:

```bash
# 1. Bootstrap (if not already done)
./scripts/bootstrap.sh

# 2. Start the server
cargo run --bin server

# Server runs indefinitely, processing events and updating projections
```

The server will:
- ✅ Connect to PostgreSQL (event store)
- ✅ Connect to RedPanda (event bus)
- ✅ Subscribe projections to all event topics
- ✅ Update read models in real-time
- ✅ Log all activity with structured logging

## 📊 Monitor the System

### View Infrastructure Status
```bash
./scripts/status.sh
```

Output:
```
📊 Ticketing System Status
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
🐳 Containers:
   NAME                  STATUS    PORTS
   ticketing-postgres    Up        0.0.0.0:5432->5432/tcp
   ticketing-redpanda    Up        0.0.0.0:9092->9092/tcp
   ticketing-console     Up        0.0.0.0:8080->8080/tcp

🗄️  PostgreSQL:
   ✅ Status: Running
   ✅ Database 'ticketing' exists
   📊 Tables: 2/2 (events, snapshots)
   📝 Events stored: 42

🔴 RedPanda:
   ✅ Status: Running (3-broker cluster)
   📡 Topics: 3
```

### View RedPanda Console (Web UI)
Open http://localhost:8080 in your browser to see:
- 📊 All event topics
- 📨 Live messages flowing through
- 📈 Consumer lag and throughput
- 🔍 Message inspection

### View PostgreSQL Data
```bash
# Connect to database
docker compose exec postgres psql -U postgres -d ticketing

# Query events
SELECT stream_id, version, event_type, created_at FROM events ORDER BY created_at DESC LIMIT 10;

# Count events by type
SELECT event_type, COUNT(*) FROM events GROUP BY event_type;

# Exit
\q
```

### View Logs
```bash
# All services
docker compose logs -f

# Just PostgreSQL
docker compose logs -f postgres

# Just RedPanda
docker compose logs -f redpanda
```

## 🔄 Common Operations

### Reset Data (Keep Containers Running)
```bash
./scripts/reset.sh
```
- ⚠️ Deletes all events and topics
- ✅ Database and containers remain running
- ✅ Ready for fresh demo run

### Full Cleanup (Stop Everything)
```bash
./scripts/cleanup.sh
```
Options:
- Stop containers only (preserves data)
- Stop + remove volumes (complete teardown)

### Restart After Cleanup
```bash
./scripts/bootstrap.sh   # Fresh start
# OR
docker compose up -d     # Resume with existing data
```

## 📁 Project Structure

```
examples/ticketing/
├── src/
│   ├── aggregates/        # Business logic (inventory, reservation, payment)
│   ├── app/              # Application wiring (coordinator, services)
│   ├── bin/              # Executables
│   │   ├── server.rs     # Production server
│   │   └── demo.rs       # Interactive demo
│   ├── projections/      # Read models (available_seats, sales_analytics)
│   ├── config.rs         # Configuration management
│   ├── types.rs          # Domain types
│   └── lib.rs           # Library exports
├── scripts/              # Operational scripts
│   ├── bootstrap.sh      # Fresh start
│   ├── reset.sh          # Clear data
│   ├── cleanup.sh        # Stop/remove
│   └── status.sh         # Show status
├── docker-compose.yml    # Infrastructure definition
├── .env.example          # Configuration template
└── QUICKSTART.md        # This file
```

## 🎯 What's Happening Under the Hood?

### Event Flow
```
Command (e.g., ReserveSeats)
    ↓
Service.handle()
    ↓
1. Load state from PostgreSQL event store
2. Execute reducer (pure business logic)
3. Persist events to PostgreSQL (source of truth)
4. Publish events to RedPanda (distribution)
    ↓
RedPanda distributes to all subscribers
    ↓
Projections update in real-time
    ↓
Query models reflect latest state
```

### Architecture
```
┌─────────────┐
│   Client    │  (demo/server binaries)
└─────────────┘
       │
       ▼
┌─────────────────────────────────┐
│      TicketingApp               │
│  (Coordinator)                  │
│                                 │
│  ┌───────────┐  ┌────────────┐ │
│  │ Services  │  │Projections │ │
│  └───────────┘  └────────────┘ │
└─────────────────────────────────┘
       │           │
       ▼           ▼
┌─────────┐  ┌──────────┐
│PostgreSQL│  │ RedPanda │
│(Events)  │  │ (Bus)    │
└─────────┘  └──────────┘
```

## 🐛 Troubleshooting

### Docker not running
```
❌ Error: Docker is not running
```
**Fix:** Start Docker Desktop

### Port already in use
```
❌ Error: Port 5432 already allocated
```
**Fix:**
```bash
# Find what's using the port
lsof -i :5432

# Stop the conflicting service or change port in docker-compose.yml
```

### Database connection failed
```
❌ Database error: connection refused
```
**Fix:**
```bash
# Check PostgreSQL is running
docker compose ps postgres

# Check health
docker compose exec postgres pg_isready -U postgres

# Restart if needed
docker compose restart postgres
```

### RedPanda not starting
```bash
# Check logs
docker compose logs redpanda

# Common fix: Reset volumes
./scripts/cleanup.sh   # Say YES to remove volumes
./scripts/bootstrap.sh
```

### Can't compile
```
error: failed to run custom build command for `ticketing`
```
**Fix:**
```bash
# Ensure migrations exist
ls ../../migrations/

# Clean and rebuild
cargo clean
cargo build
```

## 📚 Next Steps

After running the demo:

1. **Explore the code**: Start with `src/aggregates/inventory.rs` to see event sourcing in action
2. **Modify the demo**: Change quantities, add events, test edge cases
3. **Add API layer**: Wrap services in REST/GraphQL endpoints (next sprint)
4. **Add auth**: Implement authentication/authorization
5. **Scale**: Add more RedPanda brokers, read replicas

## 🎓 Learning Resources

- **Event Sourcing**: Every state change is an immutable event
- **CQRS**: Commands (writes) separate from Queries (reads)
- **Saga Pattern**: Multi-step workflows with compensation
- **Projection**: Denormalized read model updated from events

See the main README for deep dives into each concept.

## ✨ You're Ready!

Run `./scripts/bootstrap.sh` and `cargo run --bin demo` to see it all in action!

For questions or issues: https://github.com/anthropics/composable-rust/issues
