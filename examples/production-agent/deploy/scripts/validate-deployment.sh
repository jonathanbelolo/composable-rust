#!/usr/bin/env bash
# Validate production-agent deployment configuration
#
# This script checks that all deployment configurations are valid
# and ready for production deployment.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../..\" && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

PASS=0
FAIL=0
WARN=0

log_pass() {
    echo -e "${GREEN}✓${NC} $1"
    ((PASS++))
}

log_fail() {
    echo -e "${RED}✗${NC} $1"
    ((FAIL++))
}

log_warn() {
    echo -e "${YELLOW}⚠${NC} $1"
    ((WARN++))
}

log_info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

log_section() {
    echo ""
    echo -e "${BLUE}=== $1 ===${NC}"
    echo ""
}

# Check file exists
check_file() {
    if [ -f "$1" ]; then
        log_pass "File exists: $1"
        return 0
    else
        log_fail "File missing: $1"
        return 1
    fi
}

# Check directory exists
check_dir() {
    if [ -d "$1" ]; then
        log_pass "Directory exists: $1"
        return 0
    else
        log_fail "Directory missing: $1"
        return 1
    fi
}

# Check file is executable
check_executable() {
    if [ -x "$1" ]; then
        log_pass "File is executable: $1"
        return 0
    else
        log_fail "File not executable: $1"
        return 1
    fi
}

# Check command exists
check_command() {
    if command -v "$1" &> /dev/null; then
        log_pass "Command available: $1"
        return 0
    else
        log_warn "Command not found: $1 (optional)"
        return 1
    fi
}

echo "Production Agent Deployment Validation"
echo "======================================="

# Core Files
log_section "Core Application Files"
check_file "${PROJECT_ROOT}/Cargo.toml"
check_file "${PROJECT_ROOT}/src/main.rs"
check_file "${PROJECT_ROOT}/src/environment.rs"
check_file "${PROJECT_ROOT}/src/reducer.rs"
check_file "${PROJECT_ROOT}/src/server.rs"
check_file "${PROJECT_ROOT}/src/types.rs"

# Configuration Files
log_section "Configuration Files"
check_file "${PROJECT_ROOT}/.env.example"
check_file "${PROJECT_ROOT}/config.toml"

# Docker Deployment
log_section "Docker Deployment"
check_file "${PROJECT_ROOT}/Dockerfile"
check_file "${PROJECT_ROOT}/.dockerignore"
check_file "${PROJECT_ROOT}/deploy/docker/docker-compose.yml"
check_file "${PROJECT_ROOT}/deploy/docker/prometheus.yml"
check_file "${PROJECT_ROOT}/deploy/docker/README.md"
check_dir "${PROJECT_ROOT}/deploy/docker/init-db"
check_file "${PROJECT_ROOT}/deploy/docker/init-db/01-init.sql"
check_dir "${PROJECT_ROOT}/deploy/docker/grafana"

# Fly.io Deployment
log_section "Fly.io Deployment"
check_file "${PROJECT_ROOT}/fly.toml"
check_file "${PROJECT_ROOT}/deploy/fly/DEPLOY.md"
check_executable "${PROJECT_ROOT}/deploy/scripts/deploy-fly.sh"

# Kubernetes Deployment
log_section "Kubernetes Deployment"
check_file "${PROJECT_ROOT}/deploy/k8s/deployment.yaml"
check_file "${PROJECT_ROOT}/deploy/k8s/service.yaml"
check_file "${PROJECT_ROOT}/deploy/k8s/configmap.yaml"
check_file "${PROJECT_ROOT}/deploy/k8s/hpa.yaml"
check_file "${PROJECT_ROOT}/deploy/k8s/pdb.yaml"
check_file "${PROJECT_ROOT}/deploy/k8s/rbac.yaml"

# Deployment Scripts
log_section "Deployment Scripts"
check_executable "${PROJECT_ROOT}/deploy/scripts/deploy-docker.sh"
check_executable "${PROJECT_ROOT}/deploy/scripts/deploy-fly.sh"
check_executable "${PROJECT_ROOT}/deploy/scripts/deploy-k8s.sh"
check_executable "${PROJECT_ROOT}/deploy/scripts/build.sh"

# Documentation
log_section "Documentation"
check_file "${PROJECT_ROOT}/README.md"
check_file "${PROJECT_ROOT}/QUICKSTART.md"
check_file "${PROJECT_ROOT}/deploy/README.md"
check_file "${PROJECT_ROOT}/deploy/docker/README.md"
check_file "${PROJECT_ROOT}/deploy/fly/DEPLOY.md"

# External Dependencies (Optional)
log_section "External Tools (Optional)"
check_command "docker"
check_command "docker-compose"
check_command "fly"
check_command "kubectl"
check_command "cargo"

# Validate Configuration Files
log_section "Configuration Validation"

# Check Dockerfile syntax
if check_file "${PROJECT_ROOT}/Dockerfile"; then
    if grep -q "FROM rust:1.85" "${PROJECT_ROOT}/Dockerfile"; then
        log_pass "Dockerfile uses correct Rust version"
    else
        log_warn "Dockerfile might use outdated Rust version"
    fi

    if grep -q "COPY anthropic" "${PROJECT_ROOT}/Dockerfile"; then
        log_pass "Dockerfile includes anthropic crate"
    else
        log_fail "Dockerfile missing anthropic crate"
    fi
fi

# Check docker-compose.yml
if check_file "${PROJECT_ROOT}/deploy/docker/docker-compose.yml"; then
    if grep -q "postgres:" "${PROJECT_ROOT}/deploy/docker/docker-compose.yml"; then
        log_pass "Docker Compose includes PostgreSQL"
    else
        log_fail "Docker Compose missing PostgreSQL"
    fi

    if grep -q "redis:" "${PROJECT_ROOT}/deploy/docker/docker-compose.yml"; then
        log_pass "Docker Compose includes Redis"
    else
        log_fail "Docker Compose missing Redis"
    fi

    if grep -q "redpanda:" "${PROJECT_ROOT}/deploy/docker/docker-compose.yml"; then
        log_pass "Docker Compose includes Redpanda"
    else
        log_fail "Docker Compose missing Redpanda"
    fi
fi

# Check fly.toml
if check_file "${PROJECT_ROOT}/fly.toml"; then
    if grep -q "app = \"production-agent\"" "${PROJECT_ROOT}/fly.toml"; then
        log_pass "Fly.toml has correct app name"
    else
        log_warn "Fly.toml might have custom app name"
    fi

    if grep -q "dockerfile = \"Dockerfile\"" "${PROJECT_ROOT}/fly.toml"; then
        log_pass "Fly.toml references Dockerfile"
    else
        log_fail "Fly.toml missing Dockerfile reference"
    fi

    if grep -q "/health/live" "${PROJECT_ROOT}/fly.toml"; then
        log_pass "Fly.toml has health checks"
    else
        log_fail "Fly.toml missing health checks"
    fi
fi

# Build Test (if cargo available)
log_section "Build Test"
if command -v cargo &> /dev/null; then
    log_info "Attempting to build production-agent..."
    cd "${PROJECT_ROOT}"
    if SQLX_OFFLINE=true cargo build --release --bin production-agent 2>&1 | tail -5; then
        log_pass "Production agent builds successfully"
    else
        log_fail "Production agent build failed"
    fi
else
    log_warn "Cargo not available, skipping build test"
fi

# Summary
echo ""
echo "======================================"
echo "Validation Summary:"
echo "======================================"
echo -e "${GREEN}Passed:${NC} $PASS"
echo -e "${YELLOW}Warnings:${NC} $WARN"
echo -e "${RED}Failed:${NC} $FAIL"
echo ""

if [ $FAIL -eq 0 ]; then
    echo -e "${GREEN}✓ Deployment configuration is valid!${NC}"
    echo ""
    echo "Next steps:"
    echo "  1. Configure environment: cp .env.example .env"
    echo "  2. Set ANTHROPIC_API_KEY in .env"
    echo "  3. Deploy locally: ./deploy/scripts/deploy-docker.sh up"
    echo "  4. Deploy to Fly.io: ./deploy/scripts/deploy-fly.sh setup"
    echo ""
    exit 0
else
    echo -e "${RED}✗ Deployment configuration has errors${NC}"
    echo ""
    echo "Please fix the failed checks above before deploying."
    echo ""
    exit 1
fi
