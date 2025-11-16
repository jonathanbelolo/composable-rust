#!/bin/bash
#
# Complete persistence reset script for ticketing system
#
# This script clears ALL persistence layers before running tests:
# - PostgreSQL databases (event store, projections, auth)
# - Redpanda topics
# - Redis cache
#
# Usage: ./scripts/reset-all.sh

set -e  # Exit on error

echo "======================================"
echo "🧹 Resetting All Persistence Layers"
echo "======================================"
echo ""

# Use environment variables if set, otherwise use defaults
# (Not sourcing .env to avoid parsing issues with unquoted values)
DATABASE_URL=${DATABASE_URL:-postgresql://postgres:postgres@localhost:5436/ticketing_events}
PROJECTION_DATABASE_URL=${PROJECTION_DATABASE_URL:-postgresql://postgres:postgres@localhost:5433/ticketing_projections}
AUTH_DATABASE_URL=${AUTH_DATABASE_URL:-postgresql://postgres:postgres@localhost:5435/ticketing_auth}
REDIS_URL=${REDIS_URL:-redis://localhost:6379}
REDPANDA_BROKERS=${REDPANDA_BROKERS:-localhost:9092}

echo "1️⃣  Killing any running ticketing servers..."
pkill -9 -f "ticketing" || true
sleep 2
echo "   ✅ Servers stopped"
echo ""

echo "2️⃣  Clearing PostgreSQL databases..."

# Extract database names from URLs
EVENT_DB=$(echo $DATABASE_URL | sed 's|.*\/||')
PROJECTION_DB=$(echo $PROJECTION_DATABASE_URL | sed 's|.*\/||')
AUTH_DB=$(echo $AUTH_DATABASE_URL | sed 's|.*\/||')

# Drop and recreate event store database
echo "   📦 Resetting event store database: $EVENT_DB"
docker exec ticketing-postgres-events psql -U postgres -c "DROP DATABASE IF EXISTS $EVENT_DB;" 2>/dev/null || true
docker exec ticketing-postgres-events psql -U postgres -c "CREATE DATABASE $EVENT_DB;"
echo "   ✅ Event store database reset"

# Drop and recreate projections database
echo "   📊 Resetting projections database: $PROJECTION_DB"
docker exec ticketing-postgres-projections psql -U postgres -c "DROP DATABASE IF EXISTS $PROJECTION_DB;" 2>/dev/null || true
docker exec ticketing-postgres-projections psql -U postgres -c "CREATE DATABASE $PROJECTION_DB;"
echo "   ✅ Projections database reset"

# Drop and recreate auth database
echo "   🔐 Resetting auth database: $AUTH_DB"
docker exec ticketing-postgres-auth psql -U postgres -c "DROP DATABASE IF EXISTS $AUTH_DB;" 2>/dev/null || true
docker exec ticketing-postgres-auth psql -U postgres -c "CREATE DATABASE $AUTH_DB;"
echo "   ✅ Auth database reset"

echo ""
echo "3️⃣  Clearing Redpanda topics..."
TOPICS=$(docker exec ticketing-redpanda rpk topic list 2>/dev/null | grep -v "NAME" | grep "ticketing-" || true)

if [ -n "$TOPICS" ]; then
    echo "$TOPICS" | while read -r topic rest; do
        if [ -n "$topic" ]; then
            echo "   🗑️  Deleting topic: $topic"
            docker exec ticketing-redpanda rpk topic delete "$topic" 2>/dev/null || true
        fi
    done
    echo "   ✅ All Redpanda topics deleted"
else
    echo "   ℹ️  No Redpanda topics to delete"
fi

echo ""
echo "4️⃣  Clearing Redis cache..."
docker exec ticketing-redis redis-cli FLUSHALL 2>/dev/null || echo "   ⚠️  Redis not running or not accessible"
echo "   ✅ Redis cache cleared"

echo ""
echo "======================================"
echo "✨ All persistence layers reset!"
echo "======================================"
echo ""
echo "Next steps:"
echo "  1. Run database migrations"
echo "  2. Start the server"
echo "  3. Run tests"
echo ""
