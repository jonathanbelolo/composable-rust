//! HTTP request handlers.
//!
//! This module contains all HTTP handlers organized by domain.

pub mod health;

// Re-export common handler utilities
pub use health::health_check;
