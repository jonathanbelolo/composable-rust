#!/usr/bin/env bash
set -euo pipefail

# Bootstrap script for Ticketing System
# Starts infrastructure and initializes databases

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "🎫 Bootstrapping Ticketing System..."
echo ""

# 1. Check if Docker is running
echo "1️⃣  Checking Docker..."
if ! docker info > /dev/null 2>&1; then
    echo "❌ Docker is not running. Please start Docker Desktop."
    exit 1
fi
echo "   ✓ Docker is running"
echo ""

# 2. Start infrastructure
echo "2️⃣  Starting infrastructure (PostgreSQL + RedPanda)..."
cd "$PROJECT_DIR"
docker compose up -d

# Wait for PostgreSQL to be ready
echo "   ⏳ Waiting for PostgreSQL to be ready..."
for i in {1..30}; do
    if docker compose exec -T postgres pg_isready -U postgres > /dev/null 2>&1; then
        echo "   ✓ PostgreSQL is ready"
        break
    fi
    if [ $i -eq 30 ]; then
        echo "   ❌ PostgreSQL failed to start"
        exit 1
    fi
    sleep 1
done

# Wait for RedPanda to be ready
echo "   ⏳ Waiting for RedPanda to be ready..."
sleep 5
echo "   ✓ RedPanda is ready"
echo ""

# 3. Create database
echo "3️⃣  Creating database..."
docker compose exec -T postgres psql -U postgres -c "
    SELECT 'CREATE DATABASE ticketing'
    WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'ticketing')\\gexec
" > /dev/null 2>&1 || true
echo "   ✓ Database 'ticketing' ready"
echo ""

# 4. Run migrations (will be done by the app on first start)
echo "4️⃣  Migrations will run automatically on first app start"
echo "   Location: ../../migrations/"
echo "   - 001_create_events_table.sql"
echo "   - 002_create_snapshots_table.sql"
echo ""

# 5. Show status
echo "5️⃣  Infrastructure Status:"
echo ""
docker compose ps
echo ""

echo "✅ Bootstrap complete!"
echo ""
echo "📋 What's Running:"
echo "   - PostgreSQL: localhost:5432 (ticketing database)"
echo "   - RedPanda: localhost:9092 (3-broker cluster)"
echo "   - RedPanda Console: http://localhost:8080"
echo ""
echo "🚀 Next Steps:"
echo "   # Run the server:"
echo "   cargo run --bin server"
echo ""
echo "   # Or run the demo:"
echo "   cargo run --bin demo"
echo ""
echo "   # View logs:"
echo "   docker compose logs -f"
echo ""
echo "   # Stop everything:"
echo "   ./scripts/cleanup.sh"
