//! Core domain types for Order Processing example.
//!
//! This module defines the domain model for an order processing system using
//! event sourcing. Orders progress through states: Draft → Placed → (Cancelled|Shipped)

use chrono::{DateTime, Utc};
use composable_rust_core::event::SerializedEvent;
use composable_rust_core::stream::Version;
use composable_rust_macros::{Action, State};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique identifier for an order
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrderId(String);

impl OrderId {
    /// Creates a new `OrderId` from a string
    #[must_use]
    pub const fn new(id: String) -> Self {
        Self(id)
    }

    /// Returns the inner string value
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OrderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a customer
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CustomerId(String);

impl CustomerId {
    /// Creates a new `CustomerId` from a string
    #[must_use]
    pub const fn new(id: String) -> Self {
        Self(id)
    }

    /// Returns the inner string value
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CustomerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A single line item in an order
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineItem {
    /// Product identifier
    pub product_id: String,
    /// Product name for display
    pub name: String,
    /// Quantity ordered
    pub quantity: u32,
    /// Price per unit in cents
    pub unit_price: Money,
}

impl LineItem {
    /// Creates a new line item
    #[must_use]
    pub const fn new(product_id: String, name: String, quantity: u32, unit_price: Money) -> Self {
        Self {
            product_id,
            name,
            quantity,
            unit_price,
        }
    }

    /// Calculates the total price for this line item
    #[must_use]
    pub const fn total(&self) -> Money {
        Money(self.unit_price.0 * self.quantity as i64)
    }
}

/// Money amount in cents (to avoid floating point issues)
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Money(i64);

impl Money {
    /// Creates a new money amount from cents
    #[must_use]
    pub const fn from_cents(cents: i64) -> Self {
        Self(cents)
    }

    /// Creates a new money amount from dollars (converted to cents)
    #[must_use]
    pub const fn from_dollars(dollars: i64) -> Self {
        Self(dollars * 100)
    }

    /// Returns the value in cents
    #[must_use]
    pub const fn cents(&self) -> i64 {
        self.0
    }

    /// Returns the value in dollars (as floating point)
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // i64 to f64 precision loss is acceptable for display
    pub fn dollars(&self) -> f64 {
        self.0 as f64 / 100.0
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${:.2}", self.dollars())
    }
}

/// Status of an order in its lifecycle
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    /// Order is being created (not yet placed)
    Draft,
    /// Order has been placed and is awaiting fulfillment
    Placed,
    /// Order was cancelled before shipping
    Cancelled,
    /// Order has been shipped to customer
    Shipped,
}

impl fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Draft => write!(f, "Draft"),
            Self::Placed => write!(f, "Placed"),
            Self::Cancelled => write!(f, "Cancelled"),
            Self::Shipped => write!(f, "Shipped"),
        }
    }
}

/// State of an order aggregate
///
/// This represents the current state of an order, which is derived from
/// replaying all events in the order's event stream.
#[derive(State, Clone, Debug, Serialize, Deserialize)]
pub struct OrderState {
    /// Order identifier (None for new orders)
    pub order_id: Option<OrderId>,
    /// Customer who placed the order
    pub customer_id: Option<CustomerId>,
    /// Line items in the order
    pub items: Vec<LineItem>,
    /// Current order status
    pub status: OrderStatus,
    /// Total order value in cents
    pub total: Money,
    /// Current version in the event stream (None for new orders)
    #[version]
    pub version: Option<Version>,
    /// Last validation error (if any)
    pub last_error: Option<String>,
}

impl OrderState {
    /// Creates a new empty order state
    #[must_use]
    pub const fn new() -> Self {
        Self {
            order_id: None,
            customer_id: None,
            items: Vec::new(),
            status: OrderStatus::Draft,
            total: Money::from_cents(0),
            version: None,
            last_error: None,
        }
    }

    /// Checks if the order can be cancelled
    ///
    /// Only placed orders can be cancelled
    #[must_use]
    pub const fn can_cancel(&self) -> bool {
        matches!(self.status, OrderStatus::Placed)
    }

    /// Checks if the order can be shipped
    ///
    /// Only placed orders can be shipped
    #[must_use]
    pub const fn can_ship(&self) -> bool {
        matches!(self.status, OrderStatus::Placed)
    }
}

impl Default for OrderState {
    fn default() -> Self {
        Self::new()
    }
}

/// Actions representing commands and events for orders
///
/// This enum combines both commands (intent to do something) and events
/// (something that happened). Commands are validated by the reducer and
/// produce events. Events are persisted to the event store and used to
/// reconstruct state.
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum OrderAction {
    // ========== Commands ==========
    /// Command: Place a new order
    #[command]
    PlaceOrder {
        /// Unique order identifier
        order_id: OrderId,
        /// Customer placing the order
        customer_id: CustomerId,
        /// Items to order
        items: Vec<LineItem>,
    },

    /// Command: Cancel an existing order
    #[command]
    CancelOrder {
        /// Order to cancel
        order_id: OrderId,
        /// Reason for cancellation
        reason: String,
    },

    /// Command: Ship an order
    #[command]
    ShipOrder {
        /// Order to ship
        order_id: OrderId,
        /// Tracking number
        tracking: String,
    },

    // ========== Events ==========
    /// Event: Order was successfully placed
    #[event]
    OrderPlaced {
        /// Order identifier
        order_id: OrderId,
        /// Customer who placed the order
        customer_id: CustomerId,
        /// Items in the order
        items: Vec<LineItem>,
        /// Total order value
        total: Money,
        /// When the order was placed
        timestamp: DateTime<Utc>,
    },

    /// Event: Order was cancelled
    #[event]
    OrderCancelled {
        /// Order identifier
        order_id: OrderId,
        /// Reason for cancellation
        reason: String,
        /// When the order was cancelled
        timestamp: DateTime<Utc>,
    },

    /// Event: Order was shipped
    #[event]
    OrderShipped {
        /// Order identifier
        order_id: OrderId,
        /// Tracking number
        tracking: String,
        /// When the order was shipped
        timestamp: DateTime<Utc>,
    },

    /// Event: Command validation failed
    #[event]
    ValidationFailed {
        /// Error message
        error: String,
    },

    /// Internal: Event was successfully persisted to event store
    ///
    /// This action is produced by the `EventStore` effect callback and carries
    /// both the event that was persisted and the resulting version.
    EventPersisted {
        /// The event that was successfully persisted
        event: Box<OrderAction>,
        /// The version after persisting (used to update state.version)
        version: u64,
    },
}

impl OrderAction {
    /// Deserialize an event from a serialized event
    ///
    /// This is used during event replay to reconstruct aggregate state from
    /// persisted events.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The event data cannot be deserialized from bincode format
    /// - The event type doesn't match an expected format
    ///
    /// # Example
    ///
    /// ```ignore
    /// let serialized = SerializedEvent::new(
    ///     "OrderPlaced.v1".to_string(),
    ///     bincode::serialize(&event)?,
    ///     None,
    /// );
    ///
    /// let event = OrderAction::from_serialized(&serialized)?;
    /// ```
    pub fn from_serialized(serialized: &SerializedEvent) -> Result<Self, String> {
        bincode::deserialize(&serialized.data)
            .map_err(|e| format!("Failed to deserialize event: {e}"))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // Test code can use unwrap/expect
mod tests {
    use super::*;

    #[test]
    fn money_from_cents() {
        let m = Money::from_cents(1234);
        assert_eq!(m.cents(), 1234);
        assert!((m.dollars() - 12.34).abs() < 0.01);
    }

    #[test]
    fn money_from_dollars() {
        let m = Money::from_dollars(12);
        assert_eq!(m.cents(), 1200);
        assert!((m.dollars() - 12.0).abs() < 0.01);
    }

    #[test]
    fn line_item_total() {
        let item = LineItem::new(
            "prod-1".to_string(),
            "Widget".to_string(),
            3,
            Money::from_dollars(10),
        );
        assert_eq!(item.total(), Money::from_dollars(30));
    }

    #[test]
    fn order_state_can_cancel() {
        let mut state = OrderState::new();
        assert!(!state.can_cancel()); // Draft cannot be cancelled

        state.status = OrderStatus::Placed;
        assert!(state.can_cancel()); // Placed can be cancelled

        state.status = OrderStatus::Shipped;
        assert!(!state.can_cancel()); // Shipped cannot be cancelled
    }

    #[test]
    fn order_state_can_ship() {
        let mut state = OrderState::new();
        assert!(!state.can_ship()); // Draft cannot be shipped

        state.status = OrderStatus::Placed;
        assert!(state.can_ship()); // Placed can be shipped

        state.status = OrderStatus::Cancelled;
        assert!(!state.can_ship()); // Cancelled cannot be shipped
    }

    #[test]
    fn order_action_event_type() {
        let action = OrderAction::OrderPlaced {
            order_id: OrderId::new("order-1".to_string()),
            customer_id: CustomerId::new("cust-1".to_string()),
            items: vec![],
            total: Money::from_cents(0),
            timestamp: Utc::now(),
        };
        assert_eq!(action.event_type(), "OrderPlaced.v1");
    }

    #[test]
    fn order_action_is_event() {
        let placed = OrderAction::OrderPlaced {
            order_id: OrderId::new("order-1".to_string()),
            customer_id: CustomerId::new("cust-1".to_string()),
            items: vec![],
            total: Money::from_cents(0),
            timestamp: Utc::now(),
        };
        assert!(placed.is_event());

        let place_command = OrderAction::PlaceOrder {
            order_id: OrderId::new("order-1".to_string()),
            customer_id: CustomerId::new("cust-1".to_string()),
            items: vec![],
        };
        assert!(!place_command.is_event());
    }

    #[test]
    #[allow(clippy::expect_used)] // Test code
    fn event_serialization_roundtrip() {
        let original = OrderAction::OrderPlaced {
            order_id: OrderId::new("order-123".to_string()),
            customer_id: CustomerId::new("cust-456".to_string()),
            items: vec![LineItem::new(
                "prod-1".to_string(),
                "Widget".to_string(),
                2,
                Money::from_dollars(10),
            )],
            total: Money::from_dollars(20),
            timestamp: Utc::now(),
        };

        // Serialize
        let event_type = original.event_type().to_string();
        let data = bincode::serialize(&original).expect("Failed to serialize");
        let serialized = SerializedEvent::new(event_type.clone(), data, None);

        // Deserialize
        let deserialized =
            OrderAction::from_serialized(&serialized).expect("Failed to deserialize");

        // Verify event type matches
        assert_eq!(original.event_type(), deserialized.event_type());
        assert_eq!(serialized.event_type, deserialized.event_type());

        // Verify it's an event
        assert!(deserialized.is_event());
    }
}
