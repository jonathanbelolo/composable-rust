#!/bin/bash
# Helper script to run full deployment tests
# This script starts the server, runs the tests, and cleans up

set -e

# Configuration
export AUTH_JWT_SECRET="test-secret-at-least-32-bytes-long-abc123"
export AUTH_EXPOSE_MAGIC_LINKS_FOR_TESTING=true
export AUTH_RATE_LIMIT_REQUESTS=1000

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${YELLOW}🚀 Full Deployment Test Runner${NC}"
echo ""

# Check if Docker Compose is running
echo -e "${YELLOW}1. Checking Docker Compose services...${NC}"
if ! docker compose ps | grep -q "Up"; then
    echo -e "${RED}❌ Docker Compose services are not running${NC}"
    echo -e "${YELLOW}   Starting services with: docker compose up -d${NC}"
    docker compose up -d
    echo -e "${YELLOW}   Waiting for services to be healthy...${NC}"
    sleep 10
fi
echo -e "${GREEN}✅ Docker Compose services are running${NC}"
echo ""

# Clear Redis rate limits
echo -e "${YELLOW}2. Clearing Redis rate limits...${NC}"
docker exec ticketing-redis redis-cli FLUSHALL > /dev/null
echo -e "${GREEN}✅ Redis cleared${NC}"
echo ""

# Clear PostgreSQL databases
echo -e "${YELLOW}3. Clearing PostgreSQL databases...${NC}"
echo -e "${YELLOW}   Truncating event store...${NC}"
docker exec ticketing-events psql -U postgres -d ticketing_events -c "TRUNCATE events CASCADE;" > /dev/null 2>&1 || true
echo -e "${YELLOW}   Truncating projections...${NC}"
docker exec ticketing-projections psql -U postgres -d ticketing_projections -c "TRUNCATE available_seats, payments, reservations, ownership_indices CASCADE;" > /dev/null 2>&1 || true
echo -e "${YELLOW}   Truncating auth...${NC}"
docker exec ticketing-auth psql -U postgres -d ticketing_auth -c "TRUNCATE users, sessions, magic_links, oauth_states CASCADE;" > /dev/null 2>&1 || true
echo -e "${GREEN}✅ PostgreSQL databases cleared${NC}"
echo ""

# Kill any existing server on port 8080
echo -e "${YELLOW}4. Cleaning up any existing server...${NC}"
if lsof -ti:8080 > /dev/null 2>&1; then
    echo -e "${YELLOW}   Killing existing server on port 8080...${NC}"
    lsof -ti:8080 | xargs kill -9 2>/dev/null || true
    sleep 2
fi
echo -e "${GREEN}✅ Port 8080 is clear${NC}"
echo ""

# Start the server in the background
echo -e "${YELLOW}5. Starting ticketing server...${NC}"
cargo run -p ticketing --bin ticketing > /tmp/ticketing-deployment-test.log 2>&1 &
SERVER_PID=$!
echo -e "${YELLOW}   Server PID: $SERVER_PID${NC}"

# Wait for server to be ready
echo -e "${YELLOW}   Waiting for server to be ready...${NC}"
for i in {1..30}; do
    if curl -s http://localhost:8080/health > /dev/null 2>&1; then
        echo -e "${GREEN}✅ Server is ready!${NC}"
        break
    fi
    if [ $i -eq 30 ]; then
        echo -e "${RED}❌ Server failed to start within 30 seconds${NC}"
        echo -e "${YELLOW}   Check logs at: /tmp/ticketing-deployment-test.log${NC}"
        kill $SERVER_PID 2>/dev/null || true
        exit 1
    fi
    sleep 1
done
echo ""

# Function to cleanup on exit
cleanup() {
    echo ""
    echo -e "${YELLOW}🧹 Cleaning up...${NC}"
    if [ ! -z "$SERVER_PID" ]; then
        echo -e "${YELLOW}   Stopping server (PID: $SERVER_PID)...${NC}"
        kill $SERVER_PID 2>/dev/null || true
    fi
    echo -e "${GREEN}✅ Cleanup complete${NC}"
}

# Set trap to cleanup on script exit
trap cleanup EXIT INT TERM

# Run the tests
echo -e "${YELLOW}6. Running deployment tests...${NC}"
echo ""
if cargo test -p ticketing --test full_deployment_test -- --test-threads=1 --nocapture; then
    echo ""
    echo -e "${GREEN}✅ All deployment tests passed!${NC}"
    EXIT_CODE=0
else
    echo ""
    echo -e "${RED}❌ Some deployment tests failed${NC}"
    echo -e "${YELLOW}   Check server logs at: /tmp/ticketing-deployment-test.log${NC}"
    EXIT_CODE=1
fi

exit $EXIT_CODE
