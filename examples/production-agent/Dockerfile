# Multi-stage build for production-agent
FROM rust:1.85-slim as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    cmake \
    g++ \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy Cargo.lock first
COPY Cargo.lock ./

# Copy all workspace library crates (needed for workspace resolution)
COPY core ./core
COPY runtime ./runtime
COPY testing ./testing
COPY postgres ./postgres
COPY redpanda ./redpanda
COPY projections ./projections
COPY macros ./macros
COPY anthropic ./anthropic
COPY tools ./tools
COPY agent-patterns ./agent-patterns
COPY auth ./auth
COPY web ./web

# Copy migrations (needed by sqlx macros at compile time)
COPY migrations ./migrations

# Copy production-agent itself
COPY examples/production-agent ./examples/production-agent

# Create a minimal workspace Cargo.toml with only the crates we've copied
RUN cat > Cargo.toml << 'EOF'
[workspace]
resolver = "2"
members = [
    "core",
    "runtime",
    "testing",
    "postgres",
    "redpanda",
    "projections",
    "macros",
    "anthropic",
    "tools",
    "agent-patterns",
    "examples/production-agent",
    "auth",
    "web",
]

default-members = [
    "core",
    "runtime",
    "testing",
    "postgres",
    "redpanda",
    "projections",
    "macros",
    "anthropic",
    "tools",
    "agent-patterns",
    "examples/production-agent",
    "web",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.85.0"
authors = ["Composable Rust Contributors"]
license = "MIT OR Apache-2.0"
repository = "https://github.com/yourusername/composable-rust"

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
futures = "0.3"
async-stream = "0.3"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
serde = { version = "1", features = ["derive"] }
bincode = "1"
thiserror = "1"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
metrics = "0.23"
metrics-exporter-prometheus = "0.15"
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "macros", "chrono", "uuid", "json"] }
rdkafka = "0.36"
chrono = { version = "0.4", features = ["serde"] }
smallvec = "1"
rand = "0.8"
proptest = "1"
tokio-test = "0.4"
criterion = "0.5"
testcontainers = "0.23"
testcontainers-modules = { version = "0.11", features = ["postgres"] }

[profile.dev]
opt-level = 0
debug = true

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
strip = false

[profile.bench]
inherits = "release"

[profile.test]
opt-level = 0

[workspace.lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"

[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
missing_const_for_fn = "warn"
cognitive_complexity = "warn"
missing_errors_doc = "warn"
missing_panics_doc = "warn"
EOF

# Build release binary (only the production-agent package)
RUN cargo build --release -p production-agent

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
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
