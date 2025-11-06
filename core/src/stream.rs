//! Event stream identification and versioning types.
//!
//! This module defines strong types for event stream identification (`StreamId`)
//! and version control (`Version`) used in event sourcing.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// Error type for `StreamId` parsing.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[error("Invalid stream ID: {0}")]
pub struct ParseStreamIdError(String);

/// Unique identifier for an event stream (aggregate instance).
///
/// A stream ID uniquely identifies a single aggregate instance in the event store.
/// For example:
/// - `"order-12345"`
/// - `"customer-abc-def"`
/// - `"payment-uuid-here"`
///
/// # Design
///
/// `StreamId` is a newtype wrapper around `String` that provides:
/// - Type safety (can't accidentally use a regular string)
/// - Clear intent in function signatures
/// - Serialization support for storage
///
/// # Validation
///
/// - `FromStr::from_str()`: Validates input (rejects empty strings)
/// - `From::from()` and `new()`: No validation (for internal use with trusted input)
///
/// Use `FromStr` when parsing external/user input. Use `new()` or `From` when
/// constructing stream IDs from application-controlled data
///
/// # Examples
///
/// ```
/// use composable_rust_core::stream::StreamId;
///
/// let stream_id = StreamId::new("order-12345");
/// assert_eq!(stream_id.as_str(), "order-12345");
///
/// let parsed: StreamId = "customer-abc".parse().unwrap();
/// assert_eq!(parsed, StreamId::new("customer-abc"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StreamId(String);

impl StreamId {
    /// Create a new `StreamId` from a string.
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_core::stream::StreamId;
    ///
    /// let id = StreamId::new("order-123");
    /// ```
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the stream ID as a string slice.
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_core::stream::StreamId;
    ///
    /// let id = StreamId::new("order-123");
    /// assert_eq!(id.as_str(), "order-123");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Convert the `StreamId` into its inner `String`.
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_core::stream::StreamId;
    ///
    /// let id = StreamId::new("order-123");
    /// let string = id.into_inner();
    /// assert_eq!(string, "order-123");
    /// ```
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

impl FromStr for StreamId {
    type Err = ParseStreamIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(ParseStreamIdError("Stream ID cannot be empty".to_string()));
        }
        Ok(Self(s.to_string()))
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

/// Event version number for optimistic concurrency control.
///
/// Versions start at 0 and increment by 1 for each event appended to a stream.
/// The version number is used to detect concurrent modifications:
///
/// - When appending events, you specify the expected version
/// - If the stream's current version doesn't match, the append fails
/// - This prevents lost updates in concurrent scenarios
///
/// # Design
///
/// `Version` is a newtype wrapper around `u64` that provides:
/// - Type safety (can't accidentally use a plain integer)
/// - Clear intent in function signatures
/// - Arithmetic operations (+1, etc.)
///
/// # Examples
///
/// ```
/// use composable_rust_core::stream::Version;
///
/// let v0 = Version::new(0);
/// let v1 = v0.next();
/// assert_eq!(v1, Version::new(1));
///
/// let v5 = Version::new(5);
/// assert_eq!(v5.value(), 5);
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Version(u64);

impl Version {
    /// The initial version (0) for a new event stream.
    pub const INITIAL: Self = Self(0);

    /// Create a new `Version` with the given value.
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_core::stream::Version;
    ///
    /// let version = Version::new(42);
    /// assert_eq!(version.value(), 42);
    /// ```
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Get the version number.
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_core::stream::Version;
    ///
    /// let version = Version::new(10);
    /// assert_eq!(version.value(), 10);
    /// ```
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    /// Get the next version (current + 1).
    ///
    /// # Overflow Behavior
    ///
    /// This operation uses wrapping arithmetic. In practice, reaching `u64::MAX`
    /// (18,446,744,073,709,551,615 events) is not a realistic concern for any
    /// event stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_core::stream::Version;
    ///
    /// let v0 = Version::new(0);
    /// let v1 = v0.next();
    /// assert_eq!(v1, Version::new(1));
    /// ```
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Check if this is the initial version (0).
    ///
    /// # Examples
    ///
    /// ```
    /// use composable_rust_core::stream::Version;
    ///
    /// assert!(Version::new(0).is_initial());
    /// assert!(!Version::new(1).is_initial());
    /// ```
    #[must_use]
    pub const fn is_initial(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for Version {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<Version> for u64 {
    fn from(version: Version) -> Self {
        version.0
    }
}

/// Arithmetic addition for `Version`.
///
/// # Overflow Behavior
///
/// Uses wrapping arithmetic. Overflow is not a practical concern given `u64::MAX`.
impl std::ops::Add<u64> for Version {
    type Output = Self;

    fn add(self, rhs: u64) -> Self::Output {
        Self(self.0 + rhs)
    }
}

/// Arithmetic subtraction for `Version`.
///
/// # Underflow Behavior
///
/// Uses wrapping arithmetic. Caller is responsible for ensuring subtraction
/// doesn't underflow below 0.
impl std::ops::Sub<u64> for Version {
    type Output = Self;

    fn sub(self, rhs: u64) -> Self::Output {
        Self(self.0 - rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod stream_id_tests {
        use super::*;

        #[test]
        fn new_creates_stream_id() {
            let id = StreamId::new("order-123");
            assert_eq!(id.as_str(), "order-123");
        }

        #[test]
        fn from_string() {
            let id = StreamId::from("order-123");
            assert_eq!(id.as_str(), "order-123");

            let id2 = StreamId::from("order-456".to_string());
            assert_eq!(id2.as_str(), "order-456");
        }

        #[test]
        #[allow(clippy::expect_used)] // Panics: Test will fail if parse fails
        fn parse_from_str() {
            let id: StreamId = "order-123".parse().expect("parse should succeed");
            assert_eq!(id, StreamId::new("order-123"));
        }

        #[test]
        fn parse_empty_string_fails() {
            let result = "".parse::<StreamId>();
            assert!(result.is_err());
        }

        #[test]
        fn display() {
            let id = StreamId::new("order-123");
            assert_eq!(format!("{id}"), "order-123");
        }

        #[test]
        fn equality() {
            let id1 = StreamId::new("order-123");
            let id2 = StreamId::new("order-123");
            let id3 = StreamId::new("order-456");

            assert_eq!(id1, id2);
            assert_ne!(id1, id3);
        }

        #[test]
        fn into_inner() {
            let id = StreamId::new("order-123");
            let string = id.into_inner();
            assert_eq!(string, "order-123");
        }
    }

    mod version_tests {
        use super::*;

        #[test]
        fn initial_version() {
            assert_eq!(Version::INITIAL, Version::new(0));
            assert!(Version::INITIAL.is_initial());
        }

        #[test]
        fn next_version() {
            let v0 = Version::new(0);
            let v1 = v0.next();
            let v2 = v1.next();

            assert_eq!(v1, Version::new(1));
            assert_eq!(v2, Version::new(2));
        }

        #[test]
        fn version_arithmetic() {
            let v5 = Version::new(5);
            assert_eq!(v5 + 3, Version::new(8));
            assert_eq!(v5 - 2, Version::new(3));
        }

        #[test]
        fn version_ordering() {
            let v1 = Version::new(1);
            let v2 = Version::new(2);
            let v3 = Version::new(3);

            assert!(v1 < v2);
            assert!(v2 < v3);
            assert!(v3 > v1);
        }

        #[test]
        fn version_from_u64() {
            let version = Version::from(42_u64);
            assert_eq!(version.value(), 42);

            let num: u64 = version.into();
            assert_eq!(num, 42);
        }

        #[test]
        fn is_initial() {
            assert!(Version::new(0).is_initial());
            assert!(!Version::new(1).is_initial());
            assert!(!Version::new(100).is_initial());
        }

        #[test]
        fn display() {
            let version = Version::new(42);
            assert_eq!(format!("{version}"), "42");
        }
    }
}
