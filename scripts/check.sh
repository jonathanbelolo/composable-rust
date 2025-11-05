#!/usr/bin/env bash
#
# Quality assurance script - runs all checks locally before pushing
#
# Usage: ./scripts/check.sh

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to print section headers
print_section() {
    echo ""
    echo -e "${YELLOW}===================================${NC}"
    echo -e "${YELLOW}$1${NC}"
    echo -e "${YELLOW}===================================${NC}"
    echo ""
}

# Function to print success
print_success() {
    echo -e "${GREEN}✓ $1${NC}"
}

# Function to print error
print_error() {
    echo -e "${RED}✗ $1${NC}"
}

# Track overall success
FAILED=0

# 1. Format check
print_section "1. Checking code formatting"
if cargo fmt --all --check; then
    print_success "Code is properly formatted"
else
    print_error "Code formatting check failed"
    echo "Run 'cargo fmt --all' to fix formatting issues"
    FAILED=1
fi

# 2. Clippy
print_section "2. Running clippy"
if cargo clippy --all-targets --all-features -- -D warnings; then
    print_success "Clippy checks passed"
else
    print_error "Clippy found issues"
    FAILED=1
fi

# 3. Build
print_section "3. Building all targets"
if cargo build --all-features; then
    print_success "Build successful"
else
    print_error "Build failed"
    FAILED=1
fi

# 4. Tests
print_section "4. Running tests"
if cargo test --all-features; then
    print_success "All tests passed"
else
    print_error "Some tests failed"
    FAILED=1
fi

# 5. Documentation
print_section "5. Building documentation"
if RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features; then
    print_success "Documentation built successfully"
else
    print_error "Documentation build failed"
    FAILED=1
fi

# Summary
echo ""
echo -e "${YELLOW}===================================${NC}"
if [ $FAILED -eq 0 ]; then
    echo -e "${GREEN}✓ All checks passed!${NC}"
    echo ""
    echo "You're ready to commit and push."
    exit 0
else
    echo -e "${RED}✗ Some checks failed${NC}"
    echo ""
    echo "Please fix the issues above before committing."
    exit 1
fi
