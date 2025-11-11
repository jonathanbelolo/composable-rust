#!/usr/bin/env bash
# Build production-agent Docker image

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_step() {
    echo -e "${BLUE}[STEP]${NC} $1"
}

# Parse arguments
IMAGE_NAME="${1:-production-agent}"
IMAGE_TAG="${2:-latest}"
FULL_IMAGE="${IMAGE_NAME}:${IMAGE_TAG}"

log_info "Building production-agent Docker image"
log_info "Image: ${FULL_IMAGE}"
log_info "Project root: ${PROJECT_ROOT}"
echo ""

log_step "Running build..."
cd "${PROJECT_ROOT}"

docker build \
    -f examples/production-agent/Dockerfile \
    -t "${FULL_IMAGE}" \
    --build-arg BUILDKIT_INLINE_CACHE=1 \
    .

log_info ""
log_info "Build complete!"
log_info "Image: ${FULL_IMAGE}"
log_info ""
log_info "Run with:"
log_info "  docker run -p 8080:8080 -p 9090:9090 ${FULL_IMAGE}"
log_info ""
log_info "Or use docker-compose:"
log_info "  cd examples/production-agent/deploy/docker"
log_info "  docker-compose up"
