#!/bin/bash
# Restore PostgreSQL databases from backup
# Usage: ./scripts/restore-postgres.sh <backup_timestamp>
# Example: ./scripts/restore-postgres.sh 20250112_143022

set -e

# Check if timestamp argument is provided
if [ -z "$1" ]; then
  echo "Usage: $0 <backup_timestamp>"
  echo "Example: $0 20250112_143022"
  echo ""
  echo "Available backups:"
  ls -1 ./backups/*_*.sql.gz 2>/dev/null | sed 's/.*_\([0-9_]*\)\.sql\.gz/\1/' | sort -u || echo "No backups found"
  exit 1
fi

TIMESTAMP=$1

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${YELLOW}WARNING: This will restore databases from backup timestamp: $TIMESTAMP${NC}"
echo -e "${YELLOW}This will OVERWRITE all current data in the databases!${NC}"
echo ""
read -p "Are you sure you want to continue? (yes/no): " CONFIRM

if [ "$CONFIRM" != "yes" ]; then
  echo "Restore cancelled."
  exit 0
fi

echo -e "${BLUE}Starting PostgreSQL restore from backup $TIMESTAMP${NC}"

# Restore each database
for DB in events projections analytics auth; do
  CONTAINER="ticketing-postgres-${DB}"
  DB_NAME="ticketing_${DB}"
  BACKUP_FILE="./backups/${DB}_${TIMESTAMP}.sql.gz"

  if [ ! -f "$BACKUP_FILE" ]; then
    echo -e "${RED}✗ Backup file not found: ${BACKUP_FILE}${NC}"
    exit 1
  fi

  echo -e "${BLUE}Restoring ${DB_NAME} to container ${CONTAINER}...${NC}"

  # Drop and recreate database
  docker exec "$CONTAINER" psql -U postgres -c "DROP DATABASE IF EXISTS ${DB_NAME};"
  docker exec "$CONTAINER" psql -U postgres -c "CREATE DATABASE ${DB_NAME};"

  # Restore from backup
  gunzip -c "$BACKUP_FILE" | docker exec -i "$CONTAINER" psql -U postgres -d "$DB_NAME"

  echo -e "${GREEN}✓ Restore complete: ${DB_NAME}${NC}"
done

echo -e "${GREEN}All databases restored successfully from backup $TIMESTAMP!${NC}"
