//! Domain types for the Banking example.
//!
//! This module demonstrates a money transfer saga with compensation:
//! - Bank accounts with deposits and withdrawals
//! - Money transfers between accounts (saga pattern)
//! - Automatic compensation if transfer fails

use chrono::{DateTime, Utc};
use composable_rust_macros::{Action, State};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Unique identifier for a bank account
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(Uuid);

impl AccountId {
    /// Creates a new random `AccountId`
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Creates an `AccountId` from a UUID
    #[must_use]
    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// Returns the inner UUID
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for AccountId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a money transfer
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransferId(Uuid);

impl TransferId {
    /// Creates a new random `TransferId`
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Creates a `TransferId` from a UUID
    #[must_use]
    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// Returns the inner UUID
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for TransferId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TransferId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Money amount in cents (avoids floating point issues)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Money(u64);

impl Money {
    /// Creates a new `Money` amount from cents
    #[must_use]
    pub const fn from_cents(cents: u64) -> Self {
        Self(cents)
    }

    /// Returns the amount in cents
    #[must_use]
    pub const fn cents(&self) -> u64 {
        self.0
    }

    /// Creates a `Money` amount from dollars
    #[must_use]
    pub const fn from_dollars(dollars: u64) -> Self {
        Self(dollars * 100)
    }

    /// Checks if this amount is zero
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

impl std::fmt::Display for Money {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "${}.{:02}", self.0 / 100, self.0 % 100)
    }
}

/// A single bank account
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Account identifier
    pub id: AccountId,
    /// Account holder name
    pub holder_name: String,
    /// Current balance in cents
    pub balance: Money,
    /// When the account was opened
    pub opened_at: DateTime<Utc>,
}

impl Account {
    /// Creates a new account
    #[must_use]
    pub const fn new(
        id: AccountId,
        holder_name: String,
        initial_balance: Money,
        opened_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            holder_name,
            balance: initial_balance,
            opened_at,
        }
    }
}

/// State of all bank accounts
#[derive(State, Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccountState {
    /// All accounts indexed by ID
    pub accounts: HashMap<AccountId, Account>,
    /// Last validation error (if any)
    pub last_error: Option<String>,
}

impl AccountState {
    /// Creates a new empty account state
    #[must_use]
    pub fn new() -> Self {
        Self {
            accounts: HashMap::new(),
            last_error: None,
        }
    }

    /// Returns the number of accounts
    #[must_use]
    pub fn count(&self) -> usize {
        self.accounts.len()
    }

    /// Returns an account by ID
    #[must_use]
    pub fn get(&self, id: &AccountId) -> Option<&Account> {
        self.accounts.get(id)
    }

    /// Checks if an account exists
    #[must_use]
    pub fn exists(&self, id: &AccountId) -> bool {
        self.accounts.contains_key(id)
    }

    /// Returns the balance for an account
    #[must_use]
    pub fn balance(&self, id: &AccountId) -> Option<Money> {
        self.accounts.get(id).map(|a| a.balance)
    }
}

/// Actions for bank accounts (commands and events)
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum AccountAction {
    // ========== Commands ==========
    /// Command: Open a new account
    #[command]
    OpenAccount {
        /// Account identifier
        id: AccountId,
        /// Account holder name
        holder_name: String,
        /// Initial balance
        initial_balance: Money,
    },

    /// Command: Deposit money into an account
    #[command]
    Deposit {
        /// Account to deposit into
        account_id: AccountId,
        /// Amount to deposit
        amount: Money,
    },

    /// Command: Withdraw money from an account
    #[command]
    Withdraw {
        /// Account to withdraw from
        account_id: AccountId,
        /// Amount to withdraw
        amount: Money,
    },

    // ========== Events ==========
    /// Event: Account was opened
    #[event]
    AccountOpened {
        /// Account identifier
        id: AccountId,
        /// Account holder name
        holder_name: String,
        /// Initial balance
        initial_balance: Money,
        /// When the account was opened
        opened_at: DateTime<Utc>,
    },

    /// Event: Money was deposited
    #[event]
    MoneyDeposited {
        /// Account identifier
        account_id: AccountId,
        /// Amount deposited
        amount: Money,
    },

    /// Event: Money was withdrawn
    #[event]
    MoneyWithdrawn {
        /// Account identifier
        account_id: AccountId,
        /// Amount withdrawn
        amount: Money,
    },

    /// Event: Command validation failed
    #[event]
    ValidationFailed {
        /// Error message
        error: String,
    },
}

/// Transfer saga state machine
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferStatus {
    /// Transfer initiated, waiting to debit from source
    Initiated,
    /// Debit applied to source account, waiting to credit destination
    Debited,
    /// Credit applied to destination account, transfer complete
    Completed,
    /// Transfer failed, needs compensation
    Failed { reason: String },
    /// Transfer was compensated (rolled back)
    Compensated,
}

/// A money transfer between accounts
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transfer {
    /// Transfer identifier
    pub id: TransferId,
    /// Source account
    pub from_account: AccountId,
    /// Destination account
    pub to_account: AccountId,
    /// Amount to transfer
    pub amount: Money,
    /// Current status
    pub status: TransferStatus,
    /// When the transfer was initiated
    pub initiated_at: DateTime<Utc>,
}

impl Transfer {
    /// Creates a new transfer
    #[must_use]
    pub const fn new(
        id: TransferId,
        from_account: AccountId,
        to_account: AccountId,
        amount: Money,
        initiated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            from_account,
            to_account,
            amount,
            status: TransferStatus::Initiated,
            initiated_at,
        }
    }
}

/// State of money transfers (saga coordinator)
#[derive(State, Clone, Debug, Default, Serialize, Deserialize)]
pub struct TransferState {
    /// All transfers indexed by ID
    pub transfers: HashMap<TransferId, Transfer>,
    /// Last error (if any)
    pub last_error: Option<String>,
}

impl TransferState {
    /// Creates a new empty transfer state
    #[must_use]
    pub fn new() -> Self {
        Self {
            transfers: HashMap::new(),
            last_error: None,
        }
    }

    /// Returns the number of transfers
    #[must_use]
    pub fn count(&self) -> usize {
        self.transfers.len()
    }

    /// Returns a transfer by ID
    #[must_use]
    pub fn get(&self, id: &TransferId) -> Option<&Transfer> {
        self.transfers.get(id)
    }

    /// Checks if a transfer exists
    #[must_use]
    pub fn exists(&self, id: &TransferId) -> bool {
        self.transfers.contains_key(id)
    }
}

/// Actions for money transfers (saga commands and events)
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum TransferAction {
    // ========== Commands ==========
    /// Command: Initiate a money transfer
    #[command]
    InitiateTransfer {
        /// Transfer identifier
        id: TransferId,
        /// Source account
        from_account: AccountId,
        /// Destination account
        to_account: AccountId,
        /// Amount to transfer
        amount: Money,
    },

    // ========== Events ==========
    /// Event: Transfer initiated
    #[event]
    TransferInitiated {
        /// Transfer identifier
        id: TransferId,
        /// Source account
        from_account: AccountId,
        /// Destination account
        to_account: AccountId,
        /// Amount to transfer
        amount: Money,
        /// When initiated
        initiated_at: DateTime<Utc>,
    },

    /// Event: Debit applied to source account
    #[event]
    DebitApplied {
        /// Transfer identifier
        transfer_id: TransferId,
        /// Source account
        account_id: AccountId,
        /// Amount debited
        amount: Money,
    },

    /// Event: Credit applied to destination account
    #[event]
    CreditApplied {
        /// Transfer identifier
        transfer_id: TransferId,
        /// Destination account
        account_id: AccountId,
        /// Amount credited
        amount: Money,
    },

    /// Event: Transfer completed successfully
    #[event]
    TransferCompleted {
        /// Transfer identifier
        transfer_id: TransferId,
    },

    /// Event: Transfer failed
    #[event]
    TransferFailed {
        /// Transfer identifier
        transfer_id: TransferId,
        /// Failure reason
        reason: String,
    },

    /// Event: Transfer was compensated (rolled back)
    #[event]
    TransferCompensated {
        /// Transfer identifier
        transfer_id: TransferId,
    },

    /// Event: Command validation failed
    #[event]
    ValidationFailed {
        /// Error message
        error: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_id_display() {
        let id = AccountId::new();
        let display = format!("{id}");
        assert!(!display.is_empty());
    }

    #[test]
    fn money_display() {
        assert_eq!(Money::from_cents(100).to_string(), "$1.00");
        assert_eq!(Money::from_cents(1050).to_string(), "$10.50");
        assert_eq!(Money::from_dollars(42).to_string(), "$42.00");
    }

    #[test]
    fn account_state_operations() {
        let mut state = AccountState::new();
        assert_eq!(state.count(), 0);

        let id = AccountId::new();
        let account = Account::new(
            id.clone(),
            "Alice".to_string(),
            Money::from_dollars(100),
            Utc::now(),
        );

        state.accounts.insert(id.clone(), account);
        assert_eq!(state.count(), 1);
        assert!(state.exists(&id));
        assert_eq!(state.balance(&id), Some(Money::from_dollars(100)));
    }

    #[test]
    fn transfer_creation() {
        let transfer = Transfer::new(
            TransferId::new(),
            AccountId::new(),
            AccountId::new(),
            Money::from_dollars(50),
            Utc::now(),
        );

        assert_eq!(transfer.status, TransferStatus::Initiated);
        assert_eq!(transfer.amount, Money::from_dollars(50));
    }

    #[test]
    fn account_action_is_command() {
        let action = AccountAction::OpenAccount {
            id: AccountId::new(),
            holder_name: "Alice".to_string(),
            initial_balance: Money::from_dollars(100),
        };
        assert!(action.is_command());
        assert!(!action.is_event());
    }

    #[test]
    fn transfer_action_is_event() {
        let action = TransferAction::TransferCompleted {
            transfer_id: TransferId::new(),
        };
        assert!(action.is_event());
        assert!(!action.is_command());
    }
}
