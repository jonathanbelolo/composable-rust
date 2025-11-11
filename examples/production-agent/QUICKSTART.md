# Production Agent - Quick Start Guide

Deploy your production-ready AI agent with Claude API in minutes!

---

## 🚀 Fastest Way: Deploy to Fly.io (5 Minutes)

**What you'll get**: Global agent running in Paris with <50ms latency

### Prerequisites
- Anthropic API key (get one at https://console.anthropic.com/)
- Fly.io account (sign up at https://fly.io/app/sign-up)

### Deploy Now

```bash
# 1. Install Fly.io CLI
curl -L https://fly.io/install.sh | sh

# 2. Run the setup script
cd examples/production-agent
./deploy/scripts/deploy-fly.sh setup

# 3. Deploy!
./deploy/scripts/deploy-fly.sh deploy

# 4. Test it
./deploy/scripts/deploy-fly.sh open
```

**That's it!** Your agent is live. 🎉

See [deploy/fly/DEPLOY.md](deploy/fly/DEPLOY.md) for full documentation.

---

## 💻 Local Development: Docker Compose

**What you'll get**: Run everything locally for testing

```bash
# 1. Set your API key
cp .env.example .env
# Edit .env and add: ANTHROPIC_API_KEY=sk-ant-...

# 2. Run locally (without infrastructure for now)
cargo run

# 3. Test
curl -X POST http://localhost:8080/chat \
  -H "Content-Type: application/json" \
  -d '{
    "user_id": "test",
    "session_id": "session1",
    "message": "Hello!"
  }'
```

**Full Docker Compose** (with PostgreSQL, Redis, Redpanda) coming in Phase 9 Part 8!

---

## 🌍 Go Global (Add More Regions)

Already deployed to Paris? Add more regions:

```bash
# Add Tokyo
./deploy/scripts/deploy-fly.sh regions add nrt
./deploy/scripts/deploy-fly.sh deploy

# Add San Francisco
./deploy/scripts/deploy-fly.sh regions add sjc
./deploy/scripts/deploy-fly.sh deploy

# Add New York
./deploy/scripts/deploy-fly.sh regions add ewr
./deploy/scripts/deploy-fly.sh deploy

# Now you're serving globally! 🌍
```

---

## 📊 Monitor Your Agent

```bash
# View status
./deploy/scripts/deploy-fly.sh status

# Stream logs
./deploy/scripts/deploy-fly.sh logs

# Open dashboard
fly dashboard
```

---

## 💰 Cost Estimates

| Deployment | Monthly Cost | What You Get |
|------------|--------------|--------------|
| **Paris only** | $3-30 | 1 region, good for EU |
| **Paris + Tokyo** | $50-100 | 2 regions, EU + Asia |
| **Global (4 regions)** | $150-250 | Worldwide <50ms latency |

**Free tier available**: $5/month credit (covers small deployments)

---

## 🔧 Configuration

All configuration is in `.env`:

```bash
# Required
ANTHROPIC_API_KEY=sk-ant-api03-YOUR_KEY_HERE

# Optional (defaults work fine)
RUST_LOG=production_agent=info
HTTP_PORT=8080
METRICS_PORT=9090
```

---

## 📚 Documentation

- **Fly.io Deployment**: [deploy/fly/DEPLOY.md](deploy/fly/DEPLOY.md) - Complete guide
- **Kubernetes Deployment**: [deploy/k8s/](deploy/k8s/) - For advanced users
- **Platform Comparison**: [../../plans/phase-9/DEPLOYMENT-PLATFORMS.md](../../plans/phase-9/DEPLOYMENT-PLATFORMS.md)
- **Architecture**: [README.md](README.md) - Full details

---

## 🆘 Troubleshooting

### Agent not responding?
```bash
# Check logs
./deploy/scripts/deploy-fly.sh logs

# Check health
curl https://your-app.fly.dev/health
```

### API key not working?
```bash
# Verify secret is set
fly secrets list

# Update it
fly secrets set ANTHROPIC_API_KEY=sk-ant-YOUR_NEW_KEY
```

### Want to start over?
```bash
# Destroy and recreate
./deploy/scripts/deploy-fly.sh destroy
./deploy/scripts/deploy-fly.sh setup
```

---

## 🎯 What's Working Now

✅ **Phase 9 Complete (Parts 1-3)**:
- Real Anthropic Claude API integration
- Environment-based configuration
- Fly.io deployment ready
- Resilience patterns (circuit breaker, rate limiting)
- Health checks and metrics
- Graceful shutdown

⏳ **Coming Soon (Parts 4+)**:
- PostgreSQL event store
- Redis sessions and cache
- Redpanda event bus
- Authentication (magic link, OAuth)
- WebSocket support
- Full Docker Compose stack

---

## 🚦 Quick Decision Guide

**Choose Fly.io if**:
- ✅ You want to deploy this week
- ✅ You want global distribution
- ✅ You're budget-conscious ($30-200/mo)
- ✅ You want simple DevOps

**Choose Kubernetes if**:
- ✅ You have K8s expertise
- ✅ You need multi-cloud
- ✅ You have a DevOps team
- ✅ You're spending >$2,000/mo

**Not sure?** Start with Fly.io. Easy to migrate later!

---

## 📞 Support

- **Fly.io Issues**: Check [deploy/fly/DEPLOY.md](deploy/fly/DEPLOY.md)
- **Framework Issues**: See main [README.md](README.md)
- **Questions**: Review [DEPLOYMENT-PLATFORMS.md](../../plans/phase-9/DEPLOYMENT-PLATFORMS.md)

---

**Ready to deploy?** → `./deploy/scripts/deploy-fly.sh setup`
