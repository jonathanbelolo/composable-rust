//! Environment trait for request lifecycle reducer.

use composable_rust_core::environment::Clock;

/// Environment dependencies for the request lifecycle reducer.
///
/// This trait follows the Composable Rust pattern of dependency injection
/// via traits. Different implementations can be provided for production,
/// testing, etc.
pub trait RequestLifecycleEnvironment: Send + Sync {
    /// Clock for getting current time.
    ///
    /// Production uses `SystemClock`, tests use `FixedClock`.
    fn clock(&self) -> &dyn Clock;
}

/// Production environment for request lifecycle.
#[derive(Clone)]
pub struct ProductionRequestLifecycleEnvironment {
    clock: std::sync::Arc<dyn Clock>,
}

impl ProductionRequestLifecycleEnvironment {
    /// Create a new production environment.
    #[must_use]
    pub fn new(clock: std::sync::Arc<dyn Clock>) -> Self {
        Self { clock }
    }
}

impl RequestLifecycleEnvironment for ProductionRequestLifecycleEnvironment {
    fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }
}
