//! Stream ID for event store addressing

use std::fmt;

/// Identifier for an event stream
///
/// Each aggregate instance has its own stream, identified by a unique `StreamId`.
/// Events are appended to and loaded from streams.
///
/// # Naming Convention
///
/// Stream IDs typically follow the pattern `{aggregate-type}-{entity-id}`:
///
/// - `event-550e8400-e29b-41d4-a716-446655440000`
/// - `order-12345`
/// - `saga-checkout-abc123`
///
/// # Examples
///
/// ```rust
/// use composable_rust_next::StreamId;
///
/// // From a string
/// let stream = StreamId::new("event-123");
/// assert_eq!(stream.as_str(), "event-123");
///
/// // From a formatted value
/// let stream = StreamId::new(format!("event-{}", "550e8400-e29b-41d4-a716-446655440000"));
/// assert!(stream.as_str().starts_with("event-"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StreamId(String);

impl StreamId {
    /// Create a new stream ID
    ///
    /// # Examples
    ///
    /// ```rust
    /// use composable_rust_next::StreamId;
    ///
    /// let stream = StreamId::new("order-12345");
    /// ```
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the stream ID as a string slice
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the stream ID and return the inner string
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for StreamId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for StreamId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl AsRef<str> for StreamId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_id_from_string() {
        let stream = StreamId::new("event-123");
        assert_eq!(stream.as_str(), "event-123");
    }

    #[test]
    fn stream_id_display() {
        let stream = StreamId::new("order-456");
        assert_eq!(format!("{stream}"), "order-456");
    }

    #[test]
    fn stream_id_equality() {
        let a = StreamId::new("test-1");
        let b = StreamId::new("test-1");
        let c = StreamId::new("test-2");

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn stream_id_from_str() {
        let stream: StreamId = "saga-abc".into();
        assert_eq!(stream.as_str(), "saga-abc");
    }
}
