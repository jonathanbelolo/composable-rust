# Multi-stage build for production-agent
FROM rust:1.85-slim as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY core ./core
COPY runtime ./runtime
COPY testing ./testing
COPY agent-patterns ./agent-patterns

# Copy production-agent
COPY examples/production-agent ./examples/production-agent

# Build release binary
RUN cargo build --release --bin production-agent

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 agent

# Copy binary from builder
COPY --from=builder /app/target/release/production-agent /usr/local/bin/production-agent

# Copy config
COPY examples/production-agent/config.toml /etc/production-agent/config.toml

# Set ownership
RUN chown -R agent:agent /etc/production-agent

# Switch to non-root user
USER agent

# Expose ports
EXPOSE 8080 9090

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/health/live || exit 1

# Run the application
ENTRYPOINT ["/usr/local/bin/production-agent"]
