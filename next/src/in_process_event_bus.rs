//! In-process event bus using broadcast channels.
//!
//! This module provides an [`InProcessEventBus`] implementation that uses
//! Tokio broadcast channels for in-process publish/subscribe communication.
//!
//! # Use Cases
//!
//! - Single-process applications that don't need distributed messaging
//! - Development and testing environments
//! - Scenarios where low latency is critical
//!
//! # Example
//!
//! ```rust,ignore
//! use composable_rust_next::InProcessEventBus;
//! use futures::FutureExt;
//!
//! let bus = InProcessEventBus::new(100);
//!
//! // Subscribe to events
//! let handle = bus.subscribe("orders", |event| {
//!     async move {
//!         println!("Received: {}", event.event_type);
//!         Ok(())
//!     }.boxed()
//! }).await?;
//!
//! // Publish an event
//! bus.publish("orders", event).await?;
//!
//! // Cancel subscription when done
//! handle.cancel();
//! ```
//!
//! # Limitations
//!
//! - Events are not persisted; if no subscribers are listening, events are lost
//! - Not suitable for distributed systems; use Redpanda/Kafka for that
//! - Topic filtering is ignored; all subscribers receive all events

use crate::{EventBus, EventBusError, SerializedEvent, SubscriptionHandle};
use futures::future::BoxFuture;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Default channel capacity for the broadcast channel.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// In-process event bus using Tokio broadcast channels.
///
/// This implementation provides pub/sub within a single process. Events
/// published to the bus are delivered to all active subscribers.
///
/// # Channel Capacity
///
/// The broadcast channel has a fixed capacity. If a subscriber falls behind,
/// older messages are dropped and the subscriber receives a `Lagged` error.
/// The default capacity is [`DEFAULT_CHANNEL_CAPACITY`].
///
/// # Thread Safety
///
/// `InProcessEventBus` is `Send + Sync` and can be shared across threads.
/// Clone is cheap (just clones the `Arc` and sender).
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_next::InProcessEventBus;
///
/// let bus = InProcessEventBus::new(100);
///
/// // Clone for use in multiple tasks
/// let bus2 = bus.clone();
/// ```
#[derive(Clone)]
pub struct InProcessEventBus {
    /// Broadcast channel sender
    sender: Arc<broadcast::Sender<SerializedEvent>>,
}

impl InProcessEventBus {
    /// Create a new in-process event bus with the specified channel capacity.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum number of messages the channel can hold.
    ///   When full, older messages are dropped for slow receivers.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let bus = InProcessEventBus::new(100);
    /// ```
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender: Arc::new(sender),
        }
    }

    /// Create a new in-process event bus with default capacity.
    ///
    /// Uses [`DEFAULT_CHANNEL_CAPACITY`] (1024 messages).
    #[must_use]
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_CHANNEL_CAPACITY)
    }

    /// Get the number of active subscribers.
    ///
    /// This is useful for debugging and monitoring.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for InProcessEventBus {
    fn default() -> Self {
        Self::with_default_capacity()
    }
}

impl std::fmt::Debug for InProcessEventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InProcessEventBus")
            .field("subscriber_count", &self.subscriber_count())
            .finish()
    }
}

impl EventBus for InProcessEventBus {
    async fn publish(&self, _topic: &str, event: SerializedEvent) -> Result<(), EventBusError> {
        // Ignore SendError (no receivers) - that's fine for pub/sub
        // Events are simply dropped if no one is listening
        let _ = self.sender.send(event);
        Ok(())
    }

    async fn publish_batch(
        &self,
        _topic: &str,
        events: &[SerializedEvent],
    ) -> Result<(), EventBusError> {
        for event in events {
            let _ = self.sender.send(event.clone());
        }
        Ok(())
    }

    async fn subscribe<F>(
        &self,
        _topic: &str,
        handler: F,
    ) -> Result<SubscriptionHandle, EventBusError>
    where
        F: Fn(SerializedEvent) -> BoxFuture<'static, Result<(), EventBusError>>
            + Send
            + Sync
            + 'static,
    {
        let mut rx = self.sender.subscribe();
        let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel();

        // Spawn a task to handle incoming events
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Check for cancellation
                    _ = &mut cancel_rx => {
                        tracing::debug!("Subscription cancelled");
                        break;
                    }
                    // Handle incoming events
                    result = rx.recv() => {
                        match result {
                            Ok(event) => {
                                // Call the handler; log errors but continue
                                if let Err(e) = handler(event).await {
                                    tracing::warn!("Event handler error: {}", e);
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(count)) => {
                                // Subscriber fell behind; log and continue
                                tracing::warn!("Subscriber lagged, missed {} events", count);
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                // Channel closed; exit
                                tracing::debug!("Broadcast channel closed");
                                break;
                            }
                        }
                    }
                }
            }
        });

        Ok(SubscriptionHandle::new(cancel_tx))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn make_event(event_type: &str) -> SerializedEvent {
        SerializedEvent {
            event_type: event_type.to_string(),
            payload: vec![1, 2, 3],
            metadata: None,
            version: None,
            stream_id: None,
        }
    }

    #[tokio::test]
    async fn publish_without_subscribers_succeeds() {
        let bus = InProcessEventBus::new(10);

        // Publishing without subscribers should succeed (events are dropped)
        let result = bus.publish("orders", make_event("OrderCreated")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn subscribe_and_receive_events() {
        let bus = InProcessEventBus::new(10);
        let received = Arc::new(AtomicUsize::new(0));
        let received_clone = received.clone();

        // Subscribe
        let handle = bus
            .subscribe("orders", move |_event| {
                let received = received_clone.clone();
                async move {
                    received.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
                .boxed()
            })
            .await
            .unwrap();

        // Give the subscription task time to start
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Publish events
        bus.publish("orders", make_event("Event1")).await.unwrap();
        bus.publish("orders", make_event("Event2")).await.unwrap();
        bus.publish("orders", make_event("Event3")).await.unwrap();

        // Give time for events to be processed
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(received.load(Ordering::SeqCst), 3);

        // Cancel subscription
        handle.cancel();
    }

    #[tokio::test]
    async fn multiple_subscribers() {
        let bus = InProcessEventBus::new(10);
        let count1 = Arc::new(AtomicUsize::new(0));
        let count2 = Arc::new(AtomicUsize::new(0));

        let count1_clone = count1.clone();
        let count2_clone = count2.clone();

        let handle1 = bus
            .subscribe("orders", move |_event| {
                let count = count1_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
                .boxed()
            })
            .await
            .unwrap();

        let handle2 = bus
            .subscribe("orders", move |_event| {
                let count = count2_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
                .boxed()
            })
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;

        // Publish
        bus.publish("orders", make_event("Event1")).await.unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Both subscribers should receive the event
        assert_eq!(count1.load(Ordering::SeqCst), 1);
        assert_eq!(count2.load(Ordering::SeqCst), 1);

        handle1.cancel();
        handle2.cancel();
    }

    #[tokio::test]
    async fn cancel_subscription() {
        let bus = InProcessEventBus::new(10);
        let received = Arc::new(AtomicUsize::new(0));
        let received_clone = received.clone();

        let handle = bus
            .subscribe("orders", move |_event| {
                let received = received_clone.clone();
                async move {
                    received.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
                .boxed()
            })
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;

        // Publish one event
        bus.publish("orders", make_event("Event1")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(received.load(Ordering::SeqCst), 1);

        // Cancel subscription
        handle.cancel();
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Publish another event after cancellation
        bus.publish("orders", make_event("Event2")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Should still be 1 (second event not received)
        assert_eq!(received.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn subscriber_count() {
        let bus = InProcessEventBus::new(10);

        assert_eq!(bus.subscriber_count(), 0);

        let handle1 = bus
            .subscribe("orders", |_| async { Ok(()) }.boxed())
            .await
            .unwrap();

        // Give time for subscription to be registered
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(bus.subscriber_count(), 1);

        let handle2 = bus
            .subscribe("orders", |_| async { Ok(()) }.boxed())
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(bus.subscriber_count(), 2);

        drop(handle1);
        drop(handle2);

        // Give time for tasks to be cancelled
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn publish_batch() {
        let bus = InProcessEventBus::new(10);
        let received = Arc::new(AtomicUsize::new(0));
        let received_clone = received.clone();

        let handle = bus
            .subscribe("orders", move |_event| {
                let received = received_clone.clone();
                async move {
                    received.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
                .boxed()
            })
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;

        // Publish batch
        bus.publish_batch(
            "orders",
            &[
                make_event("Event1"),
                make_event("Event2"),
                make_event("Event3"),
            ],
        )
        .await
        .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(received.load(Ordering::SeqCst), 3);

        handle.cancel();
    }

    #[test]
    fn default_capacity() {
        let bus = InProcessEventBus::default();
        // Just verify it creates without panic
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn debug_impl() {
        let bus = InProcessEventBus::new(10);
        let debug = format!("{bus:?}");
        assert!(debug.contains("InProcessEventBus"));
        assert!(debug.contains("subscriber_count"));
    }
}
