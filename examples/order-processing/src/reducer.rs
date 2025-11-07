//! Order reducer implementing business logic for order processing.
//!
//! The reducer handles commands, validates them, and produces events that are
//! persisted to the event store. Events are then replayed to reconstruct state.

use crate::types::{LineItem, Money, OrderAction, OrderId, OrderState, OrderStatus};
use composable_rust_core::append_events;
use composable_rust_core::effect::Effect;
use composable_rust_core::environment::Clock;
use composable_rust_core::event::SerializedEvent;
use composable_rust_core::event_store::EventStore;
use composable_rust_core::reducer::Reducer;
use composable_rust_core::stream::{StreamId, Version};
use composable_rust_core::{smallvec, SmallVec};
use std::sync::Arc;

/// Environment for order processing containing dependencies
#[derive(Clone)]
pub struct OrderEnvironment {
    /// Event store for persisting order events
    pub event_store: Arc<dyn EventStore>,
    /// Clock for generating timestamps
    pub clock: Arc<dyn Clock>,
}

impl OrderEnvironment {
    /// Creates a new order environment
    pub fn new(event_store: Arc<dyn EventStore>, clock: Arc<dyn Clock>) -> Self {
        Self { event_store, clock }
    }
}

/// Reducer implementing order processing business logic
///
/// This reducer follows the event sourcing pattern:
/// 1. Commands are validated against current state
/// 2. Valid commands produce events
/// 3. Events are persisted to the event store
/// 4. Events are applied to update state
///
/// # State Reconstruction
///
/// State can be reconstructed by replaying all events through `apply_event`.
#[derive(Clone)]
pub struct OrderReducer;

impl OrderReducer {
    /// Creates a new order reducer
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Applies an event to state (for event replay)
    ///
    /// This method is used to reconstruct state from persisted events.
    /// It should be deterministic and idempotent.
    pub fn apply_event(state: &mut OrderState, action: &OrderAction) {
        match action {
            OrderAction::OrderPlaced {
                order_id,
                customer_id,
                items,
                total,
                ..
            } => {
                state.order_id = Some(order_id.clone());
                state.customer_id = Some(customer_id.clone());
                state.items.clone_from(items);
                state.total = *total;
                state.status = OrderStatus::Placed;
            },
            OrderAction::OrderCancelled { .. } => {
                state.status = OrderStatus::Cancelled;
            },
            OrderAction::OrderShipped { tracking, .. } => {
                state.status = OrderStatus::Shipped;
                // Could store tracking number in state if needed
                tracing::info!("Order shipped with tracking: {tracking}");
            },
            OrderAction::ValidationFailed { error } => {
                // Track validation failure in state
                state.last_error = Some(error.clone());
            },
            // Commands and internal actions don't modify state directly
            OrderAction::PlaceOrder { .. }
            | OrderAction::CancelOrder { .. }
            | OrderAction::ShipOrder { .. }
            | OrderAction::EventPersisted { .. } => {
                // Commands and internal feedback actions are not applied during event replay
            },
        }
    }

    /// Validates a `PlaceOrder` command
    fn validate_place_order(state: &OrderState, items: &[LineItem]) -> Result<(), String> {
        if state.order_id.is_some() {
            return Err("Order already placed".to_string());
        }

        if items.is_empty() {
            return Err("Order must contain at least one item".to_string());
        }

        for item in items {
            if item.quantity == 0 {
                return Err(format!("Item '{}' has zero quantity", item.name));
            }
            if item.unit_price.cents() <= 0 {
                return Err(format!("Item '{}' has invalid price", item.name));
            }
        }

        Ok(())
    }

    /// Validates a `CancelOrder` command
    fn validate_cancel_order(state: &OrderState, order_id: &OrderId) -> Result<(), String> {
        if state.order_id.as_ref() != Some(order_id) {
            return Err("Order ID mismatch".to_string());
        }

        if !state.can_cancel() {
            return Err(format!(
                "Order in status '{}' cannot be cancelled",
                state.status
            ));
        }

        Ok(())
    }

    /// Validates a `ShipOrder` command
    fn validate_ship_order(
        state: &OrderState,
        order_id: &OrderId,
        tracking: &str,
    ) -> Result<(), String> {
        if state.order_id.as_ref() != Some(order_id) {
            return Err("Order ID mismatch".to_string());
        }

        if !state.can_ship() {
            return Err(format!(
                "Order in status '{}' cannot be shipped",
                state.status
            ));
        }

        if tracking.trim().is_empty() {
            return Err("Tracking number cannot be empty".to_string());
        }

        Ok(())
    }

    /// Calculates total from items
    fn calculate_total(items: &[LineItem]) -> Money {
        let total_cents: i64 = items.iter().map(|item| item.total().cents()).sum();
        Money::from_cents(total_cents)
    }

    /// Serializes an action (event) to bytes using bincode
    fn serialize_event(action: &OrderAction) -> Result<SerializedEvent, String> {
        let event_type = action.event_type().to_string();
        let data =
            bincode::serialize(action).map_err(|e| format!("Failed to serialize event: {e}"))?;

        Ok(SerializedEvent::new(event_type, data, None))
    }

    /// Creates an `EventStore` effect to append events
    fn create_append_effect(
        event_store: Arc<dyn EventStore>,
        stream_id: StreamId,
        expected_version: Option<Version>,
        event: OrderAction,
    ) -> Effect<OrderAction> {
        // Serialize the event
        let serialized_event = match Self::serialize_event(&event) {
            Ok(e) => e,
            Err(error) => {
                tracing::error!("Failed to serialize event: {error}");
                return Effect::None;
            },
        };

        append_events! {
            store: event_store,
            stream: stream_id.as_str(),
            expected_version: expected_version,
            events: vec![serialized_event],
            on_success: |version| Some(OrderAction::EventPersisted {
                event: Box::new(event.clone()),
                version: version.value(),
            }),
            on_error: |error| Some(OrderAction::ValidationFailed {
                error: error.to_string(),
            })
        }
    }
}

impl Default for OrderReducer {
    fn default() -> Self {
        Self::new()
    }
}

impl Reducer for OrderReducer {
    type State = OrderState;
    type Action = OrderAction;
    type Environment = OrderEnvironment;

    #[allow(clippy::cognitive_complexity)] // Large match statement is appropriate for reducer
    #[allow(clippy::too_many_lines)] // Comprehensive reducer logic for demonstration
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ========== Commands ==========
            OrderAction::PlaceOrder {
                order_id,
                customer_id,
                items,
            } => {
                // Validate command
                if let Err(error) = Self::validate_place_order(state, &items) {
                    tracing::warn!("PlaceOrder validation failed: {error}");
                    // Apply validation failure to state so it's observable
                    Self::apply_event(
                        state,
                        &OrderAction::ValidationFailed {
                            error: error.clone(),
                        },
                    );
                    return smallvec![Effect::None];
                }

                // Calculate total
                let total = Self::calculate_total(&items);

                // Create event
                let event = OrderAction::OrderPlaced {
                    order_id: order_id.clone(),
                    customer_id: customer_id.clone(),
                    items: items.clone(),
                    total,
                    timestamp: env.clock.now(),
                };

                // Create stream ID from order ID
                let stream_id = StreamId::new(format!("order-{}", order_id.as_str()));

                // For new orders, expected_version is None
                // For existing orders (shouldn't happen due to validation), use current version
                let expected_version = state.version;

                smallvec![Self::create_append_effect(
                    Arc::clone(&env.event_store),
                    stream_id,
                    expected_version,
                    event,
                )]
            },

            OrderAction::CancelOrder { order_id, reason } => {
                // Validate command
                if let Err(error) = Self::validate_cancel_order(state, &order_id) {
                    tracing::warn!("CancelOrder validation failed: {error}");
                    // Apply validation failure to state so it's observable
                    Self::apply_event(
                        state,
                        &OrderAction::ValidationFailed {
                            error: error.clone(),
                        },
                    );
                    return smallvec![Effect::None];
                }

                // Create event
                let event = OrderAction::OrderCancelled {
                    order_id: order_id.clone(),
                    reason: reason.clone(),
                    timestamp: env.clock.now(),
                };

                // Create stream ID
                let stream_id = StreamId::new(format!("order-{}", order_id.as_str()));

                // Use current version from state for optimistic concurrency
                let expected_version = state.version;

                smallvec![Self::create_append_effect(
                    Arc::clone(&env.event_store),
                    stream_id,
                    expected_version,
                    event,
                )]
            },

            OrderAction::ShipOrder { order_id, tracking } => {
                // Validate command
                if let Err(error) = Self::validate_ship_order(state, &order_id, &tracking) {
                    tracing::warn!("ShipOrder validation failed: {error}");
                    // Apply validation failure to state so it's observable
                    Self::apply_event(
                        state,
                        &OrderAction::ValidationFailed {
                            error: error.clone(),
                        },
                    );
                    return smallvec![Effect::None];
                }

                // Create event
                let event = OrderAction::OrderShipped {
                    order_id: order_id.clone(),
                    tracking: tracking.clone(),
                    timestamp: env.clock.now(),
                };

                // Create stream ID
                let stream_id = StreamId::new(format!("order-{}", order_id.as_str()));

                // Use current version from state for optimistic concurrency
                let expected_version = state.version;

                smallvec![Self::create_append_effect(
                    Arc::clone(&env.event_store),
                    stream_id,
                    expected_version,
                    event,
                )]
            },

            // ========== Events ==========
            OrderAction::OrderPlaced { .. }
            | OrderAction::OrderCancelled { .. }
            | OrderAction::OrderShipped { .. } => {
                // Apply event to state
                Self::apply_event(state, &action);

                // Track version during event replay
                // During normal command processing, version is updated via EventPersisted
                // During replay, events arrive directly and we must track version here
                state.version = match state.version {
                    None => Some(Version::new(1)),
                    Some(v) => Some(v.next()),
                };

                smallvec![Effect::None]
            },

            OrderAction::EventPersisted { event, version } => {
                // Apply the persisted event to state
                Self::apply_event(state, &event);
                // Update version to match the last event appended
                // This must match the replay logic where version = 1 for first event, 2 for second, etc.
                state.version = Some(Version::new(version));
                smallvec![Effect::None]
            },

            OrderAction::ValidationFailed { error } => {
                // ValidationFailed can come from either:
                // 1. Command validation failures (already applied to state in reduce())
                // 2. Event store operation failures (from effect callbacks)
                // In both cases, we've already logged and applied to state, so just continue
                tracing::debug!("ValidationFailed processed: {error}");
                smallvec![Effect::None]
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CustomerId;
    use chrono::Utc;

    fn create_test_item() -> LineItem {
        LineItem::new(
            "prod-1".to_string(),
            "Test Widget".to_string(),
            2,
            Money::from_dollars(10),
        )
    }

    #[test]
    fn apply_event_order_placed() {
        let mut state = OrderState::new();
        let order_id = OrderId::new("order-123".to_string());
        let customer_id = CustomerId::new("cust-456".to_string());
        let items = vec![create_test_item()];
        let total = Money::from_dollars(20);

        let event = OrderAction::OrderPlaced {
            order_id: order_id.clone(),
            customer_id: customer_id.clone(),
            items: items.clone(),
            total,
            timestamp: Utc::now(),
        };

        OrderReducer::apply_event(&mut state, &event);

        assert_eq!(state.order_id, Some(order_id));
        assert_eq!(state.customer_id, Some(customer_id));
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.total, total);
        assert_eq!(state.status, OrderStatus::Placed);
    }

    #[test]
    fn apply_event_order_cancelled() {
        let mut state = OrderState::new();
        state.status = OrderStatus::Placed;

        let event = OrderAction::OrderCancelled {
            order_id: OrderId::new("order-123".to_string()),
            reason: "Customer request".to_string(),
            timestamp: Utc::now(),
        };

        OrderReducer::apply_event(&mut state, &event);

        assert_eq!(state.status, OrderStatus::Cancelled);
    }

    #[test]
    #[allow(clippy::unwrap_used)] // Test verified result is Err above
    fn validate_place_order_empty_items() {
        let state = OrderState::new();
        let result = OrderReducer::validate_place_order(&state, &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at least one item"));
    }

    #[test]
    #[allow(clippy::unwrap_used)] // Test verified result is Err above
    fn validate_place_order_already_placed() {
        let mut state = OrderState::new();
        state.order_id = Some(OrderId::new("order-123".to_string()));

        let items = vec![create_test_item()];
        let result = OrderReducer::validate_place_order(&state, &items);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already placed"));
    }

    #[test]
    fn validate_cancel_order_not_placed() {
        let state = OrderState::new(); // Draft status
        let order_id = OrderId::new("order-123".to_string());

        let result = OrderReducer::validate_cancel_order(&state, &order_id);
        assert!(result.is_err());
    }

    #[test]
    #[allow(clippy::unwrap_used)] // Test verified result is Err and order_id is Some above
    fn validate_ship_order_empty_tracking() {
        let mut state = OrderState::new();
        state.order_id = Some(OrderId::new("order-123".to_string()));
        state.status = OrderStatus::Placed;

        let result =
            OrderReducer::validate_ship_order(&state, state.order_id.as_ref().unwrap(), "");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Tracking number"));
    }

    #[test]
    fn calculate_total_multiple_items() {
        let items = vec![
            LineItem::new(
                "prod-1".to_string(),
                "Widget A".to_string(),
                2,
                Money::from_dollars(10),
            ),
            LineItem::new(
                "prod-2".to_string(),
                "Widget B".to_string(),
                1,
                Money::from_dollars(15),
            ),
        ];

        let total = OrderReducer::calculate_total(&items);
        assert_eq!(total, Money::from_dollars(35)); // (2 * $10) + (1 * $15) = $35
    }

    #[test]
    fn test_event_replay_version_tracking() {
        use crate::types::CustomerId;
        use composable_rust_testing::mocks::FixedClock;
        use std::sync::Arc;

        // Create test environment
        let event_store: Arc<dyn composable_rust_core::event_store::EventStore> =
            Arc::new(composable_rust_testing::mocks::InMemoryEventStore::new());
        let clock: Arc<dyn composable_rust_core::environment::Clock> =
            Arc::new(FixedClock::new(Utc::now()));
        let env = OrderEnvironment::new(event_store, clock);

        let mut state = OrderState::new();
        let reducer = OrderReducer::new();

        // Initially version is None
        assert_eq!(state.version, None);

        // Replay first event
        let event1 = OrderAction::OrderPlaced {
            order_id: OrderId::new("order-1".to_string()),
            customer_id: CustomerId::new("cust-1".to_string()),
            items: vec![create_test_item()],
            total: Money::from_dollars(20),
            timestamp: Utc::now(),
        };

        reducer.reduce(&mut state, event1, &env);
        assert_eq!(
            state.version,
            Some(Version::new(1)),
            "Version should be 1 after first event"
        );

        // Replay second event
        let event2 = OrderAction::OrderShipped {
            order_id: OrderId::new("order-1".to_string()),
            tracking: "TRACK123".to_string(),
            timestamp: Utc::now(),
        };

        reducer.reduce(&mut state, event2, &env);
        assert_eq!(
            state.version,
            Some(Version::new(2)),
            "Version should be 2 after second event"
        );
    }
}
