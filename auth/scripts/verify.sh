#!/bin/bash
# Comprehensive verification script for auth crate
# This catches schema mismatches and compilation issues before they ship

set -e  # Exit on first error

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${YELLOW}=== Auth Crate Verification ===${NC}\n"

# Check 1: Database is running
echo -e "${YELLOW}[1/7] Checking database...${NC}"
if ! docker ps | grep -q "auth-db"; then
    echo -e "${RED}❌ PostgreSQL container 'auth-db' is not running${NC}"
    echo "Start it with: docker run -d --name auth-db -p 5434:5432 -e POSTGRES_PASSWORD=password postgres:16-alpine"
    exit 1
fi
echo -e "${GREEN}✅ Database is running${NC}\n"

# Check 2: Migrations applied
echo -e "${YELLOW}[2/7] Applying migrations...${NC}"
DATABASE_URL="postgres://postgres:password@localhost:5434/composable_auth" sqlx migrate run
echo -e "${GREEN}✅ Migrations applied${NC}\n"

# Check 3: Compile with DATABASE_URL (validates queries against schema)
echo -e "${YELLOW}[3/7] Compiling with live database validation...${NC}"
if ! DATABASE_URL="postgres://postgres:password@localhost:5434/composable_auth" cargo check --all-features 2>&1 | tee /tmp/auth-check.log; then
    echo -e "${RED}❌ Compilation failed with database validation${NC}"
    echo "Check /tmp/auth-check.log for details"
    exit 1
fi
echo -e "${GREEN}✅ Compilation passed with database validation${NC}\n"

# Check 4: Generate sqlx cache
echo -e "${YELLOW}[4/7] Generating sqlx query cache...${NC}"
PREPARE_OUTPUT=$(DATABASE_URL="postgres://postgres:password@localhost:5434/composable_auth" cargo sqlx prepare -- --all-features 2>&1)
echo "$PREPARE_OUTPUT"

# Verify it didn't say "no queries found"
if echo "$PREPARE_OUTPUT" | grep -q "no queries found"; then
    echo -e "${RED}❌ WARNING: 'no queries found' - this usually means feature flags are wrong${NC}"
    exit 1
fi

# Count cached queries
QUERY_COUNT=$(ls -1 .sqlx/query-*.json 2>/dev/null | wc -l | tr -d ' ')
if [ "$QUERY_COUNT" -lt 10 ]; then
    echo -e "${RED}❌ Only $QUERY_COUNT queries cached (expected 30+)${NC}"
    echo "This suggests queries aren't being found. Check feature flags."
    exit 1
fi
echo -e "${GREEN}✅ Generated $QUERY_COUNT query cache files${NC}\n"

# Check 5: Verify offline mode works
echo -e "${YELLOW}[5/7] Testing offline compilation mode...${NC}"
if ! SQLX_OFFLINE=true cargo check --all-features 2>&1 | tee /tmp/auth-check-offline.log; then
    echo -e "${RED}❌ Offline compilation failed${NC}"
    echo "Check /tmp/auth-check-offline.log for details"
    exit 1
fi
echo -e "${GREEN}✅ Offline mode works${NC}\n"

# Check 6: Run tests
echo -e "${YELLOW}[6/7] Running tests...${NC}"
if ! SQLX_OFFLINE=true cargo test --all-features 2>&1 | tee /tmp/auth-tests.log; then
    echo -e "${RED}❌ Tests failed${NC}"
    echo "Check /tmp/auth-tests.log for details"
    exit 1
fi
echo -e "${GREEN}✅ Tests passed${NC}\n"

# Check 7: Clippy with strict lints
echo -e "${YELLOW}[7/7] Running clippy...${NC}"
if ! SQLX_OFFLINE=true cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tee /tmp/auth-clippy.log; then
    echo -e "${RED}❌ Clippy found issues${NC}"
    echo "Check /tmp/auth-clippy.log for details"
    exit 1
fi
echo -e "${GREEN}✅ Clippy passed${NC}\n"

echo -e "${GREEN}=== ✅ All Checks Passed ===${NC}"
echo -e "${GREEN}The auth crate is ready to ship!${NC}"
