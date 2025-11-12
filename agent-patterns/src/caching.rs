//! Tool Result Caching (Phase 8.3)
//!
//! This module provides LRU caching for tool results with TTL support.
//! Reduces redundant tool executions for idempotent tools.
//!
//! ## Features
//!
//! - **LRU Eviction**: Least-recently-used eviction policy
//! - **TTL Support**: Time-to-live for cache entries
//! - **Bounded Size**: Maximum cache capacity enforced
//! - **Type-Safe**: Stores `ToolResult` not raw strings

use composable_rust_core::agent::ToolResult;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Cached tool result with timestamp
#[derive(Clone, Debug)]
pub struct CachedToolResult {
    /// The tool result
    pub result: ToolResult,
    /// When this was cached
    pub cached_at: Instant,
    /// Last time this was accessed
    pub last_accessed: Instant,
}

impl CachedToolResult {
    /// Create new cached result
    #[must_use]
    pub fn new(result: ToolResult) -> Self {
        let now = Instant::now();
        Self {
            result,
            cached_at: now,
            last_accessed: now,
        }
    }

    /// Check if entry is expired
    #[must_use]
    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.cached_at.elapsed() > ttl
    }

    /// Update last accessed time
    pub fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }
}

/// LRU cache for tool results with TTL
#[derive(Clone, Debug)]
pub struct ToolResultCache {
    /// Maximum entries
    capacity: usize,
    /// Time-to-live for entries
    ttl: Duration,
    /// Cache storage: (`tool_name`, `input_hash`) -> result
    cache: HashMap<String, CachedToolResult>,
}

impl ToolResultCache {
    /// Create new cache with capacity and TTL
    #[must_use]
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            capacity,
            ttl,
            cache: HashMap::new(),
        }
    }

    /// Get cached result if available and not expired
    pub fn get(&mut self, key: &str) -> Option<ToolResult> {
        // Check if entry exists and is not expired
        if let Some(entry) = self.cache.get_mut(key) {
            if entry.is_expired(self.ttl) {
                self.cache.remove(key);
                return None;
            }

            // Touch for LRU
            entry.touch();
            return Some(entry.result.clone());
        }

        None
    }

    /// Insert result into cache
    pub fn insert(&mut self, key: String, result: ToolResult) {
        // Evict expired entries
        self.evict_expired();

        // Evict LRU if at capacity
        if self.cache.len() >= self.capacity {
            self.evict_lru();
        }

        // Insert new entry
        self.cache.insert(key, CachedToolResult::new(result));
    }

    /// Evict expired entries
    fn evict_expired(&mut self) {
        self.cache.retain(|_, entry| !entry.is_expired(self.ttl));
    }

    /// Evict least-recently-used entry
    fn evict_lru(&mut self) {
        if let Some((lru_key, _)) = self.cache.iter()
            .min_by_key(|(_, entry)| entry.last_accessed)
            .map(|(k, v)| (k.clone(), v.clone()))
        {
            self.cache.remove(&lru_key);
        }
    }

    /// Get cache statistics
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            size: self.cache.len(),
            capacity: self.capacity,
        }
    }
}

/// Cache statistics
#[derive(Clone, Debug)]
pub struct CacheStats {
    /// Current size
    pub size: usize,
    /// Maximum capacity
    pub capacity: usize,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // Test code can use unwrap/expect
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_cache_basic() {
        let mut cache = ToolResultCache::new(2, Duration::from_secs(60));

        cache.insert("key1".to_string(), Ok("result1".to_string()));
        let result = cache.get("key1");
        assert!(result.is_some());
        assert!(result.as_ref().is_some_and(|r| r.as_ref().is_ok_and(|s| s == "result1")));
    }

    #[test]
    fn test_cache_lru_eviction() {
        let mut cache = ToolResultCache::new(2, Duration::from_secs(60));

        cache.insert("key1".to_string(), Ok("result1".to_string()));
        cache.insert("key2".to_string(), Ok("result2".to_string()));

        // Access key1 to make it more recent
        let _ = cache.get("key1");

        // Insert key3, should evict key2 (LRU)
        cache.insert("key3".to_string(), Ok("result3".to_string()));

        assert!(cache.get("key1").is_some());
        assert!(cache.get("key2").is_none());
        assert!(cache.get("key3").is_some());
    }

    #[test]
    fn test_cache_ttl() {
        let mut cache = ToolResultCache::new(10, Duration::from_millis(50));

        cache.insert("key1".to_string(), Ok("result1".to_string()));
        assert!(cache.get("key1").is_some());

        // Wait for expiration
        sleep(Duration::from_millis(60));

        assert!(cache.get("key1").is_none());
    }

    #[test]
    fn test_cache_stats() {
        let mut cache = ToolResultCache::new(5, Duration::from_secs(60));

        cache.insert("key1".to_string(), Ok("result1".to_string()));
        cache.insert("key2".to_string(), Ok("result2".to_string()));

        let stats = cache.stats();
        assert_eq!(stats.size, 2);
        assert_eq!(stats.capacity, 5);
    }
}
