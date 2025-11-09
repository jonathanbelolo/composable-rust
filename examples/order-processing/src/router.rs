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
use std::sync::Arc;

/// Create order processing router with all endpoints.
///
/// # Routes
///
/// - `POST /orders` - Place a new order
/// - `GET /orders/:id` - Get order details
/// - `POST /orders/:id/cancel` - Cancel an order
/// - `POST /orders/:id/ship` - Ship an order
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
        .route("/orders", post(handlers::place_order))
        .route("/orders/:id", get(handlers::get_order))
        .route("/orders/:id/cancel", post(handlers::cancel_order))
        .route("/orders/:id/ship", post(handlers::ship_order))
        .with_state(store)
}
