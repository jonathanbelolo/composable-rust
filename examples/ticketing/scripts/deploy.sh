#!/bin/bash
#
# Universal deployment script for Composable Rust Ticketing Application
#
# Usage:
#   ./scripts/deploy.sh [platform]
#
# Platforms:
#   fly      - Fly.io (default)
#   railway  - Railway.app
#   render   - Render.com
#   docker   - Local Docker deployment
#
# Examples:
#   ./scripts/deploy.sh           # Deploy to Fly.io (default)
#   ./scripts/deploy.sh railway   # Deploy to Railway
#   ./scripts/deploy.sh docker    # Deploy locally with Docker

set -euo pipefail

# Configuration
PLATFORM="${1:-fly}"
WORKSPACE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEPLOY_DIR="$WORKSPACE_ROOT/examples/ticketing/deploy"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Helper functions
info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

# Check prerequisites
check_prerequisites() {
    case "$PLATFORM" in
        fly)
            if ! command -v flyctl &> /dev/null; then
                error "flyctl not found. Install: curl -L https://fly.io/install.sh | sh"
            fi
            info "flyctl found: $(flyctl version)"
            ;;
        railway)
            if ! command -v railway &> /dev/null; then
                error "railway CLI not found. Install: npm install -g @railway/cli"
            fi
            info "railway found: $(railway --version)"
            ;;
        render)
            warn "Render deployment uses web dashboard or render.yaml (no CLI needed)"
            ;;
        docker)
            if ! command -v docker &> /dev/null; then
                error "docker not found. Install: https://docs.docker.com/get-docker/"
            fi
            info "docker found: $(docker --version)"
            ;;
        *)
            error "Unknown platform: $PLATFORM"
            ;;
    esac
}

# Deploy to Fly.io
deploy_fly() {
    info "Deploying to Fly.io..."

    cd "$WORKSPACE_ROOT"

    if [ ! -f "$DEPLOY_DIR/fly/fly.toml" ]; then
        error "Fly.io configuration not found: $DEPLOY_DIR/fly/fly.toml"
    fi

    info "Building and deploying..."
    flyctl deploy --config examples/ticketing/deploy/fly/fly.toml

    info "Deployment complete!"
    info "Check status: flyctl status"
    info "View logs: flyctl logs"
}

# Deploy to Railway
deploy_railway() {
    info "Deploying to Railway..."

    if [ ! -f "$DEPLOY_DIR/railway/railway.json" ]; then
        error "Railway configuration not found: $DEPLOY_DIR/railway/railway.json"
    fi

    cd "$WORKSPACE_ROOT/examples/ticketing"

    info "Deploying with Railway CLI..."
    railway up

    info "Deployment complete!"
    info "View dashboard: railway open"
}

# Deploy to Render
deploy_render() {
    info "Deploying to Render..."

    if [ ! -f "$DEPLOY_DIR/render/render.yaml" ]; then
        error "Render configuration not found: $DEPLOY_DIR/render/render.yaml"
    fi

    warn "Render deployment options:"
    echo "  1. Push to GitHub and connect repository in Render dashboard"
    echo "  2. Use render.yaml for infrastructure-as-code deployment"
    echo ""
    echo "See: $DEPLOY_DIR/render/README.md"
}

# Deploy locally with Docker
deploy_docker() {
    info "Deploying locally with Docker..."

    cd "$WORKSPACE_ROOT"

    info "Building Docker image..."
    docker build -t ticketing:latest -f examples/ticketing/Dockerfile .

    info "Checking for existing containers..."
    if docker ps -a | grep -q ticketing; then
        warn "Stopping existing ticketing container..."
        docker stop ticketing 2>/dev/null || true
        docker rm ticketing 2>/dev/null || true
    fi

    info "Starting services with Docker Compose..."
    cd examples/ticketing

    if [ ! -f docker-compose.yml ]; then
        error "docker-compose.yml not found in examples/ticketing/"
    fi

    docker-compose up -d

    info "Deployment complete!"
    info "API: http://localhost:8080"
    info "View logs: docker-compose logs -f"
    info "Stop: docker-compose down"
}

# Main deployment logic
main() {
    info "Composable Rust Ticketing - Deployment Script"
    info "Platform: $PLATFORM"
    echo ""

    check_prerequisites

    case "$PLATFORM" in
        fly)
            deploy_fly
            ;;
        railway)
            deploy_railway
            ;;
        render)
            deploy_render
            ;;
        docker)
            deploy_docker
            ;;
        *)
            error "Unknown platform: $PLATFORM"
            ;;
    esac
}

# Run main function
main
