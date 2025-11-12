//! WebSocket handler for real-time bidirectional communication.
//!
//! This module provides WebSocket support for:
//! - **Commands**: Receiving actions from clients
//! - **Events**: Streaming action broadcasts to clients
//!
//! # Architecture
//!
//! ```text
//! Client          WebSocket Handler          Store
//!   │                    │                     │
//!   ├─ Connect ─────────>│                     │
//!   │                    ├─ subscribe() ──────>│
//!   │                    │                     │
//!   ├─ Send Command ────>│                     │
//!   │                    ├─ dispatch() ───────>│
//!   │                    │                     │
//!   │                    │<── broadcast ───────┤
//!   │<─ Receive Event ───┤                     │
//! ```
//!
//! # Message Protocol
//!
//! **Client → Server (Command):**
//! ```json
//! {
//!   "type": "command",
//!   "action": { ... }
//! }
//! ```
//!
//! **Server → Client (Event):**
//! ```json
//! {
//!   "type": "event",
//!   "action": { ... }
//! }
//! ```
//!
//! **Server → Client (Error):**
//! ```json
//! {
//!   "type": "error",
//!   "message": "Invalid action format"
//! }
//! ```

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::Response,
};
use composable_rust_core::reducer::Reducer;
use composable_rust_runtime::Store;
use futures::{stream::StreamExt, SinkExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// WebSocket message envelope for client-server communication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum WsMessage<A> {
    /// Command from client (action to dispatch)
    Command {
        /// The action to dispatch
        action: A,
    },
    /// Event from server (action broadcast)
    Event {
        /// The broadcasted action
        action: A,
    },
    /// Error message
    Error {
        /// Error description
        message: String,
    },
    /// Ping message (keep-alive)
    Ping,
    /// Pong response
    Pong,
}

/// WebSocket handler for a composable-rust Store.
///
/// This handler enables real-time bidirectional communication:
/// 1. Receives commands from client → dispatches to Store
/// 2. Subscribes to Store broadcasts → streams events to client
///
/// # Type Parameters
///
/// - `S`: State type
/// - `A`: Action type (must be Serialize + Deserialize + Clone + Send)
/// - `E`: Environment type
/// - `R`: Reducer type
///
/// # Example
///
/// ```ignore
/// use composable_rust_web::handlers::websocket;
/// use axum::{Router, routing::get};
///
/// let app = Router::new()
///     .route("/ws", get(websocket::handle::<OrderState, OrderAction, _, _>))
///     .with_state(store);
/// ```
#[allow(clippy::unused_async)] // Axum handler signature requires async
pub async fn handle<S, A, E, R>(
    ws: WebSocketUpgrade,
    State(store): State<Arc<Store<S, A, E, R>>>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    A: Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + std::fmt::Debug + 'static,
    E: Clone + Send + Sync + 'static,
    R: Reducer<State = S, Action = A, Environment = E> + Clone + Send + Sync + 'static,
{
    info!("WebSocket connection requested");
    ws.on_upgrade(move |socket| handle_socket(socket, store))
}

/// Handle WebSocket connection lifecycle.
///
/// Spawns two concurrent tasks:
/// 1. **Receiver**: Process incoming messages from client
/// 2. **Sender**: Stream action broadcasts to client
#[allow(clippy::cognitive_complexity)] // WebSocket handler with multiple message types
async fn handle_socket<S, A, E, R>(socket: WebSocket, store: Arc<Store<S, A, E, R>>)
where
    S: Clone + Send + Sync + 'static,
    A: Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + std::fmt::Debug + 'static,
    E: Clone + Send + Sync + 'static,
    R: Reducer<State = S, Action = A, Environment = E> + Clone + Send + Sync + 'static,
{
    info!("WebSocket connection established");

    // Split socket into sender and receiver
    let (mut sender, mut receiver) = socket.split();

    // Subscribe to action broadcasts from store
    let mut action_rx = store.subscribe_actions();

    // Spawn task to send action broadcasts to client
    let mut send_task = tokio::spawn(async move {
        while let Ok(action) = action_rx.recv().await {
            // Serialize action to JSON
            let message = match serde_json::to_string(&WsMessage::Event { action }) {
                Ok(json) => Message::Text(json),
                Err(e) => {
                    error!(error = %e, "Failed to serialize action");
                    continue;
                }
            };

            // Send to client
            if sender.send(message).await.is_err() {
                // Client disconnected
                break;
            }
        }

        debug!("WebSocket send task terminated");
    });

    // Spawn task to receive commands from client
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    // Parse command message
                    match serde_json::from_str::<WsMessage<A>>(&text) {
                        Ok(WsMessage::Command { action }) => {
                            debug!("Received command from client");
                            // Dispatch action to store
                            if let Err(e) = store.send(action).await {
                                error!(error = %e, "Failed to dispatch action");
                            }
                        }
                        Ok(WsMessage::Ping) => {
                            debug!("Received ping from client");
                            // Pong responses handled by Axum automatically
                        }
                        Ok(msg) => {
                            warn!(?msg, "Unexpected message type from client");
                        }
                        Err(e) => {
                            error!(error = %e, "Failed to parse WebSocket message");
                            // Note: Can't send error back since we split the socket
                        }
                    }
                }
                Message::Binary(_) => {
                    warn!("Received unexpected binary message");
                }
                Message::Ping(_) => {
                    debug!("Received ping");
                    // Axum handles pong automatically
                }
                Message::Pong(_) => {
                    debug!("Received pong");
                }
                Message::Close(_) => {
                    info!("Client requested close");
                    break;
                }
            }
        }

        debug!("WebSocket receive task terminated");
    });

    // Wait for either task to complete (connection closed)
    tokio::select! {
        _ = (&mut send_task) => {
            debug!("Send task completed, aborting receive task");
            recv_task.abort();
        },
        _ = (&mut recv_task) => {
            debug!("Receive task completed, aborting send task");
            send_task.abort();
        },
    }

    info!("WebSocket connection closed");
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // Test code can use unwrap/expect
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::similar_names)] // ping and pong are standard WebSocket terms
    fn test_ws_message_serialization() {
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
        enum TestAction {
            Increment,
            Decrement,
        }

        // Test Command serialization
        let cmd = WsMessage::Command {
            action: TestAction::Increment,
        };
        let json = serde_json::to_string(&cmd).expect("Serialize");
        assert_eq!(json, r#"{"type":"command","action":"Increment"}"#);

        // Test Command deserialization
        let parsed: WsMessage<TestAction> = serde_json::from_str(&json).expect("Deserialize");
        assert!(matches!(
            parsed,
            WsMessage::Command {
                action: TestAction::Increment
            }
        ));

        // Test Event serialization
        let event = WsMessage::Event {
            action: TestAction::Decrement,
        };
        let json = serde_json::to_string(&event).expect("Serialize");
        assert_eq!(json, r#"{"type":"event","action":"Decrement"}"#);

        // Test Error serialization
        let error = WsMessage::<TestAction>::Error {
            message: "Test error".to_string(),
        };
        let json = serde_json::to_string(&error).expect("Serialize");
        assert_eq!(json, r#"{"type":"error","message":"Test error"}"#);

        // Test Ping/Pong
        let ping = WsMessage::<TestAction>::Ping;
        let json = serde_json::to_string(&ping).expect("Serialize");
        assert_eq!(json, r#"{"type":"ping"}"#);

        let pong = WsMessage::<TestAction>::Pong;
        let json = serde_json::to_string(&pong).expect("Serialize");
        assert_eq!(json, r#"{"type":"pong"}"#);
    }
}
