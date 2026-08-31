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

/// Real time, shifted by an amount the holder controls.
///
/// [`FixedClock`] freezes time, which is what a unit test around a pure
/// `process()` wants. An integration test wants something different: the
/// application under test writes `updated_at` from this clock and several
/// generated queries `ORDER BY updated_at`, so a frozen clock gives every row
/// in a scenario the same timestamp and makes that ordering arbitrary. What
/// such a test needs is time that still MOVES, and that it can also jump.
///
/// `now()` is `base.now() + offset`. [`advance`](Self::advance) adds to the
/// offset; [`set`](Self::set) pins `now()` to an instant and lets it run on from
/// there, which is how a rule about a particular weekday or hour is reached.
///
/// Cloning shares the offset, so a harness can hand the application one handle
/// and keep another to steer with — the same sharing [`FixedClock`] documents.
pub struct OffsetClock<C: Clock = SystemClock> {
    base: C,
    offset: Arc<Mutex<chrono::Duration>>,
}

impl Default for OffsetClock<SystemClock> {
    fn default() -> Self {
        Self::new(SystemClock)
    }
}

impl<C: Clock> OffsetClock<C> {
    /// A clock reading `base`, with no shift yet.
    pub fn new(base: C) -> Self {
        Self {
            base,
            offset: Arc::new(Mutex::new(chrono::Duration::zero())),
        }
    }

    /// Move time forward by `duration`.
    #[allow(clippy::expect_used)] // Intentional: mutex poisoning is unrecoverable
    pub fn advance(&self, duration: std::time::Duration) {
        let delta = chrono::Duration::from_std(duration)
            .unwrap_or_else(|_| chrono::TimeDelta::MAX);
        let mut offset = self
            .offset
            .lock()
            .expect("OffsetClock mutex poisoned - a thread panicked while holding the lock");
        *offset = offset.checked_add(&delta).unwrap_or(chrono::TimeDelta::MAX);
    }

    /// Make `now()` read `instant`, and carry on from there.
    ///
    /// Deliberately not a freeze: the shift is fixed, not the time. Later reads
    /// are `instant` plus however long has actually elapsed, so a scenario that
    /// jumps to a Thursday afternoon still gets increasing timestamps for the
    /// commands it sends next.
    #[allow(clippy::expect_used)] // Intentional: mutex poisoning is unrecoverable
    pub fn set(&self, instant: DateTime<Utc>) {
        let mut offset = self
            .offset
            .lock()
            .expect("OffsetClock mutex poisoned - a thread panicked while holding the lock");
        *offset = instant - self.base.now();
    }

    /// Drop the shift; `now()` reads the base clock again.
    #[allow(clippy::expect_used)] // Intentional: mutex poisoning is unrecoverable
    pub fn reset(&self) {
        *self
            .offset
            .lock()
            .expect("OffsetClock mutex poisoned - a thread panicked while holding the lock") =
            chrono::Duration::zero();
    }
}

impl<C: Clock + Clone> Clone for OffsetClock<C> {
    fn clone(&self) -> Self {
        Self {
            base: self.base.clone(),
            offset: Arc::clone(&self.offset),
        }
    }
}

impl<C: Clock> Clock for OffsetClock<C> {
    #[allow(clippy::expect_used)] // Intentional: mutex poisoning is unrecoverable
    fn now(&self) -> DateTime<Utc> {
        let offset = *self
            .offset
            .lock()
            .expect("OffsetClock mutex poisoned - a thread panicked while holding the lock");
        self.base.now() + offset
    }
}

/// A clock behind a shared pointer is a clock.
///
/// Without this, `Arc<dyn Clock>` does not satisfy `Clock`, so a composition
/// root cannot hold the trait — it has to name a concrete clock type in every
/// environment it builds, and the choice becomes unreachable from outside.
/// That is what happened to the generated applications: `SystemClock` was
/// hardwired at every environment, and no integration test could control time,
/// so every rule expressed in days, hours or weekdays was untestable.
///
/// `?Sized` is what makes the trait-object form work; the blanket also covers
/// `Arc<SystemClock>` and `Arc<FixedClock>` for free.
///
/// Cloning an `Arc` shares the clock, which matches [`FixedClock`]'s own
/// documented behaviour: every holder observes the same `advance`.
impl<T: Clock + ?Sized> Clock for Arc<T> {
    fn now(&self) -> DateTime<Utc> {
        (**self).now()
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

    /// A composition root must be able to hold the TRAIT, not a concrete clock.
    ///
    /// The generated applications hardwired `SystemClock` into every
    /// environment because `Arc<dyn Clock>` did not satisfy `Clock`, which left
    /// integration tests unable to control time at all.
    fn read_generically<C: Clock>(clock: &C) -> DateTime<Utc> {
        clock.now()
    }

    /// An offset clock JUMPS but does not FREEZE.
    ///
    /// That distinction is the reason it exists. The generated applications
    /// write `updated_at` from the injected clock and several generated queries
    /// order by it, so a frozen clock would give every row in one scenario the
    /// same timestamp and make that ordering arbitrary.
    #[test]
    fn an_offset_clock_jumps_without_freezing() {
        let base = FixedClock::new(Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap());
        let clock = OffsetClock::new(base.clone());
        assert_eq!(clock.now(), base.now(), "no shift to begin with");

        clock.advance(Duration::from_secs(14 * 24 * 3600));
        assert_eq!(clock.now(), Utc.with_ymd_and_hms(2025, 1, 29, 10, 0, 0).unwrap());

        // Time still moves underneath the shift — this is what a frozen clock
        // cannot do, and what keeps `ORDER BY updated_at` meaningful.
        base.advance(Duration::from_secs(1));
        assert_eq!(clock.now(), Utc.with_ymd_and_hms(2025, 1, 29, 10, 0, 1).unwrap());

        // `set` pins the instant, then lets it run on from there.
        let thursday = Utc.with_ymd_and_hms(2025, 3, 6, 14, 0, 0).unwrap();
        clock.set(thursday);
        assert_eq!(clock.now(), thursday);
        base.advance(Duration::from_secs(60));
        assert_eq!(clock.now(), thursday + chrono::Duration::seconds(60));

        clock.reset();
        assert_eq!(clock.now(), base.now());
    }

    /// Clones share the shift, so a harness keeps the lever after handing the
    /// clock to the application.
    #[test]
    fn offset_clock_clones_share_the_shift() {
        let clock = OffsetClock::new(FixedClock::new(
            Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap(),
        ));
        let erased: Arc<dyn Clock> = Arc::new(clock.clone());

        clock.advance(Duration::from_secs(3600));
        assert_eq!(erased.now(), Utc.with_ymd_and_hms(2025, 1, 15, 11, 0, 0).unwrap());
    }

    #[test]
    fn a_clock_behind_a_shared_pointer_is_a_clock() {
        let fixed = FixedClock::new(Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap());
        let erased: Arc<dyn Clock> = Arc::new(fixed.clone());

        // The trait object reads through to the clock it wraps.
        assert_eq!(erased.now(), fixed.now());

        // And advancing the original is visible through the erased handle, so a
        // test harness can keep the lever after handing the clock to the app.
        fixed.advance(Duration::from_secs(3600));
        assert_eq!(erased.now(), Utc.with_ymd_and_hms(2025, 1, 15, 11, 0, 0).unwrap());

        // Accepting a generic `C: Clock` is what an environment does, and it is
        // the bound the blanket impl has to satisfy.
        assert_eq!(read_generically(&erased), fixed.now());
        assert!(read_generically(&Arc::new(SystemClock)).timestamp() > 0);
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
