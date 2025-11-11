# Kubernetes Deployment Guide for Agent Systems

Production-ready Kubernetes manifests for deploying agent systems with full observability, autoscaling, and security.

## Table of Contents

- [Architecture](#architecture)
- [Prerequisites](#prerequisites)
- [Quick Start](#quick-start)
- [Deployment](#deployment)
- [Configuration](#configuration)
- [Monitoring](#monitoring)
- [Scaling](#scaling)
- [Security](#security)
- [Troubleshooting](#troubleshooting)

## Architecture

### Resource Overview

```
agent-system-prod (namespace)
├── Deployment (agent-system)
│   ├── 5 replicas (min)
│   ├── Resource requests/limits
│   ├── Health probes
│   └── Security context
├── Service (ClusterIP)
├── Ingress (TLS-enabled)
├── HorizontalPodAutoscaler (3-50 replicas)
├── PodDisruptionBudget (minAvailable: 2)
├── NetworkPolicy (egress/ingress rules)
├── ServiceMonitor (Prometheus scraping)
├── ConfigMap (non-sensitive config)
├── Secret (credentials)
├── ServiceAccount + RBAC
└── Dependencies:
    ├── PostgreSQL (event store)
    ├── Redpanda (event bus)
    ├── Jaeger (tracing)
    ├── Prometheus (metrics)
    └── Grafana (visualization)
```

## Prerequisites

### Required Tools

```bash
# Kubernetes cluster
kubectl version --client  # v1.28+

# Kustomize (bundled with kubectl)
kubectl kustomize --help

# Optional but recommended
helm version  # v3.12+ (for dependency charts)
```

### Cluster Requirements

- **Kubernetes**: 1.28+
- **Node resources**: 3+ nodes with 4 CPU, 16GB RAM each
- **Storage**: Dynamic provisioning (e.g., AWS EBS, GCE PD)
- **Ingress Controller**: nginx-ingress or similar
- **cert-manager**: For automatic TLS certificates (optional)
- **Prometheus Operator**: For ServiceMonitor support (optional)

### Verify Cluster Access

```bash
# Check connectivity
kubectl cluster-info

# Check available resources
kubectl top nodes

# Verify storage classes
kubectl get storageclasses
```

## Quick Start

### 1. Deploy Base Configuration

```bash
# Navigate to k8s directory
cd agent-patterns/k8s

# Preview manifests
kubectl kustomize base

# Apply base configuration
kubectl apply -k base

# Watch rollout
kubectl rollout status deployment/agent-system -n agent-system
```

### 2. Deploy Production Configuration

```bash
# Preview production manifests
kubectl kustomize overlays/production

# Apply production configuration
kubectl apply -k overlays/production

# Verify deployment
kubectl get all -n agent-system-prod
```

## Deployment

### Step-by-Step Production Deployment

#### 1. Create Namespace

```bash
kubectl apply -f base/namespace.yaml
```

#### 2. Configure Secrets

**IMPORTANT**: Replace placeholder values with actual secrets!

```bash
# Method 1: kubectl create secret
kubectl create secret generic agent-secrets \
  --from-literal=DATABASE_PASSWORD='your-secure-password' \
  --from-literal=DATABASE_URL='postgres://user:pass@host:5432/db' \
  --from-literal=LLM_API_KEY='your-api-key' \
  --from-literal=ANTHROPIC_API_KEY='your-anthropic-key' \
  --from-literal=JWT_SECRET='your-jwt-secret' \
  -n agent-system-prod

# Method 2: Use sealed-secrets (recommended)
# Install: kubectl apply -f https://github.com/bitnami-labs/sealed-secrets/releases/download/v0.24.0/controller.yaml
echo -n 'your-password' | kubectl create secret generic agent-secrets \
  --dry-run=client --from-file=DATABASE_PASSWORD=/dev/stdin -o yaml | \
  kubeseal -o yaml > sealed-secret.yaml
kubectl apply -f sealed-secret.yaml

# Method 3: Use external-secrets (recommended for cloud)
# See: https://external-secrets.io/
```

#### 3. Apply Configuration

```bash
# Apply production overlay
kubectl apply -k overlays/production

# Verify resources created
kubectl get all,ing,hpa,pdb,netpol -n agent-system-prod
```

#### 4. Verify Deployment

```bash
# Check pod status
kubectl get pods -n agent-system-prod -w

# Check logs
kubectl logs -f deployment/agent-system -n agent-system-prod

# Check events
kubectl get events -n agent-system-prod --sort-by='.lastTimestamp'
```

#### 5. Test Endpoints

```bash
# Port-forward for local testing
kubectl port-forward svc/agent-system 8080:80 -n agent-system-prod

# Test health endpoint
curl http://localhost:8080/health/liveness
curl http://localhost:8080/health/readiness

# Test metrics endpoint
kubectl port-forward svc/agent-system 9090:9090 -n agent-system-prod
curl http://localhost:9090/metrics
```

## Configuration

### Environment-Specific Overlays

The repository uses Kustomize overlays for environment-specific configuration:

```
k8s/
├── base/              # Base configuration (shared)
└── overlays/
    ├── production/    # Production overrides
    └── staging/       # Staging overrides (TODO)
```

### Customizing Configuration

#### Update ConfigMap

```bash
# Edit configmap
kubectl edit configmap agent-config -n agent-system-prod

# Or update via Kustomize
# Edit k8s/overlays/production/kustomization.yaml
# Add to configMapGenerator section

kubectl apply -k overlays/production
```

#### Update Image Tag

```bash
# Edit overlays/production/kustomization.yaml
images:
  - name: agent-system
    newName: your-registry.io/agent-system
    newTag: v1.2.4  # Update this

kubectl apply -k overlays/production
```

### Resource Limits

Edit `overlays/production/deployment-patch.yaml`:

```yaml
resources:
  requests:
    cpu: 1000m      # Guaranteed CPU
    memory: 1Gi     # Guaranteed memory
  limits:
    cpu: 4000m      # Max CPU
    memory: 4Gi     # Max memory (hard limit)
```

## Monitoring

### Prometheus Integration

#### Using ServiceMonitor (Prometheus Operator)

The `servicemonitor.yaml` automatically configures Prometheus to scrape metrics:

```bash
# Verify ServiceMonitor is created
kubectl get servicemonitor -n agent-system-prod

# Check if Prometheus is discovering targets
kubectl port-forward -n monitoring svc/prometheus 9090:9090
# Visit: http://localhost:9090/targets
```

#### Using Pod Annotations (Alternative)

If not using Prometheus Operator, pods have annotations:

```yaml
annotations:
  prometheus.io/scrape: "true"
  prometheus.io/port: "9090"
  prometheus.io/path: "/metrics"
```

Prometheus will auto-discover these pods if configured for Kubernetes SD.

### Grafana Dashboards

```bash
# Import dashboards from agent-patterns/dashboards/
# 1. Port-forward to Grafana
kubectl port-forward -n monitoring svc/grafana 3000:3000

# 2. Open http://localhost:3000
# 3. Import agent-overview.json and performance-slo.json
```

### Distributed Tracing

```bash
# Port-forward to Jaeger UI
kubectl port-forward -n tracing svc/jaeger-query 16686:16686

# Open http://localhost:16686
```

### Logs

```bash
# View logs from all pods
kubectl logs -f -l app=agent-system -n agent-system-prod

# View logs from specific pod
kubectl logs -f agent-system-7d8c9b5f4-abcde -n agent-system-prod

# View previous container logs (if crashed)
kubectl logs agent-system-7d8c9b5f4-abcde -p -n agent-system-prod

# Stream logs using stern (recommended)
stern agent-system -n agent-system-prod
```

## Scaling

### Manual Scaling

```bash
# Scale deployment
kubectl scale deployment agent-system --replicas=10 -n agent-system-prod

# Verify
kubectl get deployment agent-system -n agent-system-prod
```

### Horizontal Pod Autoscaler (HPA)

HPA automatically scales based on:
- CPU utilization (target: 70%)
- Memory utilization (target: 80%)
- Custom metrics (e.g., active agents)

```bash
# View HPA status
kubectl get hpa -n agent-system-prod

# Describe HPA
kubectl describe hpa agent-system -n agent-system-prod

# Edit HPA thresholds
kubectl edit hpa agent-system -n agent-system-prod
```

### Vertical Pod Autoscaler (VPA)

For automatic resource limit adjustments:

```bash
# Install VPA (if not already installed)
kubectl apply -f https://github.com/kubernetes/autoscaler/releases/latest/download/vertical-pod-autoscaler.yaml

# Create VPA resource
cat <<EOF | kubectl apply -f -
apiVersion: autoscaling.k8s.io/v1
kind: VerticalPodAutoscaler
metadata:
  name: agent-system
  namespace: agent-system-prod
spec:
  targetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: agent-system
  updatePolicy:
    updateMode: "Auto"
EOF
```

## Security

### Pod Security Standards

Pods are configured with restrictive security contexts:

```yaml
securityContext:
  runAsNonRoot: true
  runAsUser: 1000
  fsGroup: 1000
  readOnlyRootFilesystem: true
  allowPrivilegeEscalation: false
  capabilities:
    drop: ["ALL"]
```

### Network Policies

Network policies restrict pod communication:

```bash
# View network policies
kubectl get networkpolicy -n agent-system-prod

# Test connectivity
kubectl run -it --rm debug \
  --image=nicolaka/netshoot \
  -n agent-system-prod \
  -- bash
```

### RBAC

ServiceAccount has minimal required permissions:

```bash
# View service account
kubectl get sa agent-system -n agent-system-prod

# View role bindings
kubectl get rolebinding -n agent-system-prod
```

### Secrets Management

**Best Practices**:

1. **Never commit secrets to Git**
2. **Use sealed-secrets or external-secrets**
3. **Rotate secrets regularly**
4. **Use separate secrets per environment**

#### Rotate Secrets

```bash
# Update secret
kubectl create secret generic agent-secrets \
  --from-literal=DATABASE_PASSWORD='new-password' \
  --dry-run=client -o yaml | \
  kubectl apply -n agent-system-prod -f -

# Restart pods to pick up new secret
kubectl rollout restart deployment/agent-system -n agent-system-prod
```

### Pod Disruption Budget

Ensures high availability during updates:

```yaml
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: agent-system
spec:
  minAvailable: 2  # Always keep 2 pods running
```

## Troubleshooting

### Pod Not Starting

```bash
# Check pod status
kubectl describe pod <pod-name> -n agent-system-prod

# Common issues:
# 1. ImagePullBackOff - check image name/tag
# 2. CrashLoopBackOff - check logs
# 3. Pending - check resource requests vs node capacity
```

### Failed Health Checks

```bash
# Check probe configuration
kubectl get pod <pod-name> -n agent-system-prod -o yaml | grep -A10 Probe

# Test health endpoint manually
kubectl exec -it <pod-name> -n agent-system-prod -- wget -O- http://localhost:9090/health/liveness
```

### High Memory Usage

```bash
# Check current usage
kubectl top pod -n agent-system-prod

# Increase memory limits in deployment-patch.yaml
# Then apply:
kubectl apply -k overlays/production
```

### Networking Issues

```bash
# Test pod-to-pod connectivity
kubectl run -it --rm debug --image=nicolaka/netshoot -n agent-system-prod -- bash
# Inside pod: curl http://agent-system:80/health/liveness

# Check network policies
kubectl describe networkpolicy -n agent-system-prod

# Check service endpoints
kubectl get endpoints agent-system -n agent-system-prod
```

### Database Connection Failures

```bash
# Verify database is accessible
kubectl run -it --rm psql --image=postgres:16 -n agent-system-prod -- \
  psql -h postgres-service -U postgres -d agent_system

# Check connection string in secret
kubectl get secret agent-secrets -n agent-system-prod -o jsonpath='{.data.DATABASE_URL}' | base64 -d
```

### View All Events

```bash
# Recent events
kubectl get events -n agent-system-prod --sort-by='.lastTimestamp' | tail -20

# Watch events in real-time
kubectl get events -n agent-system-prod --watch
```

## Deployment Checklist

Before deploying to production:

- [ ] Update image tag to specific version (not `latest`)
- [ ] Replace all secret placeholders with actual values
- [ ] Configure TLS certificates (cert-manager or manual)
- [ ] Update ingress host to actual domain
- [ ] Set appropriate resource requests/limits
- [ ] Configure HPA min/max replicas for expected load
- [ ] Set up Prometheus alerting rules
- [ ] Configure backup strategy for PostgreSQL
- [ ] Test health checks and readiness probes
- [ ] Verify network policies allow necessary traffic
- [ ] Set up log aggregation (e.g., ELK, Loki)
- [ ] Configure pod disruption budget
- [ ] Test disaster recovery procedure
- [ ] Document runbook for common issues

## Additional Resources

- [Kubernetes Best Practices](https://kubernetes.io/docs/concepts/configuration/overview/)
- [Kustomize Documentation](https://kustomize.io/)
- [Prometheus Operator](https://prometheus-operator.dev/)
- [cert-manager](https://cert-manager.io/)
- [Sealed Secrets](https://github.com/bitnami-labs/sealed-secrets)
- [External Secrets](https://external-secrets.io/)

## Support

For agent-patterns specific issues:
1. Check pod logs: `kubectl logs -f deployment/agent-system -n agent-system-prod`
2. Review events: `kubectl get events -n agent-system-prod`
3. Check health endpoints: `/health/liveness`, `/health/readiness`
4. Review metrics: `/metrics`
5. Consult main agent-patterns documentation
