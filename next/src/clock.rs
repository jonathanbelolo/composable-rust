//! Clock abstraction for time-dependent operations

use chrono::{DateTime, Utc};
use std::sync::{Arc, Mutex};

/// Clock abstraction for getting the current time
///
/// This trait enables deterministic testing by allowing time to be controlled.
/// In production, use [`SystemClock`]. In tests, use [`FixedClock`].
///
/// # Examples
///
/// ## Production
///
/// ```rust
/// use composable_rust_next::{Clock, SystemClock};
///
/// let clock = SystemClock;
/// println!("Current time: {}", clock.now());
/// ```
///
/// ## Testing
///
/// ```rust
/// use composable_rust_next::{Clock, FixedClock};
/// use chrono::{TimeZone, Utc};
///
/// let clock = FixedClock::new(Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap());
/// assert_eq!(clock.now(), Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap());
///
/// // Advance time for testing timeouts, delays, etc.
/// clock.advance(std::time::Duration::from_secs(60));
/// assert_eq!(clock.now(), Utc.with_ymd_and_hms(2025, 1, 15, 10, 1, 0).unwrap());
/// ```
pub trait Clock: Send + Sync {
    /// Get the current time
    fn now(&self) -> DateTime<Utc>;
}

/// System clock that returns the real current time
///
/// Use this in production environments.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl SystemClock {
    /// Create a new system clock
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Fixed clock for deterministic testing
///
/// This clock always returns a fixed time that can be advanced manually.
/// Essential for testing time-dependent business logic.
///
/// # Thread Safety
///
/// The internal time is protected by a mutex, making it safe to share
/// across threads (e.g., in async tests).
///
/// # Cloning
///
/// `FixedClock` implements `Clone` via `Arc`, so clones share the same
/// underlying time. This is intentional—when you clone a `TestEnvironment`,
/// all components see the same clock state.
///
/// # Examples
///
/// ```rust
/// use composable_rust_next::{Clock, FixedClock};
/// use chrono::{TimeZone, Utc};
/// use std::time::Duration;
///
/// let clock = FixedClock::new(Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap());
///
/// // Time doesn't advance on its own
/// assert_eq!(clock.now(), Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap());
/// assert_eq!(clock.now(), Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap());
///
/// // Manually advance time
/// clock.advance(Duration::from_secs(3600)); // 1 hour
/// assert_eq!(clock.now(), Utc.with_ymd_and_hms(2025, 1, 15, 11, 0, 0).unwrap());
///
/// // Clones share the same time
/// let clock2 = clock.clone();
/// clock.advance(Duration::from_secs(60));
/// assert_eq!(clock.now(), clock2.now()); // Both see the same time
/// ```
#[derive(Debug, Clone)]
pub struct FixedClock {
    time: Arc<Mutex<DateTime<Utc>>>,
}

impl FixedClock {
    /// Create a new fixed clock at the given time
    #[must_use]
    pub fn new(time: DateTime<Utc>) -> Self {
        Self {
            time: Arc::new(Mutex::new(time)),
        }
    }

    /// Create a fixed clock at the Unix epoch (1970-01-01 00:00:00 UTC)
    #[must_use]
    pub fn epoch() -> Self {
        Self::new(DateTime::UNIX_EPOCH)
    }

    /// Advance the clock by the given duration
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned (another thread panicked while holding it).
    #[allow(clippy::expect_used)] // Intentional: mutex poisoning is unrecoverable
    pub fn advance(&self, duration: std::time::Duration) {
        let mut time = self
            .time
            .lock()
            .expect("FixedClock mutex poisoned - a thread panicked while holding the lock");
        #[allow(clippy::expect_used)] // Duration conversion should not fail for reasonable values
        {
            *time += chrono::Duration::from_std(duration).expect("Duration overflow");
        }
    }

    /// Set the clock to a specific time
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned.
    #[allow(clippy::expect_used)] // Intentional: mutex poisoning is unrecoverable
    pub fn set(&self, new_time: DateTime<Utc>) {
        let mut time = self
            .time
            .lock()
            .expect("FixedClock mutex poisoned - a thread panicked while holding the lock");
        *time = new_time;
    }
}

impl Clock for FixedClock {
    #[allow(clippy::expect_used)] // Intentional: mutex poisoning is unrecoverable
    fn now(&self) -> DateTime<Utc> {
        *self
            .time
            .lock()
            .expect("FixedClock mutex poisoned - a thread panicked while holding the lock")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::time::Duration;

    #[test]
    fn system_clock_returns_current_time() {
        let clock = SystemClock::new();
        let before = Utc::now();
        let now = clock.now();
        let after = Utc::now();

        assert!(now >= before);
        assert!(now <= after);
    }

    #[test]
    fn fixed_clock_returns_fixed_time() {
        let time = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
        let clock = FixedClock::new(time);

        assert_eq!(clock.now(), time);
        assert_eq!(clock.now(), time); // Still the same
    }

    #[test]
    fn fixed_clock_advance_works() {
        let time = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
        let clock = FixedClock::new(time);

        clock.advance(Duration::from_secs(60));

        let expected = Utc.with_ymd_and_hms(2025, 1, 15, 10, 1, 0).unwrap();
        assert_eq!(clock.now(), expected);
    }

    #[test]
    fn fixed_clock_set_works() {
        let clock = FixedClock::epoch();
        let new_time = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();

        clock.set(new_time);

        assert_eq!(clock.now(), new_time);
    }

    #[test]
    fn fixed_clock_clones_share_state() {
        let clock1 = FixedClock::new(Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap());
        let clock2 = clock1.clone();

        // Both start at the same time
        assert_eq!(clock1.now(), clock2.now());

        // Advance clock1
        clock1.advance(Duration::from_secs(60));

        // clock2 sees the same advancement (shared state)
        assert_eq!(clock1.now(), clock2.now());
        assert_eq!(
            clock1.now(),
            Utc.with_ymd_and_hms(2025, 1, 15, 10, 1, 0).unwrap()
        );
    }
}
