#!/bin/bash
# Wait for all infrastructure services to be healthy
# Usage: ./scripts/wait-for-services.sh

set -e

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${BLUE}Waiting for all services to be healthy...${NC}"

# Function to wait for a port
wait_for_port() {
  local HOST=$1
  local PORT=$2
  local SERVICE=$3
  local MAX_WAIT=${4:-60}

  echo -n "Waiting for $SERVICE on $HOST:$PORT... "

  local COUNT=0
  until nc -z "$HOST" "$PORT" 2>/dev/null; do
    if [ $COUNT -ge $MAX_WAIT ]; then
      echo -e "${YELLOW}timeout (${MAX_WAIT}s)${NC}"
      return 1
    fi
    sleep 1
    COUNT=$((COUNT + 1))
  done

  echo -e "${GREEN}ready (${COUNT}s)${NC}"
  return 0
}

# Check if running in Docker environment
if [ -n "$DOCKER_ENV" ]; then
  HOST="host.docker.internal"
else
  HOST="localhost"
fi

# Wait for all services
FAILED=0

wait_for_port "$HOST" 5436 "PostgreSQL Events" || FAILED=1
wait_for_port "$HOST" 5433 "PostgreSQL Projections" || FAILED=1
wait_for_port "$HOST" 5434 "PostgreSQL Analytics" || FAILED=1
wait_for_port "$HOST" 5435 "PostgreSQL Auth" || FAILED=1
wait_for_port "$HOST" 6379 "Redis" || FAILED=1
wait_for_port "$HOST" 9092 "Redpanda" 90 || FAILED=1

if [ $FAILED -eq 0 ]; then
  echo -e "${GREEN}All services are ready!${NC}"
  exit 0
else
  echo -e "${YELLOW}Some services failed to start${NC}"
  exit 1
fi
