#!/bin/bash
#
# Reset Test Data
#
# Clears all stateful data (PostgreSQL, Redis) without restarting containers.
# Run this between individual tests for isolation.

set -e

# Flush Redis (rate limits, sessions)
docker exec ticketing-redis redis-cli FLUSHALL >/dev/null

# Clear PostgreSQL event store
docker exec ticketing-postgres-events psql -U postgres -d ticketing_events -c "TRUNCATE TABLE events CASCADE;" >/dev/null 2>&1

# Clear PostgreSQL projections
docker exec ticketing-postgres-projections psql -U postgres -d ticketing_projections -c "
  TRUNCATE TABLE available_seats CASCADE;
  TRUNCATE TABLE payments CASCADE;
" >/dev/null 2>&1

# Clear auth database
docker exec ticketing-postgres-auth psql -U postgres -d ticketing_auth -c "
  TRUNCATE TABLE magic_links CASCADE;
  TRUNCATE TABLE sessions CASCADE;
" >/dev/null 2>&1

echo "✅ Test data cleared"
