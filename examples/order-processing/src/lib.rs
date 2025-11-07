//! Order Processing example demonstrating event sourcing with `EventStore`.
//!
//! This example shows how to build an event-sourced order processing system
//! using the Composable Rust architecture. It demonstrates:
//!
//! - Command/Event pattern with validation
//! - Event persistence to `EventStore`
//! - State reconstruction from events
//! - Business logic in reducers
//!
//! # Architecture
//!
//! The order processing system follows these patterns:
//!
//! 1. **Commands** (`PlaceOrder`, `CancelOrder`, `ShipOrder`) represent intent
//! 2. **Validation** checks business rules before producing events
//! 3. **Events** (`OrderPlaced`, `OrderCancelled`, `OrderShipped`) record what happened
//! 4. **Event Store** persists events immutably
//! 5. **State Reconstruction** replays events to rebuild aggregate state
//!
//! # Example Usage
//!
//! ```no_run
//! use order_processing::{OrderAction, OrderEnvironment, OrderReducer, OrderState};
//! use order_processing::types::{CustomerId, LineItem, Money, OrderId};
//! use composable_rust_runtime::Store;
//! use composable_rust_testing::mocks::InMemoryEventStore;
//! use composable_rust_core::event_store::EventStore;
//! use composable_rust_core::environment::{Clock, SystemClock};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Set up environment with event store and clock
//! let event_store: Arc<dyn EventStore> = Arc::new(InMemoryEventStore::new());
//! let clock: Arc<dyn Clock> = Arc::new(SystemClock);
//! let env = OrderEnvironment::new(event_store, clock);
//!
//! // Create store with order reducer
//! let store = Store::new(
//!     OrderState::new(),
//!     OrderReducer::new(),
//!     env,
//! );
//!
//! // Place an order
//! let mut handle = store.send(OrderAction::PlaceOrder {
//!     order_id: OrderId::new("order-123".to_string()),
//!     customer_id: CustomerId::new("cust-456".to_string()),
//!     items: vec![
//!         LineItem::new(
//!             "prod-1".to_string(),
//!             "Widget".to_string(),
//!             2,
//!             Money::from_dollars(10),
//!         )
//!     ],
//! }).await?;
//!
//! // Wait for effects to complete
//! handle.wait().await;
//!
//! // Read final state
//! let state = store.state(|s| s.clone()).await;
//! # Ok(())
//! # }
//! ```

pub mod reducer;
pub mod types;

// Re-export commonly used types
pub use reducer::{OrderEnvironment, OrderReducer};
pub use types::{CustomerId, LineItem, Money, OrderAction, OrderId, OrderState, OrderStatus};
