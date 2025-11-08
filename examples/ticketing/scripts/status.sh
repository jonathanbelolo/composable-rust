#!/usr/bin/env bash
set -euo pipefail

# Status script for Ticketing System
# Shows current state of all infrastructure components

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "📊 Ticketing System Status"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

cd "$PROJECT_DIR"

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo "❌ Docker is not running"
    exit 1
fi

# Check containers
echo "🐳 Containers:"
docker compose ps
echo ""

# Check PostgreSQL
echo "🗄️  PostgreSQL:"
if docker compose exec -T postgres pg_isready -U postgres > /dev/null 2>&1; then
    echo "   ✅ Status: Running"

    # Check database
    DB_EXISTS=$(docker compose exec -T postgres psql -U postgres -tAc "SELECT 1 FROM pg_database WHERE datname='ticketing'" 2>/dev/null || echo "")
    if [ "$DB_EXISTS" = "1" ]; then
        echo "   ✅ Database 'ticketing' exists"

        # Check tables
        TABLES=$(docker compose exec -T postgres psql -U postgres -d ticketing -tAc "
            SELECT COUNT(*) FROM information_schema.tables
            WHERE table_schema = 'public' AND table_name IN ('events', 'snapshots')
        " 2>/dev/null || echo "0")
        echo "   📊 Tables: $TABLES/2 (events, snapshots)"

        # Count events
        EVENT_COUNT=$(docker compose exec -T postgres psql -U postgres -d ticketing -tAc "
            SELECT COUNT(*) FROM events
        " 2>/dev/null || echo "0")
        echo "   📝 Events stored: $EVENT_COUNT"
    else
        echo "   ⚠️  Database 'ticketing' not created yet"
    fi
else
    echo "   ❌ Status: Not running"
fi
echo ""

# Check RedPanda
echo "🔴 RedPanda:"
if docker compose exec -T redpanda-1 rpk cluster info > /dev/null 2>&1; then
    echo "   ✅ Status: Running (3-broker cluster)"

    # List topics
    TOPICS=$(docker compose exec -T redpanda-1 rpk topic list 2>/dev/null | grep "ticketing-" | wc -l || echo "0")
    echo "   📡 Topics: $TOPICS"

    if [ "$TOPICS" -gt 0 ]; then
        echo "   Topics:"
        docker compose exec -T redpanda-1 rpk topic list 2>/dev/null | grep "ticketing-" | sed 's/^/      /' || true
    fi
else
    echo "   ❌ Status: Not running"
fi
echo ""

# Check RedPanda Console
echo "🖥️  RedPanda Console:"
if curl -s http://localhost:8080 > /dev/null 2>&1; then
    echo "   ✅ Available at: http://localhost:8080"
else
    echo "   ❌ Not accessible"
fi
echo ""

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "💡 Management commands:"
echo "   ./scripts/bootstrap.sh  - Fresh start"
echo "   ./scripts/reset.sh      - Clear all data"
echo "   ./scripts/cleanup.sh    - Stop and remove"
echo "   ./scripts/status.sh     - Show this status"
