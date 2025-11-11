# Docker Deployment Guide for Agent Systems

Complete guide for containerizing and deploying agent systems using Docker and Docker Compose.

## Table of Contents

- [Quick Start](#quick-start)
- [Architecture](#architecture)
- [Building Images](#building-images)
- [Running with Docker Compose](#running-with-docker-compose)
- [Configuration](#configuration)
- [Monitoring](#monitoring)
- [Troubleshooting](#troubleshooting)
- [Production Deployment](#production-deployment)

## Quick Start

### Prerequisites

- Docker 24.0+ ([install](https://docs.docker.com/get-docker/))
- Docker Compose 2.20+ (included with Docker Desktop)
- 4GB+ RAM allocated to Docker
- 10GB+ disk space

### Start All Services

```bash
# Start all infrastructure services
docker-compose up -d

# View logs
docker-compose logs -f

# Check service health
docker-compose ps
```

### Access Services

Once running, access:

- **Grafana**: http://localhost:3000 (admin/admin)
- **Prometheus**: http://localhost:9090
- **Jaeger UI**: http://localhost:16686
- **PostgreSQL**: localhost:5434 (postgres/password)
- **Redpanda Console**: http://localhost:9644/v1/status
- **Redis**: localhost:6379

## Architecture

### Services Overview

```
┌─────────────────────────────────────────────────────────┐
│                    Agent Application                     │
│  (Your Rust service using agent-patterns library)       │
│  - HTTP API (:8080)                                     │
│  - Metrics endpoint (:9090/metrics)                     │
└──────────┬────────────┬─────────────┬────────────────────┘
           │            │             │
     ┌─────▼────┐  ┌────▼─────┐  ┌───▼────┐
     │ Postgres │  │ Redpanda │  │ Jaeger │
     │ (Events) │  │  (Bus)   │  │(Traces)│
     └──────────┘  └──────────┘  └────────┘
                        │
                   ┌────▼──────┐
                   │Prometheus │
                   │ (Metrics) │
                   └─────┬─────┘
                         │
                    ┌────▼────┐
                    │ Grafana │
                    │  (UI)   │
                    └─────────┘
```

### Container Details

| Service    | Image                        | Port(s)          | Purpose                    |
|------------|------------------------------|------------------|----------------------------|
| postgres   | postgres:16-alpine           | 5434             | Event store                |
| redpanda   | redpanda:v24.1.1             | 19092, 18081     | Event bus (Kafka-compat)   |
| jaeger     | jaegertracing/all-in-one     | 16686, 4317      | Distributed tracing        |
| prometheus | prom/prometheus:v2.49        | 9090             | Metrics collection         |
| grafana    | grafana/grafana:10.3         | 3000             | Visualization & dashboards |
| redis      | redis:7-alpine               | 6379             | Cache/sessions (optional)  |

## Building Images

### Development Build

```bash
# Build development image (with debugging symbols)
docker build -t agent-system:dev .

# Build with build arguments
docker build \
  --build-arg SQLX_OFFLINE=true \
  -t agent-system:dev \
  .
```

### Production Build

```bash
# Multi-stage optimized build
docker build \
  --target runtime \
  --tag agent-system:latest \
  --tag agent-system:0.1.0 \
  .

# Check image size
docker images agent-system
```

### Build Performance Tips

1. **Enable BuildKit** for parallel builds:
   ```bash
   export DOCKER_BUILDKIT=1
   docker build -t agent-system .
   ```

2. **Use cache mounts** for faster Cargo builds:
   ```dockerfile
   RUN --mount=type=cache,target=/usr/local/cargo/registry \
       cargo build --release
   ```

3. **Layer caching**: The Dockerfile uses multi-stage builds to cache dependencies separately from source code.

## Running with Docker Compose

### Basic Operations

```bash
# Start all services in background
docker-compose up -d

# Start specific services
docker-compose up -d postgres redpanda

# View logs from all services
docker-compose logs -f

# View logs from specific service
docker-compose logs -f postgres

# Stop all services (keeps volumes)
docker-compose stop

# Stop and remove containers (keeps volumes)
docker-compose down

# Stop and remove everything including volumes
docker-compose down -v

# Restart a service
docker-compose restart postgres
```

### Development Workflow

```bash
# 1. Start infrastructure
docker-compose up -d

# 2. Run your agent application locally
# (connects to Docker services via localhost ports)
cargo run --package your-agent-app

# 3. View metrics in Grafana
open http://localhost:3000

# 4. View traces in Jaeger
open http://localhost:16686

# 5. Cleanup when done
docker-compose down
```

### Running Agent in Docker

If you have a compiled binary, add it to docker-compose.yml:

```yaml
  agent-app:
    build:
      context: ../..
      dockerfile: agent-patterns/Dockerfile
    container_name: agent-app
    environment:
      RUST_LOG: info
      DATABASE_URL: postgres://postgres:password@postgres:5432/agent_system
      KAFKA_BROKERS: redpanda:9092
      JAEGER_AGENT_HOST: jaeger
      JAEGER_AGENT_PORT: 6831
    ports:
      - "8080:8080"  # HTTP API
      - "9090:9090"  # Metrics
    depends_on:
      postgres:
        condition: service_healthy
      redpanda:
        condition: service_healthy
    networks:
      - agent-net
```

## Configuration

### Environment Variables

Configure services via `.env` file:

```bash
# Database
POSTGRES_USER=postgres
POSTGRES_PASSWORD=password
POSTGRES_DB=agent_system
POSTGRES_PORT=5434

# Redpanda
REDPANDA_ADVERTISE_KAFKA_ADDR=localhost:19092

# Grafana
GF_SECURITY_ADMIN_USER=admin
GF_SECURITY_ADMIN_PASSWORD=your-secure-password
GF_INSTALL_PLUGINS=grafana-piechart-panel

# Jaeger
JAEGER_COLLECTOR_OTLP_ENABLED=true

# Application
RUST_LOG=info,agent_patterns=debug
METRICS_PORT=9090
API_PORT=8080
```

Then reference in docker-compose.yml:

```yaml
services:
  postgres:
    environment:
      POSTGRES_USER: ${POSTGRES_USER}
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
```

### Volume Mounts

Data is persisted in named volumes:

```bash
# List volumes
docker volume ls | grep agent-

# Inspect volume
docker volume inspect agent-postgres-data

# Backup volume
docker run --rm \
  -v agent-postgres-data:/data \
  -v $(pwd):/backup \
  alpine tar czf /backup/postgres-backup.tar.gz -C /data .

# Restore volume
docker run --rm \
  -v agent-postgres-data:/data \
  -v $(pwd):/backup \
  alpine tar xzf /backup/postgres-backup.tar.gz -C /data
```

### Network Configuration

Services communicate via `agent-net` bridge network:

```bash
# Inspect network
docker network inspect agent-network

# Add external container to network
docker run --network agent-network my-service
```

## Monitoring

### Grafana Dashboards

1. **Import dashboards automatically** (via provisioning):
   - Dashboards in `./dashboards/` are auto-imported
   - Edit `docker-compose.yml` to add dashboard path:
     ```yaml
     volumes:
       - ./dashboards:/etc/grafana/provisioning/dashboards:ro
     ```

2. **Manual import**:
   - Navigate to http://localhost:3000
   - Go to **Dashboards** → **Import**
   - Upload `agent-overview.json` or `performance-slo.json`

### Prometheus Queries

Access Prometheus UI at http://localhost:9090 to run queries:

```promql
# Agent execution rate
rate(agent_execution_duration_seconds_count[5m])

# P99 latency by agent
histogram_quantile(0.99, sum(rate(agent_execution_duration_seconds_bucket[5m])) by (le, agent_name))

# Error rate
sum(rate(agent_errors_total[5m])) by (error_type)

# Active agents
agent_active_count
```

### Distributed Tracing

1. Access Jaeger UI: http://localhost:16686
2. Select service from dropdown
3. Click **Find Traces**
4. Click trace to view span details

### Health Checks

Check service health:

```bash
# All services
docker-compose ps

# Specific service logs
docker-compose logs postgres

# Database connection
docker-compose exec postgres psql -U postgres -d agent_system -c "SELECT 1;"

# Redpanda status
docker-compose exec redpanda rpk cluster health

# Redis ping
docker-compose exec redis redis-cli ping
```

## Troubleshooting

### Common Issues

#### 1. Port Already in Use

**Error**: `Bind for 0.0.0.0:5434 failed: port is already allocated`

**Solution**:
```bash
# Find process using port
lsof -i :5434

# Kill process or change port in docker-compose.yml
```

#### 2. Out of Disk Space

**Error**: `no space left on device`

**Solution**:
```bash
# Remove unused containers, images, volumes
docker system prune -a --volumes

# Check Docker disk usage
docker system df
```

#### 3. Postgres Connection Refused

**Error**: `connection refused` when connecting to PostgreSQL

**Solution**:
```bash
# Check if container is running
docker-compose ps postgres

# Check logs
docker-compose logs postgres

# Verify health check
docker-compose exec postgres pg_isready -U postgres

# Wait for service to be ready
docker-compose up -d postgres && \
  until docker-compose exec postgres pg_isready -U postgres; do sleep 1; done
```

#### 4. Redpanda Not Starting

**Error**: Redpanda container exits immediately

**Solution**:
```bash
# Check logs
docker-compose logs redpanda

# Redpanda needs sufficient memory
# Ensure Docker has at least 4GB RAM allocated

# Try restarting
docker-compose restart redpanda
```

#### 5. Grafana Dashboards Not Loading

**Error**: Dashboards folder is empty

**Solution**:
```bash
# Verify volume mount
docker-compose exec grafana ls /etc/grafana/provisioning/dashboards

# Check dashboard JSON syntax
cat dashboards/agent-overview.json | jq .

# Restart Grafana
docker-compose restart grafana
```

### Debug Mode

Enable verbose logging:

```yaml
# docker-compose.override.yml
version: "3.9"

services:
  agent-app:
    environment:
      RUST_LOG: trace
      RUST_BACKTRACE: 1
```

Run with override:
```bash
docker-compose -f docker-compose.yml -f docker-compose.override.yml up
```

### Resource Limits

If services are slow, increase resources:

```yaml
services:
  postgres:
    deploy:
      resources:
        limits:
          cpus: '2'
          memory: 2G
        reservations:
          cpus: '0.5'
          memory: 512M
```

## Production Deployment

### Security Hardening

1. **Change default passwords**:
   ```yaml
   environment:
     POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}  # Use secrets
     GF_SECURITY_ADMIN_PASSWORD: ${GRAFANA_PASSWORD}
   ```

2. **Use secrets** instead of environment variables:
   ```yaml
   secrets:
     postgres_password:
       external: true

   services:
     postgres:
       secrets:
         - postgres_password
       environment:
         POSTGRES_PASSWORD_FILE: /run/secrets/postgres_password
   ```

3. **Disable unnecessary ports**:
   ```yaml
   # Remove external port mappings in production
   # Access services via internal network only
   ```

4. **Run as non-root** (already configured in Dockerfile):
   ```dockerfile
   USER agent
   ```

### Resource Planning

**Minimum Production Requirements**:
- **CPU**: 4 cores
- **RAM**: 8GB
- **Disk**: 50GB (SSD recommended)
- **Network**: 1 Gbps

**Recommended Configuration**:
```yaml
services:
  postgres:
    deploy:
      resources:
        limits:
          memory: 2G
        reservations:
          memory: 1G

  redpanda:
    deploy:
      resources:
        limits:
          memory: 4G
        reservations:
          memory: 2G
```

### Backup Strategy

```bash
# Automated backup script
#!/bin/bash
BACKUP_DIR=/backups/$(date +%Y%m%d)
mkdir -p $BACKUP_DIR

# Backup PostgreSQL
docker-compose exec -T postgres pg_dump -U postgres agent_system \
  > $BACKUP_DIR/postgres.sql

# Backup volumes
docker run --rm \
  -v agent-postgres-data:/data \
  -v $BACKUP_DIR:/backup \
  alpine tar czf /backup/postgres-data.tar.gz -C /data .

# Cleanup old backups (keep last 7 days)
find /backups -type d -mtime +7 -exec rm -rf {} \;
```

Schedule with cron:
```cron
0 2 * * * /usr/local/bin/backup-agent-system.sh
```

### Health Monitoring

Production docker-compose with health monitoring:

```yaml
services:
  agent-app:
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:9090/health/liveness"]
      interval: 30s
      timeout: 3s
      start_period: 40s
      retries: 3
    restart: unless-stopped
    logging:
      driver: "json-file"
      options:
        max-size: "10m"
        max-file: "3"
```

### Deployment Checklist

- [ ] Change all default passwords
- [ ] Configure TLS/SSL certificates
- [ ] Set up automated backups
- [ ] Configure log rotation
- [ ] Set resource limits
- [ ] Enable health checks
- [ ] Configure monitoring alerts
- [ ] Test disaster recovery procedure
- [ ] Document runbook procedures
- [ ] Set up CI/CD pipeline

## Additional Resources

- [Docker Best Practices](https://docs.docker.com/develop/dev-best-practices/)
- [Docker Compose Reference](https://docs.docker.com/compose/compose-file/)
- [PostgreSQL Docker Hub](https://hub.docker.com/_/postgres)
- [Redpanda Documentation](https://docs.redpanda.com/)
- [Grafana Provisioning](https://grafana.com/docs/grafana/latest/administration/provisioning/)
- [Prometheus Configuration](https://prometheus.io/docs/prometheus/latest/configuration/configuration/)

## Support

For issues specific to agent-patterns Docker setup:
1. Check service logs: `docker-compose logs <service>`
2. Verify health: `docker-compose ps`
3. Review this guide's troubleshooting section
4. Check main agent-patterns documentation
