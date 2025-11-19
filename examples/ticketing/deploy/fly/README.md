# Fly.io Deployment Guide

Complete guide for deploying the Composable Rust Ticketing application to Fly.io.

## Prerequisites

1. **Fly.io Account**: Sign up at [fly.io](https://fly.io)
2. **Flyctl CLI**: Install the Fly.io command-line tool
   ```bash
   # macOS/Linux
   curl -L https://fly.io/install.sh | sh

   # Or via Homebrew
   brew install flyctl
   ```
3. **Authentication**: Log in to Fly.io
   ```bash
   flyctl auth login
   ```

## Architecture Overview

The ticketing application requires:

- **Application Server**: Event-sourced ticketing API (this deployment)
- **PostgreSQL** (×3): Event store, projections database, auth database
- **Redpanda/Kafka**: Event bus for saga coordination
- **Redis** (optional): Session storage (future)

## Step-by-Step Deployment

### 1. Create PostgreSQL Databases

Create three separate PostgreSQL instances for proper CQRS separation:

```bash
# Event Store (write side - event-sourced aggregates)
flyctl postgres create \
  --name ticketing-events \
  --region sjc \
  --initial-cluster-size 1 \
  --vm-size shared-cpu-1x \
  --volume-size 10

# Projections Database (read side - CQRS read models)
flyctl postgres create \
  --name ticketing-projections \
  --region sjc \
  --initial-cluster-size 1 \
  --vm-size shared-cpu-1x \
  --volume-size 10

# Auth Database (user authentication and sessions)
flyctl postgres create \
  --name ticketing-auth \
  --region sjc \
  --initial-cluster-size 1 \
  --vm-size shared-cpu-1x \
  --volume-size 10
```

**Note**: `sjc` = San Jose, California. Choose a region close to your users:
- `sjc` - San Jose (US West)
- `ewr` - New Jersey (US East)
- `lhr` - London (Europe)
- `syd` - Sydney (Australia)

### 2. Create Fly.io Application

```bash
# From workspace root (not examples/ticketing/)
cd /path/to/composable-rust

# Create app (reserves the name)
flyctl apps create composable-ticketing --org personal
```

**Note**: The app name must match `app = "composable-ticketing"` in `fly.toml`.

### 3. Attach Databases

Connect the three PostgreSQL databases to your application:

```bash
# Event Store
flyctl postgres attach \
  --app composable-ticketing \
  ticketing-events \
  --database-name events \
  --variable-name DATABASE_URL

# Projections Database
flyctl postgres attach \
  --app composable-ticketing \
  ticketing-projections \
  --database-name projections \
  --variable-name PROJECTION_DATABASE_URL

# Auth Database
flyctl postgres attach \
  --app composable-ticketing \
  ticketing-auth \
  --database-name auth \
  --variable-name AUTH_DATABASE_URL
```

This sets environment variables:
- `DATABASE_URL` → Event store connection string
- `PROJECTION_DATABASE_URL` → Projections DB connection string
- `AUTH_DATABASE_URL` → Auth DB connection string

### 4. Set Secrets

Generate and set authentication secrets:

```bash
# JWT signing secret (32 bytes = 64 hex chars)
flyctl secrets set AUTH_JWT_SECRET=$(openssl rand -hex 32)

# Signing key for magic links (32 bytes)
flyctl secrets set AUTH_SIGNING_KEY=$(openssl rand -hex 32)
```

### 5. Configure Event Bus (Redpanda/Kafka)

**Option A: Local Deployment (MVP/Demo)**

For testing, use console mode (events logged to stdout):

```bash
# Already configured in fly.toml:
# EMAIL_PROVIDER=console
```

**Option B: Upstash Kafka (Managed Kafka)**

1. Sign up at [upstash.com](https://upstash.com)
2. Create a Kafka cluster (free tier available)
3. Get connection details and set secrets:

```bash
flyctl secrets set \
  REDPANDA_BROKERS=your-cluster.upstash.io:9092 \
  REDPANDA_SASL_USERNAME=your-username \
  REDPANDA_SASL_PASSWORD=your-password \
  REDPANDA_SASL_MECHANISM=SCRAM-SHA-256
```

**Option C: Self-Hosted Redpanda on Fly.io**

See `deploy/fly/fly-redpanda.toml` (TODO: create separate guide).

### 6. Configure Email Provider

**Option A: Console Mode (Development)**

Already configured in `fly.toml`. Magic links will be logged to stdout.

```bash
# Check logs for magic links
flyctl logs
```

**Option B: SMTP (Production)**

Use SendGrid, Mailgun, or another SMTP provider:

```bash
# Example: SendGrid
flyctl secrets set \
  EMAIL_PROVIDER=smtp \
  SMTP_HOST=smtp.sendgrid.net \
  SMTP_PORT=587 \
  SMTP_USERNAME=apikey \
  SMTP_PASSWORD=your-sendgrid-api-key \
  EMAIL_FROM=noreply@yourdomain.com
```

### 7. Deploy Application

Deploy from the workspace root (NOT from `examples/ticketing/`):

```bash
# From composable-rust/ workspace root
flyctl deploy --config examples/ticketing/deploy/fly/fly.toml
```

This will:
1. Build the Docker image (multi-stage build)
2. Upload to Fly.io registry
3. Deploy to your region
4. Run database migrations automatically (via `migrations/` in Dockerfile)
5. Start the application

### 8. Verify Deployment

```bash
# Check application status
flyctl status

# Check logs
flyctl logs

# Test health endpoint
curl https://composable-ticketing.fly.dev/health

# Test readiness endpoint
curl https://composable-ticketing.fly.dev/ready
```

Expected response from `/health`:
```json
{
  "status": "healthy",
  "postgres": "connected",
  "timestamp": "2025-11-19T..."
}
```

### 9. Access the Application

Your app is now live at:
```
https://composable-ticketing.fly.dev
```

Test authentication:
```bash
# Request magic link
curl -X POST https://composable-ticketing.fly.dev/auth/magic-link/request \
  -H "Content-Type: application/json" \
  -d '{"email":"test@example.com"}'

# Check logs for magic link URL (console mode)
flyctl logs | grep "magic_link_url"
```

## Database Migrations

Migrations run automatically on deployment (via Dockerfile `COPY migrations/`).

To manually run migrations:

```bash
# SSH into the app
flyctl ssh console

# Run migrations
cd /app
DATABASE_URL=$DATABASE_URL sqlx migrate run
```

## Scaling

### Vertical Scaling (Larger VMs)

```bash
# Upgrade to 2 vCPUs, 2GB RAM
flyctl scale vm dedicated-cpu-2x
```

### Horizontal Scaling (Multiple Instances)

```bash
# Add more machines in same region
flyctl scale count 3

# Add machines in new regions
flyctl regions add lhr syd
flyctl scale count 5  # Distributed across regions
```

### Database Scaling

```bash
# Upgrade event store
flyctl postgres update --app ticketing-events --vm-size dedicated-cpu-2x

# Add read replicas for projections
flyctl postgres create \
  --name ticketing-projections-read \
  --region lhr \
  --initial-cluster-size 1
```

## Monitoring

### View Logs

```bash
# Live tail
flyctl logs

# Filter by level
flyctl logs --filter error

# Last 100 lines
flyctl logs --lines 100
```

### Metrics

```bash
# Application metrics
flyctl metrics

# PostgreSQL metrics
flyctl postgres metrics --app ticketing-events
```

### Prometheus Integration

The application exposes metrics on port 9090 (not publicly exposed by default).

To enable:
1. Update `fly.toml` to expose port 9090
2. Configure Fly Grafana: https://fly.io/docs/reference/metrics/

## Troubleshooting

### Application Won't Start

```bash
# Check logs
flyctl logs

# Common issues:
# - DATABASE_URL not set → run step 3
# - Migrations failed → check Postgres access
# - Port 8080 blocked → check fly.toml [http_service]
```

### Database Connection Errors

```bash
# Verify database attachment
flyctl postgres list

# Check connection strings
flyctl secrets list

# Test database connectivity
flyctl ssh console
psql $DATABASE_URL
```

### High Memory Usage

```bash
# Check current usage
flyctl vm status

# Scale up
flyctl scale vm shared-cpu-2x  # 1GB → 2GB RAM
```

## Cost Estimate

**Free Tier (Hobby Plan)**:
- 3 shared-cpu-1x VMs: $0/month (within free allowance)
- 3× PostgreSQL (10GB each): ~$5-10/month
- Bandwidth: 100GB free, then $0.02/GB

**Total**: ~$5-10/month for MVP deployment

**Production Tier**:
- 3× dedicated-cpu-2x VMs: ~$30/month
- 3× PostgreSQL (100GB each, HA): ~$50/month
- Upstash Kafka: ~$10-20/month

**Total**: ~$90-100/month for production

## Next Steps

1. **Custom Domain**: Configure DNS
   ```bash
   flyctl certs create yourdomain.com
   ```

2. **SSL Certificates**: Automatic via Let's Encrypt

3. **CI/CD**: Set up GitHub Actions
   ```yaml
   - name: Deploy to Fly.io
     uses: superfly/flyctl-actions@v1
     with:
       args: "deploy --config examples/ticketing/deploy/fly/fly.toml"
     env:
       FLY_API_TOKEN: ${{ secrets.FLY_API_TOKEN }}
   ```

4. **Monitoring**: Integrate with Sentry, Datadog, or Honeycomb

5. **Backup**: Configure PostgreSQL snapshots
   ```bash
   flyctl postgres backup list --app ticketing-events
   ```

## Resources

- Fly.io Docs: https://fly.io/docs/
- Fly Postgres: https://fly.io/docs/postgres/
- Scaling Guide: https://fly.io/docs/reference/scaling/
- Multi-Region: https://fly.io/docs/reference/regions/

## Support

- Fly.io Community: https://community.fly.io/
- Composable Rust Issues: https://github.com/your-org/composable-rust/issues
