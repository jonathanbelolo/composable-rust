//! HTTP handlers for authentication endpoints.
//!
//! This module contains Axum handlers that implement the Composable Rust
//! request-response pattern using `send_and_wait_for()`.

pub mod magic_link;
pub mod oauth;
pub mod passkey;
pub mod session;
