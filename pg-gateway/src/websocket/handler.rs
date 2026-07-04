//! WebSocket HTTP handler.
//!
//! This module provides the Axum handler for WebSocket connections,
//! including authentication, message processing, and event forwarding.

use super::{
    ConnectionId, Subscription, WsManager,
    protocol::{ClientMessage, ServerMessage, error_codes},
};
use crate::identity::Identity;
use axum::{
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use std::sync::Arc;

/// Application state required for WebSocket handling.
///
/// This should be included in your application state and provided
/// to the WebSocket handler.
#[derive(Clone)]
pub struct WsState {
    /// WebSocket connection manager.
    pub manager: Arc<WsManager>,
}

impl WsState {
    /// Create new WebSocket state.
    #[must_use]
    pub const fn new(manager: Arc<WsManager>) -> Self {
        Self { manager }
    }
}

/// WebSocket handler for event streaming.
///
/// This handler upgrades HTTP connections to WebSocket and manages
/// the full lifecycle of the connection including authentication,
/// subscriptions, and event forwarding.
///
/// # Authentication
///
/// Authentication is performed before the WebSocket upgrade using
/// the `Identity` extractor. The identity is then associated with
/// the WebSocket connection for tenant-based event filtering.
///
/// # Example
///
/// ```rust,ignore
/// use axum::{Router, routing::get};
/// use composable_rust_pg_gateway::websocket::{ws_handler, WsState, WsManager};
///
/// let manager = Arc::new(WsManager::new(1000));
/// let ws_state = WsState::new(manager);
///
/// let app = Router::new()
///     .route("/ws/events", get(ws_handler))
///     .with_state(app_state);
/// ```
pub async fn ws_handler(
    State(ws_state): State<WsState>,
    identity: Identity,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| handle_connection(socket, identity, ws_state.manager))
}

/// Handle a WebSocket connection after upgrade.
#[allow(clippy::cognitive_complexity)]
async fn handle_connection(socket: WebSocket, identity: Identity, manager: Arc<WsManager>) {
    let (mut sender, mut receiver) = socket.split();

    // Register connection
    let (conn_id, mut message_rx) = manager.register(identity.clone());

    tracing::info!(
        connection_id = %conn_id,
        user_id = %identity.user_id,
        tenant_id = %identity.tenant_id,
        "WebSocket connection established"
    );

    // Subscribe to broadcast events
    let mut broadcast_rx = manager.subscribe_to_broadcasts();

    // Task to forward messages to WebSocket
    let manager_for_send = manager.clone();
    let send_task = tokio::spawn(async move {
        while let Some(message) = message_rx.recv().await {
            if sender.send(Message::Text(message.into())).await.is_err() {
                break;
            }
        }
    });

    // Task to forward broadcast events to this connection
    let manager_for_broadcast = manager.clone();
    let broadcast_task = tokio::spawn(async move {
        loop {
            match broadcast_rx.recv().await {
                Ok(event) => {
                    // Check if this connection is subscribed to this event
                    if manager_for_broadcast.is_subscribed(conn_id, &event) {
                        let msg = ServerMessage::event(&event);
                        if !manager_for_broadcast.send_to(conn_id, &msg).await {
                            break;
                        }
                    }
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    tracing::warn!(
                        connection_id = %conn_id,
                        lagged_count = count,
                        "WebSocket connection lagging behind broadcasts"
                    );
                },
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                },
            }
        }
    });

    // Main loop: handle incoming messages
    while let Some(result) = receiver.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                tracing::debug!(
                    connection_id = %conn_id,
                    error = %e,
                    "WebSocket receive error"
                );
                break;
            },
        };

        match msg {
            Message::Text(text) => {
                if let Err(e) = handle_message(&manager_for_send, conn_id, &text).await {
                    tracing::debug!(
                        connection_id = %conn_id,
                        error = %e,
                        "Error handling WebSocket message"
                    );
                }
            },
            Message::Binary(_) => {
                // Binary messages not supported
                let error = ServerMessage::error(
                    error_codes::INVALID_MESSAGE,
                    "Binary messages not supported",
                );
                let _ = manager_for_send.send_to(conn_id, &error).await;
            },
            Message::Ping(data) => {
                // Axum handles ping/pong automatically, but we can respond too
                let _ = manager_for_send
                    .send_to(conn_id, &ServerMessage::pong())
                    .await;
                tracing::trace!(connection_id = %conn_id, data_len = data.len(), "Received ping");
            },
            Message::Pong(_) => {
                // Ignore pong messages
            },
            Message::Close(_) => {
                tracing::debug!(connection_id = %conn_id, "WebSocket close received");
                break;
            },
        }
    }

    // Cleanup
    manager.unregister(conn_id);
    send_task.abort();
    broadcast_task.abort();

    tracing::info!(
        connection_id = %conn_id,
        "WebSocket connection closed"
    );
}

/// Handle a client message.
async fn handle_message(
    manager: &WsManager,
    conn_id: ConnectionId,
    text: &str,
) -> Result<(), String> {
    let message: ClientMessage =
        serde_json::from_str(text).map_err(|e| format!("Invalid JSON: {e}"))?;

    match message {
        ClientMessage::Subscribe { subscriptions } => {
            handle_subscribe(manager, conn_id, subscriptions).await
        },
        ClientMessage::Unsubscribe { subscriptions } => {
            handle_unsubscribe(manager, conn_id, subscriptions).await
        },
        ClientMessage::Ping => {
            let _ = manager.send_to(conn_id, &ServerMessage::pong()).await;
            Ok(())
        },
    }
}

/// Handle subscribe command.
async fn handle_subscribe(
    manager: &WsManager,
    conn_id: ConnectionId,
    requests: Vec<super::protocol::SubscriptionRequest>,
) -> Result<(), String> {
    let mut added = Vec::new();
    let mut errors = Vec::new();

    for request in requests {
        match request.into_subscription() {
            Ok(sub) => {
                let display = sub.to_display_string();
                if manager.subscribe(conn_id, sub) {
                    added.push(display);
                }
            },
            Err(e) => {
                errors.push(e);
            },
        }
    }

    // Send subscription confirmation with ALL active subscriptions.
    // This intentionally includes pre-existing subscriptions so the client
    // knows their complete subscription state after this operation.
    if !added.is_empty() {
        let all_subscriptions = manager.get_subscription_strings(conn_id);
        let msg = ServerMessage::subscribed(all_subscriptions);
        let _ = manager.send_to(conn_id, &msg).await;
    }

    // Send errors if any
    for error in errors {
        let msg = ServerMessage::error(error_codes::INVALID_SUBSCRIPTION, error);
        let _ = manager.send_to(conn_id, &msg).await;
    }

    Ok(())
}

/// Handle unsubscribe command.
async fn handle_unsubscribe(
    manager: &WsManager,
    conn_id: ConnectionId,
    subscription_strings: Vec<String>,
) -> Result<(), String> {
    let mut removed = Vec::new();

    for sub_str in subscription_strings {
        // Parse subscription string back to Subscription
        let subscription = parse_subscription_string(&sub_str);
        if manager.unsubscribe(conn_id, &subscription) {
            removed.push(sub_str);
        }
    }

    // Send confirmation
    if !removed.is_empty() {
        let msg = ServerMessage::unsubscribed(removed);
        let _ = manager.send_to(conn_id, &msg).await;
    }

    Ok(())
}

/// Parse a subscription display string back to a Subscription.
fn parse_subscription_string(s: &str) -> Subscription {
    if let Some(stream_id) = s.strip_prefix("stream:") {
        Subscription::Stream(stream_id.to_string())
    } else if let Some((context, event_type)) = s.split_once(':') {
        Subscription::Pattern {
            context: context.to_string(),
            event_type: event_type.to_string(),
        }
    } else {
        Subscription::Context(s.to_string())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_context_subscription() {
        let sub = parse_subscription_string("sales");
        assert!(matches!(sub, Subscription::Context(c) if c == "sales"));
    }

    #[test]
    fn parse_stream_subscription() {
        let sub = parse_subscription_string("stream:order-123");
        assert!(matches!(sub, Subscription::Stream(s) if s == "order-123"));
    }

    #[test]
    fn parse_pattern_subscription() {
        let sub = parse_subscription_string("shipping:ShipmentCreated");
        assert!(matches!(
            sub,
            Subscription::Pattern { context, event_type }
            if context == "shipping" && event_type == "ShipmentCreated"
        ));
    }
}
