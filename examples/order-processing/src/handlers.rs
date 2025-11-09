//! HTTP handlers for order processing API.
//!
//! Implements request-response pattern using `send_and_wait_for()`.

use crate::types::{CustomerId, LineItem, Money, OrderAction, OrderId, OrderState, OrderStatus};
use crate::{OrderEnvironment, OrderReducer};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use composable_rust_runtime::Store;
use composable_rust_web::{AppError, CorrelationId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Request to place a new order.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaceOrderRequest {
    /// Customer placing the order.
    pub customer_id: String,

    /// Items to order.
    pub items: Vec<LineItemDto>,
}

/// Line item in the request.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LineItemDto {
    /// Product identifier.
    pub product_id: String,

    /// Product name.
    pub name: String,

    /// Quantity.
    pub quantity: u32,

    /// Price per unit in cents.
    pub unit_price_cents: i64,
}

/// Response after placing an order.
#[derive(Debug, Clone, Serialize)]
pub struct PlaceOrderResponse {
    /// Order ID.
    pub order_id: String,

    /// Order status.
    pub status: String,

    /// Total order value in cents.
    pub total_cents: i64,

    /// Placed timestamp (ISO 8601).
    pub placed_at: String,
}

/// Request to cancel an order.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CancelOrderRequest {
    /// Reason for cancellation.
    pub reason: String,
}

/// Response after cancelling an order.
#[derive(Debug, Clone, Serialize)]
pub struct CancelOrderResponse {
    /// Order ID.
    pub order_id: String,

    /// Order status.
    pub status: String,

    /// Cancellation reason.
    pub reason: String,

    /// Cancelled timestamp (ISO 8601).
    pub cancelled_at: String,
}

/// Request to ship an order.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ShipOrderRequest {
    /// Tracking number.
    pub tracking: String,
}

/// Response after shipping an order.
#[derive(Debug, Clone, Serialize)]
pub struct ShipOrderResponse {
    /// Order ID.
    pub order_id: String,

    /// Order status.
    pub status: String,

    /// Tracking number.
    pub tracking: String,

    /// Shipped timestamp (ISO 8601).
    pub shipped_at: String,
}

/// Response with order details.
#[derive(Debug, Clone, Serialize)]
pub struct GetOrderResponse {
    /// Order ID.
    pub order_id: String,

    /// Customer ID.
    pub customer_id: String,

    /// Order status.
    pub status: String,

    /// Line items.
    pub items: Vec<LineItemDto>,

    /// Total order value in cents.
    pub total_cents: i64,
}

/// Place a new order.
///
/// # Endpoint
///
/// ```text
/// POST /orders
/// Content-Type: application/json
///
/// {
///   "customer_id": "cust-123",
///   "items": [
///     {
///       "product_id": "prod-1",
///       "name": "Widget",
///       "quantity": 2,
///       "unit_price_cents": 1000
///     }
///   ]
/// }
/// ```
///
/// # Response
///
/// ```json
/// {
///   "order_id": "order-abc123",
///   "status": "Placed",
///   "total_cents": 2000,
///   "placed_at": "2024-01-01T00:00:00Z"
/// }
/// ```
pub async fn place_order(
    State(store): State<Arc<Store<OrderState, OrderAction, OrderEnvironment, OrderReducer>>>,
    _correlation_id: CorrelationId,
    Json(request): Json<PlaceOrderRequest>,
) -> Result<(StatusCode, Json<PlaceOrderResponse>), AppError> {
    // Generate order ID
    let order_id = OrderId::new(format!("order-{}", uuid::Uuid::new_v4()));

    // Convert DTOs to domain types
    let items: Vec<LineItem> = request
        .items
        .into_iter()
        .map(|item| {
            LineItem::new(
                item.product_id,
                item.name,
                item.quantity,
                Money::from_cents(item.unit_price_cents),
            )
        })
        .collect();

    // Build action
    let action = OrderAction::PlaceOrder {
        order_id: order_id.clone(),
        customer_id: CustomerId::new(request.customer_id),
        items,
    };

    // Dispatch and wait for result
    let result = store
        .send_and_wait_for(
            action,
            |a| {
                matches!(
                    a,
                    OrderAction::OrderPlaced { .. } | OrderAction::ValidationFailed { .. }
                )
            },
            Duration::from_secs(10),
        )
        .await
        .map_err(|_| AppError::timeout("Order placement timed out"))?;

    // Map result to HTTP response
    match result {
        OrderAction::OrderPlaced {
            order_id,
            total,
            timestamp,
            ..
        } => Ok((
            StatusCode::CREATED,
            Json(PlaceOrderResponse {
                order_id: order_id.as_str().to_string(),
                status: OrderStatus::Placed.to_string(),
                total_cents: total.cents(),
                placed_at: timestamp.to_rfc3339(),
            }),
        )),
        OrderAction::ValidationFailed { error } => Err(AppError::bad_request(error)),
        _ => Err(AppError::internal("Unexpected action received")),
    }
}

/// Get order details.
///
/// # Endpoint
///
/// ```text
/// GET /orders/:id
/// ```
///
/// # Response
///
/// ```json
/// {
///   "order_id": "order-abc123",
///   "customer_id": "cust-123",
///   "status": "Placed",
///   "items": [...],
///   "total_cents": 2000
/// }
/// ```
pub async fn get_order(
    State(store): State<Arc<Store<OrderState, OrderAction, OrderEnvironment, OrderReducer>>>,
    Path(order_id): Path<String>,
) -> Result<Json<GetOrderResponse>, AppError> {
    // Get current state
    let state = store.state(Clone::clone).await;

    // Check if this is the right order
    if state
        .order_id
        .as_ref()
        .map(|id| id.as_str() != order_id.as_str())
        .unwrap_or(true)
    {
        return Err(AppError::not_found("Order", &order_id));
    }

    // Convert to response
    Ok(Json(GetOrderResponse {
        order_id: state.order_id.as_ref().map(|id| id.as_str().to_string()).unwrap_or_default(),
        customer_id: state.customer_id.as_ref().map(|id| id.as_str().to_string()).unwrap_or_default(),
        status: state.status.to_string(),
        items: state
            .items
            .into_iter()
            .map(|item| LineItemDto {
                product_id: item.product_id,
                name: item.name,
                quantity: item.quantity,
                unit_price_cents: item.unit_price.cents(),
            })
            .collect(),
        total_cents: state.total.cents(),
    }))
}

/// Cancel an order.
///
/// # Endpoint
///
/// ```text
/// POST /orders/:id/cancel
/// Content-Type: application/json
///
/// {
///   "reason": "Customer requested cancellation"
/// }
/// ```
///
/// # Response
///
/// ```json
/// {
///   "order_id": "order-abc123",
///   "status": "Cancelled",
///   "reason": "Customer requested cancellation",
///   "cancelled_at": "2024-01-01T00:00:00Z"
/// }
/// ```
pub async fn cancel_order(
    State(store): State<Arc<Store<OrderState, OrderAction, OrderEnvironment, OrderReducer>>>,
    Path(order_id): Path<String>,
    Json(request): Json<CancelOrderRequest>,
) -> Result<(StatusCode, Json<CancelOrderResponse>), AppError> {
    // Build action
    let action = OrderAction::CancelOrder {
        order_id: OrderId::new(order_id),
        reason: request.reason,
    };

    // Dispatch and wait for result
    let result = store
        .send_and_wait_for(
            action,
            |a| {
                matches!(
                    a,
                    OrderAction::OrderCancelled { .. } | OrderAction::ValidationFailed { .. }
                )
            },
            Duration::from_secs(10),
        )
        .await
        .map_err(|_| AppError::timeout("Order cancellation timed out"))?;

    // Map result to HTTP response
    match result {
        OrderAction::OrderCancelled {
            order_id,
            reason,
            timestamp,
        } => Ok((
            StatusCode::OK,
            Json(CancelOrderResponse {
                order_id: order_id.as_str().to_string(),
                status: OrderStatus::Cancelled.to_string(),
                reason,
                cancelled_at: timestamp.to_rfc3339(),
            }),
        )),
        OrderAction::ValidationFailed { error } => Err(AppError::bad_request(error)),
        _ => Err(AppError::internal("Unexpected action received")),
    }
}

/// Ship an order.
///
/// # Endpoint
///
/// ```text
/// POST /orders/:id/ship
/// Content-Type: application/json
///
/// {
///   "tracking": "TRACK123456"
/// }
/// ```
///
/// # Response
///
/// ```json
/// {
///   "order_id": "order-abc123",
///   "status": "Shipped",
///   "tracking": "TRACK123456",
///   "shipped_at": "2024-01-01T00:00:00Z"
/// }
/// ```
pub async fn ship_order(
    State(store): State<Arc<Store<OrderState, OrderAction, OrderEnvironment, OrderReducer>>>,
    Path(order_id): Path<String>,
    Json(request): Json<ShipOrderRequest>,
) -> Result<(StatusCode, Json<ShipOrderResponse>), AppError> {
    // Build action
    let action = OrderAction::ShipOrder {
        order_id: OrderId::new(order_id),
        tracking: request.tracking,
    };

    // Dispatch and wait for result
    let result = store
        .send_and_wait_for(
            action,
            |a| {
                matches!(
                    a,
                    OrderAction::OrderShipped { .. } | OrderAction::ValidationFailed { .. }
                )
            },
            Duration::from_secs(10),
        )
        .await
        .map_err(|_| AppError::timeout("Order shipping timed out"))?;

    // Map result to HTTP response
    match result {
        OrderAction::OrderShipped {
            order_id,
            tracking,
            timestamp,
        } => Ok((
            StatusCode::OK,
            Json(ShipOrderResponse {
                order_id: order_id.as_str().to_string(),
                status: OrderStatus::Shipped.to_string(),
                tracking,
                shipped_at: timestamp.to_rfc3339(),
            }),
        )),
        OrderAction::ValidationFailed { error } => Err(AppError::bad_request(error)),
        _ => Err(AppError::internal("Unexpected action received")),
    }
}
