//! PostgreSQL storage implementations.
//!
//! This module provides persistent storage using PostgreSQL for:
//! - Device registry (permanent audit trail)
//! - User accounts
//! - OAuth links
//! - Passkey credentials
//! - Magic link tokens

pub mod device;

// Re-exports
pub use device::PostgresDeviceRepository;
