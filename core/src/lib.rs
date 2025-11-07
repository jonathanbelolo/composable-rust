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
pub use smallvec::{smallvec, SmallVec};

// Phase 2: Event sourcing modules
pub mod event;
pub mod event_store;
pub mod stream;

// Phase 3: Event bus for cross-aggregate communication
pub mod event_bus;

// Phase 3: Reducer composition utilities
pub mod composition;

// Phase 5: Projection system for read models (query side of CQRS)
pub mod projection;

// Phase 5: Effect helper macros for ergonomic effect construction
pub mod effect_macros;

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
    use smallvec::SmallVec;

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
        /// A `SmallVec` of effects to be executed by the runtime.
        ///
        /// `SmallVec<[Effect; 4]>` stores up to 4 effects inline on the stack,
        /// avoiding heap allocations for the common case of 0-3 effects per action.
        fn reduce(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            env: &Self::Environment,
        ) -> SmallVec<[Effect<Self::Action>; 4]>;
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

    use crate::event::SerializedEvent;
    use crate::event_bus::{EventBus, EventBusError};
    use crate::event_store::{EventStore, EventStoreError};
    use crate::stream::{StreamId, Version};
    use std::sync::Arc;

    /// Type alias for snapshot data: `(Version, Vec<u8>)`
    type SnapshotData = (Version, Vec<u8>);

    /// Event store operation descriptions for the `Effect::EventStore` variant.
    ///
    /// These operations describe event sourcing persistence operations that will be
    /// executed by the runtime with access to the `EventStore` implementation.
    ///
    /// Each operation includes success and error callbacks that produce optional actions,
    /// allowing the effect system to feed results back into the reducer loop.
    ///
    /// # Type Parameters
    ///
    /// - `Action`: The action type that callbacks can produce
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use composable_rust_core::effect::EventStoreOperation;
    ///
    /// let op = EventStoreOperation::AppendEvents {
    ///     stream_id: StreamId::new("order-123"),
    ///     expected_version: Some(Version::new(5)),
    ///     events: vec![serialized_event],
    ///     on_success: Box::new(|version| {
    ///         Some(OrderAction::EventsAppended { version })
    ///     }),
    ///     on_error: Box::new(|error| {
    ///         Some(OrderAction::AppendFailed { error: error.to_string() })
    ///     }),
    /// };
    /// ```
    pub enum EventStoreOperation<Action> {
        /// Append events to a stream with optimistic concurrency control.
        AppendEvents {
            /// The event store implementation to use
            event_store: Arc<dyn EventStore>,
            /// The stream to append events to
            stream_id: StreamId,
            /// Expected current version for optimistic concurrency
            expected_version: Option<Version>,
            /// Events to append
            events: Vec<SerializedEvent>,
            /// Callback invoked on success with the new version
            on_success: Box<dyn Fn(Version) -> Option<Action> + Send + Sync>,
            /// Callback invoked on error
            on_error: Box<dyn Fn(EventStoreError) -> Option<Action> + Send + Sync>,
        },

        /// Load events from a stream.
        LoadEvents {
            /// The event store implementation to use
            event_store: Arc<dyn EventStore>,
            /// The stream to load events from
            stream_id: StreamId,
            /// Optional starting version (None = load all events)
            from_version: Option<Version>,
            /// Callback invoked on success with the loaded events
            on_success: Box<dyn Fn(Vec<SerializedEvent>) -> Option<Action> + Send + Sync>,
            /// Callback invoked on error
            on_error: Box<dyn Fn(EventStoreError) -> Option<Action> + Send + Sync>,
        },

        /// Save a state snapshot for an aggregate.
        SaveSnapshot {
            /// The event store implementation to use
            event_store: Arc<dyn EventStore>,
            /// The stream this snapshot belongs to
            stream_id: StreamId,
            /// The version of the stream at snapshot time
            version: Version,
            /// The serialized state data
            state: Vec<u8>,
            /// Callback invoked on success
            on_success: Box<dyn Fn(()) -> Option<Action> + Send + Sync>,
            /// Callback invoked on error
            on_error: Box<dyn Fn(EventStoreError) -> Option<Action> + Send + Sync>,
        },

        /// Load the latest snapshot for a stream.
        LoadSnapshot {
            /// The event store implementation to use
            event_store: Arc<dyn EventStore>,
            /// The stream to load snapshot from
            stream_id: StreamId,
            /// Callback invoked on success with optional snapshot data
            on_success: Box<dyn Fn(Option<SnapshotData>) -> Option<Action> + Send + Sync>,
            /// Callback invoked on error
            on_error: Box<dyn Fn(EventStoreError) -> Option<Action> + Send + Sync>,
        },
    }

    /// Event bus operation descriptions for the `Effect::PublishEvent` variant.
    ///
    /// These operations describe event publishing operations that will be executed
    /// by the runtime with access to the `EventBus` implementation.
    ///
    /// Each operation includes success and error callbacks that produce optional actions,
    /// allowing the effect system to feed results back into the reducer loop.
    ///
    /// # Type Parameters
    ///
    /// - `Action`: The action type that callbacks can produce
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use composable_rust_core::effect::EventBusOperation;
    ///
    /// let op = EventBusOperation::Publish {
    ///     event_bus: Arc::clone(&event_bus),
    ///     topic: "order-events".to_string(),
    ///     event: serialized_event,
    ///     on_success: Box::new(|| Some(OrderAction::EventPublished)),
    ///     on_error: Box::new(|error| {
    ///         Some(OrderAction::PublishFailed { error: error.to_string() })
    ///     }),
    /// };
    /// ```
    pub enum EventBusOperation<Action> {
        /// Publish an event to a topic.
        Publish {
            /// The event bus implementation to use
            event_bus: Arc<dyn EventBus>,
            /// The topic to publish to (e.g., "order-events")
            topic: String,
            /// The event to publish
            event: SerializedEvent,
            /// Callback invoked on successful publish
            on_success: Box<dyn Fn(()) -> Option<Action> + Send + Sync>,
            /// Callback invoked on error
            on_error: Box<dyn Fn(EventBusError) -> Option<Action> + Send + Sync>,
        },
    }

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

        /// Event store operation (Phase 2)
        ///
        /// Describes an event sourcing persistence operation to be executed by
        /// the runtime with access to the `EventStore` implementation.
        ///
        /// # Examples
        ///
        /// ```rust,ignore
        /// use composable_rust_core::effect::{Effect, EventStoreOperation};
        ///
        /// let effect = Effect::EventStore(EventStoreOperation::AppendEvents {
        ///     stream_id: StreamId::new("order-123"),
        ///     expected_version: Some(Version::new(5)),
        ///     events: vec![serialized_event],
        ///     on_success: Box::new(|version| {
        ///         Some(OrderAction::EventsAppended { version })
        ///     }),
        ///     on_error: Box::new(|error| {
        ///         Some(OrderAction::AppendFailed { error: error.to_string() })
        ///     }),
        /// });
        /// ```
        EventStore(EventStoreOperation<Action>),

        /// Event bus operation (Phase 3)
        ///
        /// Describes an event publishing operation to be executed by the runtime
        /// with access to the `EventBus` implementation. Events are published to
        /// topics after being persisted to the event store.
        ///
        /// # Examples
        ///
        /// ```rust,ignore
        /// use composable_rust_core::effect::{Effect, EventBusOperation};
        ///
        /// let effect = Effect::PublishEvent(EventBusOperation::Publish {
        ///     event_bus: Arc::clone(&event_bus),
        ///     topic: "order-events".to_string(),
        ///     event: serialized_event,
        ///     on_success: Box::new(|| Some(OrderAction::EventPublished)),
        ///     on_error: Box::new(|error| {
        ///         Some(OrderAction::PublishFailed { error: error.to_string() })
        ///     }),
        /// });
        /// ```
        PublishEvent(EventBusOperation<Action>),
        // Additional effect variants will be added in future phases:
        // - Http { request, on_success, on_error }
        // - Cancellable { id, effect }
        // - DispatchCommand(Command) - for saga coordination
    }

    // Manual Debug implementation since Future and EventStoreOperation don't implement Debug
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
                Effect::EventStore(op) => match op {
                    EventStoreOperation::AppendEvents {
                        stream_id,
                        expected_version,
                        events,
                        ..
                    } => f
                        .debug_struct("Effect::EventStore::AppendEvents")
                        .field("stream_id", stream_id)
                        .field("expected_version", expected_version)
                        .field("event_count", &events.len())
                        .field("event_store", &"<event_store>")
                        .finish(),
                    EventStoreOperation::LoadEvents {
                        stream_id,
                        from_version,
                        ..
                    } => f
                        .debug_struct("Effect::EventStore::LoadEvents")
                        .field("stream_id", stream_id)
                        .field("from_version", from_version)
                        .field("event_store", &"<event_store>")
                        .finish(),
                    EventStoreOperation::SaveSnapshot {
                        stream_id,
                        version,
                        state,
                        ..
                    } => f
                        .debug_struct("Effect::EventStore::SaveSnapshot")
                        .field("stream_id", stream_id)
                        .field("version", version)
                        .field("state_size", &state.len())
                        .field("event_store", &"<event_store>")
                        .finish(),
                    EventStoreOperation::LoadSnapshot { stream_id, .. } => f
                        .debug_struct("Effect::EventStore::LoadSnapshot")
                        .field("stream_id", stream_id)
                        .field("event_store", &"<event_store>")
                        .finish(),
                },
                Effect::PublishEvent(op) => match op {
                    EventBusOperation::Publish { topic, event, .. } => f
                        .debug_struct("Effect::PublishEvent::Publish")
                        .field("topic", topic)
                        .field("event_type", &event.event_type)
                        .field("event_bus", &"<event_bus>")
                        .finish(),
                },
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

        /// Transform the action type of this effect
        ///
        /// This is useful for composing effects from different reducers or
        /// wrapping actions in a higher-level action type.
        ///
        /// # Type Parameters
        ///
        /// - `B`: The target action type
        /// - `F`: Function that transforms `Action` to `B`
        ///
        /// # Arguments
        ///
        /// - `f`: The transformation function
        ///
        /// # Returns
        ///
        /// A new effect that produces actions of type `B`
        ///
        /// # Examples
        ///
        /// ```rust,ignore
        /// // Transform counter actions to app-level actions
        /// let counter_effect: Effect<CounterAction> = Effect::Delay {
        ///     duration: Duration::from_secs(1),
        ///     action: Box::new(CounterAction::Increment),
        /// };
        ///
        /// let app_effect: Effect<AppAction> = counter_effect.map(|a| AppAction::Counter(a));
        /// ```
        pub fn map<B, F>(self, f: F) -> Effect<B>
        where
            F: Fn(Action) -> B + Send + Sync + 'static + Clone,
            Action: 'static,
            B: Send + 'static,
        {
            match self {
                Effect::None => Effect::None,
                Effect::Parallel(effects) => {
                    let mapped: Vec<Effect<B>> = effects
                        .into_iter()
                        .map(|e| {
                            let f_clone = f.clone();
                            map_effect(e, f_clone)
                        })
                        .collect();
                    Effect::Parallel(mapped)
                },
                Effect::Sequential(effects) => {
                    let mapped: Vec<Effect<B>> = effects
                        .into_iter()
                        .map(|e| {
                            let f_clone = f.clone();
                            map_effect(e, f_clone)
                        })
                        .collect();
                    Effect::Sequential(mapped)
                },
                Effect::Delay { duration, action } => Effect::Delay {
                    duration,
                    action: Box::new(f(*action)),
                },
                Effect::Future(fut) => Effect::Future(Box::pin(async move { fut.await.map(f) })),
                Effect::EventStore(op) => Effect::EventStore(map_event_store_operation(op, f)),
                Effect::PublishEvent(op) => Effect::PublishEvent(map_event_bus_operation(op, f)),
            }
        }
    }

    // Helper function to avoid recursion in type system
    fn map_effect<A, B, F>(effect: Effect<A>, f: F) -> Effect<B>
    where
        F: Fn(A) -> B + Send + Sync + 'static + Clone,
        A: 'static,
        B: Send + 'static,
    {
        match effect {
            Effect::None => Effect::None,
            Effect::Parallel(effects) => {
                let mapped: Vec<Effect<B>> = effects
                    .into_iter()
                    .map(|e| {
                        let f_clone = f.clone();
                        map_effect(e, f_clone)
                    })
                    .collect();
                Effect::Parallel(mapped)
            },
            Effect::Sequential(effects) => {
                let mapped: Vec<Effect<B>> = effects
                    .into_iter()
                    .map(|e| {
                        let f_clone = f.clone();
                        map_effect(e, f_clone)
                    })
                    .collect();
                Effect::Sequential(mapped)
            },
            Effect::Delay { duration, action } => Effect::Delay {
                duration,
                action: Box::new(f(*action)),
            },
            Effect::Future(fut) => Effect::Future(Box::pin(async move { fut.await.map(f) })),
            Effect::EventStore(op) => Effect::EventStore(map_event_store_operation(op, f)),
            Effect::PublishEvent(op) => Effect::PublishEvent(map_event_bus_operation(op, f)),
        }
    }

    // Helper function to map EventStoreOperation callbacks to new action type
    fn map_event_store_operation<A, B, F>(
        op: EventStoreOperation<A>,
        f: F,
    ) -> EventStoreOperation<B>
    where
        F: Fn(A) -> B + Send + Sync + 'static + Clone,
        A: 'static,
        B: Send + 'static,
    {
        match op {
            EventStoreOperation::AppendEvents {
                event_store,
                stream_id,
                expected_version,
                events,
                on_success,
                on_error,
            } => {
                let f_success = f.clone();
                let f_error = f.clone();
                EventStoreOperation::AppendEvents {
                    event_store,
                    stream_id,
                    expected_version,
                    events,
                    on_success: Box::new(move |version| {
                        on_success(version).map(|a| f_success.clone()(a))
                    }),
                    on_error: Box::new(move |error| on_error(error).map(|a| f_error.clone()(a))),
                }
            },
            EventStoreOperation::LoadEvents {
                event_store,
                stream_id,
                from_version,
                on_success,
                on_error,
            } => {
                let f_success = f.clone();
                let f_error = f.clone();
                EventStoreOperation::LoadEvents {
                    event_store,
                    stream_id,
                    from_version,
                    on_success: Box::new(move |events| {
                        on_success(events).map(|a| f_success.clone()(a))
                    }),
                    on_error: Box::new(move |error| on_error(error).map(|a| f_error.clone()(a))),
                }
            },
            EventStoreOperation::SaveSnapshot {
                event_store,
                stream_id,
                version,
                state,
                on_success,
                on_error,
            } => {
                let f_success = f.clone();
                let f_error = f.clone();
                EventStoreOperation::SaveSnapshot {
                    event_store,
                    stream_id,
                    version,
                    state,
                    on_success: Box::new(move |unit| {
                        on_success(unit).map(|a| f_success.clone()(a))
                    }),
                    on_error: Box::new(move |error| on_error(error).map(|a| f_error.clone()(a))),
                }
            },
            EventStoreOperation::LoadSnapshot {
                event_store,
                stream_id,
                on_success,
                on_error,
            } => {
                let f_success = f.clone();
                let f_error = f;
                EventStoreOperation::LoadSnapshot {
                    event_store,
                    stream_id,
                    on_success: Box::new(move |snapshot| {
                        on_success(snapshot).map(|a| f_success.clone()(a))
                    }),
                    on_error: Box::new(move |error| on_error(error).map(|a| f_error.clone()(a))),
                }
            },
        }
    }

    // Helper function to map EventBusOperation callbacks to new action type
    fn map_event_bus_operation<A, B, F>(op: EventBusOperation<A>, f: F) -> EventBusOperation<B>
    where
        F: Fn(A) -> B + Send + Sync + 'static + Clone,
        A: 'static,
        B: Send + 'static,
    {
        match op {
            EventBusOperation::Publish {
                event_bus,
                topic,
                event,
                on_success,
                on_error,
            } => {
                let f_success = f.clone();
                let f_error = f;
                EventBusOperation::Publish {
                    event_bus,
                    topic,
                    event,
                    on_success: Box::new(move |unit| {
                        on_success(unit).map(|a| f_success.clone()(a))
                    }),
                    on_error: Box::new(move |error| on_error(error).map(|a| f_error.clone()(a))),
                }
            },
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
    /// ```
    /// use composable_rust_core::environment::{Clock, SystemClock};
    ///
    /// // Production - uses system clock
    /// let clock = SystemClock;
    /// let now = clock.now();
    /// ```
    pub trait Clock: Send + Sync {
        /// Get the current time
        fn now(&self) -> DateTime<Utc>;
    }

    /// Production clock implementation that uses the system time.
    ///
    /// This is a zero-sized type that delegates to `chrono::Utc::now()`.
    ///
    /// # Example
    ///
    /// ```
    /// use composable_rust_core::environment::{Clock, SystemClock};
    ///
    /// let clock = SystemClock;
    /// let now = clock.now();
    /// println!("Current time: {}", now);
    /// ```
    #[derive(Debug, Clone, Copy, Default)]
    pub struct SystemClock;

    impl Clock for SystemClock {
        fn now(&self) -> DateTime<Utc> {
            Utc::now()
        }
    }

    // Additional traits will be defined during Phase 1:
    // - Database: Event store operations
    // - EventPublisher: Event bus publishing
    // - HttpClient: External HTTP calls
    // - IdGenerator: ID generation for aggregates
}

// Placeholder test module
#[cfg(test)]
#[allow(clippy::panic)] // Tests can panic for assertions
#[allow(clippy::similar_names)] // Test variable names can be similar
#[allow(clippy::redundant_closure)] // Test closures can be explicit for clarity
mod tests {
    use super::effect::Effect;
    use std::time::Duration;

    #[derive(Debug, Clone, PartialEq)]
    enum TestAction {
        Action1,
        Action2,
        Action3,
    }

    #[derive(Debug, Clone, PartialEq)]
    enum MappedAction {
        Mapped(TestAction),
    }

    #[test]
    fn test_effect_merge() {
        let effect1 = Effect::None;
        let effect2 = Effect::<TestAction>::None;

        let merged = Effect::merge(vec![effect1, effect2]);

        match merged {
            Effect::Parallel(effects) => {
                assert_eq!(effects.len(), 2);
            },
            _ => panic!("Expected Parallel effect"),
        }
    }

    #[test]
    fn test_effect_chain() {
        let effect1 = Effect::None;
        let effect2 = Effect::<TestAction>::None;

        let chained = Effect::chain(vec![effect1, effect2]);

        match chained {
            Effect::Sequential(effects) => {
                assert_eq!(effects.len(), 2);
            },
            _ => panic!("Expected Sequential effect"),
        }
    }

    #[test]
    fn test_effect_map_none() {
        let effect: Effect<TestAction> = Effect::None;
        let mapped: Effect<MappedAction> = effect.map(|a| MappedAction::Mapped(a));

        match mapped {
            Effect::None => {},
            _ => panic!("Expected None effect"),
        }
    }

    #[test]
    fn test_effect_map_delay() {
        let effect: Effect<TestAction> = Effect::Delay {
            duration: Duration::from_secs(1),
            action: Box::new(TestAction::Action1),
        };

        let mapped: Effect<MappedAction> = effect.map(|a| MappedAction::Mapped(a));

        match mapped {
            Effect::Delay { duration, action } => {
                assert_eq!(duration, Duration::from_secs(1));
                assert_eq!(*action, MappedAction::Mapped(TestAction::Action1));
            },
            _ => panic!("Expected Delay effect"),
        }
    }

    #[test]
    fn test_effect_map_parallel() {
        let effect: Effect<TestAction> = Effect::Parallel(vec![
            Effect::None,
            Effect::Delay {
                duration: Duration::from_millis(100),
                action: Box::new(TestAction::Action2),
            },
        ]);

        let mapped: Effect<MappedAction> = effect.map(|a| MappedAction::Mapped(a));

        match mapped {
            Effect::Parallel(effects) => {
                assert_eq!(effects.len(), 2);
                // First should be None
                matches!(effects[0], Effect::None);
                // Second should be Delay with mapped action
                match &effects[1] {
                    Effect::Delay { action, .. } => {
                        assert_eq!(**action, MappedAction::Mapped(TestAction::Action2));
                    },
                    _ => panic!("Expected Delay in parallel"),
                }
            },
            _ => panic!("Expected Parallel effect"),
        }
    }

    #[test]
    fn test_effect_map_sequential() {
        let effect: Effect<TestAction> = Effect::Sequential(vec![
            Effect::Delay {
                duration: Duration::from_millis(100),
                action: Box::new(TestAction::Action1),
            },
            Effect::Delay {
                duration: Duration::from_millis(200),
                action: Box::new(TestAction::Action2),
            },
        ]);

        let mapped: Effect<MappedAction> = effect.map(|a| MappedAction::Mapped(a));

        match mapped {
            Effect::Sequential(effects) => {
                assert_eq!(effects.len(), 2);
                // Verify both delays are mapped correctly
                for effect in effects {
                    match effect {
                        Effect::Delay { action, .. } => {
                            // Verify it's a Mapped variant
                            assert!(matches!(*action, MappedAction::Mapped(_)));
                        },
                        _ => panic!("Expected Delay in sequential"),
                    }
                }
            },
            _ => panic!("Expected Sequential effect"),
        }
    }

    #[tokio::test]
    async fn test_effect_map_future() {
        let effect: Effect<TestAction> =
            Effect::Future(Box::pin(async { Some(TestAction::Action1) }));

        let mapped: Effect<MappedAction> = effect.map(|a| MappedAction::Mapped(a));

        match mapped {
            Effect::Future(fut) => {
                let result = fut.await;
                assert_eq!(result, Some(MappedAction::Mapped(TestAction::Action1)));
            },
            _ => panic!("Expected Future effect"),
        }
    }

    #[tokio::test]
    async fn test_effect_map_future_none() {
        let effect: Effect<TestAction> = Effect::Future(Box::pin(async { None }));

        let mapped: Effect<MappedAction> = effect.map(|a| MappedAction::Mapped(a));

        match mapped {
            Effect::Future(fut) => {
                let result = fut.await;
                assert_eq!(result, None);
            },
            _ => panic!("Expected Future effect"),
        }
    }

    #[test]
    fn test_effect_map_nested() {
        // Test mapping nested effects (Parallel containing Sequential)
        let effect: Effect<TestAction> = Effect::Parallel(vec![
            Effect::Sequential(vec![
                Effect::Delay {
                    duration: Duration::from_millis(100),
                    action: Box::new(TestAction::Action1),
                },
                Effect::None,
            ]),
            Effect::Delay {
                duration: Duration::from_millis(200),
                action: Box::new(TestAction::Action3),
            },
        ]);

        let mapped: Effect<MappedAction> = effect.map(|a| MappedAction::Mapped(a));

        match mapped {
            Effect::Parallel(effects) => {
                assert_eq!(effects.len(), 2);
                // Verify nested structure is preserved
                match &effects[0] {
                    Effect::Sequential(inner) => {
                        assert_eq!(inner.len(), 2);
                    },
                    _ => panic!("Expected Sequential in Parallel"),
                }
            },
            _ => panic!("Expected Parallel effect"),
        }
    }

    // ========== SmallVec Spillover Tests ==========

    #[test]
    fn test_smallvec_inline_storage() {
        use crate::{smallvec, SmallVec};

        // Test that ≤4 effects stay on stack (no heap allocation)
        let effects: SmallVec<[Effect<TestAction>; 4]> = smallvec![
            Effect::None,
            Effect::None,
            Effect::None,
            Effect::None,
        ];

        assert_eq!(effects.len(), 4);
        assert!(!effects.spilled(), "Should NOT spill to heap with 4 effects");
    }

    #[test]
    fn test_smallvec_heap_spillover() {
        use crate::{smallvec, SmallVec};

        // Test that >4 effects correctly spill to heap
        let effects: SmallVec<[Effect<TestAction>; 4]> = smallvec![
            Effect::None,
            Effect::None,
            Effect::None,
            Effect::None,
            Effect::None, // 5th effect triggers heap allocation
        ];

        assert_eq!(effects.len(), 5);
        assert!(effects.spilled(), "SHOULD spill to heap with 5 effects");

        // Verify all effects are accessible
        for (i, effect) in effects.iter().enumerate() {
            assert!(
                matches!(effect, Effect::None),
                "Effect {i} should be None"
            );
        }
    }

    #[test]
    fn test_smallvec_many_effects() {
        use crate::{smallvec, SmallVec};

        // Test with significantly more effects (10)
        let effects: SmallVec<[Effect<TestAction>; 4]> = smallvec![
            Effect::None,
            Effect::None,
            Effect::None,
            Effect::None,
            Effect::None,
            Effect::None,
            Effect::None,
            Effect::None,
            Effect::None,
            Effect::None,
        ];

        assert_eq!(effects.len(), 10);
        assert!(effects.spilled(), "Should spill with 10 effects");

        // Verify iteration works correctly
        assert_eq!(effects.iter().count(), 10);
    }

    #[test]
    fn test_smallvec_collect_into() {
        use crate::SmallVec;

        // Test that collect works correctly (common pattern in reducers)
        let effects: SmallVec<[Effect<TestAction>; 4]> = (0..6)
            .map(|_| Effect::None)
            .collect();

        assert_eq!(effects.len(), 6);
        assert!(effects.spilled(), "Collected 6 effects should spill");
    }
}
