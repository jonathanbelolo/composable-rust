# Production Agent Deployment Checklist

Complete checklist for deploying the production agent to various platforms.

---

## Pre-Deployment Checklist

### ☑️ Prerequisites

- [ ] Anthropic API key obtained from https://console.anthropic.com/
- [ ] Git repository up to date
- [ ] All tests passing (`cargo test --all-features`)
- [ ] No clippy warnings (`cargo clippy --all-targets --all-features`)
- [ ] Documentation reviewed

### ☑️ Configuration

- [ ] `.env` file created from `.env.example`
- [ ] `ANTHROPIC_API_KEY` set in `.env`
- [ ] Reviewed `config.toml` settings
- [ ] Secrets management strategy decided
- [ ] Logging level configured (RUST_LOG)

### ☑️ Infrastructure Decisions

- [ ] Deployment platform chosen (Fly.io, Kubernetes, Docker Compose)
- [ ] Regions selected for deployment
- [ ] Resource requirements estimated
- [ ] Budget approved
- [ ] Monitoring strategy defined

---

## Local Deployment (Docker Compose)

### ☑️ Setup

- [ ] Docker and Docker Compose installed
- [ ] `.env` file configured
- [ ] Ports available (8080, 5432, 6379, 9092, 9091, 3000, 16686)

### ☑️ Deployment Steps

```bash
# 1. Validate configuration
./deploy/scripts/validate-deployment.sh

# 2. Start services
./deploy/scripts/deploy-docker.sh up

# 3. Verify health
./deploy/scripts/deploy-docker.sh health

# 4. Test API
curl http://localhost:8080/health/live
curl -X POST http://localhost:8080/chat \
  -H "Content-Type: application/json" \
  -d '{"user_id":"test","session_id":"s1","message":"Hello"}'
```

- [ ] All services started successfully
- [ ] Health checks passing
- [ ] API responding correctly
- [ ] Metrics accessible at http://localhost:9090/metrics
- [ ] Prometheus UI accessible at http://localhost:9091
- [ ] Grafana accessible at http://localhost:3000

### ☑️ Post-Deployment

- [ ] Logs reviewed (`docker-compose logs`)
- [ ] Database initialized (`docker-compose exec postgres psql -U postgres -d production_agent -c '\dt'`)
- [ ] Redis accessible (`docker-compose exec redis redis-cli PING`)
- [ ] Redpanda healthy (`docker-compose exec redpanda rpk cluster health`)

---

## Fly.io Deployment

### ☑️ Prerequisites

- [ ] Fly.io account created (https://fly.io/app/sign-up)
- [ ] `flyctl` CLI installed
- [ ] Credit card added to Fly.io account (free tier available)
- [ ] Deployment region decided (default: cdg - Paris)

### ☑️ Initial Setup

```bash
# 1. Login to Fly.io
fly auth login

# 2. Run setup script
cd examples/production-agent
./deploy/scripts/deploy-fly.sh setup

# Enter your Anthropic API key when prompted
```

- [ ] Logged into Fly.io
- [ ] App created in chosen region
- [ ] Secrets configured (ANTHROPIC_API_KEY)

### ☑️ First Deployment

```bash
# Deploy the application
./deploy/scripts/deploy-fly.sh deploy
```

- [ ] Build completed successfully
- [ ] App deployed to region
- [ ] Health checks passing
- [ ] App accessible via Fly.io URL

### ☑️ Verification

```bash
# Check status
./deploy/scripts/deploy-fly.sh status

# View logs
./deploy/scripts/deploy-fly.sh logs

# Test API
APP_URL=$(fly info --json | jq -r .hostname)
curl https://$APP_URL/health/live
```

- [ ] App status shows "running"
- [ ] Logs show no errors
- [ ] API responding correctly
- [ ] HTTPS working (automatic TLS)

### ☑️ Scaling (Optional)

```bash
# Vertical scaling (more resources)
fly scale vm dedicated-cpu-1x

# Horizontal scaling (more instances)
fly scale count 2

# Regional scaling (global deployment)
fly regions add nrt  # Tokyo
fly regions add sjc  # San Francisco
fly regions add ewr  # New York
fly deploy
```

- [ ] Resources scaled as needed
- [ ] Multiple regions configured (if required)
- [ ] Deployment successful across all regions

### ☑️ Managed Services (When Ready)

```bash
# Add PostgreSQL
fly postgres create production-agent-db
fly postgres attach production-agent-db

# Add Redis
fly redis create production-agent-cache
fly redis attach production-agent-cache
```

- [ ] PostgreSQL created and attached (if needed)
- [ ] Redis created and attached (if needed)
- [ ] Database migrations run
- [ ] Connection strings verified

---

## Kubernetes Deployment

### ☑️ Prerequisites

- [ ] Kubernetes cluster available (GKE, EKS, AKS, or local)
- [ ] `kubectl` CLI installed and configured
- [ ] Cluster has sufficient resources
- [ ] Container registry accessible
- [ ] Ingress controller installed

### ☑️ Build and Push Image

```bash
# Build Docker image
./deploy/scripts/build.sh

# Tag for registry
docker tag production-agent:latest YOUR_REGISTRY/production-agent:latest

# Push to registry
docker push YOUR_REGISTRY/production-agent:latest
```

- [ ] Image built successfully
- [ ] Image tagged correctly
- [ ] Image pushed to registry
- [ ] Registry accessible from cluster

### ☑️ Configure Secrets

```bash
# Create namespace
kubectl create namespace production-agent

# Create secret for Anthropic API key
kubectl create secret generic anthropic-api-key \
  --from-literal=ANTHROPIC_API_KEY=sk-ant-your-key-here \
  -n production-agent
```

- [ ] Namespace created
- [ ] Secrets configured
- [ ] Secrets verified (`kubectl get secrets -n production-agent`)

### ☑️ Deploy Application

```bash
# Update image in deployment.yaml
# Set YOUR_REGISTRY/production-agent:latest

# Deploy
kubectl apply -f deploy/k8s/
```

- [ ] All manifests applied successfully
- [ ] Deployment created
- [ ] Service created
- [ ] ConfigMap created
- [ ] HPA created
- [ ] PDB created
- [ ] RBAC configured

### ☑️ Verification

```bash
# Check deployment
kubectl get deployments -n production-agent

# Check pods
kubectl get pods -n production-agent

# Check service
kubectl get svc -n production-agent

# View logs
kubectl logs -f deployment/production-agent -n production-agent

# Port forward for testing
kubectl port-forward svc/production-agent 8080:8080 -n production-agent
```

- [ ] Deployment shows ready replicas
- [ ] All pods are running
- [ ] Service has endpoints
- [ ] Logs show no errors
- [ ] API accessible via port-forward

### ☑️ Ingress Setup (Production)

```bash
# Configure ingress with your domain
# Edit deploy/k8s/ingress.yaml

kubectl apply -f deploy/k8s/ingress.yaml
```

- [ ] Ingress configured
- [ ] DNS pointing to load balancer
- [ ] TLS certificate obtained
- [ ] HTTPS working

---

## Post-Deployment Monitoring

### ☑️ Observability

- [ ] Metrics endpoint accessible (`/metrics`)
- [ ] Prometheus scraping metrics
- [ ] Grafana dashboards configured
- [ ] Alerts configured (if applicable)
- [ ] Log aggregation working
- [ ] Distributed tracing enabled (Jaeger)

### ☑️ Health Checks

- [ ] Liveness probe passing (`/health/live`)
- [ ] Readiness probe passing (`/health/ready`)
- [ ] Startup probe passing (if configured)
- [ ] Health check frequency appropriate

### ☑️ Performance

- [ ] Response times acceptable (<200ms for health checks)
- [ ] API latency within targets (<500ms for chat)
- [ ] Resource usage reasonable (CPU <70%, Memory <80%)
- [ ] No memory leaks observed
- [ ] Connection pools configured correctly

### ☑️ Security

- [ ] Secrets not exposed in logs
- [ ] HTTPS enabled (for external access)
- [ ] API rate limiting enabled
- [ ] Circuit breaker configured
- [ ] Security headers set
- [ ] Non-root user in container
- [ ] Regular security updates scheduled

---

## Operational Procedures

### ☑️ Backup and Recovery

- [ ] Database backup strategy defined
- [ ] Backup schedule configured
- [ ] Restore procedure tested
- [ ] Disaster recovery plan documented

### ☑️ Rollback Plan

- [ ] Previous version tagged
- [ ] Rollback procedure documented
- [ ] Team trained on rollback process

### ☑️ Scaling

- [ ] Horizontal Pod Autoscaler configured (K8s)
- [ ] Scaling limits defined
- [ ] Cost implications understood
- [ ] Performance tested at scale

### ☑️ Maintenance

- [ ] Update schedule defined
- [ ] Maintenance windows communicated
- [ ] Zero-downtime deployment verified
- [ ] Team on-call schedule established

---

## Troubleshooting Guide

### Common Issues

#### Agent Won't Start

```bash
# Check logs
# Docker: docker-compose logs production-agent
# Fly.io: fly logs
# K8s: kubectl logs deployment/production-agent -n production-agent

# Common causes:
# - Missing ANTHROPIC_API_KEY
# - Database connection failure
# - Port conflict
# - Insufficient resources
```

#### Health Checks Failing

```bash
# Test health endpoint directly
curl http://localhost:8080/health/live

# Check container logs for errors
# Verify all dependencies are healthy (DB, Redis, etc.)
```

#### High Latency

```bash
# Check resource usage
# Verify database connection pool settings
# Check Claude API rate limits
# Review circuit breaker status
# Verify network latency to dependencies
```

#### Database Connection Issues

```bash
# Verify connection string
# Check database is running
# Verify network connectivity
# Check connection pool exhaustion
```

---

## Validation Script

Run the automated validation script to verify your deployment configuration:

```bash
./deploy/scripts/validate-deployment.sh
```

This script checks:
- ✅ All required files present
- ✅ Configuration files valid
- ✅ Deployment scripts executable
- ✅ Build succeeds
- ✅ Documentation complete

---

## Support and Resources

### Documentation

- [QUICKSTART.md](./QUICKSTART.md) - 5-minute deployment guide
- [README.md](./README.md) - Complete application documentation
- [deploy/fly/DEPLOY.md](./deploy/fly/DEPLOY.md) - Fly.io deployment guide
- [deploy/docker/README.md](./deploy/docker/README.md) - Docker Compose guide
- [DEPLOYMENT-PLATFORMS.md](../../plans/phase-9/DEPLOYMENT-PLATFORMS.md) - Platform comparison

### Platform Support

- **Fly.io**: https://community.fly.io/
- **Kubernetes**: https://kubernetes.io/docs/
- **Docker**: https://docs.docker.com/

### Application Support

- **Repository**: https://github.com/your-org/composable-rust
- **Issues**: https://github.com/your-org/composable-rust/issues

---

## Sign-Off

### Deployment Approval

- [ ] Technical review completed
- [ ] Security review completed
- [ ] Stakeholder approval obtained
- [ ] Budget approved
- [ ] Rollback plan reviewed

**Deployed by**: ________________
**Date**: ________________
**Environment**: ________________
**Version**: ________________

**Verified by**: ________________
**Date**: ________________

---

**Ready for Production?**

If all checkboxes are marked ✅, you're ready to deploy to production! 🚀
