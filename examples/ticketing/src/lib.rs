//! Event Ticketing System - A comprehensive event-sourced ticketing platform
//!
//! This example demonstrates a production-ready event ticketing system using the
//! Composable Rust framework. It showcases:
//!
//! - **Multiple aggregates**: Event, Inventory, Reservation (saga), Payment
//! - **Concurrency handling**: Preventing double-booking in high-traffic scenarios
//! - **Saga pattern**: Multi-step workflows with compensation (timeout, payment failure)
//! - **Event sourcing**: Complete audit trail of all state changes
//! - **CQRS**: Separate read models (projections) for queries
//!
//! # Architecture
//!
//! ```text
//! Write Side (Event Sourcing):
//! ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
//! │    Event     │  │   Inventory  │  │ Reservation  │  │   Payment    │
//! │  Aggregate   │  │  Aggregate   │  │    (Saga)    │  │  Aggregate   │
//! └──────────────┘  └──────────────┘  └──────────────┘  └──────────────┘
//!        │                 │                  │                  │
//!        └─────────────────┴──────────────────┴──────────────────┘
//!                                 │
//!                           Event Stream
//!                                 │
//!                                 ▼
//!                        ┌─────────────────┐
//!                        │   Event Bus     │
//!                        │   (Redpanda)    │
//!                        └─────────────────┘
//!                                 │
//!                                 ▼
//! Read Side (Projections):
//! ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
//! │   Available  │  │    Sales     │  │   Customer   │
//! │    Seats     │  │  Analytics   │  │   History    │
//! └──────────────┘  └──────────────┘  └──────────────┘
//! ```
//!
//! # Key Features
//!
//! ## 1. Concurrency-Safe Inventory Management
//!
//! The inventory aggregate implements atomic seat reservation to prevent double-booking:
//!
//! ```text
//! CRITICAL: Check availability includes BOTH reserved and sold seats
//!
//! actually_available = total_capacity - reserved - sold
//!
//! if actually_available < quantity {
//!     return InsufficientInventory // One wins, others fail gracefully
//! }
//! ```
//!
//! ## 2. Time-Based Saga with Compensation
//!
//! Reservations have a 5-minute timeout. If payment isn't completed, seats are automatically
//! released back to the available pool:
//!
//! ```text
//! Reservation Flow:
//! 1. Initiate → Reserve Seats (5-min timer starts)
//! 2. Payment Pending
//! 3a. Payment Success → Confirm Seats → Complete
//! 3b. Payment Failure → Release Seats → Compensate
//! 3c. Timeout → Release Seats → Expire
//! ```
//!
//! ## 3. Event Sourcing
//!
//! All state changes are recorded as events:
//! - Commands express intent (`CreateEvent`, `ReserveSeats`)
//! - Events record what happened (`EventCreated`, `SeatsReserved`)
//! - State is derived from event history
//!
//! # Usage
//!
//! See the [aggregates] module for reducer implementations and tests.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod aggregates;
pub mod app;
pub mod auth;
pub mod config;
pub mod projections;
pub mod types;

pub use aggregates::{EventAction, EventReducer, InventoryAction, InventoryReducer};
pub use app::TicketingApp;
pub use config::Config;
pub use projections::{
    AvailableSeatsProjection, CustomerHistoryProjection, Projection, SalesAnalyticsProjection,
    TicketingEvent,
};
pub use types::*;
