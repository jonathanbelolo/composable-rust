//! # Anthropic Claude API Client
//!
//! Rust client library for the Anthropic Claude API with support for
//! messages, tool use, and streaming responses.
//!
//! ## Example
//!
//! ```no_run
//! use composable_rust_anthropic::{AnthropicClient, MessagesRequest};
//! use composable_rust_anthropic::types::Message;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create client from ANTHROPIC_API_KEY environment variable
//!     let client = AnthropicClient::from_env()?;
//!
//!     // Create a simple request
//!     let request = MessagesRequest::new(vec![
//!         Message::user("Hello, Claude!")
//!     ]);
//!
//!     // Get response
//!     let response = client.messages(request).await?;
//!
//!     println!("Response: {:?}", response);
//!     Ok(())
//! }
//! ```
//!
//! ## Features
//!
//! - Non-streaming messages API
//! - Streaming responses with Server-Sent Events (SSE)
//! - Tool use support
//! - Type-safe message and content blocks
//! - Cost calculation utilities

pub mod client;
pub mod error;
pub mod messages;
pub mod types;

// Re-export main types for convenience
pub use client::AnthropicClient;
pub use error::ClaudeError;
pub use messages::{
    ContentDelta, MessageDelta, MessageStart, MessagesRequest, MessagesResponse, StreamEvent,
};
pub use types::{
    ContentBlock, Message, Role, StopReason, Tool, Usage, PricingModel,
    CLAUDE_SONNET_4_5_PRICING,
};
