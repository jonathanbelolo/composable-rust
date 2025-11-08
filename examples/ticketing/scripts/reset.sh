#!/usr/bin/env bash
set -euo pipefail

# Reset script for Ticketing System
# Clears all data and restarts fresh (keeps containers running)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "🔄 Resetting Ticketing System..."
echo ""
echo "⚠️  WARNING: This will delete ALL data!"
echo "   - All events in PostgreSQL"
echo "   - All snapshots"
echo "   - All RedPanda topics and messages"
echo ""
read -p "Continue? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "❌ Reset cancelled"
    exit 0
fi
echo ""

cd "$PROJECT_DIR"

# 1. Drop and recreate database
echo "1️⃣  Resetting PostgreSQL database..."
docker compose exec -T postgres psql -U postgres -c "DROP DATABASE IF EXISTS ticketing;" > /dev/null 2>&1
docker compose exec -T postgres psql -U postgres -c "CREATE DATABASE ticketing;" > /dev/null 2>&1
echo "   ✓ Database reset (migrations will run on next app start)"
echo ""

# 2. Clear RedPanda topics
echo "2️⃣  Clearing RedPanda topics..."
# Delete all ticketing topics
for topic in ticketing-inventory-events ticketing-reservation-events ticketing-payment-events; do
    docker compose exec -T redpanda-1 rpk topic delete "$topic" 2>/dev/null || true
done
echo "   ✓ Topics cleared (will be recreated on first publish)"
echo ""

echo "✅ Reset complete!"
echo ""
echo "🚀 System is ready for a fresh start:"
echo "   cargo run --bin server"
echo "   cargo run --bin demo"
