#!/usr/bin/env bash
# Deploy production-agent to Kubernetes

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
K8S_DIR="${SCRIPT_DIR}/../k8s"
NAMESPACE="${NAMESPACE:-default}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_step() {
    echo -e "${BLUE}[STEP]${NC} $1"
}

# Check if kubectl is available
if ! command -v kubectl &> /dev/null; then
    log_error "kubectl is not installed or not in PATH"
    exit 1
fi

# Parse arguments
ACTION="${1:-deploy}"

case "${ACTION}" in
    deploy)
        log_info "Deploying production-agent to Kubernetes namespace: ${NAMESPACE}"

        log_step "1. Creating namespace (if needed)..."
        kubectl create namespace "${NAMESPACE}" --dry-run=client -o yaml | kubectl apply -f -

        log_step "2. Applying RBAC configuration..."
        kubectl apply -n "${NAMESPACE}" -f "${K8S_DIR}/rbac.yaml"

        log_step "3. Applying ConfigMap..."
        kubectl apply -n "${NAMESPACE}" -f "${K8S_DIR}/configmap.yaml"

        log_step "4. Applying Service..."
        kubectl apply -n "${NAMESPACE}" -f "${K8S_DIR}/service.yaml"

        log_step "5. Applying Deployment..."
        kubectl apply -n "${NAMESPACE}" -f "${K8S_DIR}/deployment.yaml"

        log_step "6. Applying PodDisruptionBudget..."
        kubectl apply -n "${NAMESPACE}" -f "${K8S_DIR}/pdb.yaml"

        log_step "7. Applying HorizontalPodAutoscaler..."
        kubectl apply -n "${NAMESPACE}" -f "${K8S_DIR}/hpa.yaml"

        log_info ""
        log_info "Waiting for deployment to be ready..."
        kubectl rollout status deployment/production-agent -n "${NAMESPACE}" --timeout=300s

        log_info ""
        log_info "Deployment complete!"
        log_info "Check status with: kubectl get pods -n ${NAMESPACE}"
        ;;

    undeploy)
        log_info "Removing production-agent from namespace: ${NAMESPACE}"

        kubectl delete -n "${NAMESPACE}" -f "${K8S_DIR}/hpa.yaml" --ignore-not-found=true
        kubectl delete -n "${NAMESPACE}" -f "${K8S_DIR}/pdb.yaml" --ignore-not-found=true
        kubectl delete -n "${NAMESPACE}" -f "${K8S_DIR}/deployment.yaml" --ignore-not-found=true
        kubectl delete -n "${NAMESPACE}" -f "${K8S_DIR}/service.yaml" --ignore-not-found=true
        kubectl delete -n "${NAMESPACE}" -f "${K8S_DIR}/configmap.yaml" --ignore-not-found=true
        kubectl delete -n "${NAMESPACE}" -f "${K8S_DIR}/rbac.yaml" --ignore-not-found=true

        log_info "Undeployment complete"
        ;;

    status)
        log_info "Checking production-agent status in namespace: ${NAMESPACE}"

        echo ""
        log_step "Deployment Status:"
        kubectl get deployment production-agent -n "${NAMESPACE}" -o wide || log_warn "Deployment not found"

        echo ""
        log_step "Pod Status:"
        kubectl get pods -n "${NAMESPACE}" -l app=production-agent -o wide || log_warn "No pods found"

        echo ""
        log_step "Service Status:"
        kubectl get svc production-agent -n "${NAMESPACE}" || log_warn "Service not found"

        echo ""
        log_step "HPA Status:"
        kubectl get hpa production-agent -n "${NAMESPACE}" || log_warn "HPA not found"

        echo ""
        log_step "Recent Events:"
        kubectl get events -n "${NAMESPACE}" --sort-by='.lastTimestamp' | grep production-agent | tail -10 || log_warn "No events found"
        ;;

    logs)
        POD="${2:-}"
        if [ -z "${POD}" ]; then
            # Get the first pod
            POD=$(kubectl get pods -n "${NAMESPACE}" -l app=production-agent -o jsonpath='{.items[0].metadata.name}' 2>/dev/null || echo "")
            if [ -z "${POD}" ]; then
                log_error "No production-agent pods found in namespace ${NAMESPACE}"
                exit 1
            fi
            log_info "Following logs for pod: ${POD}"
        else
            log_info "Following logs for specified pod: ${POD}"
        fi

        kubectl logs -n "${NAMESPACE}" -f "${POD}"
        ;;

    restart)
        log_info "Restarting production-agent deployment in namespace: ${NAMESPACE}"
        kubectl rollout restart deployment/production-agent -n "${NAMESPACE}"
        kubectl rollout status deployment/production-agent -n "${NAMESPACE}"
        log_info "Restart complete"
        ;;

    scale)
        REPLICAS="${2:-3}"
        log_info "Scaling production-agent to ${REPLICAS} replicas in namespace: ${NAMESPACE}"
        kubectl scale deployment/production-agent -n "${NAMESPACE}" --replicas="${REPLICAS}"
        kubectl rollout status deployment/production-agent -n "${NAMESPACE}"
        log_info "Scaling complete"
        ;;

    health)
        log_info "Checking production-agent health in namespace: ${NAMESPACE}"

        PODS=$(kubectl get pods -n "${NAMESPACE}" -l app=production-agent -o jsonpath='{.items[*].metadata.name}')

        if [ -z "${PODS}" ]; then
            log_error "No production-agent pods found"
            exit 1
        fi

        for POD in ${PODS}; do
            echo ""
            log_step "Checking pod: ${POD}"

            # Port forward in background
            kubectl port-forward -n "${NAMESPACE}" "${POD}" 8081:8080 &
            PF_PID=$!
            sleep 2

            # Check health endpoints
            if curl -f http://localhost:8081/health/live &>/dev/null; then
                log_info "✓ Liveness: healthy"
            else
                log_error "✗ Liveness: unhealthy"
            fi

            if curl -f http://localhost:8081/health/ready &>/dev/null; then
                log_info "✓ Readiness: healthy"
            else
                log_warn "✗ Readiness: not ready"
            fi

            # Kill port forward
            kill "${PF_PID}" 2>/dev/null || true
        done
        ;;

    port-forward)
        log_info "Setting up port forwarding for production-agent in namespace: ${NAMESPACE}"

        POD=$(kubectl get pods -n "${NAMESPACE}" -l app=production-agent -o jsonpath='{.items[0].metadata.name}' 2>/dev/null || echo "")
        if [ -z "${POD}" ]; then
            log_error "No production-agent pods found in namespace ${NAMESPACE}"
            exit 1
        fi

        log_info "Forwarding ports for pod: ${POD}"
        log_info "  - HTTP API: http://localhost:8080"
        log_info "  - Metrics: http://localhost:9090/metrics"
        log_info ""
        log_info "Press Ctrl+C to stop"

        kubectl port-forward -n "${NAMESPACE}" "${POD}" 8080:8080 9090:9090
        ;;

    *)
        log_error "Unknown action: ${ACTION}"
        echo ""
        echo "Usage: $0 {deploy|undeploy|status|logs|restart|scale|health|port-forward} [args]"
        echo ""
        echo "Actions:"
        echo "  deploy          - Deploy production-agent to Kubernetes"
        echo "  undeploy        - Remove production-agent from Kubernetes"
        echo "  status          - Show deployment status"
        echo "  logs [pod]      - Follow logs (auto-selects pod if not specified)"
        echo "  restart         - Restart the deployment"
        echo "  scale N         - Scale to N replicas (default: 3)"
        echo "  health          - Check health of all pods"
        echo "  port-forward    - Forward ports to local machine"
        echo ""
        echo "Environment variables:"
        echo "  NAMESPACE       - Kubernetes namespace (default: default)"
        exit 1
        ;;
esac
