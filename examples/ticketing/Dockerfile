# Multi-stage Dockerfile for Ticketing Application
#
# This Dockerfile creates an optimized production image:
# - Builder stage: Compiles the Rust application with all dependencies
# - Runtime stage: Minimal Debian image with only the binary and migrations
# - Target size: < 100MB for runtime image
#
# Build from workspace root:
#   docker build -t ticketing:latest -f examples/ticketing/Dockerfile .

# ============================================================================
# Builder Stage
# ============================================================================
FROM rust:1.85-slim as builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests
COPY Cargo.toml Cargo.lock ./

# Copy all crate manifests
COPY core/Cargo.toml core/
COPY runtime/Cargo.toml runtime/
COPY testing/Cargo.toml testing/
COPY postgres/Cargo.toml postgres/
COPY redpanda/Cargo.toml redpanda/
COPY projections/Cargo.toml projections/
COPY web/Cargo.toml web/
COPY auth/Cargo.toml auth/
COPY anthropic/Cargo.toml anthropic/
COPY agent-patterns/Cargo.toml agent-patterns/
COPY tools/Cargo.toml tools/
COPY macros/Cargo.toml macros/
COPY examples/ticketing/Cargo.toml examples/ticketing/

# Build dependencies only (cache layer)
# Create dummy source files to satisfy cargo build
RUN mkdir -p core/src runtime/src testing/src postgres/src redpanda/src \
    projections/src web/src auth/src anthropic/src agent-patterns/src \
    tools/src macros/src examples/ticketing/src && \
    echo "fn main() {}" > examples/ticketing/src/main.rs && \
    touch core/src/lib.rs runtime/src/lib.rs testing/src/lib.rs \
    postgres/src/lib.rs redpanda/src/lib.rs projections/src/lib.rs \
    web/src/lib.rs auth/src/lib.rs anthropic/src/lib.rs \
    agent-patterns/src/lib.rs tools/src/lib.rs macros/src/lib.rs && \
    cargo build --release --package ticketing && \
    rm -rf target/release/.fingerprint/ticketing-* \
    target/release/deps/ticketing-* \
    target/release/ticketing \
    target/release/ticketing.d

# Copy actual source code
COPY core core
COPY runtime runtime
COPY testing testing
COPY postgres postgres
COPY redpanda redpanda
COPY projections projections
COPY web web
COPY auth auth
COPY anthropic anthropic
COPY agent-patterns agent-patterns
COPY tools tools
COPY macros macros
COPY examples/ticketing examples/ticketing

# Build application with SQLX offline mode
ENV SQLX_OFFLINE=true
RUN cargo build --release --package ticketing

# ============================================================================
# Runtime Stage
# ============================================================================
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user for security
RUN useradd -m -u 1000 -s /bin/bash ticketing && \
    chown -R ticketing:ticketing /app

# Copy binary from builder
COPY --from=builder /app/target/release/ticketing /app/ticketing

# Copy migrations
COPY --from=builder /app/examples/ticketing/migrations /app/migrations

# Set ownership
RUN chown -R ticketing:ticketing /app

# Switch to non-root user
USER ticketing

# Expose ports
# 8080: HTTP API
# 9090: Metrics endpoint (Prometheus)
EXPOSE 8080 9090

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/app/ticketing", "healthcheck"] || exit 1

# Run application
CMD ["/app/ticketing"]
