# Deploy Production Agent to Fly.io

Complete guide for deploying the production agent to Fly.io with global distribution.

---

## Prerequisites

1. **Fly.io Account**
   ```bash
   # Sign up at https://fly.io/app/sign-up

   # Install flyctl CLI
   # macOS/Linux:
   curl -L https://fly.io/install.sh | sh

   # Add to PATH (add to ~/.zshrc or ~/.bashrc)
   export FLYCTL_INSTALL="$HOME/.fly"
   export PATH="$FLYCTL_INSTALL/bin:$PATH"
   ```

2. **Anthropic API Key**
   - Get your API key from https://console.anthropic.com/
   - You'll set this as a Fly.io secret

3. **Credit Card** (for Fly.io)
   - Free tier available: $5/month credit
   - Only charged if you exceed free tier

---

## Quick Start (Deploy to Paris in 5 Minutes)

```bash
# 1. Navigate to production-agent directory
cd examples/production-agent

# 2. Login to Fly.io
fly auth login

# 3. Create the app (one-time setup)
fly apps create production-agent --region cdg

# 4. Set your Anthropic API key
fly secrets set ANTHROPIC_API_KEY=sk-ant-api03-YOUR_KEY_HERE

# 5. Deploy!
fly deploy

# 6. Open in browser
fly open

# 7. Check status
fly status
```

**That's it!** Your agent is now running in Paris. 🇫🇷

---

## Detailed Deployment Steps

### Step 1: Install and Authenticate

```bash
# Install flyctl
curl -L https://fly.io/install.sh | sh

# Login (opens browser)
fly auth login

# Verify installation
fly version
```

### Step 2: Create Your Application

```bash
# Create app in Paris (Europe)
fly apps create production-agent --region cdg

# OR choose a different region:
# fly apps create production-agent --region nrt  # Tokyo
# fly apps create production-agent --region sjc  # San Francisco
# fly apps create production-agent --region ewr  # New York

# View available regions
fly platform regions
```

### Step 3: Set Secrets

```bash
# Anthropic API key (required)
fly secrets set ANTHROPIC_API_KEY=sk-ant-api03-YOUR_ACTUAL_KEY_HERE

# Optional: PostgreSQL URL (if using external DB)
# fly secrets set DATABASE_URL=postgres://user:pass@host/db

# Optional: Redis URL (if using external Redis)
# fly secrets set REDIS_URL=redis://host:6379

# List all secrets (values are hidden)
fly secrets list
```

### Step 4: Deploy

```bash
# Deploy from current directory
fly deploy

# Watch the deployment
# Fly.io will:
# 1. Build your Dockerfile
# 2. Push to Fly.io registry
# 3. Create VM
# 4. Start your app
# 5. Run health checks

# This takes 2-5 minutes
```

### Step 5: Verify Deployment

```bash
# Check app status
fly status

# View logs
fly logs

# Check health
fly checks

# Open in browser
fly open
```

---

## Test Your Deployment

```bash
# Get your app URL
APP_URL=$(fly info --json | jq -r .hostname)

# Test health endpoint
curl https://$APP_URL/health

# Test chat endpoint
curl -X POST https://$APP_URL/chat \
  -H "Content-Type: application/json" \
  -d '{
    "user_id": "test-user",
    "session_id": "test-session",
    "message": "What is Rust?"
  }'

# You should get a response from Claude! 🎉
```

---

## Add Database (PostgreSQL)

Fly.io provides managed PostgreSQL with automatic backups and replication.

```bash
# Create PostgreSQL cluster
fly postgres create production-agent-db --region cdg

# This creates:
# - 1 primary database in Paris
# - Automatic daily backups
# - 1GB storage (expandable)

# Attach to your app (automatically sets DATABASE_URL secret)
fly postgres attach production-agent-db --app production-agent

# Check database status
fly postgres db list --app production-agent-db

# Connect to database (for debugging)
fly postgres connect --app production-agent-db
```

**Database Configuration**:
- Free tier: 256MB RAM, 1GB storage
- Paid: Starting at $15/mo for 1GB RAM, 10GB storage
- Includes: Daily backups, point-in-time recovery, monitoring

---

## Add Redis (Cache/Sessions)

```bash
# Create Redis instance
fly redis create production-agent-cache --region cdg

# Attach to your app (sets REDIS_URL secret)
fly redis attach production-agent-cache --app production-agent

# Check Redis status
fly redis status production-agent-cache

# Get connection info
fly redis connect production-agent-cache
```

**Redis Configuration**:
- Free tier: 256MB RAM
- Paid: Starting at $10/mo for 1GB RAM
- Eviction policy: allkeys-lru (automatic)

---

## Scale Your Application

### Vertical Scaling (Bigger Machines)

```bash
# Check current resources
fly scale show

# Scale up to 512MB RAM
fly scale memory 512

# Scale to dedicated CPU (better performance)
fly scale vm dedicated-cpu-1x

# Available VM sizes:
# shared-cpu-1x:      1 vCPU, 256MB RAM  (~$2/mo)
# shared-cpu-2x:      1 vCPU, 512MB RAM  (~$5/mo)
# dedicated-cpu-1x:   1 vCPU, 2GB RAM    (~$23/mo)
# dedicated-cpu-2x:   2 vCPU, 4GB RAM    (~$46/mo)
```

### Horizontal Scaling (More Instances)

```bash
# Scale to 3 instances (for high availability)
fly scale count 3

# Scale to specific count per region
fly scale count 2 --region cdg
fly scale count 1 --region nrt

# Auto-scaling (coming soon to Fly.io)
# For now, use manual scaling based on metrics
```

---

## Multi-Region Deployment (Going Global!)

### Add Tokyo Region

```bash
# Add Tokyo as a deployment region
fly regions add nrt

# Deploy to both Paris and Tokyo
fly deploy

# Fly.io automatically:
# - Routes users to nearest region (Anycast)
# - Balances load across regions
# - Handles TLS in all regions

# Check where your instances are running
fly status
```

### Add More Regions

```bash
# Add San Francisco (US West)
fly regions add sjc

# Add New York (US East)
fly regions add ewr

# Deploy to all regions
fly deploy

# Now you're serving users globally with <50ms latency! 🌍
```

### Regional Database Replication

```bash
# Add read replica in Tokyo
fly postgres attach production-agent-db --region nrt

# Add read replica in San Francisco
fly postgres attach production-agent-db --region sjc

# Fly.io automatically:
# - Routes writes to primary (Paris)
# - Routes reads to nearest replica
# - Keeps replicas in sync
```

---

## Monitoring and Observability

### View Logs

```bash
# Stream live logs
fly logs

# Filter by instance
fly logs --instance 06e82da4d13908

# Last 100 lines
fly logs --lines 100

# Search logs
fly logs | grep ERROR
```

### Metrics

```bash
# View metrics in dashboard
fly dashboard

# Or access Prometheus metrics endpoint
fly proxy 9090
# Then visit http://localhost:9090/metrics
```

### Health Checks

```bash
# View health check status
fly checks

# Debug failing health checks
fly logs | grep health
```

### SSH Into Instance

```bash
# SSH into running instance (for debugging)
fly ssh console

# Run command
fly ssh console -C "ps aux"

# Check environment
fly ssh console -C "env"
```

---

## Cost Optimization

### Free Tier Usage

**What's Free**:
- Up to 3 shared-cpu-1x machines (256MB each)
- 3GB persistent volume storage
- 160GB outbound data transfer/month

**Strategy for Free Tier**:
```bash
# Use shared CPU
fly scale vm shared-cpu-1x

# Use 1-2 instances
fly scale count 1

# Use free Postgres dev tier
fly postgres create --vm-size shared-cpu-1x
```

### Paid Tier Costs

**Example: Paris Only**
```
App: shared-cpu-1x (256MB)        $3/mo
PostgreSQL: postgres-flex-1x      $15/mo
Redis: 1GB                        $10/mo
Total:                            $28/mo
```

**Example: Global (4 regions)**
```
App: 4x dedicated-cpu-1x          $92/mo
PostgreSQL: 1 primary + 3 replicas $60/mo
Redis: 4x 1GB                     $40/mo
Total:                            $192/mo
```

### Cost Saving Tips

1. **Start Small**: Use shared-cpu in dev, dedicated-cpu in prod
2. **Scale Incrementally**: Add regions only when you have users there
3. **Use Autostop**: Machines stop when idle (good for dev/staging)
4. **Monitor Usage**: `fly dashboard` shows costs in real-time
5. **Set Billing Alerts**: Get notified at spending thresholds

---

## Continuous Deployment

### GitHub Actions

Create `.github/workflows/fly-deploy.yml`:

```yaml
name: Deploy to Fly.io

on:
  push:
    branches: [main]
  workflow_dispatch:

env:
  FLY_API_TOKEN: ${{ secrets.FLY_API_TOKEN }}

jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: superfly/flyctl-actions/setup-flyctl@master

      - name: Deploy to Fly.io
        run: flyctl deploy --remote-only
        working-directory: ./examples/production-agent
```

**Setup**:
```bash
# Get your Fly.io API token
fly auth token

# Add to GitHub Secrets as FLY_API_TOKEN
# Settings → Secrets → Actions → New repository secret
```

Now every push to main automatically deploys! 🚀

---

## Troubleshooting

### App Won't Start

```bash
# Check logs for errors
fly logs

# Common issues:
# 1. Missing secrets
fly secrets list

# 2. Port not matching
# Make sure Dockerfile EXPOSE 8080 matches fly.toml internal_port

# 3. Health check failing
curl https://your-app.fly.dev/health/live
```

### Database Connection Issues

```bash
# Check DATABASE_URL secret is set
fly secrets list | grep DATABASE_URL

# Test database connection
fly postgres connect --app production-agent-db

# Check database is running
fly status --app production-agent-db
```

### High Latency

```bash
# Check which regions you're deployed in
fly regions list

# Check instance location
fly status

# Solution: Add more regions
fly regions add nrt sjc ewr
fly deploy
```

### Out of Memory

```bash
# Check memory usage
fly ssh console -C "free -h"

# Scale up memory
fly scale memory 512

# Or upgrade VM
fly scale vm dedicated-cpu-1x
```

---

## Useful Commands Reference

```bash
# Deployment
fly deploy                      # Deploy app
fly deploy --strategy immediate # Deploy without health checks

# Scaling
fly scale count 3              # Scale to 3 instances
fly scale memory 512           # Scale to 512MB RAM
fly scale vm dedicated-cpu-1x  # Change VM type

# Regions
fly regions list               # Show current regions
fly regions add nrt            # Add Tokyo region
fly regions remove sjc         # Remove SF region

# Monitoring
fly status                     # App status
fly logs                       # Stream logs
fly checks                     # Health check status
fly dashboard                  # Open web dashboard

# Secrets
fly secrets set KEY=value      # Set secret
fly secrets list               # List secrets
fly secrets unset KEY          # Remove secret

# Database
fly postgres create            # Create database
fly postgres attach            # Attach to app
fly postgres connect           # Connect via psql

# SSH/Debug
fly ssh console                # SSH into instance
fly proxy 9090                 # Port forward

# Management
fly apps list                  # List all apps
fly apps destroy production-agent  # Delete app
fly info                       # Show app details
```

---

## Regional Deployment Strategy

### Recommended Rollout

**Week 1: Paris (Europe)**
```bash
fly apps create production-agent --region cdg
fly deploy
# Cost: ~$30/mo
# Serves: Europe with great latency
```

**Month 2: Add Tokyo (Asia)**
```bash
fly regions add nrt
fly deploy
# Cost: ~$60/mo
# Serves: Europe + Asia
```

**Month 3: Add North America**
```bash
fly regions add sjc ewr
fly deploy
# Cost: ~$200/mo
# Serves: Global
```

### Global Latency Targets

With 4 regions (Paris, Tokyo, SF, NYC):
```
User Location   → Nearest Region → Latency
─────────────────────────────────────────────
Paris          → Paris (CDG)     → 10-20ms ✅
London         → Paris (CDG)     → 15-25ms ✅
Berlin         → Paris (CDG)     → 20-30ms ✅
Tokyo          → Tokyo (NRT)     → 10-20ms ✅
Seoul          → Tokyo (NRT)     → 30-40ms ✅
Singapore      → Tokyo (NRT)     → 70-80ms ⚠️
San Francisco  → San Jose (SJC)  → 10-20ms ✅
Los Angeles    → San Jose (SJC)  → 15-25ms ✅
New York       → New York (EWR)  → 10-20ms ✅
Boston         → New York (EWR)  → 15-25ms ✅
```

---

## Migration from Fly.io to Kubernetes

If you outgrow Fly.io (>$2,000/mo), migration is straightforward:

```bash
# 1. Export database
fly postgres export production-agent-db > backup.sql

# 2. Deploy to K8s (we have manifests ready!)
kubectl apply -f ../k8s/

# 3. Import database
kubectl exec -it postgres-0 -- psql < backup.sql

# 4. Update DNS
# Point domain to K8s load balancer

# 5. Decommission Fly.io
fly apps destroy production-agent
```

---

## Security Checklist

- [ ] Use secrets for API keys (`fly secrets set`)
- [ ] Enable TLS (automatic with Fly.io)
- [ ] Set up private networking for databases
- [ ] Use strong PostgreSQL passwords
- [ ] Enable 2FA on Fly.io account
- [ ] Rotate secrets regularly
- [ ] Monitor logs for suspicious activity
- [ ] Set up billing alerts
- [ ] Use separate apps for dev/staging/prod

---

## Support and Resources

- **Fly.io Docs**: https://fly.io/docs/
- **Community Forum**: https://community.fly.io/
- **Status Page**: https://status.fly.io/
- **Support**: support@fly.io (email)
- **Our K8s Fallback**: `../k8s/` directory

---

## Summary

**What You Have Now**:
- ✅ Production-ready Fly.io configuration
- ✅ Simple deployment process (5 minutes)
- ✅ Global deployment capability (4 regions)
- ✅ Cost-effective ($30-200/mo depending on scale)
- ✅ Easy to migrate to K8s if needed

**Next Steps**:
1. Deploy to Paris: `fly deploy`
2. Test with real users
3. Add regions as you expand
4. Monitor and optimize

Happy deploying! 🚀
