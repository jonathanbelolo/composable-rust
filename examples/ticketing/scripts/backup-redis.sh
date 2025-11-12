#!/bin/bash
# Backup Redis data
# Usage: ./scripts/backup-redis.sh

set -e

# Create backups directory if it doesn't exist
mkdir -p ./backups

# Timestamp for backup file
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
BACKUP_FILE="./backups/redis_${TIMESTAMP}.rdb"

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}Starting Redis backup at $TIMESTAMP${NC}"

# Trigger Redis BGSAVE
docker exec ticketing-redis redis-cli BGSAVE

# Wait for BGSAVE to complete (simple approach: just wait 3 seconds)
echo "Waiting for background save to complete..."
sleep 3

# Copy dump.rdb from container
docker cp ticketing-redis:/data/dump.rdb "$BACKUP_FILE"

if [ -f "$BACKUP_FILE" ]; then
  SIZE=$(du -h "$BACKUP_FILE" | cut -f1)
  echo -e "${GREEN}✓ Redis backup complete: ${BACKUP_FILE} (${SIZE})${NC}"
else
  echo -e "${RED}✗ Redis backup failed${NC}"
  exit 1
fi
