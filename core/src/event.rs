//! Event trait and related types for event sourcing.
//!
//! This module defines the core abstraction for events in an event-sourced system.
//! Events represent facts about things that have happened in the past and are immutable.
//!
//! # Design
//!
//! Events in this system are serialized using `bincode` for maximum performance and minimal
//! storage overhead. While this means events are not human-readable in the database, it
//! provides significant benefits:
//!
//! - 5-10x faster serialization compared to JSON
//! - 30-70% smaller storage footprint
//! - All-Rust services can use the same binary format
//!
//! # Example
//!
//! ```
//! use composable_rust_core::event::Event;
//! use serde::{Serialize, Deserialize};
//!
//! #[derive(Clone, Debug, Serialize, Deserialize)]
//! enum OrderEvent {
//!     OrderPlaced { order_id: String, total: f64 },
//!     OrderShipped { order_id: String, tracking: String },
//! }
//!
//! impl Event for OrderEvent {
//!     fn event_type(&self) -> &'static str {
//!         match self {
//!             OrderEvent::OrderPlaced { .. } => "OrderPlaced.v1",
//!             OrderEvent::OrderShipped { .. } => "OrderShipped.v1",
//!         }
//!     }
//! }
//! ```

use serde::{Serialize, de::DeserializeOwned};
use std::fmt;
use thiserror::Error;

/// Error types for event operations.
#[derive(Error, Debug)]
pub enum EventError {
    /// Failed to serialize event to bytes.
    #[error("Failed to serialize event: {0}")]
    SerializationError(String),

    /// Failed to deserialize event from bytes.
    #[error("Failed to deserialize event: {0}")]
    DeserializationError(String),

    /// Unknown event type encountered during deserialization.
    #[error("Unknown event type: {0}")]
    UnknownEventType(String),
}

/// An event that can be stored in an event store and replayed to reconstruct state.
///
/// Events represent immutable facts about things that have happened in the past.
/// They are the source of truth in an event-sourced system.
///
/// # Event Naming Convention
///
/// The `event_type()` method should return a stable string identifier that includes
/// a version number. This allows for schema evolution over time. For example:
///
/// - `"OrderPlaced.v1"`
/// - `"OrderCancelled.v1"`
/// - `"OrderShipped.v2"` (after schema change)
///
/// # Serialization
///
/// Events are serialized to binary format using `bincode` for performance and
/// storage efficiency. The trait provides default implementations that work for
/// any type implementing `Serialize` and `DeserializeOwned`.
///
/// # Thread Safety
///
/// Events must be `Send + Sync + 'static` to be safely passed between threads
/// in the async runtime and stored in the event store.
pub trait Event: Send + Sync + 'static {
    /// Returns the event type identifier for this event.
    ///
    /// This string is used for:
    /// - Storing the event type in the database
    /// - Routing events to the correct deserializer
    /// - Versioning event schemas
    ///
    /// # Convention
    ///
    /// Use a descriptive name with a version suffix:
    /// - `"OrderPlaced.v1"`
    /// - `"PaymentProcessed.v2"`
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_core::event::Event;
    /// # use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Clone, Debug, Serialize, Deserialize)]
    /// enum OrderEvent {
    ///     OrderPlaced { order_id: String },
    /// }
    ///
    /// impl Event for OrderEvent {
    ///     fn event_type(&self) -> &'static str {
    ///         match self {
    ///             OrderEvent::OrderPlaced { .. } => "OrderPlaced.v1",
    ///         }
    ///     }
    /// }
    /// ```
    fn event_type(&self) -> &'static str;

    /// Serialize this event to bincode bytes.
    ///
    /// # Errors
    ///
    /// Returns `EventError::SerializationError` if the event cannot be serialized.
    /// This can happen if the event contains unsupported types, though this is rare
    /// with bincode.
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_core::event::Event;
    /// # use serde::{Serialize, Deserialize};
    /// # #[derive(Clone, Debug, Serialize, Deserialize)]
    /// # enum OrderEvent {
    /// #     OrderPlaced { order_id: String },
    /// # }
    /// # impl Event for OrderEvent {
    /// #     fn event_type(&self) -> &'static str { "OrderPlaced.v1" }
    /// # }
    ///
    /// let event = OrderEvent::OrderPlaced {
    ///     order_id: "order-123".to_string(),
    /// };
    ///
    /// let bytes = event.to_bytes().expect("serialization should succeed");
    /// assert!(!bytes.is_empty());
    /// ```
    fn to_bytes(&self) -> Result<Vec<u8>, EventError>
    where
        Self: Serialize,
    {
        bincode::serialize(self).map_err(|e| EventError::SerializationError(e.to_string()))
    }

    /// Deserialize an event from bincode bytes.
    ///
    /// # Errors
    ///
    /// Returns `EventError::DeserializationError` if the bytes cannot be deserialized
    /// into this event type. This can happen if:
    /// - The bytes are corrupted
    /// - The bytes represent a different event type
    /// - The event schema has changed incompatibly
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_core::event::Event;
    /// # use serde::{Serialize, Deserialize};
    /// # #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
    /// # enum OrderEvent {
    /// #     OrderPlaced { order_id: String },
    /// # }
    /// # impl Event for OrderEvent {
    /// #     fn event_type(&self) -> &'static str { "OrderPlaced.v1" }
    /// # }
    ///
    /// let original = OrderEvent::OrderPlaced {
    ///     order_id: "order-123".to_string(),
    /// };
    ///
    /// let bytes = original.to_bytes().unwrap();
    /// let deserialized = OrderEvent::from_bytes(&bytes).unwrap();
    ///
    /// assert_eq!(original, deserialized);
    /// ```
    fn from_bytes(bytes: &[u8]) -> Result<Self, EventError>
    where
        Self: DeserializeOwned + Sized,
    {
        bincode::deserialize(bytes).map_err(|e| EventError::DeserializationError(e.to_string()))
    }
}

/// Event metadata with strongly-typed fields.
///
/// This struct provides type-safe metadata for events, replacing the previous
/// stringly-typed `serde_json::Value` approach.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct EventMetadata {
    /// Links related events across aggregates (saga coordination).
    pub correlation_id: Option<String>,

    /// Links cause-and-effect events within a workflow.
    pub causation_id: Option<String>,

    /// The user who triggered this event.
    pub user_id: Option<String>,

    /// When the event was created (ISO 8601 timestamp).
    pub timestamp: Option<String>,
}

impl EventMetadata {
    /// Create empty metadata.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            correlation_id: None,
            causation_id: None,
            user_id: None,
            timestamp: None,
        }
    }

    /// Create metadata with just a correlation ID.
    #[must_use]
    pub fn with_correlation_id(correlation_id: impl Into<String>) -> Self {
        Self {
            correlation_id: Some(correlation_id.into()),
            causation_id: None,
            user_id: None,
            timestamp: None,
        }
    }

    /// Convert to JSON value for database storage.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "correlation_id": self.correlation_id,
            "causation_id": self.causation_id,
            "user_id": self.user_id,
            "timestamp": self.timestamp,
        })
    }

    /// Create from JSON value from database.
    ///
    /// # Errors
    ///
    /// Returns error if JSON deserialization fails.
    pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("Failed to deserialize metadata: {e}"))
    }
}

impl Default for EventMetadata {
    fn default() -> Self {
        Self::new()
    }
}

/// A serialized event ready for storage.
///
/// This struct contains the event type name and the serialized bytes,
/// along with optional metadata. It's used as the wire format between
/// the application and the event store.
///
/// # Event Versioning
///
/// The `event_version` field supports schema evolution:
/// - Extracted from the version suffix in `event_type` (e.g., "OrderPlaced.v1" → 1)
/// - Stored as a separate database column for efficient querying
/// - Enables version-specific deserialization and migration logic
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SerializedEvent {
    /// The event type identifier (e.g., "OrderPlaced.v1").
    pub event_type: String,

    /// The schema version extracted from `event_type` (e.g., 1 for ".v1").
    /// Defaults to 1 if no version suffix is found.
    pub event_version: i32,

    /// The bincode-serialized event data.
    pub data: Vec<u8>,

    /// Optional strongly-typed metadata.
    pub metadata: Option<EventMetadata>,
}

impl SerializedEvent {
    /// Extract version number from event type string.
    ///
    /// Parses version suffix like ".v1", ".v2", etc. from event type strings.
    /// Returns 1 if no version suffix is found (backward compatibility).
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_core::event::SerializedEvent;
    ///
    /// assert_eq!(SerializedEvent::extract_version("OrderPlaced.v1"), 1);
    /// assert_eq!(SerializedEvent::extract_version("OrderPlaced.v2"), 2);
    /// assert_eq!(SerializedEvent::extract_version("OrderPlaced"), 1); // No version = v1
    /// assert_eq!(SerializedEvent::extract_version("PaymentProcessed.v10"), 10);
    /// ```
    #[must_use]
    pub fn extract_version(event_type: &str) -> i32 {
        // Look for version pattern: ".v" followed by digits
        if let Some(pos) = event_type.rfind(".v") {
            let version_str = &event_type[pos + 2..];
            version_str.parse::<i32>().unwrap_or(1)
        } else {
            1 // Default to version 1 if no version suffix
        }
    }

    /// Create a new serialized event.
    ///
    /// The version is automatically extracted from the `event_type` string
    /// (e.g., "OrderPlaced.v1" → version = 1).
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_core::event::{SerializedEvent, EventMetadata};
    ///
    /// let event = SerializedEvent::new(
    ///     "OrderPlaced.v1".to_string(),
    ///     vec![1, 2, 3, 4],
    ///     None,
    /// );
    /// assert_eq!(event.event_version, 1);
    ///
    /// // With metadata
    /// let event_with_metadata = SerializedEvent::new(
    ///     "OrderPlaced.v2".to_string(),
    ///     vec![1, 2, 3, 4],
    ///     Some(EventMetadata::with_correlation_id("abc-123")),
    /// );
    /// assert_eq!(event_with_metadata.event_version, 2);
    /// ```
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // Parameters cannot be const-constructed
    pub fn new(event_type: String, data: Vec<u8>, metadata: Option<EventMetadata>) -> Self {
        let event_version = Self::extract_version(&event_type);
        Self {
            event_type,
            event_version,
            data,
            metadata,
        }
    }

    /// Create a serialized event from an `Event` trait object.
    ///
    /// The version is automatically extracted from the event type returned by
    /// `event.event_type()`.
    ///
    /// # Errors
    ///
    /// Returns `EventError::SerializationError` if the event cannot be serialized.
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_core::event::{Event, SerializedEvent};
    /// # use serde::{Serialize, Deserialize};
    /// # #[derive(Clone, Debug, Serialize, Deserialize)]
    /// # enum OrderEvent {
    /// #     OrderPlaced { order_id: String },
    /// # }
    /// # impl Event for OrderEvent {
    /// #     fn event_type(&self) -> &'static str { "OrderPlaced.v1" }
    /// # }
    ///
    /// let event = OrderEvent::OrderPlaced {
    ///     order_id: "order-123".to_string(),
    /// };
    ///
    /// let serialized = SerializedEvent::from_event(&event, None).unwrap();
    /// assert_eq!(serialized.event_type, "OrderPlaced.v1");
    /// assert_eq!(serialized.event_version, 1);
    /// ```
    pub fn from_event<E: Event + Serialize>(
        event: &E,
        metadata: Option<EventMetadata>,
    ) -> Result<Self, EventError> {
        let event_type = event.event_type().to_string();
        let event_version = Self::extract_version(&event_type);
        Ok(Self {
            event_type,
            event_version,
            data: event.to_bytes()?,
            metadata,
        })
    }
}

impl fmt::Display for SerializedEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SerializedEvent {{ type: {}, size: {} bytes }}",
            self.event_type,
            self.data.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
    enum TestEvent {
        Created { id: String, value: i32 },
        Updated { id: String, new_value: i32 },
    }

    impl Event for TestEvent {
        fn event_type(&self) -> &'static str {
            match self {
                TestEvent::Created { .. } => "TestEvent.Created.v1",
                TestEvent::Updated { .. } => "TestEvent.Updated.v1",
            }
        }
    }

    #[test]
    fn event_type_returns_correct_identifier() {
        let event = TestEvent::Created {
            id: "test-1".to_string(),
            value: 42,
        };
        assert_eq!(event.event_type(), "TestEvent.Created.v1");
    }

    #[test]
    #[allow(clippy::expect_used)] // Panics: Test will fail if serialization fails
    fn event_serialization_roundtrip() {
        let event = TestEvent::Created {
            id: "test-1".to_string(),
            value: 42,
        };

        let bytes = event.to_bytes().expect("serialization should succeed");
        let deserialized = TestEvent::from_bytes(&bytes).expect("deserialization should succeed");

        assert_eq!(event, deserialized);
    }

    #[test]
    #[allow(clippy::expect_used)] // Panics: Test will fail if serialization fails
    fn serialized_event_from_event() {
        let event = TestEvent::Updated {
            id: "test-1".to_string(),
            new_value: 100,
        };

        let metadata = EventMetadata {
            user_id: Some("user-123".to_string()),
            correlation_id: Some("corr-456".to_string()),
            causation_id: None,
            timestamp: None,
        };

        let serialized = SerializedEvent::from_event(&event, Some(metadata.clone()))
            .expect("serialization should succeed");

        assert_eq!(serialized.event_type, "TestEvent.Updated.v1");
        assert_eq!(serialized.event_version, 1); // Verify version extracted
        assert!(!serialized.data.is_empty());
        assert_eq!(serialized.metadata, Some(metadata));
    }

    #[test]
    fn serialized_event_display() {
        let serialized =
            SerializedEvent::new("TestEvent.v1".to_string(), vec![1, 2, 3, 4, 5], None);

        let display = format!("{serialized}");
        assert!(display.contains("TestEvent.v1"));
        assert!(display.contains("5 bytes"));
    }

    #[test]
    fn extract_version_from_event_type() {
        // Test version extraction
        assert_eq!(SerializedEvent::extract_version("OrderPlaced.v1"), 1);
        assert_eq!(SerializedEvent::extract_version("OrderPlaced.v2"), 2);
        assert_eq!(SerializedEvent::extract_version("OrderPlaced.v10"), 10);
        assert_eq!(SerializedEvent::extract_version("PaymentProcessed.v123"), 123);

        // Test without version suffix (backward compatibility)
        assert_eq!(SerializedEvent::extract_version("OrderPlaced"), 1);
        assert_eq!(SerializedEvent::extract_version("SomeEvent"), 1);

        // Test malformed version (falls back to default)
        assert_eq!(SerializedEvent::extract_version("Event.vABC"), 1);
    }

    #[test]
    fn serialized_event_new_extracts_version() {
        let event_v1 = SerializedEvent::new("OrderPlaced.v1".to_string(), vec![1, 2, 3], None);
        assert_eq!(event_v1.event_version, 1);

        let event_v2 = SerializedEvent::new("OrderPlaced.v2".to_string(), vec![1, 2, 3], None);
        assert_eq!(event_v2.event_version, 2);

        let event_no_version = SerializedEvent::new("OrderPlaced".to_string(), vec![1, 2, 3], None);
        assert_eq!(event_no_version.event_version, 1); // Default to v1
    }
}
