//! Health Check Framework for Production Agents (Phase 8.4 Part 2.1)
//!
//! Kubernetes-ready health check system with liveness and readiness probes.
//!
//! ## Architecture
//!
//! - **`HealthCheckable` trait**: Components implement this to report health
//! - **`SystemHealthCheck`**: Aggregates multiple component health checks
//! - **`K8sHealthEndpoints`**: Provides /health/liveness and /health/readiness
//!
//! ## Usage
//!
//! ```ignore
//! use agent_patterns::health::*;
//!
//! // Create system health checker
//! let mut system_health = SystemHealthCheck::new();
//! system_health.add_check(Arc::new(DatabaseHealthCheck::new(pool)));
//! system_health.add_check(Arc::new(LLMHealthCheck::new(client)));
//!
//! // Create K8s endpoints
//! let k8s_health = K8sHealthEndpoints::new(Arc::new(system_health));
//!
//! // Liveness: Is process alive?
//! let (status, body) = k8s_health.liveness().await;
//!
//! // Readiness: Can process accept traffic?
//! let (status, body) = k8s_health.readiness().await;
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// Health status for a component
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// Component is fully operational
    Healthy,
    /// Component is degraded but functional (e.g., high latency, limited capacity)
    Degraded,
    /// Component is not operational
    Unhealthy,
}

impl HealthStatus {
    /// Check if status is healthy or degraded (can accept traffic)
    #[must_use]
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Healthy | Self::Degraded)
    }

    /// Check if status is healthy
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Healthy)
    }
}

/// Health check result with details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    /// Current health status
    pub status: HealthStatus,
    /// Human-readable message
    pub message: String,
    /// Timestamp of this health check
    pub last_check: SystemTime,
    /// Additional details (latency, error counts, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<HashMap<String, serde_json::Value>>,
}

impl ComponentHealth {
    /// Create healthy status
    #[must_use]
    pub fn healthy(message: impl Into<String>) -> Self {
        Self {
            status: HealthStatus::Healthy,
            message: message.into(),
            last_check: SystemTime::now(),
            details: None,
        }
    }

    /// Create degraded status
    #[must_use]
    pub fn degraded(message: impl Into<String>) -> Self {
        Self {
            status: HealthStatus::Degraded,
            message: message.into(),
            last_check: SystemTime::now(),
            details: None,
        }
    }

    /// Create unhealthy status
    #[must_use]
    pub fn unhealthy(message: impl Into<String>) -> Self {
        Self {
            status: HealthStatus::Unhealthy,
            message: message.into(),
            last_check: SystemTime::now(),
            details: None,
        }
    }

    /// Add detail to health check
    #[must_use]
    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.details.get_or_insert_with(HashMap::new).insert(key.into(), value.into());
        self
    }
}

/// Trait for health-checkable components
///
/// Implement this trait for any component that needs health monitoring:
/// - Database connections
/// - External APIs (LLM, search, etc.)
/// - Message queues
/// - Caches
#[async_trait]
pub trait HealthCheckable: Send + Sync {
    /// Check component health
    ///
    /// Should complete quickly (<1s) to avoid blocking health checks.
    async fn check_health(&self) -> ComponentHealth;

    /// Component name for reporting
    fn component_name(&self) -> &str;
}

/// Aggregate health checker for multiple components
///
/// Collects health status from all registered components and
/// determines overall system health.
pub struct SystemHealthCheck {
    checks: Vec<Arc<dyn HealthCheckable>>,
}

impl SystemHealthCheck {
    /// Create new system health checker
    #[must_use]
    pub fn new() -> Self {
        Self {
            checks: Vec::new(),
        }
    }

    /// Add a health check
    pub fn add_check(&mut self, check: Arc<dyn HealthCheckable>) {
        self.checks.push(check);
    }

    /// Check all components
    ///
    /// Returns a map of component name → health status.
    pub async fn check_all(&self) -> HashMap<String, ComponentHealth> {
        let mut results = HashMap::new();

        // Run all health checks in parallel
        let check_futures: Vec<_> = self
            .checks
            .iter()
            .map(|check| async move {
                let name = check.component_name().to_string();
                let health = check.check_health().await;
                (name, health)
            })
            .collect();

        // Wait for all checks to complete
        let check_results = futures::future::join_all(check_futures).await;

        for (name, health) in check_results {
            results.insert(name, health);
        }

        results
    }

    /// Determine overall system health
    ///
    /// - Unhealthy if ANY component is unhealthy
    /// - Degraded if ANY component is degraded
    /// - Healthy if ALL components are healthy
    pub async fn overall_health(&self) -> HealthStatus {
        let results = self.check_all().await;

        if results.values().any(|h| h.status == HealthStatus::Unhealthy) {
            HealthStatus::Unhealthy
        } else if results.values().any(|h| h.status == HealthStatus::Degraded) {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        }
    }
}

impl Default for SystemHealthCheck {
    fn default() -> Self {
        Self::new()
    }
}

/// Kubernetes-style health endpoints
///
/// Provides two endpoints for Kubernetes health probes:
/// - **Liveness**: Is the process alive? (should restart if fails)
/// - **Readiness**: Can the process accept traffic? (remove from load balancer if fails)
#[derive(Clone)]
pub struct K8sHealthEndpoints {
    system_health: Arc<SystemHealthCheck>,
}

impl K8sHealthEndpoints {
    /// Create new K8s health endpoints
    #[must_use]
    pub fn new(system_health: Arc<SystemHealthCheck>) -> Self {
        Self { system_health }
    }

    /// Liveness probe - "is the process alive?"
    ///
    /// Should only fail if the process needs to be restarted.
    /// Typically checks if the process can still respond.
    ///
    /// # Returns
    ///
    /// `(status_code, body)` where:
    /// - `200 OK` = alive
    /// - `503 Service Unavailable` = dead (restart needed)
    #[allow(clippy::unused_async)]
    pub async fn liveness(&self) -> (u16, &'static str) {
        // Basic liveness - just check if we can respond
        // If this function executes, the process is alive
        (200, "alive")
    }

    /// Readiness probe - "can the process accept traffic?"
    ///
    /// Fails if critical dependencies are unavailable (database, LLM API, etc.).
    /// Process stays alive but is removed from load balancer rotation.
    ///
    /// # Returns
    ///
    /// `(status_code, body)` where:
    /// - `200 OK` = ready (healthy)
    /// - `200 OK` = degraded (still accepting traffic but degraded)
    /// - `503 Service Unavailable` = not ready (remove from load balancer)
    pub async fn readiness(&self) -> (u16, String) {
        match self.system_health.overall_health().await {
            HealthStatus::Healthy => (200, "ready".to_string()),
            HealthStatus::Degraded => (200, "degraded".to_string()),
            HealthStatus::Unhealthy => (503, "not ready".to_string()),
        }
    }

    /// Detailed health check (for debugging)
    ///
    /// Returns full health status for all components.
    /// Use this for monitoring dashboards and troubleshooting.
    pub async fn health_detailed(&self) -> HashMap<String, ComponentHealth> {
        self.system_health.check_all().await
    }
}

/// Health check with timeout wrapper
///
/// Wraps any health check with a timeout to prevent hanging.
pub struct TimeoutHealthCheck {
    inner: Arc<dyn HealthCheckable>,
    timeout: Duration,
}

impl TimeoutHealthCheck {
    /// Create new timeout health check
    #[must_use]
    pub fn new(inner: Arc<dyn HealthCheckable>, timeout: Duration) -> Self {
        Self { inner, timeout }
    }
}

#[async_trait]
impl HealthCheckable for TimeoutHealthCheck {
    #[allow(clippy::cast_possible_truncation)]
    async fn check_health(&self) -> ComponentHealth {
        let start = Instant::now();

        match tokio::time::timeout(self.timeout, self.inner.check_health()).await {
            Ok(health) => {
                let duration = start.elapsed();
                health.with_detail("check_duration_ms", duration.as_millis() as i64)
            }
            Err(_) => ComponentHealth::unhealthy(format!(
                "Health check timed out after {:?}",
                self.timeout
            ))
            .with_detail("timeout_ms", self.timeout.as_millis() as i64),
        }
    }

    fn component_name(&self) -> &str {
        self.inner.component_name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock health check for testing
    struct MockHealthCheck {
        name: String,
        status: HealthStatus,
    }

    impl MockHealthCheck {
        fn new(name: impl Into<String>, status: HealthStatus) -> Self {
            Self {
                name: name.into(),
                status,
            }
        }
    }

    #[async_trait]
    impl HealthCheckable for MockHealthCheck {
        async fn check_health(&self) -> ComponentHealth {
            match self.status {
                HealthStatus::Healthy => ComponentHealth::healthy("OK"),
                HealthStatus::Degraded => ComponentHealth::degraded("Slow"),
                HealthStatus::Unhealthy => ComponentHealth::unhealthy("Failed"),
            }
        }

        fn component_name(&self) -> &str {
            &self.name
        }
    }

    #[test]
    fn test_health_status_is_ready() {
        assert!(HealthStatus::Healthy.is_ready());
        assert!(HealthStatus::Degraded.is_ready());
        assert!(!HealthStatus::Unhealthy.is_ready());
    }

    #[test]
    fn test_health_status_is_healthy() {
        assert!(HealthStatus::Healthy.is_healthy());
        assert!(!HealthStatus::Degraded.is_healthy());
        assert!(!HealthStatus::Unhealthy.is_healthy());
    }

    #[test]
    fn test_component_health_constructors() {
        let healthy = ComponentHealth::healthy("All good");
        assert_eq!(healthy.status, HealthStatus::Healthy);
        assert_eq!(healthy.message, "All good");

        let degraded = ComponentHealth::degraded("Slow response");
        assert_eq!(degraded.status, HealthStatus::Degraded);

        let unhealthy = ComponentHealth::unhealthy("Connection failed");
        assert_eq!(unhealthy.status, HealthStatus::Unhealthy);
    }

    #[test]
    fn test_component_health_with_detail() {
        let health = ComponentHealth::healthy("OK")
            .with_detail("latency_ms", 150)
            .with_detail("connections", 5);

        let details = health.details.unwrap();
        assert_eq!(details.get("latency_ms").unwrap(), &serde_json::json!(150));
        assert_eq!(details.get("connections").unwrap(), &serde_json::json!(5));
    }

    #[tokio::test]
    async fn test_system_health_check_all_healthy() {
        let mut system_health = SystemHealthCheck::new();
        system_health.add_check(Arc::new(MockHealthCheck::new("db", HealthStatus::Healthy)));
        system_health.add_check(Arc::new(MockHealthCheck::new("api", HealthStatus::Healthy)));

        let results = system_health.check_all().await;
        assert_eq!(results.len(), 2);
        assert_eq!(results.get("db").unwrap().status, HealthStatus::Healthy);
        assert_eq!(results.get("api").unwrap().status, HealthStatus::Healthy);
    }

    #[tokio::test]
    async fn test_system_health_overall_health_all_healthy() {
        let mut system_health = SystemHealthCheck::new();
        system_health.add_check(Arc::new(MockHealthCheck::new("db", HealthStatus::Healthy)));
        system_health.add_check(Arc::new(MockHealthCheck::new("api", HealthStatus::Healthy)));

        let overall = system_health.overall_health().await;
        assert_eq!(overall, HealthStatus::Healthy);
    }

    #[tokio::test]
    async fn test_system_health_overall_health_one_degraded() {
        let mut system_health = SystemHealthCheck::new();
        system_health.add_check(Arc::new(MockHealthCheck::new("db", HealthStatus::Healthy)));
        system_health.add_check(Arc::new(MockHealthCheck::new("api", HealthStatus::Degraded)));

        let overall = system_health.overall_health().await;
        assert_eq!(overall, HealthStatus::Degraded);
    }

    #[tokio::test]
    async fn test_system_health_overall_health_one_unhealthy() {
        let mut system_health = SystemHealthCheck::new();
        system_health.add_check(Arc::new(MockHealthCheck::new("db", HealthStatus::Healthy)));
        system_health.add_check(Arc::new(MockHealthCheck::new("api", HealthStatus::Unhealthy)));

        let overall = system_health.overall_health().await;
        assert_eq!(overall, HealthStatus::Unhealthy);
    }

    #[tokio::test]
    async fn test_k8s_liveness_always_ok() {
        let system_health = SystemHealthCheck::new();
        let k8s = K8sHealthEndpoints::new(Arc::new(system_health));

        let (status, body) = k8s.liveness().await;
        assert_eq!(status, 200);
        assert_eq!(body, "alive");
    }

    #[tokio::test]
    async fn test_k8s_readiness_healthy() {
        let mut system_health = SystemHealthCheck::new();
        system_health.add_check(Arc::new(MockHealthCheck::new("db", HealthStatus::Healthy)));

        let k8s = K8sHealthEndpoints::new(Arc::new(system_health));

        let (status, body) = k8s.readiness().await;
        assert_eq!(status, 200);
        assert_eq!(body, "ready");
    }

    #[tokio::test]
    async fn test_k8s_readiness_degraded() {
        let mut system_health = SystemHealthCheck::new();
        system_health.add_check(Arc::new(MockHealthCheck::new("api", HealthStatus::Degraded)));

        let k8s = K8sHealthEndpoints::new(Arc::new(system_health));

        let (status, body) = k8s.readiness().await;
        assert_eq!(status, 200);
        assert_eq!(body, "degraded");
    }

    #[tokio::test]
    async fn test_k8s_readiness_unhealthy() {
        let mut system_health = SystemHealthCheck::new();
        system_health.add_check(Arc::new(MockHealthCheck::new("db", HealthStatus::Unhealthy)));

        let k8s = K8sHealthEndpoints::new(Arc::new(system_health));

        let (status, body) = k8s.readiness().await;
        assert_eq!(status, 503);
        assert_eq!(body, "not ready");
    }

    #[tokio::test]
    async fn test_timeout_health_check() {
        struct SlowHealthCheck;

        #[async_trait]
        impl HealthCheckable for SlowHealthCheck {
            async fn check_health(&self) -> ComponentHealth {
                tokio::time::sleep(Duration::from_secs(10)).await;
                ComponentHealth::healthy("Should timeout before this")
            }

            fn component_name(&self) -> &str {
                "slow"
            }
        }

        let timeout_check = TimeoutHealthCheck::new(
            Arc::new(SlowHealthCheck),
            Duration::from_millis(100),
        );

        let result = timeout_check.check_health().await;
        assert_eq!(result.status, HealthStatus::Unhealthy);
        assert!(result.message.contains("timed out"));
    }
}
