#!/usr/bin/env bash
# Database migration helper script
# Requires sqlx-cli: cargo install sqlx-cli --no-default-features --features postgres

set -euo pipefail

# Load environment variables from .env if it exists
if [ -f .env ]; then
    set -a
    source .env
    set +a
fi

# Check if DATABASE_URL is set
if [ -z "${DATABASE_URL:-}" ]; then
    echo "Error: DATABASE_URL is not set"
    echo "Please create a .env file from .env.example"
    exit 1
fi

# Parse command
COMMAND="${1:-help}"

case "$COMMAND" in
    setup)
        echo "Creating database..."
        sqlx database create
        echo "Running migrations..."
        sqlx migrate run --source migrations
        echo "✅ Database setup complete"
        ;;

    run)
        echo "Running migrations..."
        sqlx migrate run --source migrations
        echo "✅ Migrations complete"
        ;;

    revert)
        echo "Reverting last migration..."
        sqlx migrate revert --source migrations
        echo "✅ Migration reverted"
        ;;

    drop)
        echo "⚠️  Dropping database..."
        read -p "Are you sure? This will delete all data. (y/N) " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            sqlx database drop -y
            echo "✅ Database dropped"
        else
            echo "Cancelled"
        fi
        ;;

    reset)
        echo "⚠️  Resetting database..."
        read -p "This will drop and recreate the database. Continue? (y/N) " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            sqlx database drop -y
            sqlx database create
            sqlx migrate run --source migrations
            echo "✅ Database reset complete"
        else
            echo "Cancelled"
        fi
        ;;

    status)
        echo "Checking migration status..."
        sqlx migrate info --source migrations
        ;;

    help|*)
        echo "Database migration helper"
        echo ""
        echo "Usage: $0 <command>"
        echo ""
        echo "Commands:"
        echo "  setup   - Create database and run all migrations"
        echo "  run     - Run pending migrations"
        echo "  revert  - Revert the last migration"
        echo "  drop    - Drop the database (with confirmation)"
        echo "  reset   - Drop and recreate database (with confirmation)"
        echo "  status  - Show migration status"
        echo "  help    - Show this help message"
        echo ""
        echo "Prerequisites:"
        echo "  - sqlx-cli: cargo install sqlx-cli --no-default-features --features postgres"
        echo "  - .env file with DATABASE_URL (copy from .env.example)"
        ;;
esac
