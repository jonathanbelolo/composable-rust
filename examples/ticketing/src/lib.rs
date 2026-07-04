// Allow certain clippy lints at module level for this example crate
// These are acceptable in example code where clarity/demonstration is prioritized
#![allow(clippy::doc_markdown)] // Example docs use plain names without backticks for readability
#![allow(clippy::cognitive_complexity)] // Complex reducers demonstrate real-world patterns
#![allow(clippy::too_many_lines)] // Reducers naturally grow large with many actions
#![allow(clippy::match_same_arms)] // Explicit arms preferred for architecture clarity
#![allow(clippy::type_complexity)] // Complex types demonstrate composition patterns
#![allow(clippy::cast_precision_loss)] // Acceptable for metrics/analytics
#![allow(clippy::cast_possible_truncation)] // Acceptable for metrics/analytics
#![allow(clippy::cast_sign_loss)] // Acceptable for metrics/analytics
#![allow(clippy::needless_pass_by_value)] // Handler signatures require owned types
#![allow(clippy::missing_errors_doc)] // Example code focuses on demonstration not docs
#![allow(clippy::must_use_candidate)] // Not all functions need #[must_use] in examples
#![allow(clippy::missing_const_for_fn)] // const fn optimization not critical in examples
#![allow(clippy::unnecessary_literal_bound)] // &str lifetime bounds acceptable in examples
#![allow(clippy::expect_used)] // expect is acceptable in example startup/config code
#![allow(clippy::collapsible_if)] // Nested ifs can be clearer in complex business logic
#![allow(clippy::collapsible_match)] // Separate matches can be clearer
#![allow(clippy::single_match_else)] // match with else arm is often clearer
#![allow(clippy::unnecessary_map_or)] // map_or patterns acceptable for clarity
#![allow(clippy::map_unwrap_or)] // map().unwrap_or patterns acceptable
#![allow(clippy::redundant_closure)] // Closures can be clearer than function refs
#![allow(clippy::unused_async)] // Async consistency in traits/interfaces

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
//! See the `aggregates` module for reducer implementations and tests.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod auth;
pub mod bootstrap;
pub mod config;
pub mod next;
pub mod server;
pub mod types;

pub use config::Config;
pub use types::*;
