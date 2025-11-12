//! Integration tests for [`RedpandaEventBus`] with real Kafka/Redpanda instance.
//!
//! These tests use testcontainers to spin up a real Kafka instance and validate:
//! - Publish/subscribe round-trip
//! - Consumer groups and load balancing
//! - At-least-once delivery semantics
//! - Offset commits
//! - Multiple topics
//!
//! # Running These Tests
//!
//! These tests are marked as `#[ignore]` by default because they:
//! - Require Docker to be running (for testcontainers)
//! - Take 15-60 seconds per test to spin up Kafka
//! - Can be flaky due to Kafka's distributed nature and timing
//!
//! To run explicitly:
//! ```bash
//! cargo test -p composable-rust-redpanda --test integration_tests -- --ignored
//! ```
//!
//! # Panics
//!
//! These tests use `expect()` and `panic!()` for setup failures, which is acceptable in test code.

#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use composable_rust_core::event::SerializedEvent;
use composable_rust_core::event_bus::EventBus;
use composable_rust_redpanda::RedpandaEventBus;
use futures::StreamExt;
use std::collections::HashSet;
use std::time::Duration;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::kafka::{KAFKA_PORT, Kafka};

/// Helper to create a test event
fn test_event(event_type: &str, data: Vec<u8>) -> SerializedEvent {
    SerializedEvent::new(event_type.to_string(), data, None)
}

/// Helper to wait for Kafka to be ready
async fn wait_for_kafka_ready(brokers: &str) {
    let max_attempts = 60;
    for attempt in 1..=max_attempts {
        if let Ok(bus) = RedpandaEventBus::builder()
            .brokers(brokers)
            .auto_offset_reset("earliest")
            .build()
        {
            // Try to publish a test event
            let event = test_event("warmup", vec![1, 2, 3]);
            if bus.publish("warmup-topic", &event).await.is_ok() {
                // Give Kafka time to fully initialize
                tokio::time::sleep(Duration::from_millis(500)).await;
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            attempt != max_attempts,
            "Kafka failed to become ready after {max_attempts} attempts"
        );
    }
}

/// Helper to ensure topic exists and metadata has propagated
/// This publishes a warmup event to trigger auto-creation
async fn ensure_topic_exists(event_bus: &RedpandaEventBus, topic: &str) {
    // Publish warmup event to trigger topic auto-creation
    let warmup = test_event("warmup", vec![255]);
    for attempt in 1..=30 {
        if event_bus.publish(topic, &warmup).await.is_ok() {
            // Wait for topic metadata to propagate throughout cluster
            tokio::time::sleep(Duration::from_secs(3)).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(attempt != 30, "Failed to create topic {topic}");
    }
}

#[tokio::test]
#[ignore]
async fn test_publish_and_subscribe_round_trip() {
    // Start Kafka container
    let kafka = Kafka::default()
        .with_env_var("KAFKA_AUTO_CREATE_TOPICS_ENABLE", "true")
        .start()
        .await
        .expect("Failed to start Kafka container");

    let host = kafka.get_host().await.expect("Failed to get host");
    let port = kafka
        .get_host_port_ipv4(KAFKA_PORT)
        .await
        .expect("Failed to get port");
    let brokers = format!("{host}:{port}");
    wait_for_kafka_ready(&brokers).await;

    // Create event bus
    let event_bus = RedpandaEventBus::builder()
        .brokers(&brokers)
        .auto_offset_reset("earliest") // Read from beginning for testing
        .build()
        .expect("Failed to create event bus");

    // Wait for topic to be ready
    ensure_topic_exists(&event_bus, "test-events").await;

    // Subscribe before publishing to ensure we don't miss events
    let mut stream = event_bus
        .subscribe(&["test-events"])
        .await
        .expect("Failed to subscribe");

    // Give consumer time to subscribe and rebalance
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Publish test events
    let event1 = test_event("OrderPlaced", vec![1, 2, 3]);
    let event2 = test_event("PaymentCompleted", vec![4, 5, 6]);

    event_bus
        .publish("test-events", &event1)
        .await
        .expect("Failed to publish event1");
    event_bus
        .publish("test-events", &event2)
        .await
        .expect("Failed to publish event2");

    // Receive events (skip warmup events)
    let mut received = Vec::new();
    let timeout = tokio::time::timeout(Duration::from_secs(10), async {
        while received.len() < 2 {
            if let Some(result) = stream.next().await {
                let event = result.expect("Failed to receive event");
                if event.event_type != "warmup" {
                    received.push(event);
                }
            }
        }
    });

    timeout.await.expect("Timeout waiting for events");

    // Verify events
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].event_type, "OrderPlaced");
    assert_eq!(received[0].data, vec![1, 2, 3]);
    assert_eq!(received[1].event_type, "PaymentCompleted");
    assert_eq!(received[1].data, vec![4, 5, 6]);
}

#[tokio::test]
#[ignore]
async fn test_consumer_groups_load_balancing() {
    // Start Kafka container
    let kafka = Kafka::default()
        .with_env_var("KAFKA_AUTO_CREATE_TOPICS_ENABLE", "true")
        .start()
        .await
        .expect("Failed to start Kafka container");

    let host = kafka.get_host().await.expect("Failed to get host");
    let port = kafka
        .get_host_port_ipv4(KAFKA_PORT)
        .await
        .expect("Failed to get port");
    let brokers = format!("{host}:{port}");
    wait_for_kafka_ready(&brokers).await;

    // Create event bus with explicit consumer group
    let event_bus = RedpandaEventBus::builder()
        .brokers(&brokers)
        .consumer_group("test-group")
        .auto_offset_reset("earliest")
        .build()
        .expect("Failed to create event bus");

    // Wait for topic to be ready
    ensure_topic_exists(&event_bus, "load-balance-events").await;

    // Create two consumers in the same group
    let mut stream1 = event_bus
        .subscribe(&["load-balance-events"])
        .await
        .expect("Failed to subscribe consumer 1");

    let mut stream2 = event_bus
        .subscribe(&["load-balance-events"])
        .await
        .expect("Failed to subscribe consumer 2");

    // Give consumers time to join group and rebalance
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Publish 10 events
    for i in 0..10 {
        let event = test_event(&format!("Event{i}"), vec![i as u8]);
        event_bus
            .publish("load-balance-events", &event)
            .await
            .expect("Failed to publish event");
    }

    // Collect events from both consumers (skip warmup events)
    let mut received1 = HashSet::new();
    let mut received2 = HashSet::new();

    let timeout = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                Some(result) = stream1.next() => {
                    if let Ok(event) = result {
                        if event.event_type != "warmup" {
                            received1.insert(event.event_type.clone());
                        }
                    }
                }
                Some(result) = stream2.next() => {
                    if let Ok(event) = result {
                        if event.event_type != "warmup" {
                            received2.insert(event.event_type.clone());
                        }
                    }
                }
            }

            // Stop when we've received all 10 events total
            if received1.len() + received2.len() >= 10 {
                break;
            }
        }
    });

    timeout.await.expect("Timeout waiting for events");

    // Verify load balancing: both consumers should have received some events
    // (though not necessarily equal distribution due to partitioning)
    assert!(
        !received1.is_empty(),
        "Consumer 1 should have received events"
    );
    assert!(
        !received2.is_empty(),
        "Consumer 2 should have received events"
    );

    // Verify no duplicates across consumers
    assert!(
        received1.is_disjoint(&received2),
        "Consumers should not receive duplicate events"
    );

    // Verify all events received
    let total: HashSet<_> = received1.union(&received2).cloned().collect();
    assert_eq!(total.len(), 10, "Should have received all 10 unique events");
}

#[tokio::test]
#[ignore]
async fn test_multiple_topics() {
    // Start Kafka container
    let kafka = Kafka::default()
        .with_env_var("KAFKA_AUTO_CREATE_TOPICS_ENABLE", "true")
        .start()
        .await
        .expect("Failed to start Kafka container");

    let host = kafka.get_host().await.expect("Failed to get host");
    let port = kafka
        .get_host_port_ipv4(KAFKA_PORT)
        .await
        .expect("Failed to get port");
    let brokers = format!("{host}:{port}");
    wait_for_kafka_ready(&brokers).await;

    // Create event bus
    let event_bus = RedpandaEventBus::builder()
        .brokers(&brokers)
        .auto_offset_reset("earliest")
        .build()
        .expect("Failed to create event bus");

    // Wait for topics to be ready
    ensure_topic_exists(&event_bus, "orders").await;
    ensure_topic_exists(&event_bus, "payments").await;

    // Subscribe to multiple topics
    let mut stream = event_bus
        .subscribe(&["orders", "payments"])
        .await
        .expect("Failed to subscribe");

    // Give consumer time to subscribe and rebalance
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Publish to both topics
    let order_event = test_event("OrderPlaced", vec![1, 2]);
    let payment_event = test_event("PaymentCompleted", vec![3, 4]);

    event_bus
        .publish("orders", &order_event)
        .await
        .expect("Failed to publish order event");
    event_bus
        .publish("payments", &payment_event)
        .await
        .expect("Failed to publish payment event");

    // Receive events (skip warmup events)
    let mut received = Vec::new();
    let timeout = tokio::time::timeout(Duration::from_secs(10), async {
        while received.len() < 2 {
            if let Some(result) = stream.next().await {
                let event = result.expect("Failed to receive event");
                if event.event_type != "warmup" {
                    received.push(event);
                }
            }
        }
    });

    timeout.await.expect("Timeout waiting for events");

    // Verify we got both events (order may vary)
    assert_eq!(received.len(), 2);
    let event_types: HashSet<_> = received.iter().map(|e| e.event_type.as_str()).collect();
    assert!(event_types.contains("OrderPlaced"));
    assert!(event_types.contains("PaymentCompleted"));
}

#[tokio::test]
#[ignore]
async fn test_at_least_once_delivery() {
    // Start Kafka container
    let kafka = Kafka::default()
        .with_env_var("KAFKA_AUTO_CREATE_TOPICS_ENABLE", "true")
        .start()
        .await
        .expect("Failed to start Kafka container");

    let host = kafka.get_host().await.expect("Failed to get host");
    let port = kafka
        .get_host_port_ipv4(KAFKA_PORT)
        .await
        .expect("Failed to get port");
    let brokers = format!("{host}:{port}");
    wait_for_kafka_ready(&brokers).await;

    // Create event bus with explicit consumer group
    let event_bus = RedpandaEventBus::builder()
        .brokers(&brokers)
        .consumer_group("at-least-once-test")
        .auto_offset_reset("earliest")
        .build()
        .expect("Failed to create event bus");

    // Publish events before subscribing
    let event1 = test_event("Event1", vec![1]);
    let event2 = test_event("Event2", vec![2]);

    event_bus
        .publish("persistence-test", &event1)
        .await
        .expect("Failed to publish event1");
    event_bus
        .publish("persistence-test", &event2)
        .await
        .expect("Failed to publish event2");

    // Wait for events to be persisted
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Subscribe and verify we get events from the beginning
    let mut stream = event_bus
        .subscribe(&["persistence-test"])
        .await
        .expect("Failed to subscribe");

    let mut received = Vec::new();
    let timeout = tokio::time::timeout(Duration::from_secs(10), async {
        while received.len() < 2 {
            if let Some(result) = stream.next().await {
                let event = result.expect("Failed to receive event");
                received.push(event);
            }
        }
    });

    timeout.await.expect("Timeout waiting for events");

    // Verify events persisted and delivered
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].event_type, "Event1");
    assert_eq!(received[1].event_type, "Event2");
}

#[tokio::test]
#[ignore]
async fn test_event_ordering_within_partition() {
    // Start Kafka container
    let kafka = Kafka::default()
        .with_env_var("KAFKA_AUTO_CREATE_TOPICS_ENABLE", "true")
        .start()
        .await
        .expect("Failed to start Kafka container");

    let host = kafka.get_host().await.expect("Failed to get host");
    let port = kafka
        .get_host_port_ipv4(KAFKA_PORT)
        .await
        .expect("Failed to get port");
    let brokers = format!("{host}:{port}");
    wait_for_kafka_ready(&brokers).await;

    // Create event bus
    let event_bus = RedpandaEventBus::builder()
        .brokers(&brokers)
        .auto_offset_reset("earliest")
        .build()
        .expect("Failed to create event bus");

    // Wait for topic to be ready
    ensure_topic_exists(&event_bus, "ordering-test").await;

    // Subscribe
    let mut stream = event_bus
        .subscribe(&["ordering-test"])
        .await
        .expect("Failed to subscribe");

    // Give consumer time to subscribe and rebalance
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Publish events of the same type (will go to same partition due to event_type key)
    for i in 0..5 {
        let event = test_event("OrderEvent", vec![i as u8]);
        event_bus
            .publish("ordering-test", &event)
            .await
            .expect("Failed to publish event");
    }

    // Receive events and verify ordering (skip warmup events)
    let mut received = Vec::new();
    let timeout = tokio::time::timeout(Duration::from_secs(10), async {
        while received.len() < 5 {
            if let Some(result) = stream.next().await {
                let event = result.expect("Failed to receive event");
                if event.event_type != "warmup" {
                    received.push(event);
                }
            }
        }
    });

    timeout.await.expect("Timeout waiting for events");

    // Verify ordering maintained
    assert_eq!(received.len(), 5);
    for (i, event) in received.iter().enumerate() {
        assert_eq!(event.data, vec![i as u8]);
    }
}

#[tokio::test]
#[ignore]
async fn test_producer_configuration() {
    // Start Kafka container
    let kafka = Kafka::default()
        .with_env_var("KAFKA_AUTO_CREATE_TOPICS_ENABLE", "true")
        .start()
        .await
        .expect("Failed to start Kafka container");

    let host = kafka.get_host().await.expect("Failed to get host");
    let port = kafka
        .get_host_port_ipv4(KAFKA_PORT)
        .await
        .expect("Failed to get port");
    let brokers = format!("{host}:{port}");
    wait_for_kafka_ready(&brokers).await;

    // Test builder configuration
    let event_bus = RedpandaEventBus::builder()
        .brokers(&brokers)
        .producer_acks("all") // Wait for all replicas
        .compression("lz4")
        .timeout(Duration::from_secs(10))
        .buffer_size(5000)
        .consumer_group("custom-group")
        .auto_offset_reset("earliest")
        .build()
        .expect("Failed to create event bus");

    // Verify it works
    let event = test_event("ConfigTest", vec![1, 2, 3]);
    event_bus
        .publish("config-test", &event)
        .await
        .expect("Failed to publish with custom config");

    // Verify brokers accessor
    assert_eq!(event_bus.brokers(), brokers);
}
