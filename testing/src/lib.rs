//! # Composable Rust Testing
//!
//! Testing utilities and helpers for the Composable Rust architecture.
//!
//! This crate provides:
//! - Mock implementations of Environment traits
//! - Test helpers and builders
//! - Property-based testing utilities
//! - Assertion helpers for reducers and stores
//!
//! ## Example
//!
//! ```ignore
//! use composable_rust_testing::test_clock;
//! use composable_rust_runtime::Store;
//!
//! #[tokio::test]
//! async fn test_order_flow() {
//!     let env = test_environment();
//!     let store = OrderStore::new(OrderState::default(), OrderReducer, env);
//!
//!     store.send(OrderAction::PlaceOrder {
//!         customer_id: CustomerId::new(1),
//!         items: vec![],
//!     }).await;
//!
//!     let state = store.state(|s| s.clone()).await;
//!     assert_eq!(state.orders.len(), 1);
//! }
//! ```

use chrono::{DateTime, Utc};
use composable_rust_core::environment::Clock;

/// Mock implementations of Environment traits
///
/// # Phase 1 Implementation
///
/// This module will contain:
/// - `MockDatabase`: In-memory event store
/// - `FixedClock`: Deterministic time
/// - `MockEventPublisher`: Captures published events
/// - `MockHttpClient`: Stubbed HTTP responses
/// - `SequentialIdGenerator`: Predictable IDs
///
/// Mock implementations for testing.
pub mod mocks {
    use super::{Clock, DateTime, Utc};

    /// Fixed clock for deterministic tests
    ///
    /// Always returns the same time, making tests reproducible.
    ///
    /// # Example
    ///
    /// ```
    /// use composable_rust_testing::mocks::FixedClock;
    /// use composable_rust_core::environment::Clock;
    /// use chrono::Utc;
    ///
    /// let clock = FixedClock::new(Utc::now());
    /// let time1 = clock.now();
    /// let time2 = clock.now();
    /// assert_eq!(time1, time2); // Always the same!
    /// ```
    #[derive(Debug, Clone)]
    pub struct FixedClock {
        time: DateTime<Utc>,
    }

    impl FixedClock {
        /// Create a new fixed clock with the given time
        #[must_use]
        pub const fn new(time: DateTime<Utc>) -> Self {
            Self { time }
        }
    }

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.time
        }
    }

    /// Create a default fixed clock for tests (2025-01-01 00:00:00 UTC)
    ///
    /// # Panics
    ///
    /// This function will panic if the hardcoded timestamp fails to parse,
    /// which should never happen in practice.
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn test_clock() -> FixedClock {
        FixedClock::new(
            DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .expect("hardcoded timestamp should always parse")
                .with_timezone(&Utc),
        )
    }
}

/// Test helpers and utilities
///
/// # Phase 1 Implementation
///
/// This module will contain:
/// - Builder patterns for common test scenarios
/// - Assertion helpers
/// - Test data generators
///
/// Test helpers and utilities.
pub mod helpers {
    // Placeholder for test helpers
}

/// Property-based testing utilities
///
/// # Phase 1 Implementation
///
/// This module will contain:
/// - proptest Arbitrary implementations
/// - Custom strategies for domain types
/// - Property test helpers
///
/// Property-based testing utilities using proptest.
pub mod properties {
    // Placeholder for property test utilities
}

// Re-export commonly used items
pub use mocks::{FixedClock, test_clock};

// Placeholder test module
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixed_clock() {
        let clock = test_clock();
        let time1 = clock.now();
        let time2 = clock.now();
        assert_eq!(time1, time2);
    }
}
