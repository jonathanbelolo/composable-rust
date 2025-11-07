//! Declarative macros for ergonomic effect construction
//!
//! These macros reduce boilerplate when creating `Effect` variants, particularly
//! for event sourcing and event bus operations.

/// Create an `Effect::EventStore` with `AppendEvents` operation
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_core::append_events;
///
/// append_events! {
///     store: event_store,
///     stream: "order-123",
///     expected_version: Some(Version::new(5)),
///     events: vec![serialized_event],
///     on_success: |version| Some(OrderAction::EventsAppended { version }),
///     on_error: |error| Some(OrderAction::AppendFailed { error: error.to_string() })
/// }
/// ```
#[macro_export]
macro_rules! append_events {
    (
        store: $store:expr,
        stream: $stream:expr,
        expected_version: $expected:expr,
        events: $events:expr,
        on_success: |$success_param:ident| $success_body:expr,
        on_error: |$error_param:ident| $error_body:expr
    ) => {
        $crate::effect::Effect::EventStore(
            $crate::effect::EventStoreOperation::AppendEvents {
                event_store: ::std::sync::Arc::clone(&$store),
                stream_id: $crate::stream::StreamId::new($stream),
                expected_version: $expected,
                events: $events,
                on_success: ::std::boxed::Box::new(move |$success_param| $success_body),
                on_error: ::std::boxed::Box::new(move |$error_param| $error_body),
            }
        )
    };
}

/// Create an `Effect::EventStore` with `LoadEvents` operation
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_core::load_events;
///
/// load_events! {
///     store: event_store,
///     stream: "order-123",
///     from_version: None,
///     on_success: |events| Some(OrderAction::EventsLoaded { events }),
///     on_error: |error| Some(OrderAction::LoadFailed { error: error.to_string() })
/// }
/// ```
#[macro_export]
macro_rules! load_events {
    (
        store: $store:expr,
        stream: $stream:expr,
        from_version: $from:expr,
        on_success: |$success_param:ident| $success_body:expr,
        on_error: |$error_param:ident| $error_body:expr
    ) => {
        $crate::effect::Effect::EventStore(
            $crate::effect::EventStoreOperation::LoadEvents {
                event_store: ::std::sync::Arc::clone(&$store),
                stream_id: $crate::stream::StreamId::new($stream),
                from_version: $from,
                on_success: ::std::boxed::Box::new(move |$success_param| $success_body),
                on_error: ::std::boxed::Box::new(move |$error_param| $error_body),
            }
        )
    };
}

/// Create an `Effect::PublishEvent` operation
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_core::publish_event;
///
/// publish_event! {
///     bus: event_bus,
///     topic: "order-events",
///     event: serialized_event,
///     on_success: || Some(OrderAction::EventPublished),
///     on_error: |error| Some(OrderAction::PublishFailed { error: error.to_string() })
/// }
/// ```
#[macro_export]
macro_rules! publish_event {
    (
        bus: $bus:expr,
        topic: $topic:expr,
        event: $event:expr,
        on_success: || $success_body:expr,
        on_error: |$error_param:ident| $error_body:expr
    ) => {
        $crate::effect::Effect::PublishEvent(
            $crate::effect::EventBusOperation::Publish {
                event_bus: ::std::sync::Arc::clone(&$bus),
                topic: $topic.to_string(),
                event: $event,
                on_success: ::std::boxed::Box::new(move |()| $success_body),
                on_error: ::std::boxed::Box::new(move |$error_param| $error_body),
            }
        )
    };
}

/// Create an `Effect::Future` from an async block
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_core::async_effect;
///
/// async_effect! {
///     let response = http_client.get("https://api.example.com").await?;
///     Some(OrderAction::ResponseReceived { response })
/// }
/// ```
#[macro_export]
macro_rules! async_effect {
    ($($body:tt)*) => {
        $crate::effect::Effect::Future(
            ::std::boxed::Box::pin(async move { $($body)* })
        )
    };
}

/// Create an `Effect::Delay` for scheduling delayed actions
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_core::delay;
/// use std::time::Duration;
///
/// delay! {
///     duration: Duration::from_secs(30),
///     action: OrderAction::TimeoutExpired
/// }
/// ```
#[macro_export]
macro_rules! delay {
    (
        duration: $duration:expr,
        action: $action:expr
    ) => {
        $crate::effect::Effect::Delay {
            duration: $duration,
            action: ::std::boxed::Box::new($action),
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::effect::Effect;
    use std::time::Duration;

    #[derive(Clone, Debug)]
    enum TestAction {
        AsyncResult { value: i32 },
        TimeoutExpired,
    }

    #[test]
    fn test_async_effect_macro() {
        let effect = async_effect! {
            // Simulate async work
            Some(TestAction::AsyncResult { value: 42 })
        };

        assert!(matches!(effect, Effect::Future(_)));
    }

    #[test]
    fn test_delay_macro() {
        let effect = delay! {
            duration: Duration::from_secs(30),
            action: TestAction::TimeoutExpired
        };

        assert!(matches!(effect, Effect::Delay { .. }));
    }

    // Note: append_events!, load_events!, and publish_event! macros are tested
    // in integration tests where we have access to actual EventStore and EventBus
    // implementations from the testing crate.
}
