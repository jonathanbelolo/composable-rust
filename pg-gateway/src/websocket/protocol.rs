//! WebSocket protocol message types.
//!
//! This module defines the JSON message format for client-server
//! communication over WebSocket connections.

use super::EventNotification;
use serde::{Deserialize, Serialize};

/// Client → Server message.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Subscribe to event notifications.
    Subscribe {
        /// List of subscriptions to add.
        subscriptions: Vec<SubscriptionRequest>,
    },

    /// Unsubscribe from event notifications.
    Unsubscribe {
        /// List of subscription identifiers to remove.
        subscriptions: Vec<String>,
    },

    /// Ping (keep-alive).
    Ping,
}

/// Subscription request from client.
///
/// Supports three formats:
/// - `{ "context": "sales" }` - subscribe to all events in context
/// - `{ "stream": "order-123" }` - subscribe to specific stream
/// - `{ "context": "sales", "event_type": "OrderCreated" }` - pattern match
#[derive(Debug, Clone, Deserialize)]
pub struct SubscriptionRequest {
    /// Bounded context name.
    #[serde(default)]
    pub context: Option<String>,

    /// Stream ID.
    #[serde(default)]
    pub stream: Option<String>,

    /// Event type for pattern matching.
    #[serde(default)]
    pub event_type: Option<String>,
}

impl SubscriptionRequest {
    /// Convert to a subscription if valid.
    ///
    /// # Errors
    ///
    /// Returns an error string if the request is invalid.
    pub fn into_subscription(self) -> Result<super::Subscription, String> {
        match (self.context, self.stream, self.event_type) {
            // Pattern: context + event_type
            (Some(ctx), None, Some(evt)) => Ok(super::Subscription::Pattern {
                context: ctx,
                event_type: evt,
            }),
            // Context only
            (Some(ctx), None, None) => Ok(super::Subscription::Context(ctx)),
            // Stream only
            (None, Some(stream), None) => Ok(super::Subscription::Stream(stream)),
            // Invalid combinations
            (None, None, _) => Err("Must specify 'context' or 'stream'".to_string()),
            (Some(_), Some(_), _) => Err("Cannot specify both 'context' and 'stream'".to_string()),
            (None, Some(_), Some(_)) => {
                Err("Cannot use 'event_type' with 'stream' subscription".to_string())
            },
        }
    }
}

/// Server → Client message.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Subscription confirmed.
    Subscribed {
        /// List of active subscription identifiers.
        subscriptions: Vec<String>,
    },

    /// Unsubscription confirmed.
    Unsubscribed {
        /// List of removed subscription identifiers.
        subscriptions: Vec<String>,
    },

    /// Event notification.
    Event(EventPayload),

    /// Pong response.
    Pong,

    /// Error message.
    Error {
        /// Error code.
        code: String,
        /// Human-readable error message.
        message: String,
    },
}

/// Event payload for server messages.
///
/// Flattened to include event fields directly in the message.
#[derive(Debug, Clone, Serialize)]
pub struct EventPayload {
    /// Global event ID.
    pub global_id: i64,

    /// Bounded context name.
    pub context: String,

    /// Stream ID.
    pub stream_id: String,

    /// Stream version.
    pub version: i32,

    /// Event type.
    pub event_type: String,

    /// Event payload.
    pub payload: serde_json::Value,

    /// Event timestamp.
    pub timestamp: String,
}

impl From<&EventNotification> for EventPayload {
    fn from(event: &EventNotification) -> Self {
        Self {
            global_id: event.global_id(),
            context: event.context().to_string(),
            stream_id: event.stream_id().to_string(),
            version: event.version(),
            event_type: event.event_type().to_string(),
            payload: event.payload().clone(),
            timestamp: event.timestamp().to_rfc3339(),
        }
    }
}

impl ServerMessage {
    /// Create a subscribed confirmation message.
    #[must_use]
    pub const fn subscribed(subscriptions: Vec<String>) -> Self {
        Self::Subscribed { subscriptions }
    }

    /// Create an unsubscribed confirmation message.
    #[must_use]
    pub const fn unsubscribed(subscriptions: Vec<String>) -> Self {
        Self::Unsubscribed { subscriptions }
    }

    /// Create an event notification message.
    #[must_use]
    pub fn event(notification: &EventNotification) -> Self {
        Self::Event(EventPayload::from(notification))
    }

    /// Create a pong message.
    #[must_use]
    pub const fn pong() -> Self {
        Self::Pong
    }

    /// Create an error message.
    #[must_use]
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Error {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Serialize to JSON string.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails (should not happen for valid messages).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// Error codes for WebSocket error messages.
pub mod error_codes {
    /// Invalid subscription format.
    pub const INVALID_SUBSCRIPTION: &str = "invalid_subscription";

    /// Invalid message format.
    pub const INVALID_MESSAGE: &str = "invalid_message";

    /// Authentication required.
    pub const UNAUTHORIZED: &str = "unauthorized";

    /// Internal server error.
    pub const INTERNAL_ERROR: &str = "internal_error";
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn parse_subscribe_context() {
        let json = r#"{"type": "subscribe", "subscriptions": [{"context": "sales"}]}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();

        let ClientMessage::Subscribe { subscriptions } = msg else {
            panic!("Expected Subscribe message");
        };

        assert_eq!(subscriptions.len(), 1);
        let sub = subscriptions
            .into_iter()
            .next()
            .unwrap()
            .into_subscription()
            .unwrap();
        assert!(matches!(sub, super::super::Subscription::Context(ctx) if ctx == "sales"));
    }

    #[test]
    fn parse_subscribe_stream() {
        let json = r#"{"type": "subscribe", "subscriptions": [{"stream": "order-123"}]}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();

        let ClientMessage::Subscribe { subscriptions } = msg else {
            panic!("Expected Subscribe message");
        };

        let sub = subscriptions
            .into_iter()
            .next()
            .unwrap()
            .into_subscription()
            .unwrap();
        assert!(matches!(sub, super::super::Subscription::Stream(s) if s == "order-123"));
    }

    #[test]
    fn parse_subscribe_pattern() {
        let json = r#"{"type": "subscribe", "subscriptions": [{"context": "shipping", "event_type": "ShipmentCreated"}]}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();

        let ClientMessage::Subscribe { subscriptions } = msg else {
            panic!("Expected Subscribe message");
        };

        let sub = subscriptions
            .into_iter()
            .next()
            .unwrap()
            .into_subscription()
            .unwrap();
        assert!(matches!(
            sub,
            super::super::Subscription::Pattern { context, event_type }
            if context == "shipping" && event_type == "ShipmentCreated"
        ));
    }

    #[test]
    fn parse_unsubscribe() {
        let json = r#"{"type": "unsubscribe", "subscriptions": ["sales", "stream:order-123"]}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();

        let ClientMessage::Unsubscribe { subscriptions } = msg else {
            panic!("Expected Unsubscribe message");
        };

        assert_eq!(subscriptions, vec!["sales", "stream:order-123"]);
    }

    #[test]
    fn parse_ping() {
        let json = r#"{"type": "ping"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::Ping));
    }

    #[test]
    fn invalid_subscription_no_context_or_stream() {
        let req = SubscriptionRequest {
            context: None,
            stream: None,
            event_type: None,
        };
        assert!(req.into_subscription().is_err());
    }

    #[test]
    fn invalid_subscription_both_context_and_stream() {
        let req = SubscriptionRequest {
            context: Some("sales".to_string()),
            stream: Some("order-123".to_string()),
            event_type: None,
        };
        assert!(req.into_subscription().is_err());
    }

    #[test]
    fn serialize_subscribed() {
        let msg =
            ServerMessage::subscribed(vec!["sales".to_string(), "stream:order-123".to_string()]);
        let json = msg.to_json().unwrap();
        assert!(json.contains("\"type\":\"subscribed\""));
        assert!(json.contains("\"subscriptions\""));
    }

    #[test]
    fn serialize_event() {
        let event = super::super::EventNotification::new(
            123,
            "sales".to_string(),
            "order-456".to_string(),
            5,
            "OrderConfirmed".to_string(),
            serde_json::json!({"amount": 100}),
            Utc::now(),
            None,
        );

        let msg = ServerMessage::event(&event);
        let json = msg.to_json().unwrap();

        assert!(json.contains("\"type\":\"event\""));
        assert!(json.contains("\"global_id\":123"));
        assert!(json.contains("\"context\":\"sales\""));
    }

    #[test]
    fn serialize_pong() {
        let msg = ServerMessage::pong();
        let json = msg.to_json().unwrap();
        assert_eq!(json, r#"{"type":"pong"}"#);
    }

    #[test]
    fn serialize_error() {
        let msg = ServerMessage::error(error_codes::INVALID_SUBSCRIPTION, "Unknown context: foo");
        let json = msg.to_json().unwrap();

        assert!(json.contains("\"type\":\"error\""));
        assert!(json.contains("\"code\":\"invalid_subscription\""));
        assert!(json.contains("\"message\":\"Unknown context: foo\""));
    }
}
