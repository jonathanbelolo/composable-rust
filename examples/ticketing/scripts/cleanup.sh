#!/usr/bin/env bash
set -euo pipefail

# Cleanup script for Ticketing System
# Stops all containers and removes volumes (complete teardown)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "🧹 Cleaning up Ticketing System..."
echo ""

cd "$PROJECT_DIR"

# 1. Stop containers
echo "1️⃣  Stopping containers..."
docker compose down
echo "   ✓ Containers stopped"
echo ""

# 2. Remove volumes (optional - ask user)
read -p "Remove volumes (deletes ALL data permanently)? (y/N) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    echo "2️⃣  Removing volumes..."
    docker compose down -v
    echo "   ✓ Volumes removed"
else
    echo "2️⃣  Keeping volumes (data preserved for next start)"
fi
echo ""

echo "✅ Cleanup complete!"
echo ""
echo "💡 Next time, run:"
echo "   ./scripts/bootstrap.sh    # Fresh start"
echo "   docker compose up -d      # Resume with existing data"
