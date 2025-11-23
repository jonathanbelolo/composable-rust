#!/bin/bash
#
# Reset Test Environment
#
# Completely tears down and rebuilds the Docker environment for integration tests.
# Run this before the test suite to ensure a clean slate.

set -e

echo "🧹 Resetting test environment..."

# Stop and remove all containers
echo "  → Stopping containers..."
docker compose down -v 2>/dev/null || true

# Start fresh containers
echo "  → Starting fresh containers..."
docker compose up -d

# Wait for services to be healthy
echo "  → Waiting for services to be ready..."
sleep 10

# Note: Migrations run automatically on server startup

# Create Redpanda topics
echo "  → Creating Redpanda topics..."
docker exec ticketing-redpanda rpk topic create events --partitions 1 --replicas 1 2>&1 | grep -q "OK\|already exists" || true
docker exec ticketing-redpanda rpk topic create ticketing-reservation-events --partitions 1 --replicas 1 2>&1 | grep -q "OK\|already exists" || true
docker exec ticketing-redpanda rpk topic create ticketing-inventory-events --partitions 1 --replicas 1 2>&1 | grep -q "OK\|already exists" || true
docker exec ticketing-redpanda rpk topic create ticketing-payment-events --partitions 1 --replicas 1 2>&1 | grep -q "OK\|already exists" || true
docker exec ticketing-redpanda rpk topic create projection.completed --partitions 1 --replicas 1 2>&1 | grep -q "OK\|already exists" || true

echo "✅ Test environment ready!"
