//! Usage Analytics (Phase 8.3)
//!
//! This module provides agent usage tracking and analytics with bounded memory.
//! Tracks tool calls, latencies, errors, and provides aggregated metrics.
//!
//! ## Features
//!
//! - **Tool Usage**: Track frequency and success rate per tool
//! - **Latency Tracking**: Record execution times (circular buffer)
//! - **Error Tracking**: Count and categorize errors (circular buffer)
//! - **Bounded Growth**: All collections use fixed-size circular buffers

#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use std::collections::HashMap;
use std::time::Duration;

/// Circular buffer with fixed capacity
#[derive(Clone, Debug)]
struct CircularBuffer<T> {
    items: Vec<T>,
    capacity: usize,
    next_index: usize,
}

impl<T: Clone> CircularBuffer<T> {
    /// Create new circular buffer with capacity
    fn new(capacity: usize) -> Self {
        Self {
            items: Vec::with_capacity(capacity),
            capacity,
            next_index: 0,
        }
    }

    /// Push item (overwrites oldest if at capacity)
    fn push(&mut self, item: T) {
        if self.items.len() < self.capacity {
            self.items.push(item);
        } else {
            self.items[self.next_index] = item;
            self.next_index = (self.next_index + 1) % self.capacity;
        }
    }

    /// Get all items in order (newest last)
    fn items(&self) -> &[T] {
        &self.items
    }

    /// Get length
    fn len(&self) -> usize {
        self.items.len()
    }

    /// Check if empty
    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Agent metrics tracker with bounded growth
#[derive(Clone, Debug)]
pub struct AgentMetrics {
    /// Tool call counts
    tool_calls: HashMap<String, usize>,
    /// Tool success counts
    tool_successes: HashMap<String, usize>,
    /// Tool failure counts
    tool_failures: HashMap<String, usize>,
    /// Recent latencies (circular buffer)
    latencies: CircularBuffer<Duration>,
    /// Recent errors (circular buffer)
    errors: CircularBuffer<String>,
}

impl AgentMetrics {
    /// Create new metrics tracker
    #[must_use]
    pub fn new() -> Self {
        Self {
            tool_calls: HashMap::new(),
            tool_successes: HashMap::new(),
            tool_failures: HashMap::new(),
            latencies: CircularBuffer::new(100), // Keep last 100 latencies
            errors: CircularBuffer::new(50),     // Keep last 50 errors
        }
    }

    /// Record tool call
    pub fn record_tool_call(&mut self, tool_name: &str, success: bool, latency: Duration) {
        // Increment total calls
        *self.tool_calls.entry(tool_name.to_string()).or_insert(0) += 1;

        // Increment success/failure
        if success {
            *self.tool_successes.entry(tool_name.to_string()).or_insert(0) += 1;
        } else {
            *self.tool_failures.entry(tool_name.to_string()).or_insert(0) += 1;
        }

        // Record latency
        self.latencies.push(latency);
    }

    /// Record error
    pub fn record_error(&mut self, error: String) {
        self.errors.push(error);
    }

    /// Get metrics snapshot
    #[must_use]
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            total_tool_calls: self.tool_calls.values().sum(),
            total_successes: self.tool_successes.values().sum(),
            total_failures: self.tool_failures.values().sum(),
            tool_calls: self.tool_calls.clone(),
            tool_successes: self.tool_successes.clone(),
            tool_failures: self.tool_failures.clone(),
            recent_latencies_count: self.latencies.len(),
            recent_errors_count: self.errors.len(),
        }
    }

    /// Get success rate for a tool
    #[must_use]
    pub fn tool_success_rate(&self, tool_name: &str) -> Option<f64> {
        let calls = self.tool_calls.get(tool_name)?;
        if *calls == 0 {
            return None;
        }

        let successes = self.tool_successes.get(tool_name).copied().unwrap_or(0);
        Some(successes as f64 / *calls as f64)
    }

    /// Get average latency
    #[must_use]
    pub fn average_latency(&self) -> Option<Duration> {
        if self.latencies.is_empty() {
            return None;
        }

        let total: Duration = self.latencies.items().iter().sum();
        Some(total / self.latencies.len() as u32)
    }

    /// Get recent errors
    #[must_use]
    pub fn recent_errors(&self) -> &[String] {
        self.errors.items()
    }
}

impl Default for AgentMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Metrics snapshot (immutable view)
#[derive(Clone, Debug)]
pub struct MetricsSnapshot {
    /// Total tool calls across all tools
    pub total_tool_calls: usize,
    /// Total successes
    pub total_successes: usize,
    /// Total failures
    pub total_failures: usize,
    /// Per-tool call counts
    pub tool_calls: HashMap<String, usize>,
    /// Per-tool success counts
    pub tool_successes: HashMap<String, usize>,
    /// Per-tool failure counts
    pub tool_failures: HashMap<String, usize>,
    /// Number of recent latencies tracked
    pub recent_latencies_count: usize,
    /// Number of recent errors tracked
    pub recent_errors_count: usize,
}

impl MetricsSnapshot {
    /// Get overall success rate
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        if self.total_tool_calls == 0 {
            return 0.0;
        }
        self.total_successes as f64 / self.total_tool_calls as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circular_buffer() {
        let mut buf = CircularBuffer::new(3);

        buf.push(1);
        buf.push(2);
        buf.push(3);
        assert_eq!(buf.items(), &[1, 2, 3]);

        // Should overwrite oldest
        buf.push(4);
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.items(), &[4, 2, 3]);
    }

    #[test]
    fn test_agent_metrics() {
        let mut metrics = AgentMetrics::new();

        metrics.record_tool_call("tool1", true, Duration::from_millis(100));
        metrics.record_tool_call("tool1", true, Duration::from_millis(150));
        metrics.record_tool_call("tool1", false, Duration::from_millis(50));

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_tool_calls, 3);
        assert_eq!(snapshot.total_successes, 2);
        assert_eq!(snapshot.total_failures, 1);

        // Success rate
        let rate = metrics.tool_success_rate("tool1");
        assert!(rate.is_some());
        assert!((rate.expect("rate") - 0.666).abs() < 0.01);

        // Average latency
        let avg = metrics.average_latency();
        assert!(avg.is_some());
        assert_eq!(avg.expect("avg"), Duration::from_millis(100));
    }

    #[test]
    fn test_error_tracking() {
        let mut metrics = AgentMetrics::new();

        metrics.record_error("Error 1".to_string());
        metrics.record_error("Error 2".to_string());

        assert_eq!(metrics.recent_errors().len(), 2);
        assert_eq!(metrics.recent_errors()[0], "Error 1");
    }

    #[test]
    fn test_bounded_growth() {
        let mut metrics = AgentMetrics::new();

        // Add more than capacity
        for i in 0..150 {
            metrics.record_tool_call("tool1", true, Duration::from_millis(i));
        }

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_tool_calls, 150);
        // But latencies should be bounded to 100
        assert_eq!(snapshot.recent_latencies_count, 100);
    }
}
