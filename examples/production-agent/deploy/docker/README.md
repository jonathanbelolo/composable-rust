# Docker Compose Deployment

Complete local development environment with all infrastructure components.

---

## What's Included

This Docker Compose setup provides a complete production agent stack:

- **Production Agent**: Your AI agent with Claude API integration
- **PostgreSQL 16**: Event store and audit logs
- **Redis 7**: Session store and caching
- **Redpanda**: Kafka-compatible event bus
- **Prometheus**: Metrics collection
- **Grafana**: Visualization dashboards
- **Jaeger**: Distributed tracing (optional)

---

## Quick Start

### 1. Prerequisites

```bash
# Install Docker and Docker Compose
# https://docs.docker.com/get-docker/

# Verify installation
docker --version
docker-compose --version
```

### 2. Configure Environment

```bash
# Copy environment template
cd examples/production-agent
cp .env.example .env

# Edit .env and set your Anthropic API key
# ANTHROPIC_API_KEY=sk-ant-api03-YOUR_KEY_HERE
```

### 3. Start All Services

```bash
# Start everything
docker-compose -f deploy/docker/docker-compose.yml up -d

# Watch logs
docker-compose -f deploy/docker/docker-compose.yml logs -f production-agent

# Check status
docker-compose -f deploy/docker/docker-compose.yml ps
```

### 4. Verify Deployment

```bash
# Health check
curl http://localhost:8080/health/live
curl http://localhost:8080/health/ready

# Test chat endpoint
curl -X POST http://localhost:8080/chat \
  -H "Content-Type: application/json" \
  -d '{
    "user_id": "test-user",
    "session_id": "test-session",
    "message": "Hello, what is Rust?"
  }'

# Check metrics
curl http://localhost:9090/metrics
```

### 5. Access UIs

- **Agent API**: http://localhost:8080
- **Prometheus**: http://localhost:9091
- **Grafana**: http://localhost:3000 (admin/admin)
- **Jaeger**: http://localhost:16686
- **Redpanda Console**: http://localhost:9644

---

## Service Details

### Production Agent

```yaml
Ports:
  - 8080: HTTP API
  - 9090: Prometheus metrics

Environment:
  - ANTHROPIC_API_KEY: Claude API key
  - DATABASE_URL: PostgreSQL connection
  - REDIS_URL: Redis connection
  - KAFKA_BROKERS: Redpanda brokers

Health Checks:
  - Startup: 30s
  - Interval: 10s
```

### PostgreSQL

```yaml
Port: 5432
Database: production_agent
User: postgres
Password: password

Volumes:
  - postgres-data: Persistent storage
  - init-db: Initialization scripts

Tables:
  - events: Event store
  - audit_logs: Audit trail
  - sessions: User sessions
  - users: User accounts
```

### Redis

```yaml
Port: 6379
Mode: AOF persistence

Volumes:
  - redis-data: Persistent storage

Health Check:
  - Command: redis-cli ping
  - Interval: 5s
```

### Redpanda

```yaml
Ports:
  - 9092: Kafka API
  - 9644: Admin API
  - 8081: Schema Registry

Volumes:
  - redpanda-data: Persistent storage

Configuration:
  - Single node (development)
  - Overprovisioned mode
```

---

## Common Operations

### View Logs

```bash
# All services
docker-compose -f deploy/docker/docker-compose.yml logs -f

# Specific service
docker-compose -f deploy/docker/docker-compose.yml logs -f production-agent
docker-compose -f deploy/docker/docker-compose.yml logs -f postgres
docker-compose -f deploy/docker/docker-compose.yml logs -f redis
docker-compose -f deploy/docker/docker-compose.yml logs -f redpanda
```

### Restart Services

```bash
# Restart agent only
docker-compose -f deploy/docker/docker-compose.yml restart production-agent

# Restart all
docker-compose -f deploy/docker/docker-compose.yml restart

# Rebuild and restart agent
docker-compose -f deploy/docker/docker-compose.yml up -d --build production-agent
```

### Stop Services

```bash
# Stop all
docker-compose -f deploy/docker/docker-compose.yml stop

# Stop specific service
docker-compose -f deploy/docker/docker-compose.yml stop production-agent
```

### Clean Up

```bash
# Stop and remove containers
docker-compose -f deploy/docker/docker-compose.yml down

# Remove volumes (DELETES ALL DATA)
docker-compose -f deploy/docker/docker-compose.yml down -v

# Remove everything including images
docker-compose -f deploy/docker/docker-compose.yml down -v --rmi all
```

### Database Access

```bash
# Connect to PostgreSQL
docker-compose -f deploy/docker/docker-compose.yml exec postgres psql -U postgres -d production_agent

# Run SQL query
docker-compose -f deploy/docker/docker-compose.yml exec postgres \
  psql -U postgres -d production_agent -c "SELECT COUNT(*) FROM events;"

# Backup database
docker-compose -f deploy/docker/docker-compose.yml exec postgres \
  pg_dump -U postgres production_agent > backup.sql

# Restore database
cat backup.sql | docker-compose -f deploy/docker/docker-compose.yml exec -T postgres \
  psql -U postgres -d production_agent
```

### Redis Access

```bash
# Connect to Redis CLI
docker-compose -f deploy/docker/docker-compose.yml exec redis redis-cli

# Check Redis info
docker-compose -f deploy/docker/docker-compose.yml exec redis redis-cli INFO

# List keys
docker-compose -f deploy/docker/docker-compose.yml exec redis redis-cli KEYS '*'
```

### Redpanda Operations

```bash
# List topics
docker-compose -f deploy/docker/docker-compose.yml exec redpanda \
  rpk topic list

# Create topic
docker-compose -f deploy/docker/docker-compose.yml exec redpanda \
  rpk topic create agent-events

# Consume messages
docker-compose -f deploy/docker/docker-compose.yml exec redpanda \
  rpk topic consume agent-events

# Cluster health
docker-compose -f deploy/docker/docker-compose.yml exec redpanda \
  rpk cluster health
```

---

## Development Workflow

### 1. Code Changes

```bash
# Edit your code
vim src/reducer.rs

# Rebuild and restart
docker-compose -f deploy/docker/docker-compose.yml up -d --build production-agent

# Watch logs
docker-compose -f deploy/docker/docker-compose.yml logs -f production-agent
```

### 2. Database Migrations

```bash
# Add new SQL file to init-db/
echo "CREATE TABLE new_table (...);" > deploy/docker/init-db/02-new-table.sql

# Recreate database (WARNING: deletes data)
docker-compose -f deploy/docker/docker-compose.yml down -v
docker-compose -f deploy/docker/docker-compose.yml up -d postgres
```

### 3. Testing

```bash
# Run tests inside container
docker-compose -f deploy/docker/docker-compose.yml exec production-agent \
  cargo test

# Run specific test
docker-compose -f deploy/docker/docker-compose.yml exec production-agent \
  cargo test test_reducer
```

---

## Troubleshooting

### Agent Won't Start

```bash
# Check logs
docker-compose -f deploy/docker/docker-compose.yml logs production-agent

# Common issues:
# 1. Missing API key
#    → Check ANTHROPIC_API_KEY in .env

# 2. Database not ready
#    → Wait for postgres health check to pass

# 3. Port conflicts
#    → Check if ports 8080, 5432, 6379, 9092 are free
```

### Database Connection Issues

```bash
# Verify PostgreSQL is running
docker-compose -f deploy/docker/docker-compose.yml ps postgres

# Test connection
docker-compose -f deploy/docker/docker-compose.yml exec postgres \
  pg_isready -U postgres

# Check connection from agent
docker-compose -f deploy/docker/docker-compose.yml exec production-agent \
  env | grep DATABASE_URL
```

### Redis Connection Issues

```bash
# Verify Redis is running
docker-compose -f deploy/docker/docker-compose.yml ps redis

# Test connection
docker-compose -f deploy/docker/docker-compose.yml exec redis redis-cli PING

# Check connection from agent
docker-compose -f deploy/docker/docker-compose.yml exec production-agent \
  env | grep REDIS_URL
```

### Redpanda Issues

```bash
# Check Redpanda status
docker-compose -f deploy/docker/docker-compose.yml exec redpanda \
  rpk cluster health

# View Redpanda logs
docker-compose -f deploy/docker/docker-compose.yml logs redpanda

# Restart Redpanda
docker-compose -f deploy/docker/docker-compose.yml restart redpanda
```

### Performance Issues

```bash
# Check resource usage
docker stats

# Increase Docker resources
# Docker Desktop → Preferences → Resources
# Recommended: 4 CPUs, 8GB RAM

# Check individual service
docker stats production-agent postgres redis redpanda
```

---

## Production Considerations

### Security

```bash
# Change default passwords in docker-compose.yml:
# - PostgreSQL: POSTGRES_PASSWORD
# - Grafana: GF_SECURITY_ADMIN_PASSWORD

# Use secrets management
# Don't commit .env file with real credentials

# Enable TLS for external access
# Use nginx or traefik as reverse proxy
```

### Persistence

```bash
# Backup volumes regularly
docker run --rm -v production-agent_postgres-data:/data \
  -v $(pwd):/backup alpine \
  tar czf /backup/postgres-backup.tar.gz /data

# Restore volume
docker run --rm -v production-agent_postgres-data:/data \
  -v $(pwd):/backup alpine \
  tar xzf /backup/postgres-backup.tar.gz -C /
```

### Scaling

```bash
# Scale agent horizontally
docker-compose -f deploy/docker/docker-compose.yml up -d --scale production-agent=3

# Note: Requires load balancer (nginx, traefik, etc.)
```

---

## Migration to Production

When ready to deploy to production:

1. **Use managed services**:
   - PostgreSQL: AWS RDS, Google Cloud SQL
   - Redis: AWS ElastiCache, Redis Cloud
   - Redpanda: Redpanda Cloud

2. **Deploy agent**:
   - Fly.io: See `deploy/fly/DEPLOY.md`
   - Kubernetes: See `deploy/k8s/`

3. **Update connection strings**:
   ```bash
   DATABASE_URL=postgres://user:pass@managed-postgres:5432/db
   REDIS_URL=redis://managed-redis:6379
   KAFKA_BROKERS=managed-redpanda:9092
   ```

---

## Useful Commands

```bash
# Quick status check
docker-compose -f deploy/docker/docker-compose.yml ps

# Resource usage
docker-compose -f deploy/docker/docker-compose.yml top

# Execute command in container
docker-compose -f deploy/docker/docker-compose.yml exec production-agent bash

# Follow all logs
docker-compose -f deploy/docker/docker-compose.yml logs -f --tail=100

# Prune unused Docker resources
docker system prune -a --volumes
```

---

## Support

- **Docker Issues**: Check [Docker documentation](https://docs.docker.com/)
- **Agent Issues**: See main [README.md](../../README.md)
- **Deployment Options**: See [QUICKSTART.md](../../QUICKSTART.md)
