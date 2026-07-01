//! WebSocket connection manager.
//!
//! This module provides centralized management of all active WebSocket
//! connections, including broadcasting events to subscribed clients.

use super::{ConnectionId, EventNotification, Subscription, WsConnection, protocol::ServerMessage};
use crate::identity::Identity;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

/// Manages all active WebSocket connections.
///
/// The `WsManager` maintains a thread-safe registry of connections and
/// provides methods for:
/// - Registering and unregistering connections
/// - Managing subscriptions
/// - Broadcasting events to subscribed clients
///
/// # Thread Safety
///
/// Uses [`DashMap`] for lock-free concurrent access to connections.
/// The broadcast channel allows multiple listeners without blocking.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::websocket::WsManager;
/// use std::sync::Arc;
///
/// let manager = Arc::new(WsManager::new(1000));
///
/// // Register a connection
/// let (conn_id, rx) = manager.register(identity);
///
/// // Subscribe to events
/// manager.subscribe(conn_id, Subscription::context("sales"));
///
/// // Broadcast will automatically filter to subscribed connections
/// manager.broadcast(&event);
/// ```
#[derive(Debug)]
pub struct WsManager {
    /// Active connections indexed by ID.
    connections: DashMap<ConnectionId, WsConnection>,

    /// Broadcast sender for event notifications.
    ///
    /// Used by the `pg_notify` listener to send events to all connections.
    broadcast_tx: broadcast::Sender<EventNotification>,
}

impl WsManager {
    /// Create a new WebSocket manager.
    ///
    /// # Arguments
    ///
    /// * `broadcast_capacity` - Size of the broadcast channel buffer.
    ///   If slow clients fall behind by more than this many events,
    ///   they will start missing events.
    #[must_use]
    pub fn new(broadcast_capacity: usize) -> Self {
        let (broadcast_tx, _) = broadcast::channel(broadcast_capacity);
        Self {
            connections: DashMap::new(),
            broadcast_tx,
        }
    }

    /// Register a new WebSocket connection.
    ///
    /// Returns the connection ID and a receiver for messages to send to the client.
    ///
    /// # Arguments
    ///
    /// * `identity` - The authenticated user identity for this connection.
    #[must_use]
    pub fn register(&self, identity: Identity) -> (ConnectionId, mpsc::Receiver<String>) {
        let conn_id = ConnectionId::new();
        let (tx, rx) = mpsc::channel(100);

        let connection = WsConnection::new(conn_id, identity, tx);
        self.connections.insert(conn_id, connection);

        tracing::debug!(connection_id = %conn_id, "WebSocket connection registered");

        (conn_id, rx)
    }

    /// Unregister a WebSocket connection.
    ///
    /// Removes the connection and all its subscriptions.
    pub fn unregister(&self, conn_id: ConnectionId) {
        if self.connections.remove(&conn_id).is_some() {
            tracing::debug!(connection_id = %conn_id, "WebSocket connection unregistered");
        }
    }

    /// Add a subscription for a connection.
    ///
    /// # Returns
    ///
    /// Returns `true` if subscription was added, `false` if connection not found.
    #[must_use]
    pub fn subscribe(&self, conn_id: ConnectionId, subscription: Subscription) -> bool {
        if let Some(mut conn) = self.connections.get_mut(&conn_id) {
            conn.subscribe(subscription);
            true
        } else {
            false
        }
    }

    /// Remove a subscription from a connection.
    ///
    /// # Returns
    ///
    /// Returns `true` if removed, `false` if connection not found.
    #[must_use]
    pub fn unsubscribe(&self, conn_id: ConnectionId, subscription: &Subscription) -> bool {
        if let Some(mut conn) = self.connections.get_mut(&conn_id) {
            conn.unsubscribe(subscription);
            true
        } else {
            false
        }
    }

    /// Get subscription display strings for a connection.
    #[must_use]
    pub fn get_subscription_strings(&self, conn_id: ConnectionId) -> Vec<String> {
        self.connections
            .get(&conn_id)
            .map(|conn| {
                conn.subscriptions()
                    .iter()
                    .map(Subscription::to_display_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Subscribe to the broadcast channel.
    ///
    /// Used by individual connection handlers to receive events.
    #[must_use]
    pub fn subscribe_to_broadcasts(&self) -> broadcast::Receiver<EventNotification> {
        self.broadcast_tx.subscribe()
    }

    /// Broadcast an event notification.
    ///
    /// The event is sent to the broadcast channel. Individual connection
    /// handlers will filter based on their subscriptions.
    pub fn broadcast(&self, event: EventNotification) {
        // If no receivers, this returns an error but we don't care
        let _ = self.broadcast_tx.send(event);
    }

    /// Send a message to a specific connection.
    ///
    /// Returns `true` if the message was queued successfully.
    pub async fn send_to(&self, conn_id: ConnectionId, message: &ServerMessage) -> bool {
        // Clone the sender to avoid holding DashMap Ref across await point.
        // This prevents contention when other tasks access the same DashMap shard.
        let sender = {
            let Some(conn) = self.connections.get(&conn_id) else {
                return false;
            };
            conn.sender().clone()
        };

        let Ok(json) = message.to_json() else {
            return false;
        };

        sender.send(json).await.is_ok()
    }

    /// Get the number of active connections.
    #[must_use]
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Check if a connection is subscribed to an event.
    #[must_use]
    pub fn is_subscribed(&self, conn_id: ConnectionId, event: &EventNotification) -> bool {
        self.connections
            .get(&conn_id)
            .is_some_and(|conn| conn.is_subscribed_to(event))
    }
}

impl Default for WsManager {
    fn default() -> Self {
        Self::new(1000)
    }
}

/// Builder for creating a `WsManager` with custom configuration.
#[derive(Debug, Default)]
pub struct WsManagerBuilder {
    broadcast_capacity: Option<usize>,
}

impl WsManagerBuilder {
    /// Create a new builder with default settings.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            broadcast_capacity: None,
        }
    }

    /// Set the broadcast channel capacity.
    #[must_use]
    pub const fn broadcast_capacity(mut self, capacity: usize) -> Self {
        self.broadcast_capacity = Some(capacity);
        self
    }

    /// Build the `WsManager`.
    #[must_use]
    pub fn build(self) -> WsManager {
        WsManager::new(self.broadcast_capacity.unwrap_or(1000))
    }
}

/// Shared state for WebSocket handlers.
pub type SharedWsManager = Arc<WsManager>;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn test_identity(tenant_id: &str) -> Identity {
        Identity::new(
            "user-123".to_string(),
            tenant_id.to_string(),
            vec!["user".to_string()],
        )
    }

    fn test_event(tenant_id: Option<&str>, context: &str) -> EventNotification {
        EventNotification::new(
            1,
            context.to_string(),
            "stream-1".to_string(),
            1,
            "TestEvent".to_string(),
            serde_json::json!({}),
            Utc::now(),
            tenant_id.map(String::from),
        )
    }

    #[test]
    fn register_and_unregister() {
        let manager = WsManager::new(100);

        let (conn_id, _rx) = manager.register(test_identity("tenant-1"));
        assert_eq!(manager.connection_count(), 1);

        manager.unregister(conn_id);
        assert_eq!(manager.connection_count(), 0);
    }

    #[test]
    fn subscribe_and_unsubscribe() {
        let manager = WsManager::new(100);
        let (conn_id, _rx) = manager.register(test_identity("tenant-1"));

        // Subscribe
        let sub = Subscription::context("sales");
        assert!(manager.subscribe(conn_id, sub.clone()));

        let subs = manager.get_subscription_strings(conn_id);
        assert_eq!(subs, vec!["sales"]);

        // Unsubscribe
        assert!(manager.unsubscribe(conn_id, &sub));
        let subs = manager.get_subscription_strings(conn_id);
        assert!(subs.is_empty());
    }

    #[test]
    fn subscribe_to_unknown_connection() {
        let manager = WsManager::new(100);
        let unknown_id = ConnectionId::new();

        assert!(!manager.subscribe(unknown_id, Subscription::context("sales")));
    }

    #[test]
    fn is_subscribed_filtering() {
        let manager = WsManager::new(100);
        let (conn_id, _rx) = manager.register(test_identity("tenant-1"));
        assert!(manager.subscribe(conn_id, Subscription::context("sales")));

        // Same tenant, subscribed context - should match
        let event1 = test_event(Some("tenant-1"), "sales");
        assert!(manager.is_subscribed(conn_id, &event1));

        // Different tenant - should NOT match (security)
        let event2 = test_event(Some("tenant-2"), "sales");
        assert!(!manager.is_subscribed(conn_id, &event2));

        // Different context - should NOT match
        let event3 = test_event(Some("tenant-1"), "inventory");
        assert!(!manager.is_subscribed(conn_id, &event3));
    }

    #[tokio::test]
    async fn broadcast_to_subscribers() {
        let manager = WsManager::new(100);
        let mut rx = manager.subscribe_to_broadcasts();

        // Broadcast an event
        let event = test_event(Some("tenant-1"), "sales");
        manager.broadcast(event.clone());

        // Should receive the event
        let received = rx.recv().await.unwrap();
        assert_eq!(received.global_id(), event.global_id());
    }

    #[tokio::test]
    async fn send_to_connection() {
        let manager = WsManager::new(100);
        let (conn_id, mut rx) = manager.register(test_identity("tenant-1"));

        let msg = ServerMessage::pong();
        let result = manager.send_to(conn_id, &msg).await;
        assert!(result);

        let received = rx.recv().await.unwrap();
        assert!(received.contains("pong"));
    }

    #[tokio::test]
    async fn send_to_unknown_connection() {
        let manager = WsManager::new(100);
        let unknown_id = ConnectionId::new();

        let msg = ServerMessage::pong();
        let result = manager.send_to(unknown_id, &msg).await;
        assert!(!result);
    }

    #[test]
    fn builder_pattern() {
        let manager = WsManagerBuilder::new().broadcast_capacity(500).build();

        assert_eq!(manager.connection_count(), 0);
    }
}
