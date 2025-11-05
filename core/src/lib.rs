//! # Composable Rust Core
//!
//! Core traits and types for the Composable Rust architecture.
//!
//! This crate provides the fundamental abstractions for building event-driven,
//! functional backend systems using the Reducer pattern with CQRS and Event Sourcing.
//!
//! ## Core Concepts
//!
//! - **State**: Domain state for a feature
//! - **Action**: All possible inputs to a reducer (commands, events, cross-aggregate events)
//! - **Reducer**: Pure function `(State, Action, Environment) → (State, Effects)`
//! - **Effect**: Side effect descriptions (not execution)
//! - **Environment**: Injected dependencies via traits
//!
//! ## Architecture Principles
//!
//! - Functional Core, Imperative Shell
//! - Unidirectional Data Flow
//! - Explicit Effects (no hidden I/O)
//! - Dependency Injection via Environment
//! - Zero-Cost Abstractions
//!
//! ## Example
//!
//! ```ignore
//! use composable_rust_core::*;
//!
//! // Define your state
//! #[derive(Clone, Debug)]
//! struct OrderState {
//!     orders: HashMap<OrderId, Order>,
//! }
//!
//! // Define your actions
//! #[derive(Clone, Debug)]
//! enum OrderAction {
//!     PlaceOrder { customer_id: CustomerId, items: Vec<LineItem> },
//!     OrderPlaced { order_id: OrderId, timestamp: DateTime<Utc> },
//! }
//!
//! // Implement the reducer
//! impl Reducer for OrderReducer {
//!     type State = OrderState;
//!     type Action = OrderAction;
//!     type Environment = OrderEnvironment;
//!
//!     fn reduce(
//!         &self,
//!         state: &mut OrderState,
//!         action: OrderAction,
//!         env: &OrderEnvironment,
//!     ) -> Vec<Effect<OrderAction>> {
//!         // Business logic goes here
//!         vec![]
//!     }
//! }
//! ```

// Re-export commonly used types
pub use chrono::{DateTime, Utc};
pub use serde::{Deserialize, Serialize};

/// Action module - Unified input type for reducers (commands, events, cross-aggregate events)
///
/// # Phase 1 Implementation
///
/// This module will contain:
/// - Action trait (if needed for common behavior)
/// - Action composition utilities
/// - Action type helpers
///
/// Actions represent all possible state transitions in the system.
/// They unify commands (requests to change state) and events (facts about what happened).
pub mod action {}

/// State module - Domain state types and utilities
///
/// # Phase 1 Implementation
///
/// This module will contain:
/// - State trait requirements (Clone, Debug)
/// - State helpers and utilities
/// - Common state patterns
///
/// State represents the current domain state of a feature.
/// It should be owned data, Clone-able, and avoid lifetimes where possible.
pub mod state {}

/// Reducer module - The core trait for business logic
///
/// # Phase 1 Implementation
///
/// This module will contain:
/// - Reducer trait definition
/// - Reducer composition utilities (`combine_reducers`, `scope_reducer`)
/// - Helper macros for deriving reducers
///
/// Reducers are pure functions: `(State, Action, Environment) → (State, Effects)`
///
/// They contain all business logic and are deterministic and testable.
pub mod reducer {
    use super::effect::Effect;

    /// The Reducer trait - core abstraction for business logic
    ///
    /// # Type Parameters
    ///
    /// - `State`: The domain state this reducer operates on
    /// - `Action`: The action type this reducer processes
    /// - `Environment`: The injected dependencies this reducer needs
    ///
    /// # Example
    ///
    /// ```ignore
    /// impl Reducer for OrderReducer {
    ///     type State = OrderState;
    ///     type Action = OrderAction;
    ///     type Environment = OrderEnvironment;
    ///
    ///     fn reduce(
    ///         &self,
    ///         state: &mut OrderState,
    ///         action: OrderAction,
    ///         env: &OrderEnvironment,
    ///     ) -> Vec<Effect<OrderAction>> {
    ///         match action {
    ///             OrderAction::PlaceOrder { customer_id, items } => {
    ///                 // Business logic here
    ///                 vec![Effect::None]
    ///             }
    ///             _ => vec![Effect::None],
    ///         }
    ///     }
    /// }
    /// ```
    pub trait Reducer {
        /// The state type this reducer operates on
        type State;

        /// The action type this reducer processes
        type Action;

        /// The environment type with injected dependencies
        type Environment;

        /// Reduce an action into state changes and effects
        ///
        /// This is a pure function that:
        /// 1. Validates the action
        /// 2. Updates state in place
        /// 3. Returns effect descriptions to be executed
        ///
        /// # Arguments
        ///
        /// - `state`: Mutable reference to current state
        /// - `action`: The action to process
        /// - `env`: Reference to injected dependencies
        ///
        /// # Returns
        ///
        /// A vector of effects to be executed by the runtime
        fn reduce(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            env: &Self::Environment,
        ) -> Vec<Effect<Self::Action>>;
    }
}

/// Effect module - Side effect descriptions
///
/// # Phase 1 Implementation
///
/// This module will contain:
/// - Effect enum with all effect variants
/// - Effect composition utilities (merge, chain)
/// - Effect execution interface (implemented in runtime crate)
///
/// Effects describe side effects to be performed by the runtime.
/// They are values (not execution) and are composable and cancellable.
pub mod effect {
    use std::future::Future;
    use std::pin::Pin;
    use std::time::Duration;

    /// Effect type - describes a side effect to be executed
    ///
    /// Effects are NOT executed immediately. They are descriptions of what should happen,
    /// returned from reducers and executed by the Store runtime.
    ///
    /// # Type Parameters
    ///
    /// - `Action`: The action type that effects can produce (feedback loop)
    ///
    /// # Phase 1 Note
    ///
    /// Some variants reference types that will be defined during implementation:
    /// - `DbOperation`: Database operation types
    /// - `HttpRequest`/`Response`: HTTP client types
    /// - `Event`: Event bus event types
    /// - `EffectId`: Effect cancellation identifiers
    #[allow(missing_docs)]
    pub enum Effect<Action> {
        /// No-op effect
        None,

        /// Run effects in parallel
        Parallel(Vec<Effect<Action>>),

        /// Run effects sequentially
        Sequential(Vec<Effect<Action>>),

        /// Delayed action (for timeouts, retries)
        Delay {
            /// How long to wait
            duration: Duration,
            /// Action to dispatch after delay
            action: Box<Action>,
        },

        /// Arbitrary async computation
        ///
        /// Returns `Option<Action>` - if Some, the action is fed back into the reducer
        Future(Pin<Box<dyn Future<Output = Option<Action>> + Send>>),
        // Additional effect variants will be added during Phase 1 implementation:
        // - Database(DbOperation)
        // - Http { request, on_success, on_error }
        // - PublishEvent(Event)
        // - Cancellable { id, effect }
        // - DispatchCommand(Command) - for saga coordination
    }

    // Manual Debug implementation since Future doesn't implement Debug
    impl<Action> std::fmt::Debug for Effect<Action>
    where
        Action: std::fmt::Debug,
    {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Effect::None => write!(f, "Effect::None"),
                Effect::Parallel(effects) => {
                    f.debug_tuple("Effect::Parallel").field(effects).finish()
                },
                Effect::Sequential(effects) => {
                    f.debug_tuple("Effect::Sequential").field(effects).finish()
                },
                Effect::Delay { duration, action } => f
                    .debug_struct("Effect::Delay")
                    .field("duration", duration)
                    .field("action", action)
                    .finish(),
                Effect::Future(_) => write!(f, "Effect::Future(<future>)"),
            }
        }
    }

    impl<Action> Effect<Action> {
        /// Combine effects to run in parallel
        #[must_use]
        pub const fn merge(effects: Vec<Effect<Action>>) -> Effect<Action> {
            Effect::Parallel(effects)
        }

        /// Chain effects to run sequentially
        #[must_use]
        pub const fn chain(effects: Vec<Effect<Action>>) -> Effect<Action> {
            Effect::Sequential(effects)
        }
    }
}

/// Environment module - Dependency injection traits
///
/// # Phase 1 Implementation
///
/// This module will contain:
/// - Core dependency traits (Database, Clock, `EventPublisher`, `HttpClient`, `IdGenerator`)
/// - Environment composition utilities
/// - Production, Test, and Development implementations
///
/// All external dependencies are abstracted behind traits and injected
/// via the Environment parameter.
pub mod environment {
    use chrono::{DateTime, Utc};

    /// Clock trait - abstracts time operations for testability
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Production - uses system clock
    /// struct SystemClock;
    /// impl Clock for SystemClock {
    ///     fn now(&self) -> DateTime<Utc> {
    ///         Utc::now()
    ///     }
    /// }
    ///
    /// // Test - fixed time for deterministic tests
    /// struct FixedClock { time: DateTime<Utc> }
    /// impl Clock for FixedClock {
    ///     fn now(&self) -> DateTime<Utc> {
    ///         self.time
    ///     }
    /// }
    /// ```
    pub trait Clock: Send + Sync {
        /// Get the current time
        fn now(&self) -> DateTime<Utc>;
    }

    // Additional traits will be defined during Phase 1:
    // - Database: Event store operations
    // - EventPublisher: Event bus publishing
    // - HttpClient: External HTTP calls
    // - IdGenerator: ID generation for aggregates
}

// Placeholder test module
#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
