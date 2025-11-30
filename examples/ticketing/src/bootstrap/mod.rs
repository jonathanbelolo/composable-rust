//! Bootstrap components for application initialization.
//!
//! This module provides high-level abstractions for setting up and configuring
//! the ticketing application using the next-generation architecture.

pub mod builder;
pub mod resources;

pub use builder::ApplicationBuilder;
pub use resources::ResourceManager;
