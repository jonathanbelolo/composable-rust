//! WebSocket subscription types.
//!
//! This module defines what clients can subscribe to for real-time
//! event notifications.

use super::EventNotification;
use serde::{Deserialize, Serialize};

/// Represents a subscription filter for event notifications.
///
/// Clients can subscribe to events in various ways:
/// - All events in a bounded context
/// - Events for a specific stream (aggregate instance)
/// - Events matching a pattern (context + event type)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Subscription {
    /// Subscribe to all events in a bounded context.
    ///
    /// # Example
    ///
    /// ```rust
    /// use composable_rust_pg_gateway::websocket::Subscription;
    ///
    /// // Subscribe to all sales events
    /// let sub = Subscription::Context("sales".to_string());
    /// ```
    Context(String),

    /// Subscribe to events for a specific stream.
    ///
    /// # Example
    ///
    /// ```rust
    /// use composable_rust_pg_gateway::websocket::Subscription;
    ///
    /// // Subscribe to events for order-123
    /// let sub = Subscription::Stream("order-123".to_string());
    /// ```
    Stream(String),

    /// Subscribe to events matching a pattern.
    ///
    /// # Example
    ///
    /// ```rust
    /// use composable_rust_pg_gateway::websocket::Subscription;
    ///
    /// // Subscribe only to ShipmentCreated events in shipping context
    /// let sub = Subscription::Pattern {
    ///     context: "shipping".to_string(),
    ///     event_type: "ShipmentCreated".to_string(),
    /// };
    /// ```
    Pattern {
        /// The bounded context name.
        context: String,
        /// The event type to filter.
        event_type: String,
    },
}

impl Subscription {
    /// Create a context subscription.
    #[must_use]
    pub fn context(name: impl Into<String>) -> Self {
        Self::Context(name.into())
    }

    /// Create a stream subscription.
    #[must_use]
    pub fn stream(id: impl Into<String>) -> Self {
        Self::Stream(id.into())
    }

    /// Create a pattern subscription.
    #[must_use]
    pub fn pattern(context: impl Into<String>, event_type: impl Into<String>) -> Self {
        Self::Pattern {
            context: context.into(),
            event_type: event_type.into(),
        }
    }

    /// Check if this subscription matches an event notification.
    #[must_use]
    pub fn matches(&self, event: &EventNotification) -> bool {
        match self {
            Self::Context(ctx) => event.context() == ctx,
            Self::Stream(stream) => event.stream_id() == stream,
            Self::Pattern { context, event_type } => {
                event.context() == context && event.event_type() == event_type
            }
        }
    }

    /// Convert to a display string for protocol messages.
    #[must_use]
    pub fn to_display_string(&self) -> String {
        match self {
            Self::Context(ctx) => ctx.clone(),
            Self::Stream(stream) => format!("stream:{stream}"),
            Self::Pattern { context, event_type } => format!("{context}:{event_type}"),
        }
    }
}

impl std::fmt::Display for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_display_string())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn test_event(context: &str, stream_id: &str, event_type: &str) -> EventNotification {
        EventNotification::new(
            1,
            context.to_string(),
            stream_id.to_string(),
            1,
            event_type.to_string(),
            serde_json::json!({}),
            Utc::now(),
            None,
        )
    }

    #[test]
    fn context_subscription_matches() {
        let sub = Subscription::context("sales");

        assert!(sub.matches(&test_event("sales", "order-1", "OrderCreated")));
        assert!(sub.matches(&test_event("sales", "order-2", "OrderConfirmed")));
        assert!(!sub.matches(&test_event("inventory", "item-1", "ItemCreated")));
    }

    #[test]
    fn stream_subscription_matches() {
        let sub = Subscription::stream("order-123");

        assert!(sub.matches(&test_event("sales", "order-123", "OrderCreated")));
        assert!(sub.matches(&test_event("sales", "order-123", "OrderConfirmed")));
        assert!(!sub.matches(&test_event("sales", "order-456", "OrderCreated")));
    }

    #[test]
    fn pattern_subscription_matches() {
        let sub = Subscription::pattern("shipping", "ShipmentCreated");

        assert!(sub.matches(&test_event("shipping", "ship-1", "ShipmentCreated")));
        assert!(!sub.matches(&test_event("shipping", "ship-1", "ShipmentDelivered")));
        assert!(!sub.matches(&test_event("sales", "order-1", "ShipmentCreated")));
    }

    #[test]
    fn display_string() {
        assert_eq!(
            Subscription::context("sales").to_display_string(),
            "sales"
        );
        assert_eq!(
            Subscription::stream("order-123").to_display_string(),
            "stream:order-123"
        );
        assert_eq!(
            Subscription::pattern("shipping", "ShipmentCreated").to_display_string(),
            "shipping:ShipmentCreated"
        );
    }

    #[test]
    fn serialization() {
        // Note: untagged enum serialization means we need to be careful
        // The actual protocol messages use a different format (see protocol.rs)
        let sub = Subscription::context("sales");
        let json = serde_json::to_string(&sub).unwrap();
        assert_eq!(json, "\"sales\"");
    }
}
