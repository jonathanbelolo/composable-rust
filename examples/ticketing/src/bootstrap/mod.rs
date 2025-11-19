//! Bootstrap components for application initialization.
//!
//! This module provides high-level abstractions for setting up and configuring
//! the ticketing application. It handles all infrastructure setup (databases,
//! event bus, auth) and consumer registration in a clean, declarative API.
//!
//! # Architecture
//!
//! The bootstrap module is designed as a **framework-level abstraction** that
//! can be reused across different applications. Each application provides its
//! own:
//! - Resource configuration (which databases, which event bus)
//! - Aggregate handlers (application-specific event processing)
//! - Projection handlers (application-specific read models)
//!
//! # Modules
//!
//! - **`resources`**: Infrastructure setup (databases, event bus, auth)
//! - **`aggregates`**: Aggregate consumer registration
//! - **`projections`**: Projection consumer registration
//!
//! # Example
//!
//! ```rust,ignore
//! // Step 1: Initialize resources (databases, event bus, auth)
//! let resources = ResourceManager::from_config(&config).await?;
//!
//! // Step 2: Register aggregates (inventory, payment)
//! let aggregate_consumers = register_aggregates(&resources, shutdown_rx)?;
//!
//! // Step 3: Register projections (sales analytics, customer history)
//! let projection_consumers = register_projections(&resources, shutdown_rx)?;
//!
//! // Step 4: Run application
//! Application::new(resources, aggregate_consumers, projection_consumers).run().await?;
//! ```

pub mod aggregates;
pub mod builder;
pub mod projections;
pub mod resources;

pub use aggregates::register_aggregate_consumers;
pub use builder::ApplicationBuilder;
pub use projections::{register_projections, ProjectionSystem};
pub use resources::ResourceManager;
