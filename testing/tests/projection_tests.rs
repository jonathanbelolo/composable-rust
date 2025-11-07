//! Tests for projection testing utilities

#![allow(clippy::unwrap_used)] // Tests can unwrap
#![allow(clippy::expect_used)] // Tests can expect

use composable_rust_core::projection::{
    EventPosition, Projection, ProjectionCheckpoint, ProjectionStore,
};
use composable_rust_testing::{
    InMemoryProjectionCheckpoint, InMemoryProjectionStore, ProjectionTestHarness,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// Simple projection for testing
#[derive(Clone, Debug)]
struct TestProjection {
    store: Arc<InMemoryProjectionStore>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum TestEvent {
    Created { id: String, data: String },
    Updated { id: String, data: String },
    Deleted { id: String },
}

impl Projection for TestProjection {
    type Event = TestEvent;

    fn name(&self) -> &'static str {
        "test_projection"
    }

    async fn apply_event(
        &self,
        event: &Self::Event,
    ) -> composable_rust_core::projection::Result<()> {
        match event {
            TestEvent::Created { id, data} | TestEvent::Updated { id, data } => {
                self.store.save(id, data.as_bytes()).await?;
            }
            TestEvent::Deleted { id } => {
                self.store.delete(id).await?;
            }
        }
        Ok(())
    }

    async fn rebuild(&self) -> composable_rust_core::projection::Result<()> {
        self.store.clear();
        Ok(())
    }
}

#[tokio::test]
async fn test_inmemory_projection_store_save_and_get() {
    let store = InMemoryProjectionStore::new();

    // Initially empty
    assert!(store.is_empty());
    assert_eq!(store.len(), 0);

    // Save some data
    store.save("key1", b"value1").await.unwrap();
    assert!(!store.is_empty());
    assert_eq!(store.len(), 1);

    // Retrieve data
    let data = store.get("key1").await.unwrap();
    assert_eq!(data, Some(b"value1".to_vec()));

    // Non-existent key
    let missing = store.get("missing").await.unwrap();
    assert_eq!(missing, None);
}

#[tokio::test]
async fn test_inmemory_projection_store_exists() {
    let store = InMemoryProjectionStore::new();

    assert!(!store.exists("key1").await.unwrap());

    store.save("key1", b"value1").await.unwrap();
    assert!(store.exists("key1").await.unwrap());

    store.delete("key1").await.unwrap();
    assert!(!store.exists("key1").await.unwrap());
}

#[tokio::test]
async fn test_inmemory_projection_store_delete() {
    let store = InMemoryProjectionStore::new();

    store.save("key1", b"value1").await.unwrap();
    store.save("key2", b"value2").await.unwrap();
    assert_eq!(store.len(), 2);

    store.delete("key1").await.unwrap();
    assert_eq!(store.len(), 1);
    assert!(!store.exists("key1").await.unwrap());
    assert!(store.exists("key2").await.unwrap());
}

#[tokio::test]
async fn test_inmemory_projection_store_contains_key() {
    let store = InMemoryProjectionStore::new();

    assert!(!store.contains_key("key1"));

    store.save("key1", b"value1").await.unwrap();
    assert!(store.contains_key("key1"));
}

#[tokio::test]
async fn test_inmemory_projection_store_keys() {
    let store = InMemoryProjectionStore::new();

    assert_eq!(store.keys().len(), 0);

    store.save("key1", b"value1").await.unwrap();
    store.save("key2", b"value2").await.unwrap();
    store.save("key3", b"value3").await.unwrap();

    let mut keys = store.keys();
    keys.sort();
    assert_eq!(keys, vec!["key1", "key2", "key3"]);
}

#[tokio::test]
async fn test_inmemory_projection_store_clear() {
    let store = InMemoryProjectionStore::new();

    store.save("key1", b"value1").await.unwrap();
    store.save("key2", b"value2").await.unwrap();
    assert_eq!(store.len(), 2);

    store.clear();
    assert_eq!(store.len(), 0);
    assert!(store.is_empty());
    assert!(!store.exists("key1").await.unwrap());
}

#[tokio::test]
async fn test_inmemory_projection_store_overwrite() {
    let store = InMemoryProjectionStore::new();

    store.save("key1", b"value1").await.unwrap();
    assert_eq!(store.get("key1").await.unwrap(), Some(b"value1".to_vec()));

    // Overwrite with new value
    store.save("key1", b"value2").await.unwrap();
    assert_eq!(store.get("key1").await.unwrap(), Some(b"value2".to_vec()));
    assert_eq!(store.len(), 1); // Still only one key
}

#[tokio::test]
async fn test_inmemory_projection_checkpoint_save_and_load() {
    use chrono::Utc;

    let checkpoint = InMemoryProjectionCheckpoint::new();

    // Initially empty
    assert!(checkpoint.is_empty());
    assert_eq!(checkpoint.len(), 0);

    // Save position
    let position = EventPosition::new(42, Utc::now());
    checkpoint
        .save_position("projection1", position)
        .await
        .unwrap();

    assert!(!checkpoint.is_empty());
    assert_eq!(checkpoint.len(), 1);

    // Load position
    let loaded = checkpoint.load_position("projection1").await.unwrap();
    assert_eq!(loaded, Some(position));

    // Non-existent projection
    let missing = checkpoint.load_position("missing").await.unwrap();
    assert_eq!(missing, None);
}

#[tokio::test]
async fn test_inmemory_projection_checkpoint_multiple_projections() {
    use chrono::Utc;

    let checkpoint = InMemoryProjectionCheckpoint::new();

    let pos1 = EventPosition::new(10, Utc::now());
    let pos2 = EventPosition::new(20, Utc::now());
    let pos3 = EventPosition::new(30, Utc::now());

    checkpoint.save_position("proj1", pos1).await.unwrap();
    checkpoint.save_position("proj2", pos2).await.unwrap();
    checkpoint.save_position("proj3", pos3).await.unwrap();

    assert_eq!(checkpoint.len(), 3);

    assert_eq!(
        checkpoint.load_position("proj1").await.unwrap(),
        Some(pos1)
    );
    assert_eq!(
        checkpoint.load_position("proj2").await.unwrap(),
        Some(pos2)
    );
    assert_eq!(
        checkpoint.load_position("proj3").await.unwrap(),
        Some(pos3)
    );
}

#[tokio::test]
async fn test_inmemory_projection_checkpoint_overwrite() {
    use chrono::Utc;

    let checkpoint = InMemoryProjectionCheckpoint::new();

    let pos1 = EventPosition::new(10, Utc::now());
    let pos2 = EventPosition::new(20, Utc::now());

    checkpoint.save_position("proj1", pos1).await.unwrap();
    assert_eq!(
        checkpoint.load_position("proj1").await.unwrap(),
        Some(pos1)
    );

    // Overwrite with new position
    checkpoint.save_position("proj1", pos2).await.unwrap();
    assert_eq!(
        checkpoint.load_position("proj1").await.unwrap(),
        Some(pos2)
    );
    assert_eq!(checkpoint.len(), 1); // Still only one projection
}

#[tokio::test]
async fn test_inmemory_projection_checkpoint_projection_names() {
    use chrono::Utc;

    let checkpoint = InMemoryProjectionCheckpoint::new();

    let pos = EventPosition::new(1, Utc::now());
    checkpoint.save_position("proj1", pos).await.unwrap();
    checkpoint.save_position("proj2", pos).await.unwrap();
    checkpoint.save_position("proj3", pos).await.unwrap();

    let mut names = checkpoint.projection_names();
    names.sort();
    assert_eq!(names, vec!["proj1", "proj2", "proj3"]);
}

#[tokio::test]
async fn test_inmemory_projection_checkpoint_clear() {
    use chrono::Utc;

    let checkpoint = InMemoryProjectionCheckpoint::new();

    let pos = EventPosition::new(1, Utc::now());
    checkpoint.save_position("proj1", pos).await.unwrap();
    checkpoint.save_position("proj2", pos).await.unwrap();
    assert_eq!(checkpoint.len(), 2);

    checkpoint.clear();
    assert_eq!(checkpoint.len(), 0);
    assert!(checkpoint.is_empty());
    assert_eq!(checkpoint.load_position("proj1").await.unwrap(), None);
}

#[tokio::test]
async fn test_projection_harness_basic_workflow() {
    let store = Arc::new(InMemoryProjectionStore::new());
    let projection = TestProjection {
        store: store.clone(),
    };
    let mut harness = ProjectionTestHarness::new(projection, store);

    // Initially empty
    assert!(harness.is_empty());

    // Apply an event
    harness
        .given_event(TestEvent::Created {
            id: "item1".to_string(),
            data: "data1".to_string(),
        })
        .await
        .unwrap();

    // Verify data was stored
    harness.then_contains("item1").await.unwrap();
    assert_eq!(harness.len(), 1);

    let data = harness.get_data("item1").await.unwrap();
    assert_eq!(data, Some(b"data1".to_vec()));
}

#[tokio::test]
async fn test_projection_harness_multiple_events() {
    let store = Arc::new(InMemoryProjectionStore::new());
    let projection = TestProjection {
        store: store.clone(),
    };
    let mut harness = ProjectionTestHarness::new(projection, store);

    let events = vec![
        TestEvent::Created {
            id: "item1".to_string(),
            data: "data1".to_string(),
        },
        TestEvent::Created {
            id: "item2".to_string(),
            data: "data2".to_string(),
        },
        TestEvent::Updated {
            id: "item1".to_string(),
            data: "updated1".to_string(),
        },
    ];

    harness.given_events(events).await.unwrap();

    harness.then_contains("item1").await.unwrap();
    harness.then_contains("item2").await.unwrap();
    assert_eq!(harness.len(), 2);

    // Verify item1 was updated
    let data = harness.get_data("item1").await.unwrap();
    assert_eq!(data, Some(b"updated1".to_vec()));
}

#[tokio::test]
async fn test_projection_harness_delete() {
    let store = Arc::new(InMemoryProjectionStore::new());
    let projection = TestProjection {
        store: store.clone(),
    };
    let mut harness = ProjectionTestHarness::new(projection, store);

    // Create and then delete
    harness
        .given_event(TestEvent::Created {
            id: "item1".to_string(),
            data: "data1".to_string(),
        })
        .await
        .unwrap();

    harness.then_contains("item1").await.unwrap();

    harness
        .given_event(TestEvent::Deleted {
            id: "item1".to_string(),
        })
        .await
        .unwrap();

    harness.then_not_contains("item1").await.unwrap();
    assert!(harness.is_empty());
}

#[tokio::test]
async fn test_projection_harness_clear() {
    let store = Arc::new(InMemoryProjectionStore::new());
    let projection = TestProjection {
        store: store.clone(),
    };
    let mut harness = ProjectionTestHarness::new(projection, store);

    harness
        .given_event(TestEvent::Created {
            id: "item1".to_string(),
            data: "data1".to_string(),
        })
        .await
        .unwrap();

    assert_eq!(harness.len(), 1);

    harness.clear();
    assert!(harness.is_empty());
    assert_eq!(harness.len(), 0);
}

#[tokio::test]
async fn test_projection_harness_store_access() {
    let store = Arc::new(InMemoryProjectionStore::new());
    let projection = TestProjection {
        store: store.clone(),
    };
    let harness = ProjectionTestHarness::new(projection, store.clone());

    // Direct store access
    let store_ref = harness.store();
    store_ref.save("direct", b"value").await.unwrap();

    assert!(store_ref.exists("direct").await.unwrap());
}

#[tokio::test]
async fn test_projection_harness_projection_access() {
    let store = Arc::new(InMemoryProjectionStore::new());
    let projection = TestProjection {
        store: store.clone(),
    };
    let harness = ProjectionTestHarness::new(projection, store);

    // Access projection properties
    assert_eq!(harness.projection().name(), "test_projection");
}
