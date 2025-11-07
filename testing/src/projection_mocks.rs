//! In-memory projection testing utilities
//!
//! Provides fast, deterministic testing infrastructure for projections:
//! - [`InMemoryProjectionStore`]: HashMap-based projection storage
//! - [`InMemoryProjectionCheckpoint`]: In-memory checkpoint tracking
//! - [`ProjectionTestHarness`]: Fluent API for projection tests

#![allow(clippy::unwrap_used)] // Test infrastructure uses unwrap for simplicity
#![allow(clippy::missing_panics_doc)] // Test utilities document panics where critical

use composable_rust_core::projection::{
    EventPosition, Projection, ProjectionCheckpoint, ProjectionStore, Result,
};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

/// In-memory projection store for fast, deterministic testing.
///
/// Complements `InMemoryEventStore` and `InMemoryEventBus` to provide
/// a complete in-memory testing infrastructure.
///
/// # Example
///
/// ```
/// use composable_rust_testing::InMemoryProjectionStore;
/// use composable_rust_core::projection::ProjectionStore;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let store = InMemoryProjectionStore::new();
///
/// // Save projection data
/// store.save("customer:123", b"customer data").await?;
///
/// // Retrieve projection data
/// let data = store.get("customer:123").await?;
/// assert!(data.is_some());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct InMemoryProjectionStore {
    data: Arc<RwLock<HashMap<String, Vec<u8>>>>,
}

impl InMemoryProjectionStore {
    /// Create a new empty in-memory projection store
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Clear all projection data (for test isolation)
    ///
    /// Useful for resetting state between tests without creating a new store.
    pub fn clear(&self) {
        self.data.write().unwrap().clear();
    }

    /// Get the number of stored projections
    ///
    /// Useful for assertions in tests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.read().unwrap().len()
    }

    /// Check if the store is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.read().unwrap().is_empty()
    }

    /// Check if a key exists in the store
    ///
    /// Useful for existence checks in tests without retrieving the data.
    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.data.read().unwrap().contains_key(key)
    }

    /// Get all keys in the store
    ///
    /// Useful for inspecting stored projections in tests.
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        self.data.read().unwrap().keys().cloned().collect()
    }
}

impl Default for InMemoryProjectionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectionStore for InMemoryProjectionStore {
    async fn save(&self, key: &str, data: &[u8]) -> Result<()> {
        self.data
            .write()
            .unwrap()
            .insert(key.to_string(), data.to_vec());
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.data.read().unwrap().get(key).cloned())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.data.write().unwrap().remove(key);
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        Ok(self.data.read().unwrap().contains_key(key))
    }
}

/// In-memory checkpoint tracking for testing projection resumption.
///
/// Stores checkpoint positions in a `HashMap` for fast, deterministic testing.
///
/// # Example
///
/// ```
/// use composable_rust_testing::InMemoryProjectionCheckpoint;
/// use composable_rust_core::projection::{EventPosition, ProjectionCheckpoint};
/// use chrono::Utc;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let checkpoint = InMemoryProjectionCheckpoint::new();
///
/// // Save checkpoint
/// let position = EventPosition::new(42, Utc::now());
/// checkpoint.save_position("my_projection", position).await?;
///
/// // Load checkpoint
/// let loaded = checkpoint.load_position("my_projection").await?;
/// assert_eq!(loaded, Some(position));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct InMemoryProjectionCheckpoint {
    positions: Arc<RwLock<HashMap<String, EventPosition>>>,
}

impl InMemoryProjectionCheckpoint {
    /// Create a new empty checkpoint tracker
    #[must_use]
    pub fn new() -> Self {
        Self {
            positions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Clear all checkpoints
    ///
    /// Useful for test isolation.
    pub fn clear(&self) {
        self.positions.write().unwrap().clear();
    }

    /// Get all projection names with checkpoints
    #[must_use]
    pub fn projection_names(&self) -> Vec<String> {
        self.positions.read().unwrap().keys().cloned().collect()
    }

    /// Get the number of tracked projections
    #[must_use]
    pub fn len(&self) -> usize {
        self.positions.read().unwrap().len()
    }

    /// Check if no projections are tracked
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.positions.read().unwrap().is_empty()
    }
}

impl Default for InMemoryProjectionCheckpoint {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectionCheckpoint for InMemoryProjectionCheckpoint {
    fn save_position(
        &self,
        projection_name: &str,
        position: EventPosition,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let projection_name = projection_name.to_string();
        Box::pin(async move {
            self.positions
                .write()
                .unwrap()
                .insert(projection_name, position);
            Ok(())
        })
    }

    fn load_position(
        &self,
        projection_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<EventPosition>>> + Send + '_>> {
        let projection_name = projection_name.to_string();
        Box::pin(async move {
            Ok(self.positions.read().unwrap().get(&projection_name).copied())
        })
    }
}

/// Test harness for projections providing a fluent testing API.
///
/// This helper makes projection tests more readable and easier to write
/// by providing a builder-style interface for applying events and making
/// assertions.
///
/// # Example
///
/// ```ignore
/// let store = Arc::new(InMemoryProjectionStore::new());
/// let projection = MyProjection::new(store.clone());
/// let harness = ProjectionTestHarness::new(projection, store);
///
/// harness
///     .given_events(vec![event1, event2])
///     .await
///     .then_contains("order-1")
///     .await;
/// ```
pub struct ProjectionTestHarness<P: Projection> {
    projection: P,
    store: Arc<InMemoryProjectionStore>,
}

impl<P: Projection> ProjectionTestHarness<P> {
    /// Create a new test harness for the given projection.
    ///
    /// The harness provides access to both the projection and its backing store
    /// for making assertions.
    ///
    /// # Arguments
    ///
    /// - `projection`: The projection to test
    /// - `store`: The store used by the projection (must be the same instance)
    #[must_use]
    pub const fn new(projection: P, store: Arc<InMemoryProjectionStore>) -> Self {
        Self { projection, store }
    }

    /// Apply a series of events to the projection.
    ///
    /// Events are applied in order. If any event fails to apply,
    /// the error is propagated.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError`] if any event fails to apply.
    pub async fn given_events(&mut self, events: Vec<P::Event>) -> Result<&mut Self> {
        for event in events {
            self.projection.apply_event(&event).await?;
        }
        Ok(self)
    }

    /// Apply a single event to the projection.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError`] if the event fails to apply.
    pub async fn given_event(&mut self, event: P::Event) -> Result<&mut Self> {
        self.projection.apply_event(&event).await?;
        Ok(self)
    }

    /// Assert that the projection store contains the given key.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError`] if the query fails.
    ///
    /// # Panics
    ///
    /// Panics if the key is not found in the store (this is a test assertion).
    #[allow(clippy::panic)] // Intentional panic for test assertions
    pub async fn then_contains(&self, key: &str) -> Result<&Self> {
        let exists = self.store.exists(key).await?;
        assert!(
            exists,
            "Expected projection store to contain key '{key}', but it was not found"
        );
        Ok(self)
    }

    /// Assert that the projection store does not contain the given key.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError`] if the query fails.
    ///
    /// # Panics
    ///
    /// Panics if the key is found in the store (this is a test assertion).
    #[allow(clippy::panic)] // Intentional panic for test assertions
    pub async fn then_not_contains(&self, key: &str) -> Result<&Self> {
        let exists = self.store.exists(key).await?;
        assert!(
            !exists,
            "Expected projection store to NOT contain key '{key}', but it was found"
        );
        Ok(self)
    }

    /// Get data from the projection store.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError`] if the query fails.
    pub async fn get_data(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.store.get(key).await
    }

    /// Get a reference to the underlying projection store.
    ///
    /// Useful for making custom assertions or inspecting the store state.
    #[must_use]
    pub const fn store(&self) -> &Arc<InMemoryProjectionStore> {
        &self.store
    }

    /// Get a reference to the projection.
    ///
    /// Useful for calling projection-specific query methods.
    #[must_use]
    pub const fn projection(&self) -> &P {
        &self.projection
    }

    /// Clear all data from the projection store.
    ///
    /// Useful for resetting state between test scenarios.
    pub fn clear(&self) {
        self.store.clear();
    }

    /// Get the number of entries in the projection store.
    #[must_use]
    pub fn len(&self) -> usize {
        self.store.len()
    }

    /// Check if the projection store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }
}
