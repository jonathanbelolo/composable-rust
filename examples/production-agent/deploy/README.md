# Production Agent Deployment Guide

This directory contains deployment configurations and scripts for running the production-agent in various environments.

---

## 🚀 Quick Start

Choose your deployment method:

1. **Fly.io** (5 minutes) - Recommended for getting started
   ```bash
   ./scripts/deploy-fly.sh setup && ./scripts/deploy-fly.sh deploy
   ```
   See [fly/DEPLOY.md](./fly/DEPLOY.md)

2. **Docker Compose** (Local development)
   ```bash
   ./scripts/deploy-docker.sh up
   ```
   See [docker/README.md](./docker/README.md)

3. **Kubernetes** (Production at scale)
   ```bash
   ./scripts/deploy-k8s.sh deploy
   ```
   See [Kubernetes Deployment](#kubernetes-deployment) below

---

## 📋 Before You Deploy

**Run the validation script**:
```bash
./scripts/validate-deployment.sh
```

**Use the deployment checklist**: See [DEPLOYMENT-CHECKLIST.md](../DEPLOYMENT-CHECKLIST.md)

---

## Directory Structure

```
deploy/
├── docker/              # Docker Compose deployment (local)
│   ├── docker-compose.yml
│   ├── init-db/         # PostgreSQL initialization
│   ├── prometheus.yml
│   ├── grafana/
│   │   ├── datasources/
│   │   └── dashboards/
│   └── README.md
├── fly/                 # Fly.io deployment (cloud)
│   └── DEPLOY.md
├── k8s/                 # Kubernetes manifests (production)
│   ├── deployment.yaml
│   ├── service.yaml
│   ├── configmap.yaml
│   ├── rbac.yaml
│   ├── pdb.yaml
│   └── hpa.yaml
└── scripts/             # Deployment automation scripts
    ├── deploy-docker.sh
    ├── deploy-fly.sh
    ├── deploy-k8s.sh
    ├── build.sh
    └── validate-deployment.sh
```

## Quick Start

### Local Development with Docker

The fastest way to run the production-agent locally with full observability stack:

```bash
# Build the Docker image
./deploy/scripts/deploy-docker.sh build

# Start all services
./deploy/scripts/deploy-docker.sh up

# Check health
./deploy/scripts/deploy-docker.sh health

# View logs
./deploy/scripts/deploy-docker.sh logs

# Stop services
./deploy/scripts/deploy-docker.sh down
```

**Available Services:**
- Production Agent API: http://localhost:8080
- Production Agent Metrics: http://localhost:9090/metrics
- Prometheus UI: http://localhost:9091
- Grafana UI: http://localhost:3000 (admin/admin)
- Jaeger UI: http://localhost:16686

### Kubernetes Deployment

Deploy to a Kubernetes cluster:

```bash
# Deploy to default namespace
./deploy/scripts/deploy-k8s.sh deploy

# Deploy to specific namespace
NAMESPACE=production ./deploy/scripts/deploy-k8s.sh deploy

# Check status
./deploy/scripts/deploy-k8s.sh status

# View logs
./deploy/scripts/deploy-k8s.sh logs

# Port forward for local access
./deploy/scripts/deploy-k8s.sh port-forward
```

## Docker Deployment

### Architecture

The Docker Compose setup includes:

- **production-agent**: The main application (ports 8080, 9090)
- **prometheus**: Metrics collection (port 9091)
- **grafana**: Metrics visualization (port 3000)
- **jaeger**: Distributed tracing (port 16686)

### Configuration

Edit `docker/prometheus.yml` to customize Prometheus scraping:

```yaml
global:
  scrape_interval: 15s

scrape_configs:
  - job_name: 'production-agent'
    static_configs:
      - targets: ['production-agent:9090']
```

### Commands

```bash
# Build image
./deploy/scripts/deploy-docker.sh build

# Start services (detached)
./deploy/scripts/deploy-docker.sh up

# Stop services
./deploy/scripts/deploy-docker.sh down

# Restart production-agent only
./deploy/scripts/deploy-docker.sh restart

# Follow logs
./deploy/scripts/deploy-docker.sh logs

# Clean up (including volumes)
./deploy/scripts/deploy-docker.sh clean

# Check health of all services
./deploy/scripts/deploy-docker.sh health
```

### Testing

Once deployed, test the API:

```bash
# Health check
curl http://localhost:8080/health

# Liveness probe
curl http://localhost:8080/health/live

# Readiness probe
curl http://localhost:8080/health/ready

# Metrics
curl http://localhost:9090/metrics

# Send a chat message
curl -X POST http://localhost:8080/chat \
  -H "Content-Type: application/json" \
  -d '{
    "user_id": "user123",
    "session_id": "session456",
    "message": "Hello, agent!",
    "source_ip": "127.0.0.1"
  }'
```

## Kubernetes Deployment

### Prerequisites

- Kubernetes cluster (v1.24+)
- kubectl configured
- Metrics Server installed (for HPA)

### Components

The Kubernetes deployment includes:

1. **Deployment** (`deployment.yaml`)
   - 3 replicas by default
   - Rolling update strategy
   - Health checks (startup, liveness, readiness)
   - Resource limits and requests
   - Security context (non-root, read-only filesystem)

2. **Service** (`service.yaml`)
   - ClusterIP service for API (port 8080)
   - Headless service for metrics scraping (port 9090)

3. **ConfigMap** (`configmap.yaml`)
   - Application configuration

4. **RBAC** (`rbac.yaml`)
   - ServiceAccount
   - Role with minimal permissions
   - RoleBinding

5. **PodDisruptionBudget** (`pdb.yaml`)
   - Ensures at least 1 pod is available during disruptions

6. **HorizontalPodAutoscaler** (`hpa.yaml`)
   - Scales between 2-10 replicas
   - Based on CPU (70%) and memory (80%) utilization

### Deployment Steps

#### 1. Build and Push Image

```bash
# Build image
docker build -f Dockerfile -t production-agent:latest ../..

# Tag for your registry
docker tag production-agent:latest your-registry/production-agent:v1.0.0

# Push to registry
docker push your-registry/production-agent:v1.0.0

# Update deployment.yaml to use your image
# Edit k8s/deployment.yaml and change:
#   image: production-agent:latest
# to:
#   image: your-registry/production-agent:v1.0.0
```

#### 2. Deploy to Kubernetes

```bash
# Deploy to default namespace
./deploy/scripts/deploy-k8s.sh deploy

# Or deploy to specific namespace
NAMESPACE=production ./deploy/scripts/deploy-k8s.sh deploy
```

#### 3. Verify Deployment

```bash
# Check status
./deploy/scripts/deploy-k8s.sh status

# Check pod logs
./deploy/scripts/deploy-k8s.sh logs

# Check health
./deploy/scripts/deploy-k8s.sh health
```

#### 4. Access the Application

```bash
# Port forward to local machine
./deploy/scripts/deploy-k8s.sh port-forward

# In another terminal, test:
curl http://localhost:8080/health
```

### Scaling

```bash
# Manual scaling
./deploy/scripts/deploy-k8s.sh scale 5

# Check HPA status
kubectl get hpa production-agent
```

### Monitoring

#### Prometheus Integration

The pods are annotated for Prometheus scraping:

```yaml
annotations:
  prometheus.io/scrape: "true"
  prometheus.io/port: "9090"
  prometheus.io/path: "/metrics"
```

If you have Prometheus Operator installed, create a ServiceMonitor:

```yaml
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: production-agent
spec:
  selector:
    matchLabels:
      app: production-agent
  endpoints:
  - port: metrics
    interval: 30s
```

#### Grafana Dashboards

1. Access Grafana (port-forward or ingress)
2. Add Prometheus as data source
3. Import dashboards from `docker/grafana/dashboards/`

### Troubleshooting

#### Pods Not Starting

```bash
# Check pod events
kubectl describe pod <pod-name>

# Check logs
./deploy/scripts/deploy-k8s.sh logs <pod-name>

# Check resource constraints
kubectl top pods -l app=production-agent
```

#### Health Checks Failing

```bash
# Check startup probe
kubectl get events --sort-by='.lastTimestamp' | grep production-agent

# Port forward and test manually
kubectl port-forward <pod-name> 8080:8080
curl http://localhost:8080/health/live
```

#### HPA Not Scaling

```bash
# Check HPA status
kubectl get hpa production-agent

# Check metrics server
kubectl top nodes
kubectl top pods

# Describe HPA for details
kubectl describe hpa production-agent
```

### Updating the Deployment

```bash
# Update configuration
kubectl edit configmap production-agent-config

# Restart to pick up changes
./deploy/scripts/deploy-k8s.sh restart

# Or do a rolling update with new image
kubectl set image deployment/production-agent \
  production-agent=your-registry/production-agent:v1.1.0
```

### Cleanup

```bash
# Remove deployment
./deploy/scripts/deploy-k8s.sh undeploy

# Or delete entire namespace
kubectl delete namespace <namespace>
```

## Production Considerations

### Security

1. **Container Security**
   - Runs as non-root user (UID 1000)
   - Read-only root filesystem
   - No privilege escalation
   - Minimal capabilities

2. **RBAC**
   - Dedicated ServiceAccount
   - Minimal required permissions
   - No cluster-wide access

3. **Network Policies**
   - Consider adding NetworkPolicy to restrict traffic

### Observability

1. **Metrics**
   - Exposed on port 9090
   - Prometheus format
   - Custom agent metrics

2. **Tracing**
   - OpenTelemetry instrumentation
   - Jaeger exporter (optional)
   - Trace context propagation

3. **Logging**
   - Structured JSON logs
   - Configurable via RUST_LOG
   - Captured by container runtime

### Resource Management

1. **Resource Requests/Limits**
   ```yaml
   resources:
     requests:
       memory: "256Mi"
       cpu: "250m"
     limits:
       memory: "512Mi"
       cpu: "500m"
   ```

2. **Autoscaling**
   - HPA configured for 2-10 replicas
   - Scales on CPU (70%) and memory (80%)
   - Gradual scale-down (300s stabilization)

3. **Pod Disruption Budget**
   - Ensures 1 pod always available
   - Protects against voluntary disruptions

### High Availability

1. **Replication**
   - 3 replicas by default
   - Pod anti-affinity (preferred)
   - Rolling update strategy

2. **Health Checks**
   - Startup probe: 5 minutes timeout
   - Liveness probe: Process health
   - Readiness probe: Service readiness

3. **Graceful Shutdown**
   - 30s termination grace period
   - Coordinated shutdown via ShutdownCoordinator

### Disaster Recovery

1. **Configuration Backup**
   ```bash
   kubectl get configmap production-agent-config -o yaml > backup-config.yaml
   ```

2. **Deployment Backup**
   ```bash
   kubectl get all -l app=production-agent -o yaml > backup-deployment.yaml
   ```

3. **Monitoring Backup**
   - Export Prometheus data
   - Export Grafana dashboards

## Advanced Configuration

### Custom Configuration

Edit `k8s/configmap.yaml` to customize:

```yaml
data:
  config.toml: |
    [agent]
    name = "production-agent"
    max_concurrent_requests = 100

    [circuit_breaker]
    failure_threshold = 5
    timeout_seconds = 60

    [rate_limiter]
    requests_per_second = 100
```

### Environment Variables

Add to `k8s/deployment.yaml`:

```yaml
env:
- name: CUSTOM_VARIABLE
  value: "custom-value"
- name: SECRET_VALUE
  valueFrom:
    secretKeyRef:
      name: production-agent-secrets
      key: api-key
```

### Persistent Storage

If you need persistent storage:

```yaml
# In deployment.yaml
volumes:
- name: data
  persistentVolumeClaim:
    claimName: production-agent-data

volumeMounts:
- name: data
  mountPath: /data
```

## Support

For issues or questions:
- Check logs: `./deploy/scripts/deploy-k8s.sh logs`
- Check health: `./deploy/scripts/deploy-k8s.sh health`
- Review documentation in `docs/agent-infrastructure.md`
