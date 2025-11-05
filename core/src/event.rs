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

/// A serialized event ready for storage.
///
/// This struct contains the event type name and the serialized bytes,
/// along with optional metadata. It's used as the wire format between
/// the application and the event store.
#[derive(Clone, Debug)]
pub struct SerializedEvent {
    /// The event type identifier (e.g., "OrderPlaced.v1").
    pub event_type: String,

    /// The bincode-serialized event data.
    pub data: Vec<u8>,

    /// Optional metadata in JSONB format.
    ///
    /// Common metadata fields:
    /// - `correlation_id`: Links related events across aggregates
    /// - `causation_id`: Links cause-and-effect events
    /// - `user_id`: The user who triggered this event
    /// - `timestamp`: When the event was created (ISO 8601)
    pub metadata: Option<serde_json::Value>,
}

impl SerializedEvent {
    /// Create a new serialized event.
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_core::event::SerializedEvent;
    ///
    /// let event = SerializedEvent::new(
    ///     "OrderPlaced.v1".to_string(),
    ///     vec![1, 2, 3, 4],
    ///     None,
    /// );
    /// ```
    #[must_use]
    pub const fn new(
        event_type: String,
        data: Vec<u8>,
        metadata: Option<serde_json::Value>,
    ) -> Self {
        Self {
            event_type,
            data,
            metadata,
        }
    }

    /// Create a serialized event from an `Event` trait object.
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
    /// ```
    pub fn from_event<E: Event + Serialize>(
        event: &E,
        metadata: Option<serde_json::Value>,
    ) -> Result<Self, EventError> {
        Ok(Self {
            event_type: event.event_type().to_string(),
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

        let metadata = serde_json::json!({
            "user_id": "user-123",
            "correlation_id": "corr-456"
        });

        let serialized = SerializedEvent::from_event(&event, Some(metadata.clone()))
            .expect("serialization should succeed");

        assert_eq!(serialized.event_type, "TestEvent.Updated.v1");
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
}
