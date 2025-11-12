//! Prometheus Metrics for Agent Systems (Phase 8.4 Part 4.1)
//!
//! Production-grade metrics collection and export for agent systems.
//!
//! ## Metric Types
//!
//! - **Counters**: Monotonically increasing values (requests, errors)
//! - **Gauges**: Values that can go up or down (active agents, queue size)
//! - **Histograms**: Distribution of values (execution time, response size)
//!
//! ## Usage
//!
//! ```ignore
//! use agent_patterns::metrics::*;
//!
//! // Initialize metrics registry
//! let registry = AgentMetricsRegistry::new();
//!
//! // Record metrics
//! registry.record_agent_execution("my_agent", Duration::from_millis(150), "success");
//! registry.record_tool_call("web_search", "success");
//! registry.increment_active_agents();
//!
//! // Export for Prometheus scraping
//! let metrics_text = registry.export_prometheus();
//! ```

use prometheus::{
    core::{AtomicU64, GenericGauge},
    opts, CounterVec, HistogramOpts, HistogramVec, Registry, TextEncoder,
};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

/// Global metrics registry (singleton)
///
/// Initialized once on first access, uses default Prometheus registry.
pub static AGENT_METRICS: LazyLock<AgentMetricsRegistry> = LazyLock::new(AgentMetricsRegistry::default);

/// Agent metrics registry for Prometheus
///
/// Provides structured metrics collection for agent systems with proper
/// labeling for Prometheus scraping.
pub struct AgentMetricsRegistry {
    /// Custom registry for metrics (allows multiple registries)
    registry: Arc<Registry>,

    /// Agent execution time histogram (ms)
    ///
    /// Labels: `agent_name`, `status` (success, error)
    agent_execution_duration: HistogramVec,

    /// Tool invocation counter
    ///
    /// Labels: `tool_name`, `status` (success, error)
    tool_invocations: CounterVec,

    /// Pattern usage counter
    ///
    /// Labels: `pattern_name` (routing, orchestrator, `prompt_chain`, etc.)
    pattern_usage: CounterVec,

    /// Active agents gauge (currently executing)
    active_agents: GenericGauge<AtomicU64>,

    /// Total agent errors counter
    ///
    /// Labels: `error_type`
    agent_errors: CounterVec,

    /// LLM token usage counter
    ///
    /// Labels: `model`, `token_type` (prompt, completion)
    llm_tokens: CounterVec,
}

impl AgentMetricsRegistry {
    /// Create new metrics registry
    ///
    /// # Errors
    ///
    /// Returns error if metric registration fails (e.g., duplicate names)
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        // Agent execution duration histogram (exponential buckets: 10ms to 30s)
        let agent_execution_duration = HistogramVec::new(
            HistogramOpts::new(
                "agent_execution_duration_seconds",
                "Agent execution time in seconds",
            )
            .buckets(vec![
                0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
            ]),
            &["agent_name", "status"],
        )?;

        // Tool invocation counter
        let tool_invocations = CounterVec::new(
            opts!(
                "agent_tool_invocations_total",
                "Total number of tool invocations"
            ),
            &["tool_name", "status"],
        )?;

        // Pattern usage counter
        let pattern_usage = CounterVec::new(
            opts!(
                "agent_pattern_usage_total",
                "Total usage count of agent patterns"
            ),
            &["pattern_name"],
        )?;

        // Active agents gauge
        let active_agents = GenericGauge::new(
            "agent_active_count",
            "Number of currently executing agents",
        )?;

        // Agent errors counter
        let agent_errors = CounterVec::new(
            opts!(
                "agent_errors_total",
                "Total number of agent errors"
            ),
            &["error_type"],
        )?;

        // LLM token usage counter
        let llm_tokens = CounterVec::new(
            opts!(
                "agent_llm_tokens_total",
                "Total number of LLM tokens used"
            ),
            &["model", "token_type"],
        )?;

        // Register all metrics with custom registry
        registry.register(Box::new(agent_execution_duration.clone()))?;
        registry.register(Box::new(tool_invocations.clone()))?;
        registry.register(Box::new(pattern_usage.clone()))?;
        registry.register(Box::new(active_agents.clone()))?;
        registry.register(Box::new(agent_errors.clone()))?;
        registry.register(Box::new(llm_tokens.clone()))?;

        Ok(Self {
            registry: Arc::new(registry),
            agent_execution_duration,
            tool_invocations,
            pattern_usage,
            active_agents,
            agent_errors,
            llm_tokens,
        })
    }

    /// Record agent execution
    ///
    /// # Arguments
    ///
    /// * `agent_name` - Name of the agent (e.g., "`customer_support`", "`code_reviewer`")
    /// * `duration` - Execution duration
    /// * `status` - "success" or "error"
    pub fn record_agent_execution(&self, agent_name: &str, duration: Duration, status: &str) {
        self.agent_execution_duration
            .with_label_values(&[agent_name, status])
            .observe(duration.as_secs_f64());
    }

    /// Record tool invocation
    ///
    /// # Arguments
    ///
    /// * `tool_name` - Name of the tool (e.g., "`web_search`", "`code_interpreter`")
    /// * `status` - "success" or "error"
    pub fn record_tool_call(&self, tool_name: &str, status: &str) {
        self.tool_invocations
            .with_label_values(&[tool_name, status])
            .inc();
    }

    /// Record pattern usage
    ///
    /// # Arguments
    ///
    /// * `pattern_name` - Name of the pattern (e.g., "routing", "orchestrator")
    pub fn record_pattern_usage(&self, pattern_name: &str) {
        self.pattern_usage
            .with_label_values(&[pattern_name])
            .inc();
    }

    /// Increment active agents count
    pub fn increment_active_agents(&self) {
        self.active_agents.inc();
    }

    /// Decrement active agents count
    pub fn decrement_active_agents(&self) {
        self.active_agents.dec();
    }

    /// Get current active agents count
    #[must_use]
    pub fn get_active_agents(&self) -> u64 {
        self.active_agents.get()
    }

    /// Record agent error
    ///
    /// # Arguments
    ///
    /// * `error_type` - Type of error (e.g., "timeout", "`tool_failure`", "`llm_error`")
    pub fn record_error(&self, error_type: &str) {
        self.agent_errors.with_label_values(&[error_type]).inc();
    }

    /// Record LLM token usage
    ///
    /// # Arguments
    ///
    /// * `model` - Model name (e.g., "gpt-4", "claude-3")
    /// * `token_type` - "prompt" or "completion"
    /// * `count` - Number of tokens
    #[allow(clippy::cast_precision_loss)]
    pub fn record_llm_tokens(&self, model: &str, token_type: &str, count: u64) {
        self.llm_tokens
            .with_label_values(&[model, token_type])
            .inc_by(count as f64);
    }

    /// Export metrics in Prometheus text format
    ///
    /// Returns a string that can be served on `/metrics` endpoint.
    ///
    /// # Errors
    ///
    /// Returns error if encoding fails
    pub fn export_prometheus(&self) -> Result<String, prometheus::Error> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        encoder
            .encode_to_string(&metric_families)
            .map_err(|e| prometheus::Error::Msg(e.to_string()))
    }

    /// Get registry for custom integration
    #[must_use]
    pub fn registry(&self) -> Arc<Registry> {
        Arc::clone(&self.registry)
    }
}

impl Default for AgentMetricsRegistry {
    /// Create default metrics registry
    ///
    /// # Panics
    ///
    /// Panics if metrics registration fails (e.g., duplicate metric names).
    #[allow(clippy::expect_used)]
    fn default() -> Self {
        Self::new().expect("Failed to create default metrics registry")
    }
}

/// RAII guard for tracking active agents
///
/// Automatically increments on creation, decrements on drop.
///
/// # Example
///
/// ```ignore
/// let _guard = ActiveAgentGuard::new(&registry);
/// // Agent execution...
/// // Automatically decrements when guard goes out of scope
/// ```
pub struct ActiveAgentGuard<'a> {
    registry: &'a AgentMetricsRegistry,
}

impl<'a> ActiveAgentGuard<'a> {
    /// Create guard and increment active agents
    #[must_use]
    pub fn new(registry: &'a AgentMetricsRegistry) -> Self {
        registry.increment_active_agents();
        Self { registry }
    }
}

impl Drop for ActiveAgentGuard<'_> {
    fn drop(&mut self) {
        self.registry.decrement_active_agents();
    }
}

/// Scoped timer for agent execution
///
/// Records execution duration automatically on drop.
///
/// # Example
///
/// ```ignore
/// {
///     let _timer = ExecutionTimer::new(&registry, "my_agent");
///     // Agent work...
/// } // Automatically records duration when scope ends
/// ```
pub struct ExecutionTimer<'a> {
    registry: &'a AgentMetricsRegistry,
    agent_name: String,
    start: std::time::Instant,
}

impl<'a> ExecutionTimer<'a> {
    /// Create timer and start measuring
    #[must_use]
    pub fn new(registry: &'a AgentMetricsRegistry, agent_name: impl Into<String>) -> Self {
        Self {
            registry,
            agent_name: agent_name.into(),
            start: std::time::Instant::now(),
        }
    }

    /// Stop timer and record with given status
    pub fn finish(self, status: &str) {
        let duration = self.start.elapsed();
        self.registry
            .record_agent_execution(&self.agent_name, duration, status);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // Test code can use unwrap/expect
mod tests {
    use super::*;

    #[test]
    fn test_metrics_registry_creation() {
        let registry = AgentMetricsRegistry::new();
        assert!(registry.is_ok());
    }

    #[test]
    fn test_record_agent_execution() {
        let registry = AgentMetricsRegistry::new().unwrap();
        registry.record_agent_execution("test_agent", Duration::from_millis(100), "success");
        registry.record_agent_execution("test_agent", Duration::from_secs(1), "error");

        let metrics = registry.export_prometheus().unwrap();
        assert!(metrics.contains("agent_execution_duration_seconds"));
        assert!(metrics.contains("test_agent"));
    }

    #[test]
    fn test_record_tool_call() {
        let registry = AgentMetricsRegistry::new().unwrap();
        registry.record_tool_call("web_search", "success");
        registry.record_tool_call("web_search", "error");

        let metrics = registry.export_prometheus().unwrap();
        assert!(metrics.contains("agent_tool_invocations_total"));
        assert!(metrics.contains("web_search"));
    }

    #[test]
    fn test_record_pattern_usage() {
        let registry = AgentMetricsRegistry::new().unwrap();
        registry.record_pattern_usage("routing");
        registry.record_pattern_usage("orchestrator");

        let metrics = registry.export_prometheus().unwrap();
        assert!(metrics.contains("agent_pattern_usage_total"));
        assert!(metrics.contains("routing"));
    }

    #[test]
    fn test_active_agents_counter() {
        let registry = AgentMetricsRegistry::new().unwrap();

        assert_eq!(registry.get_active_agents(), 0);

        registry.increment_active_agents();
        assert_eq!(registry.get_active_agents(), 1);

        registry.increment_active_agents();
        assert_eq!(registry.get_active_agents(), 2);

        registry.decrement_active_agents();
        assert_eq!(registry.get_active_agents(), 1);
    }

    #[test]
    fn test_active_agent_guard() {
        let registry = AgentMetricsRegistry::new().unwrap();

        assert_eq!(registry.get_active_agents(), 0);

        {
            let _guard = ActiveAgentGuard::new(&registry);
            assert_eq!(registry.get_active_agents(), 1);
        }

        assert_eq!(registry.get_active_agents(), 0);
    }

    #[test]
    fn test_active_agent_guard_multiple() {
        let registry = AgentMetricsRegistry::new().unwrap();

        {
            let _guard1 = ActiveAgentGuard::new(&registry);
            {
                let _guard2 = ActiveAgentGuard::new(&registry);
                assert_eq!(registry.get_active_agents(), 2);
            }
            assert_eq!(registry.get_active_agents(), 1);
        }

        assert_eq!(registry.get_active_agents(), 0);
    }

    #[test]
    fn test_record_error() {
        let registry = AgentMetricsRegistry::new().unwrap();
        registry.record_error("timeout");
        registry.record_error("llm_error");

        let metrics = registry.export_prometheus().unwrap();
        assert!(metrics.contains("agent_errors_total"));
        assert!(metrics.contains("timeout"));
    }

    #[test]
    fn test_record_llm_tokens() {
        let registry = AgentMetricsRegistry::new().unwrap();
        registry.record_llm_tokens("gpt-4", "prompt", 100);
        registry.record_llm_tokens("gpt-4", "completion", 200);

        let metrics = registry.export_prometheus().unwrap();
        assert!(metrics.contains("agent_llm_tokens_total"));
        assert!(metrics.contains("gpt-4"));
    }

    #[test]
    fn test_export_prometheus() {
        let registry = AgentMetricsRegistry::new().unwrap();

        registry.record_agent_execution("agent1", Duration::from_millis(50), "success");
        registry.record_tool_call("tool1", "success");
        registry.increment_active_agents();

        let metrics = registry.export_prometheus().unwrap();

        // Check format (Prometheus text format)
        assert!(metrics.contains("# HELP"));
        assert!(metrics.contains("# TYPE"));
        assert!(metrics.contains("agent_execution_duration_seconds"));
        assert!(metrics.contains("agent_tool_invocations_total"));
        assert!(metrics.contains("agent_active_count"));
    }

    #[test]
    fn test_execution_timer() {
        let registry = AgentMetricsRegistry::new().unwrap();

        {
            let timer = ExecutionTimer::new(&registry, "timed_agent");
            std::thread::sleep(Duration::from_millis(10));
            timer.finish("success");
        }

        let metrics = registry.export_prometheus().unwrap();
        assert!(metrics.contains("timed_agent"));
    }

    #[test]
    fn test_global_metrics_singleton() {
        // Access global singleton
        AGENT_METRICS.record_agent_execution("global_agent", Duration::from_millis(100), "success");

        let metrics = AGENT_METRICS.export_prometheus().unwrap();
        assert!(metrics.contains("global_agent"));
    }
}
