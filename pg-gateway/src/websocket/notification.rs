//! Event notification types.
//!
//! This module defines the structure of event notifications
//! sent from `PostgreSQL` via `pg_notify` and forwarded to WebSocket clients.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Event notification from `PostgreSQL`.
///
/// This structure represents an event from the global event log,
/// delivered via `pg_notify` and forwarded to subscribed WebSocket clients.
///
/// # Tenant Isolation
///
/// The `tenant_id` field is used for security filtering. WebSocket
/// connections can only receive events from their own tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventNotification {
    /// Global event ID from `global.event_log`.
    global_id: i64,

    /// Bounded context name (e.g., "sales", "inventory").
    context: String,

    /// Stream ID (aggregate instance ID).
    stream_id: String,

    /// Stream version (event sequence number within stream).
    version: i32,

    /// Event type name (e.g., "`OrderCreated`", "`ItemShipped`").
    event_type: String,

    /// Event payload as JSON.
    payload: serde_json::Value,

    /// Event timestamp.
    timestamp: DateTime<Utc>,

    /// Tenant ID for isolation (optional for system events).
    #[serde(skip_serializing_if = "Option::is_none")]
    tenant_id: Option<String>,
}

impl EventNotification {
    /// Create a new event notification.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        global_id: i64,
        context: String,
        stream_id: String,
        version: i32,
        event_type: String,
        payload: serde_json::Value,
        timestamp: DateTime<Utc>,
        tenant_id: Option<String>,
    ) -> Self {
        Self {
            global_id,
            context,
            stream_id,
            version,
            event_type,
            payload,
            timestamp,
            tenant_id,
        }
    }

    /// Get the global event ID.
    #[must_use]
    pub const fn global_id(&self) -> i64 {
        self.global_id
    }

    /// Get the bounded context name.
    #[must_use]
    pub fn context(&self) -> &str {
        &self.context
    }

    /// Get the stream ID.
    #[must_use]
    pub fn stream_id(&self) -> &str {
        &self.stream_id
    }

    /// Get the stream version.
    #[must_use]
    pub const fn version(&self) -> i32 {
        self.version
    }

    /// Get the event type.
    #[must_use]
    pub fn event_type(&self) -> &str {
        &self.event_type
    }

    /// Get the event payload.
    #[must_use]
    pub const fn payload(&self) -> &serde_json::Value {
        &self.payload
    }

    /// Get the event timestamp.
    #[must_use]
    pub const fn timestamp(&self) -> DateTime<Utc> {
        self.timestamp
    }

    /// Get the tenant ID.
    #[must_use]
    pub fn tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }
}

/// Raw notification payload from `pg_notify`.
///
/// This matches the JSON structure sent by `PostgreSQL` triggers.
#[derive(Debug, Clone, Deserialize)]
pub struct RawNotification {
    /// Global event ID.
    pub global_id: i64,

    /// Bounded context.
    pub context: String,

    /// Stream ID.
    pub stream_id: String,

    /// Stream version.
    pub version: i32,

    /// Event type.
    pub event_type: String,

    /// Event payload.
    pub payload: serde_json::Value,

    /// Timestamp as ISO 8601 string.
    pub timestamp: String,

    /// Tenant ID.
    pub tenant_id: Option<String>,
}

impl TryFrom<RawNotification> for EventNotification {
    type Error = chrono::ParseError;

    fn try_from(raw: RawNotification) -> Result<Self, Self::Error> {
        let timestamp = DateTime::parse_from_rfc3339(&raw.timestamp)?.with_timezone(&Utc);

        Ok(Self {
            global_id: raw.global_id,
            context: raw.context,
            stream_id: raw.stream_id,
            version: raw.version,
            event_type: raw.event_type,
            payload: raw.payload,
            timestamp,
            tenant_id: raw.tenant_id,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn event_notification_accessors() {
        let event = EventNotification::new(
            123,
            "sales".to_string(),
            "order-456".to_string(),
            5,
            "OrderConfirmed".to_string(),
            serde_json::json!({"amount": 100}),
            Utc::now(),
            Some("tenant-789".to_string()),
        );

        assert_eq!(event.global_id(), 123);
        assert_eq!(event.context(), "sales");
        assert_eq!(event.stream_id(), "order-456");
        assert_eq!(event.version(), 5);
        assert_eq!(event.event_type(), "OrderConfirmed");
        assert_eq!(event.payload()["amount"], 100);
        assert_eq!(event.tenant_id(), Some("tenant-789"));
    }

    #[test]
    fn serialization() {
        let event = EventNotification::new(
            1,
            "sales".to_string(),
            "order-1".to_string(),
            1,
            "OrderCreated".to_string(),
            serde_json::json!({}),
            Utc::now(),
            None,
        );

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"global_id\":1"));
        assert!(json.contains("\"context\":\"sales\""));
        // tenant_id should be omitted when None
        assert!(!json.contains("tenant_id"));
    }

    #[test]
    fn serialization_with_tenant() {
        let event = EventNotification::new(
            1,
            "sales".to_string(),
            "order-1".to_string(),
            1,
            "OrderCreated".to_string(),
            serde_json::json!({}),
            Utc::now(),
            Some("tenant-123".to_string()),
        );

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"tenant_id\":\"tenant-123\""));
    }

    #[test]
    fn raw_notification_conversion() {
        let raw = RawNotification {
            global_id: 42,
            context: "inventory".to_string(),
            stream_id: "item-1".to_string(),
            version: 3,
            event_type: "StockUpdated".to_string(),
            payload: serde_json::json!({"quantity": 50}),
            timestamp: "2024-01-15T10:30:00Z".to_string(),
            tenant_id: Some("tenant-1".to_string()),
        };

        let event: EventNotification = raw.try_into().unwrap();

        assert_eq!(event.global_id(), 42);
        assert_eq!(event.context(), "inventory");
        assert_eq!(event.event_type(), "StockUpdated");
        assert_eq!(event.payload()["quantity"], 50);
    }

    #[test]
    fn raw_notification_invalid_timestamp() {
        let raw = RawNotification {
            global_id: 1,
            context: "test".to_string(),
            stream_id: "test-1".to_string(),
            version: 1,
            event_type: "Test".to_string(),
            payload: serde_json::json!({}),
            timestamp: "not-a-timestamp".to_string(),
            tenant_id: None,
        };

        let result: Result<EventNotification, _> = raw.try_into();
        assert!(result.is_err());
    }
}
