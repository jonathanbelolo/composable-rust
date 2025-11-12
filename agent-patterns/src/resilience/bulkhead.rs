//! Bulkhead Pattern for Resource Isolation (Phase 8.4 Part 3.3)
//!
//! Isolates resources to prevent failures in one area from cascading to others.
//!
//! ## Concept
//!
//! Named after ship bulkheads that prevent water from flooding the entire ship,
//! this pattern isolates resources (threads, connections, etc.) so failures in
//! one area don't affect others.
//!
//! ## Usage
//!
//! ```ignore
//! use agent_patterns::resilience::bulkhead::*;
//!
//! let config = BulkheadConfig {
//!     max_concurrent: 10,
//!     acquire_timeout: Duration::from_secs(5),
//! };
//!
//! let bulkhead = Bulkhead::new("expensive_tool".into(), config);
//!
//! // Execute with resource isolation
//! let result = bulkhead.execute(async {
//!     expensive_operation().await
//! }).await?;
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::{info, warn};

/// Bulkhead configuration for resource isolation
#[derive(Debug, Clone)]
pub struct BulkheadConfig {
    /// Maximum concurrent operations allowed
    pub max_concurrent: usize,
    /// Timeout for acquiring a permit
    pub acquire_timeout: Duration,
}

impl Default for BulkheadConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 10,
            acquire_timeout: Duration::from_secs(5),
        }
    }
}

/// Bulkhead for isolating resource usage
///
/// Uses semaphores to limit concurrent operations, preventing
/// resource exhaustion in one area from affecting others.
pub struct Bulkhead {
    name: String,
    semaphore: Arc<Semaphore>,
    config: BulkheadConfig,
}

impl Bulkhead {
    /// Create new bulkhead
    ///
    /// # Arguments
    ///
    /// * `name` - Name for logging (e.g., "`llm_calls`", "`database_queries`")
    /// * `config` - Bulkhead configuration
    #[must_use]
    pub fn new(name: String, config: BulkheadConfig) -> Self {
        Self {
            name,
            semaphore: Arc::new(Semaphore::new(config.max_concurrent)),
            config,
        }
    }

    /// Execute function within bulkhead
    ///
    /// Acquires a permit (waits if necessary), executes function,
    /// then releases permit automatically.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Cannot acquire permit within timeout
    /// - Semaphore is closed
    pub async fn execute<F, T>(&self, f: F) -> Result<T, String>
    where
        F: std::future::Future<Output = T>,
    {
        // Try to acquire permit with timeout
        let permit = tokio::time::timeout(self.config.acquire_timeout, self.semaphore.acquire())
            .await
            .map_err(|_| {
                warn!(
                    "Bulkhead {} acquire timeout after {:?}",
                    self.name, self.config.acquire_timeout
                );
                format!(
                    "Bulkhead {} acquire timeout after {:?}",
                    self.name, self.config.acquire_timeout
                )
            })?
            .map_err(|e| {
                format!("Bulkhead {} acquire failed: {}", self.name, e)
            })?;

        info!("Acquired bulkhead permit for {}", self.name);

        // Execute function
        let result = f.await;

        // Permit is automatically released when dropped
        drop(permit);

        Ok(result)
    }

    /// Get number of available permits
    #[must_use]
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Get bulkhead name
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get max concurrent operations
    #[must_use]
    pub fn max_concurrent(&self) -> usize {
        self.config.max_concurrent
    }
}

/// Bulkhead registry for different resource types
///
/// Manages multiple bulkheads, one per resource type.
///
/// # Example
///
/// ```ignore
/// let mut registry = BulkheadRegistry::new();
///
/// registry.register("llm_api".into(), Bulkhead::new(
///     "llm_api".into(),
///     BulkheadConfig { max_concurrent: 5, ..Default::default() }
/// ));
///
/// registry.register("database".into(), Bulkhead::new(
///     "database".into(),
///     BulkheadConfig { max_concurrent: 20, ..Default::default() }
/// ));
///
/// // Use bulkheads
/// let bulkhead = registry.get("llm_api").unwrap();
/// bulkhead.execute(api_call()).await?;
/// ```
pub struct BulkheadRegistry {
    bulkheads: HashMap<String, Arc<Bulkhead>>,
}

impl BulkheadRegistry {
    /// Create new bulkhead registry
    #[must_use]
    pub fn new() -> Self {
        Self {
            bulkheads: HashMap::new(),
        }
    }

    /// Register a bulkhead
    pub fn register(&mut self, name: String, bulkhead: Bulkhead) {
        self.bulkheads.insert(name, Arc::new(bulkhead));
    }

    /// Get a bulkhead by name
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Arc<Bulkhead>> {
        self.bulkheads.get(name).cloned()
    }

    /// Get all bulkhead names
    #[must_use]
    pub fn names(&self) -> Vec<&String> {
        self.bulkheads.keys().collect()
    }

    /// Get number of registered bulkheads
    #[must_use]
    pub fn len(&self) -> usize {
        self.bulkheads.len()
    }

    /// Check if registry is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bulkheads.is_empty()
    }
}

impl Default for BulkheadRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // Test code can use unwrap/expect
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_bulkhead_allows_concurrent_operations() {
        let config = BulkheadConfig {
            max_concurrent: 3,
            acquire_timeout: Duration::from_secs(5),
        };

        let bulkhead = Arc::new(Bulkhead::new("test".to_string(), config));

        let counter = Arc::new(AtomicUsize::new(0));

        let mut handles = vec![];

        for _ in 0..3 {
            let bulkhead = Arc::clone(&bulkhead);
            let counter = Arc::clone(&counter);

            let handle = tokio::spawn(async move {
                bulkhead
                    .execute(async {
                        counter.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        counter.fetch_sub(1, Ordering::SeqCst);
                    })
                    .await
            });

            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap().unwrap();
        }

        // All should complete
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_bulkhead_limits_concurrent_operations() {
        let config = BulkheadConfig {
            max_concurrent: 2,
            acquire_timeout: Duration::from_secs(5),
        };

        let bulkhead = Arc::new(Bulkhead::new("test".to_string(), config));
        let max_concurrent = Arc::new(AtomicUsize::new(0));

        let mut handles = vec![];

        for _ in 0..5 {
            let bulkhead = Arc::clone(&bulkhead);
            let max_concurrent = Arc::clone(&max_concurrent);

            let handle = tokio::spawn(async move {
                bulkhead
                    .execute(async {
                        let current = max_concurrent.fetch_add(1, Ordering::SeqCst) + 1;

                        // Sleep to ensure overlap
                        tokio::time::sleep(Duration::from_millis(50)).await;

                        max_concurrent.fetch_sub(1, Ordering::SeqCst);

                        current
                    })
                    .await
                    .unwrap()
            });

            handles.push(handle);
        }

        let mut max_seen = 0;
        for handle in handles {
            let current = handle.await.unwrap();
            max_seen = max_seen.max(current);
        }

        // Should never exceed max_concurrent
        assert!(max_seen <= 2, "Max concurrent was {}, expected <= 2", max_seen);
    }

    #[tokio::test]
    async fn test_bulkhead_timeout() {
        let config = BulkheadConfig {
            max_concurrent: 1,
            acquire_timeout: Duration::from_millis(100),
        };

        let bulkhead = Arc::new(Bulkhead::new("test".to_string(), config));

        // Start a long-running operation
        let bulkhead1 = Arc::clone(&bulkhead);
        let handle1 = tokio::spawn(async move {
            bulkhead1
                .execute(async {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                })
                .await
        });

        // Give it time to acquire
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Try to acquire another (should timeout)
        let result = bulkhead.execute(async { "should timeout" }).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("timeout"));

        // Cleanup
        handle1.await.ok();
    }

    #[tokio::test]
    async fn test_bulkhead_available_permits() {
        let config = BulkheadConfig {
            max_concurrent: 5,
            acquire_timeout: Duration::from_secs(5),
        };

        let bulkhead = Arc::new(Bulkhead::new("test".to_string(), config));

        assert_eq!(bulkhead.available_permits(), 5);

        // Acquire one
        let _result = bulkhead.execute(async { 42 }).await.unwrap();

        // Back to 5 (permit released)
        assert_eq!(bulkhead.available_permits(), 5);
    }

    #[test]
    fn test_bulkhead_getters() {
        let config = BulkheadConfig {
            max_concurrent: 15,
            acquire_timeout: Duration::from_secs(10),
        };

        let bulkhead = Bulkhead::new("test".to_string(), config);

        assert_eq!(bulkhead.name(), "test");
        assert_eq!(bulkhead.max_concurrent(), 15);
    }

    #[test]
    fn test_bulkhead_registry() {
        let mut registry = BulkheadRegistry::new();

        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);

        registry.register(
            "llm".to_string(),
            Bulkhead::new("llm".to_string(), BulkheadConfig::default()),
        );

        registry.register(
            "db".to_string(),
            Bulkhead::new("db".to_string(), BulkheadConfig::default()),
        );

        assert_eq!(registry.len(), 2);
        assert!(!registry.is_empty());

        let llm_bulkhead = registry.get("llm");
        assert!(llm_bulkhead.is_some());
        assert_eq!(llm_bulkhead.unwrap().name(), "llm");

        let names = registry.names();
        assert_eq!(names.len(), 2);
    }

    #[tokio::test]
    async fn test_bulkhead_registry_isolation() {
        let mut registry = BulkheadRegistry::new();

        registry.register(
            "resource_a".to_string(),
            Bulkhead::new(
                "resource_a".to_string(),
                BulkheadConfig {
                    max_concurrent: 1,
                    acquire_timeout: Duration::from_secs(5),
                },
            ),
        );

        registry.register(
            "resource_b".to_string(),
            Bulkhead::new(
                "resource_b".to_string(),
                BulkheadConfig {
                    max_concurrent: 10,
                    acquire_timeout: Duration::from_secs(5),
                },
            ),
        );

        let bulkhead_a = registry.get("resource_a").unwrap();
        let bulkhead_b = registry.get("resource_b").unwrap();

        // Block resource_a
        let bulkhead_a_clone = Arc::clone(&bulkhead_a);
        let _handle = tokio::spawn(async move {
            bulkhead_a_clone
                .execute(async {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                })
                .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        // resource_b should still be available
        let result = bulkhead_b.execute(async { "success" }).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
    }
}
