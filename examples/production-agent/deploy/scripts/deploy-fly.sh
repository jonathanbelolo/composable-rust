#!/usr/bin/env bash
# Deploy production-agent to Fly.io

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_step() {
    echo -e "${BLUE}[STEP]${NC} $1"
}

# Check if flyctl is installed
if ! command -v fly &> /dev/null; then
    log_error "flyctl is not installed"
    echo ""
    echo "Install with:"
    echo "  curl -L https://fly.io/install.sh | sh"
    echo ""
    exit 1
fi

# Parse arguments
ACTION="${1:-deploy}"
APP_NAME="production-agent"
REGION="${REGION:-cdg}"  # Default to Paris

case "${ACTION}" in
    setup)
        log_info "Setting up Fly.io application: ${APP_NAME}"

        log_step "1. Checking if you're logged in..."
        if ! fly auth whoami &>/dev/null; then
            log_warn "Not logged in to Fly.io"
            log_info "Opening browser for authentication..."
            fly auth login
        else
            log_info "Already logged in to Fly.io"
        fi

        log_step "2. Creating application..."
        if fly apps list | grep -q "${APP_NAME}"; then
            log_warn "App ${APP_NAME} already exists"
        else
            fly apps create "${APP_NAME}" --region "${REGION}"
            log_info "Created app in region: ${REGION}"
        fi

        log_step "3. Setting up secrets..."
        echo ""
        echo "You need to set your Anthropic API key:"
        echo ""
        read -rp "Enter your Anthropic API key (or press Enter to skip): " api_key

        if [ -n "${api_key}" ]; then
            fly secrets set ANTHROPIC_API_KEY="${api_key}" --app "${APP_NAME}"
            log_info "API key set successfully"
        else
            log_warn "Skipped API key setup. Set it later with:"
            log_warn "  fly secrets set ANTHROPIC_API_KEY=sk-ant-... --app ${APP_NAME}"
        fi

        log_info ""
        log_info "Setup complete! Next steps:"
        log_info "  1. Deploy: ./deploy-fly.sh deploy"
        log_info "  2. Check status: ./deploy-fly.sh status"
        ;;

    deploy)
        log_info "Deploying ${APP_NAME} to Fly.io..."

        cd "${PROJECT_ROOT}"

        log_step "Building and deploying..."
        fly deploy --app "${APP_NAME}"

        log_info ""
        log_info "Deployment complete!"
        log_info "Check status: ./deploy-fly.sh status"
        log_info "View logs: ./deploy-fly.sh logs"
        ;;

    status)
        log_info "Checking ${APP_NAME} status..."

        echo ""
        log_step "Application Status:"
        fly status --app "${APP_NAME}"

        echo ""
        log_step "Health Checks:"
        fly checks --app "${APP_NAME}"

        echo ""
        log_step "Recent Events:"
        fly logs --app "${APP_NAME}" --lines 20
        ;;

    logs)
        log_info "Streaming logs for ${APP_NAME}..."
        fly logs --app "${APP_NAME}"
        ;;

    open)
        log_info "Opening ${APP_NAME} in browser..."
        fly open --app "${APP_NAME}"
        ;;

    scale)
        INSTANCES="${2:-1}"
        log_info "Scaling ${APP_NAME} to ${INSTANCES} instances..."
        fly scale count "${INSTANCES}" --app "${APP_NAME}"
        log_info "Scaled to ${INSTANCES} instances"
        ;;

    regions)
        ACTION_TYPE="${2:-list}"

        case "${ACTION_TYPE}" in
            list)
                log_info "Current regions for ${APP_NAME}:"
                fly regions list --app "${APP_NAME}"
                ;;
            add)
                NEW_REGION="${3:-nrt}"
                log_info "Adding region: ${NEW_REGION}"
                fly regions add "${NEW_REGION}" --app "${APP_NAME}"

                log_warn "Don't forget to deploy to apply changes:"
                log_warn "  ./deploy-fly.sh deploy"
                ;;
            remove)
                REMOVE_REGION="${3}"
                if [ -z "${REMOVE_REGION}" ]; then
                    log_error "Please specify region to remove"
                    exit 1
                fi

                log_info "Removing region: ${REMOVE_REGION}"
                fly regions remove "${REMOVE_REGION}" --app "${APP_NAME}"
                ;;
            *)
                log_error "Unknown regions action: ${ACTION_TYPE}"
                echo "Usage: $0 regions {list|add|remove} [region]"
                exit 1
                ;;
        esac
        ;;

    db)
        DB_ACTION="${2:-status}"
        DB_NAME="production-agent-db"

        case "${DB_ACTION}" in
            create)
                log_info "Creating PostgreSQL database: ${DB_NAME}"
                fly postgres create "${DB_NAME}" --region "${REGION}"

                log_info "Attaching database to app..."
                fly postgres attach "${DB_NAME}" --app "${APP_NAME}"

                log_info "Database created and attached!"
                ;;
            status)
                log_info "Database status:"
                fly status --app "${DB_NAME}"
                ;;
            connect)
                log_info "Connecting to database..."
                fly postgres connect --app "${DB_NAME}"
                ;;
            *)
                log_error "Unknown db action: ${DB_ACTION}"
                echo "Usage: $0 db {create|status|connect}"
                exit 1
                ;;
        esac
        ;;

    redis)
        REDIS_ACTION="${2:-status}"
        REDIS_NAME="production-agent-cache"

        case "${REDIS_ACTION}" in
            create)
                log_info "Creating Redis cache: ${REDIS_NAME}"
                fly redis create "${REDIS_NAME}" --region "${REGION}"

                log_info "Attaching Redis to app..."
                fly redis attach "${REDIS_NAME}" --app "${APP_NAME}"

                log_info "Redis created and attached!"
                ;;
            status)
                log_info "Redis status:"
                fly redis status "${REDIS_NAME}"
                ;;
            *)
                log_error "Unknown redis action: ${REDIS_ACTION}"
                echo "Usage: $0 redis {create|status}"
                exit 1
                ;;
        esac
        ;;

    secrets)
        log_info "Managing secrets for ${APP_NAME}..."
        fly secrets list --app "${APP_NAME}"

        echo ""
        echo "Set a secret with:"
        echo "  fly secrets set KEY=value --app ${APP_NAME}"
        ;;

    destroy)
        log_warn "This will PERMANENTLY delete ${APP_NAME}"
        read -rp "Are you sure? Type '${APP_NAME}' to confirm: " confirm

        if [ "${confirm}" = "${APP_NAME}" ]; then
            log_info "Destroying app..."
            fly apps destroy "${APP_NAME}" --yes
            log_info "App destroyed"
        else
            log_info "Aborted"
        fi
        ;;

    ssh)
        log_info "Opening SSH console to ${APP_NAME}..."
        fly ssh console --app "${APP_NAME}"
        ;;

    proxy)
        PORT="${2:-9090}"
        log_info "Proxying port ${PORT} from ${APP_NAME}..."
        log_info "Access at http://localhost:${PORT}"
        fly proxy "${PORT}" --app "${APP_NAME}"
        ;;

    *)
        log_error "Unknown action: ${ACTION}"
        echo ""
        echo "Usage: $0 {setup|deploy|status|logs|open|scale|regions|db|redis|secrets|destroy|ssh|proxy} [args]"
        echo ""
        echo "Actions:"
        echo "  setup              - Initial setup (create app, set secrets)"
        echo "  deploy             - Deploy to Fly.io"
        echo "  status             - Show app status"
        echo "  logs               - Stream application logs"
        echo "  open               - Open app in browser"
        echo "  scale N            - Scale to N instances"
        echo "  regions list       - List current regions"
        echo "  regions add REGION - Add new region (e.g., nrt, sjc, ewr)"
        echo "  regions remove REG - Remove region"
        echo "  db create          - Create PostgreSQL database"
        echo "  db status          - Show database status"
        echo "  db connect         - Connect to database"
        echo "  redis create       - Create Redis cache"
        echo "  redis status       - Show Redis status"
        echo "  secrets            - List secrets"
        echo "  destroy            - Delete the app (DESTRUCTIVE)"
        echo "  ssh                - SSH into running instance"
        echo "  proxy PORT         - Port forward (default: 9090)"
        echo ""
        echo "Environment variables:"
        echo "  REGION             - Deployment region (default: cdg)"
        echo ""
        echo "Examples:"
        echo "  # Initial setup"
        echo "  $0 setup"
        echo ""
        echo "  # Deploy"
        echo "  $0 deploy"
        echo ""
        echo "  # Add Tokyo region"
        echo "  $0 regions add nrt"
        echo "  $0 deploy"
        echo ""
        echo "  # Scale to 3 instances"
        echo "  $0 scale 3"
        exit 1
        ;;
esac
