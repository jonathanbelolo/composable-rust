//! Banking example demonstrating the saga pattern with money transfers.
//!
//! This example shows how to build a banking system with money transfers
//! using the saga pattern for distributed transactions. It demonstrates:
//!
//! - Multiple aggregates (Account, Transfer)
//! - Saga pattern for coordinating multi-step operations
//! - Compensation (rollback) on failures
//! - Command validation
//! - Event-driven architecture
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────┐
//! │  InitiateTransfer │
//! └────────┬──────────┘
//!          │
//!          ▼
//! ┌─────────────────────┐
//! │  TransferSaga       │ (Coordinator)
//! └─────────┬───────────┘
//!           │
//!           ├─► Withdraw from Source ──► Account Aggregate
//!           │                               │
//!           │◄─── DebitApplied ─────────────┘
//!           │
//!           ├─► Deposit to Destination ──► Account Aggregate
//!           │                               │
//!           │◄─── CreditApplied ────────────┘
//!           │
//!           └─► TransferCompleted
//!
//! Compensation (if credit fails):
//! CreditFailed ─► Deposit to Source ─► Account Aggregate
//! ```
//!
//! # Quick Start
//!
//! ```no_run
//! use banking::{
//!     AccountAction, AccountEnvironment, AccountId, AccountReducer, AccountState,
//!     Money, TransferAction, TransferEnvironment, TransferId, TransferReducer,
//!     TransferState,
//! };
//! use composable_rust_core::environment::SystemClock;
//! use composable_rust_runtime::Store;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create account environment and store
//! let account_env = AccountEnvironment::new(Arc::new(SystemClock));
//! let account_store = Store::new(AccountState::new(), AccountReducer::new(), account_env);
//!
//! // Create accounts
//! let alice_id = AccountId::new();
//! let bob_id = AccountId::new();
//!
//! account_store.send(AccountAction::OpenAccount {
//!     id: alice_id.clone(),
//!     holder_name: "Alice".to_string(),
//!     initial_balance: Money::from_dollars(1000),
//! }).await?;
//!
//! account_store.send(AccountAction::OpenAccount {
//!     id: bob_id.clone(),
//!     holder_name: "Bob".to_string(),
//!     initial_balance: Money::from_dollars(500),
//! }).await?;
//!
//! // Create transfer environment and store
//! let transfer_env = TransferEnvironment::new(Arc::new(SystemClock));
//! let transfer_store = Store::new(TransferState::new(), TransferReducer::new(), transfer_env);
//!
//! // Initiate transfer
//! transfer_store.send(TransferAction::InitiateTransfer {
//!     id: TransferId::new(),
//!     from_account: alice_id,
//!     to_account: bob_id,
//!     amount: Money::from_dollars(100),
//! }).await?;
//!
//! # Ok(())
//! # }
//! ```

pub mod account;
pub mod transfer;
pub mod types;

// Re-export commonly used types
pub use account::{AccountEnvironment, AccountReducer};
pub use transfer::{TransferEnvironment, TransferReducer};
pub use types::{
    Account, AccountAction, AccountId, AccountState, Money, Transfer, TransferAction,
    TransferId, TransferState, TransferStatus,
};
