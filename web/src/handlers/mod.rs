//! HTTP request handlers.
//!
//! This module contains all HTTP handlers organized by domain.

pub mod health;
pub mod websocket;
pub mod websocket_topics;

// Re-export common handler utilities
pub use health::health_check;
pub use websocket::WsMessage;
pub use websocket_topics::TopicBroadcaster;
