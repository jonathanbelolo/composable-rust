# Multi-stage production Dockerfile for Agent Systems
# Optimized for small image size, fast builds, and security

# Stage 1: Build dependencies (cached layer)
FROM rust:1.85-slim-bookworm AS deps

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests
COPY Cargo.toml Cargo.lock ./
COPY core/Cargo.toml ./core/
COPY runtime/Cargo.toml ./runtime/
COPY testing/Cargo.toml ./testing/
COPY postgres/Cargo.toml ./postgres/
COPY redpanda/Cargo.toml ./redpanda/
COPY web/Cargo.toml ./web/
COPY auth/Cargo.toml ./auth/
COPY projections/Cargo.toml ./projections/
COPY macros/Cargo.toml ./macros/
COPY agent-patterns/Cargo.toml ./agent-patterns/

# Create dummy source files to cache dependencies
RUN mkdir -p core/src runtime/src testing/src postgres/src redpanda/src \
    web/src auth/src projections/src macros/src agent-patterns/src && \
    echo "fn main() {}" > core/src/lib.rs && \
    echo "fn main() {}" > runtime/src/lib.rs && \
    echo "fn main() {}" > testing/src/lib.rs && \
    echo "fn main() {}" > postgres/src/lib.rs && \
    echo "fn main() {}" > redpanda/src/lib.rs && \
    echo "fn main() {}" > web/src/lib.rs && \
    echo "fn main() {}" > auth/src/lib.rs && \
    echo "fn main() {}" > projections/src/lib.rs && \
    echo "fn main() {}" > macros/src/lib.rs && \
    echo "fn main() {}" > agent-patterns/src/lib.rs

# Build dependencies (this layer is cached unless Cargo.toml changes)
RUN cargo build --release --package composable-rust-agent-patterns

# Stage 2: Build application
FROM rust:1.85-slim-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy dependency cache from previous stage
COPY --from=deps /usr/local/cargo /usr/local/cargo

# Copy entire workspace
COPY . .

# Build release binary
# Note: SQLX_OFFLINE=true assumes sqlx-data.json is committed
ARG SQLX_OFFLINE=true
RUN cargo build --release --package composable-rust-agent-patterns --all-features

# Stage 3: Runtime image (minimal, distroless-style)
FROM debian:bookworm-slim AS runtime

# Install runtime dependencies only
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 -s /bin/bash agent && \
    mkdir -p /app /var/log/agent && \
    chown -R agent:agent /app /var/log/agent

WORKDIR /app

# Copy compiled libraries from builder
COPY --from=builder --chown=agent:agent \
    /app/target/release/libcomposable_rust_agent_patterns.so \
    /usr/local/lib/ 2>/dev/null || true

# Copy agent-patterns library
COPY --from=builder --chown=agent:agent \
    /app/target/release/libcomposable_rust_agent_patterns.rlib \
    /usr/local/lib/ 2>/dev/null || true

# If you have a binary (example agent), copy it
# COPY --from=builder --chown=agent:agent /app/target/release/agent-server /app/

# Copy configuration files (if any)
COPY --chown=agent:agent agent-patterns/dashboards /app/dashboards

# Switch to non-root user
USER agent

# Expose ports
# 8080 = HTTP API
# 9090 = Metrics endpoint
EXPOSE 8080 9090

# Health check (adjust path as needed)
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:9090/health/liveness || exit 1

# Default command (override in docker-compose or k8s)
CMD ["echo", "Agent library built. Override CMD with your binary."]

# Labels for metadata
LABEL org.opencontainers.image.title="Composable Rust Agent Patterns" \
      org.opencontainers.image.description="Production-ready agent patterns with observability and resilience" \
      org.opencontainers.image.vendor="Composable Rust" \
      org.opencontainers.image.source="https://github.com/your-org/composable-rust" \
      org.opencontainers.image.version="0.1.0"
