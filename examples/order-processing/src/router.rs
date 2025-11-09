//! Order processing HTTP router.
//!
//! Composes all order handlers into a single Axum router.

use crate::handlers;
use crate::types::{OrderAction, OrderState};
use crate::{OrderEnvironment, OrderReducer};
use axum::{
    routing::{get, post},
    Router,
};
use composable_rust_runtime::Store;
use composable_rust_web::handlers::websocket;
use std::sync::Arc;

/// Create order processing router with all endpoints.
///
/// # Routes
///
/// ## HTTP Endpoints
/// - `POST /orders` - Place a new order
/// - `GET /orders/:id` - Get order details
/// - `POST /orders/:id/cancel` - Cancel an order
/// - `POST /orders/:id/ship` - Ship an order
///
/// ## WebSocket
/// - `GET /ws` - Real-time order events (upgrade to WebSocket)
///
/// # WebSocket Protocol
///
/// **Client → Server (Commands):**
/// ```json
/// {
///   "type": "command",
///   "action": {
///     "PlaceOrder": {
///       "customer_id": "cust-123",
///       "items": [...]
///     }
///   }
/// }
/// ```
///
/// **Server → Client (Events):**
/// ```json
/// {
///   "type": "event",
///   "action": {
///     "OrderPlaced": {
///       "order_id": "ord-456",
///       "status": "pending"
///     }
///   }
/// }
/// ```
///
/// # Example
///
/// ```rust,ignore
/// let store = Arc::new(Store::new(
///     OrderState::new(),
///     OrderReducer::new(),
///     environment,
/// ));
///
/// let app = Router::new()
///     .nest("/api/v1", order_router(store))
///     .layer(TraceLayer::new_for_http());
/// ```
pub fn order_router(
    store: Arc<Store<OrderState, OrderAction, OrderEnvironment, OrderReducer>>,
) -> Router {
    Router::new()
        // HTTP endpoints
        .route("/orders", post(handlers::place_order))
        .route("/orders/:id", get(handlers::get_order))
        .route("/orders/:id/cancel", post(handlers::cancel_order))
        .route("/orders/:id/ship", post(handlers::ship_order))
        // WebSocket endpoint for real-time events
        .route(
            "/ws",
            get(websocket::handle::<OrderState, OrderAction, OrderEnvironment, OrderReducer>),
        )
        .with_state(store)
}
