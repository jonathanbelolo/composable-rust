//! WebSocket connection management.
//!
//! This module provides types for managing individual WebSocket connections,
//! including unique identifiers and connection state.

use super::{EventNotification, Subscription};
use crate::identity::Identity;
use std::collections::HashSet;
use tokio::sync::mpsc;

/// Unique identifier for a WebSocket connection.
///
/// Generated using UUID v4 for guaranteed uniqueness across
/// all active connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(uuid::Uuid);

impl ConnectionId {
    /// Create a new unique connection ID.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    /// Get the underlying UUID.
    #[must_use]
    pub const fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

impl Default for ConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Represents an active WebSocket connection.
///
/// Each connection maintains:
/// - A unique identifier
/// - The authenticated user's identity
/// - A set of active subscriptions
/// - A channel for sending messages to the client
#[derive(Debug)]
pub struct WsConnection {
    /// Unique connection identifier.
    id: ConnectionId,

    /// Authenticated user identity.
    identity: Identity,

    /// Active subscriptions for this connection.
    subscriptions: HashSet<Subscription>,

    /// Channel for sending messages to the WebSocket.
    sender: mpsc::Sender<String>,
}

impl WsConnection {
    /// Create a new WebSocket connection.
    #[must_use]
    pub fn new(id: ConnectionId, identity: Identity, sender: mpsc::Sender<String>) -> Self {
        Self {
            id,
            identity,
            subscriptions: HashSet::new(),
            sender,
        }
    }

    /// Get the connection ID.
    #[must_use]
    pub const fn id(&self) -> ConnectionId {
        self.id
    }

    /// Get the user identity.
    #[must_use]
    pub const fn identity(&self) -> &Identity {
        &self.identity
    }

    /// Get the active subscriptions.
    #[must_use]
    pub const fn subscriptions(&self) -> &HashSet<Subscription> {
        &self.subscriptions
    }

    /// Add a subscription.
    pub fn subscribe(&mut self, subscription: Subscription) {
        self.subscriptions.insert(subscription);
    }

    /// Remove a subscription.
    pub fn unsubscribe(&mut self, subscription: &Subscription) {
        self.subscriptions.remove(subscription);
    }

    /// Clear all subscriptions.
    pub fn clear_subscriptions(&mut self) {
        self.subscriptions.clear();
    }

    /// Check if this connection should receive an event notification.
    ///
    /// Returns `true` if:
    /// 1. The event's tenant matches the connection's tenant (security check)
    /// 2. At least one subscription matches the event
    #[must_use]
    pub fn is_subscribed_to(&self, event: &EventNotification) -> bool {
        // Tenant isolation: only receive events from the same tenant
        if let Some(event_tenant) = event.tenant_id() {
            if event_tenant != self.identity.tenant_id {
                return false;
            }
        }

        // Check if any subscription matches
        self.subscriptions.iter().any(|sub| sub.matches(event))
    }

    /// Get the sender channel for this connection.
    #[must_use]
    pub const fn sender(&self) -> &mpsc::Sender<String> {
        &self.sender
    }

    /// Send a message to the WebSocket client.
    ///
    /// Returns `true` if the message was sent successfully,
    /// `false` if the channel is closed.
    pub async fn send(&self, message: String) -> bool {
        self.sender.send(message).await.is_ok()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn test_identity() -> Identity {
        Identity::new(
            "user-123".to_string(),
            "tenant-456".to_string(),
            vec!["user".to_string()],
        )
    }

    fn test_event(tenant_id: Option<String>, context: &str, stream_id: &str) -> EventNotification {
        EventNotification::new(
            1,
            context.to_string(),
            stream_id.to_string(),
            1,
            "TestEvent".to_string(),
            serde_json::json!({}),
            Utc::now(),
            tenant_id,
        )
    }

    #[test]
    fn connection_id_uniqueness() {
        let id1 = ConnectionId::new();
        let id2 = ConnectionId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn connection_id_display() {
        let id = ConnectionId::new();
        let display = id.to_string();
        assert!(!display.is_empty());
        assert!(display.contains('-')); // UUID format
    }

    #[tokio::test]
    async fn connection_subscriptions() {
        let (tx, _rx) = mpsc::channel(10);
        let mut conn = WsConnection::new(ConnectionId::new(), test_identity(), tx);

        assert!(conn.subscriptions().is_empty());

        // Add subscriptions
        conn.subscribe(Subscription::Context("sales".to_string()));
        conn.subscribe(Subscription::Stream("order-123".to_string()));

        assert_eq!(conn.subscriptions().len(), 2);

        // Remove one
        conn.unsubscribe(&Subscription::Context("sales".to_string()));
        assert_eq!(conn.subscriptions().len(), 1);

        // Clear all
        conn.clear_subscriptions();
        assert!(conn.subscriptions().is_empty());
    }

    #[tokio::test]
    async fn tenant_isolation() {
        let (tx, _rx) = mpsc::channel(10);
        let mut conn = WsConnection::new(ConnectionId::new(), test_identity(), tx);
        conn.subscribe(Subscription::Context("sales".to_string()));

        // Same tenant - should match
        let event_same = test_event(Some("tenant-456".to_string()), "sales", "order-1");
        assert!(conn.is_subscribed_to(&event_same));

        // Different tenant - should NOT match (security)
        let event_different = test_event(Some("other-tenant".to_string()), "sales", "order-1");
        assert!(!conn.is_subscribed_to(&event_different));
    }

    #[tokio::test]
    async fn subscription_matching() {
        let (tx, _rx) = mpsc::channel(10);
        let mut conn = WsConnection::new(ConnectionId::new(), test_identity(), tx);

        // Subscribe to context
        conn.subscribe(Subscription::Context("sales".to_string()));

        // Events in subscribed context should match
        let event1 = test_event(Some("tenant-456".to_string()), "sales", "order-1");
        assert!(conn.is_subscribed_to(&event1));

        // Events in different context should not match
        let event2 = test_event(Some("tenant-456".to_string()), "inventory", "item-1");
        assert!(!conn.is_subscribed_to(&event2));
    }

    #[tokio::test]
    async fn send_message() {
        let (tx, mut rx) = mpsc::channel(10);
        let conn = WsConnection::new(ConnectionId::new(), test_identity(), tx);

        // Send should succeed
        let result = conn.send("test message".to_string()).await;
        assert!(result);

        // Verify message received
        let received = rx.recv().await.unwrap();
        assert_eq!(received, "test message");
    }

    #[tokio::test]
    async fn send_to_closed_channel() {
        let (tx, rx) = mpsc::channel(10);
        let conn = WsConnection::new(ConnectionId::new(), test_identity(), tx);

        // Drop the receiver
        drop(rx);

        // Send should fail
        let result = conn.send("test message".to_string()).await;
        assert!(!result);
    }
}
