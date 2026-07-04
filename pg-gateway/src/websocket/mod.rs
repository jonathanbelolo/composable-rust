//! WebSocket support for real-time event streaming.
//!
//! This module provides WebSocket infrastructure for streaming `PostgreSQL`
//! events to connected clients in real-time using `pg_notify`.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────────┐
//! │                         WebSocket Architecture                            │
//! │                                                                           │
//! │  ┌─────────────┐     ┌─────────────────┐     ┌────────────────────────┐  │
//! │  │   Client    │────▶│   WsManager     │◀────│  pg_notify_listener    │  │
//! │  │  (Browser)  │◀────│                 │     │                        │  │
//! │  └─────────────┘     │  connections:   │     │  LISTEN channels       │  │
//! │                      │  DashMap<Id,Ws> │     │  broadcast events      │  │
//! │  ┌─────────────┐     │                 │     └────────────────────────┘  │
//! │  │   Client    │────▶│  broadcast_tx   │                                 │
//! │  │  (Mobile)   │◀────│                 │                                 │
//! │  └─────────────┘     └─────────────────┘                                 │
//! │                                                                           │
//! └──────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use composable_rust_pg_gateway::websocket::{
//!     WsManager, pg_notify_listener, ws_handler,
//! };
//! use axum::{Router, routing::get};
//! use std::sync::Arc;
//!
//! // Create the WebSocket manager
//! let ws_manager = Arc::new(WsManager::new(1000));
//!
//! // Spawn the pg_notify listener
//! let config = ListenerConfig::new()
//!     .with_channel("sales_events")
//!     .with_channel("inventory_events");
//!
//! let listener_handle = tokio::spawn(pg_notify_listener(
//!     pool.clone(),
//!     ws_manager.clone(),
//!     config,
//! ));
//!
//! // Add WebSocket route
//! let app = Router::new()
//!     .route("/ws/events", get(ws_handler))
//!     .with_state(app_state);
//! ```
//!
//! # Protocol
//!
//! ## Client → Server Messages
//!
//! ```json
//! // Subscribe to events
//! {
//!     "type": "subscribe",
//!     "subscriptions": [
//!         { "context": "sales" },
//!         { "stream": "order-123" },
//!         { "context": "shipping", "event_type": "ShipmentCreated" }
//!     ]
//! }
//!
//! // Unsubscribe
//! {
//!     "type": "unsubscribe",
//!     "subscriptions": ["sales"]
//! }
//!
//! // Ping (keep-alive)
//! { "type": "ping" }
//! ```
//!
//! ## Server → Client Messages
//!
//! ```json
//! // Subscription confirmed
//! {
//!     "type": "subscribed",
//!     "subscriptions": ["sales", "stream:order-123"]
//! }
//!
//! // Event notification
//! {
//!     "type": "event",
//!     "global_id": 12345,
//!     "context": "sales",
//!     "stream_id": "order-123",
//!     "version": 2,
//!     "event_type": "OrderConfirmed",
//!     "payload": { ... },
//!     "timestamp": "2024-01-15T10:30:00Z"
//! }
//!
//! // Pong response
//! { "type": "pong" }
//!
//! // Error
//! {
//!     "type": "error",
//!     "code": "invalid_subscription",
//!     "message": "Unknown context: foo"
//! }
//! ```

mod connection;
mod handler;
mod listener;
mod manager;
mod notification;
mod protocol;
mod subscription;

pub use connection::{ConnectionId, WsConnection};
pub use handler::{WsState, ws_handler};
pub use listener::{ListenerConfig, pg_notify_listener, pg_notify_listener_default};
pub use manager::{SharedWsManager, WsManager, WsManagerBuilder};
pub use notification::EventNotification;
pub use protocol::{ClientMessage, ServerMessage, SubscriptionRequest, error_codes};
pub use subscription::Subscription;
