//! Version tracking for optimistic concurrency

use std::fmt;

/// Version number for optimistic concurrency control
///
/// Each event in a stream has a version number. When appending events,
/// the expected version is checked to detect concurrent modifications.
///
/// # Concurrency Control
///
/// When multiple processes try to append to the same stream simultaneously:
///
/// 1. Process A loads stream at version 5
/// 2. Process B loads stream at version 5
/// 3. Process A appends, expecting version 5 → succeeds, now version 6
/// 4. Process B appends, expecting version 5 → **fails** (version conflict)
///
/// Process B must reload and retry with the current state.
///
/// # Examples
///
/// ```rust
/// use composable_rust_next::Version;
///
/// let v1 = Version::initial();
/// assert_eq!(v1.as_u64(), 0);
///
/// let v2 = v1.next();
/// assert_eq!(v2.as_u64(), 1);
///
/// let v10 = Version::new(10);
/// assert_eq!(v10.as_u64(), 10);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Version(u64);

impl Version {
    /// Create a version from a raw value
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The initial version (0) for empty streams
    #[must_use]
    pub const fn initial() -> Self {
        Self(0)
    }

    /// Get the next version
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Get the previous version
    ///
    /// Returns the initial version (0) if already at initial.
    #[must_use]
    pub const fn prev(self) -> Self {
        Self(self.0.saturating_sub(1))
    }

    /// Get the raw version value
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Check if this is the initial version
    #[must_use]
    pub const fn is_initial(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_version_is_zero() {
        let v = Version::initial();
        assert_eq!(v.as_u64(), 0);
        assert!(v.is_initial());
    }

    #[test]
    fn next_increments_version() {
        let v1 = Version::initial();
        let v2 = v1.next();
        let v3 = v2.next();

        assert_eq!(v1.as_u64(), 0);
        assert_eq!(v2.as_u64(), 1);
        assert_eq!(v3.as_u64(), 2);
    }

    #[test]
    fn version_ordering() {
        let v1 = Version::new(1);
        let v5 = Version::new(5);
        let v10 = Version::new(10);

        assert!(v1 < v5);
        assert!(v5 < v10);
        assert!(v1 < v10);
    }

    #[test]
    fn version_display() {
        let v = Version::new(42);
        assert_eq!(format!("{v}"), "v42");
    }

    #[test]
    fn version_from_u64() {
        let v: Version = 100.into();
        assert_eq!(v.as_u64(), 100);
    }
}
