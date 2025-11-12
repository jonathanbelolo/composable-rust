#!/bin/bash
# Full backup of all PostgreSQL databases
# Usage: ./scripts/backup-postgres.sh

set -e

# Create backups directory if it doesn't exist
mkdir -p ./backups

# Timestamp for backup files
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}Starting PostgreSQL backup at $TIMESTAMP${NC}"

# Backup each database
for DB in events projections analytics auth; do
  CONTAINER="ticketing-postgres-${DB}"
  DB_NAME="ticketing_${DB}"
  BACKUP_FILE="./backups/${DB}_${TIMESTAMP}.sql.gz"

  echo -e "${BLUE}Backing up ${DB_NAME} from container ${CONTAINER}...${NC}"

  docker exec "$CONTAINER" pg_dump -U postgres "$DB_NAME" \
    | gzip > "$BACKUP_FILE"

  # Check if backup was successful
  if [ -f "$BACKUP_FILE" ]; then
    SIZE=$(du -h "$BACKUP_FILE" | cut -f1)
    echo -e "${GREEN}✓ Backup complete: ${BACKUP_FILE} (${SIZE})${NC}"
  else
    echo -e "${RED}✗ Backup failed for ${DB_NAME}${NC}"
    exit 1
  fi
done

echo -e "${GREEN}All backups completed successfully!${NC}"
echo ""
echo "Backup files:"
ls -lh ./backups/*_${TIMESTAMP}.sql.gz
