#!/usr/bin/env bash
# Deploy production-agent with Docker Compose

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
DOCKER_DIR="${SCRIPT_DIR}/../docker"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
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

# Parse arguments
ACTION="${1:-up}"

case "${ACTION}" in
    up)
        log_info "Starting production-agent with Docker Compose..."
        cd "${DOCKER_DIR}"
        docker-compose up -d
        log_info "Waiting for services to be healthy..."
        sleep 5
        docker-compose ps
        log_info ""
        log_info "Services available at:"
        log_info "  - Production Agent API: http://localhost:8080"
        log_info "  - Production Agent Metrics: http://localhost:9090/metrics"
        log_info "  - Prometheus UI: http://localhost:9091"
        log_info "  - Grafana UI: http://localhost:3000 (admin/admin)"
        log_info "  - Jaeger UI: http://localhost:16686"
        log_info ""
        log_info "Test with: curl http://localhost:8080/health"
        ;;

    down)
        log_info "Stopping production-agent..."
        cd "${DOCKER_DIR}"
        docker-compose down
        log_info "Services stopped"
        ;;

    restart)
        log_info "Restarting production-agent..."
        cd "${DOCKER_DIR}"
        docker-compose restart production-agent
        log_info "Service restarted"
        ;;

    logs)
        log_info "Showing production-agent logs..."
        cd "${DOCKER_DIR}"
        docker-compose logs -f production-agent
        ;;

    build)
        log_info "Building production-agent Docker image..."
        cd "${PROJECT_ROOT}"
        docker build -f examples/production-agent/Dockerfile -t production-agent:latest .
        log_info "Build complete"
        ;;

    clean)
        log_info "Cleaning up containers and volumes..."
        cd "${DOCKER_DIR}"
        docker-compose down -v
        log_info "Cleanup complete"
        ;;

    health)
        log_info "Checking service health..."

        # Check production-agent
        if curl -f http://localhost:8080/health/live &>/dev/null; then
            log_info "✓ Production Agent is healthy"
        else
            log_error "✗ Production Agent is not responding"
        fi

        # Check Prometheus
        if curl -f http://localhost:9091/-/healthy &>/dev/null; then
            log_info "✓ Prometheus is healthy"
        else
            log_warn "✗ Prometheus is not responding"
        fi

        # Check Grafana
        if curl -f http://localhost:3000/api/health &>/dev/null; then
            log_info "✓ Grafana is healthy"
        else
            log_warn "✗ Grafana is not responding"
        fi
        ;;

    *)
        log_error "Unknown action: ${ACTION}"
        echo "Usage: $0 {up|down|restart|logs|build|clean|health}"
        exit 1
        ;;
esac
