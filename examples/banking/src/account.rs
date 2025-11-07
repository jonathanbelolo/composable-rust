//! Account aggregate reducer.
//!
//! Handles individual account operations: open, deposit, withdraw.

use crate::types::{Account, AccountAction, AccountId, AccountState, Money};
use composable_rust_core::{
    effect::Effect, environment::Clock, reducer::Reducer, SmallVec,
};

/// Environment dependencies for the Account reducer
#[derive(Clone)]
pub struct AccountEnvironment {
    /// Clock for generating timestamps
    pub clock: std::sync::Arc<dyn Clock>,
}

impl AccountEnvironment {
    /// Creates a new `AccountEnvironment`
    #[must_use]
    pub fn new(clock: std::sync::Arc<dyn Clock>) -> Self {
        Self { clock }
    }
}

/// Reducer for bank accounts
#[derive(Clone, Debug)]
pub struct AccountReducer;

impl AccountReducer {
    /// Creates a new `AccountReducer`
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Validates an `OpenAccount` command
    fn validate_open_account(
        state: &AccountState,
        id: &AccountId,
        holder_name: &str,
    ) -> Result<(), String> {
        if state.exists(id) {
            return Err(format!("Account with ID {id} already exists"));
        }

        if holder_name.trim().is_empty() {
            return Err("Account holder name cannot be empty".to_string());
        }

        Ok(())
    }

    /// Validates a `Deposit` command
    fn validate_deposit(
        state: &AccountState,
        account_id: &AccountId,
        amount: Money,
    ) -> Result<(), String> {
        if !state.exists(account_id) {
            return Err(format!("Account with ID {account_id} not found"));
        }

        if amount.is_zero() {
            return Err("Deposit amount must be greater than zero".to_string());
        }

        Ok(())
    }

    /// Validates a `Withdraw` command
    fn validate_withdraw(
        state: &AccountState,
        account_id: &AccountId,
        amount: Money,
    ) -> Result<(), String> {
        let Some(account) = state.get(account_id) else {
            return Err(format!("Account with ID {account_id} not found"));
        };

        if amount.is_zero() {
            return Err("Withdrawal amount must be greater than zero".to_string());
        }

        if account.balance.cents() < amount.cents() {
            return Err(format!(
                "Insufficient funds: balance {} < requested {}",
                account.balance, amount
            ));
        }

        Ok(())
    }

    /// Applies an event to state
    fn apply_event(state: &mut AccountState, action: &AccountAction) {
        match action {
            AccountAction::AccountOpened {
                id,
                holder_name,
                initial_balance,
                opened_at,
            } => {
                let account = Account::new(
                    id.clone(),
                    holder_name.clone(),
                    *initial_balance,
                    *opened_at,
                );
                state.accounts.insert(id.clone(), account);
                state.last_error = None;
            }
            AccountAction::MoneyDeposited {
                account_id,
                amount,
            } => {
                if let Some(account) = state.accounts.get_mut(account_id) {
                    account.balance = Money::from_cents(account.balance.cents() + amount.cents());
                }
                state.last_error = None;
            }
            AccountAction::MoneyWithdrawn {
                account_id,
                amount,
            } => {
                if let Some(account) = state.accounts.get_mut(account_id) {
                    account.balance = Money::from_cents(account.balance.cents() - amount.cents());
                }
                state.last_error = None;
            }
            AccountAction::ValidationFailed { error } => {
                state.last_error = Some(error.clone());
            }
            // Commands are not applied to state
            AccountAction::OpenAccount { .. }
            | AccountAction::Deposit { .. }
            | AccountAction::Withdraw { .. } => {}
        }
    }
}

impl Default for AccountReducer {
    fn default() -> Self {
        Self::new()
    }
}

impl Reducer for AccountReducer {
    type State = AccountState;
    type Action = AccountAction;
    type Environment = AccountEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ========== Commands ==========
            AccountAction::OpenAccount {
                id,
                holder_name,
                initial_balance,
            } => {
                // Validate command
                if let Err(error) = Self::validate_open_account(state, &id, &holder_name) {
                    Self::apply_event(
                        state,
                        &AccountAction::ValidationFailed {
                            error: error.clone(),
                        },
                    );
                    return SmallVec::new();
                }

                // Create event
                let event = AccountAction::AccountOpened {
                    id,
                    holder_name,
                    initial_balance,
                    opened_at: env.clock.now(),
                };

                // Apply event to state
                Self::apply_event(state, &event);

                SmallVec::new()
            }

            AccountAction::Deposit {
                account_id,
                amount,
            } => {
                // Validate command
                if let Err(error) = Self::validate_deposit(state, &account_id, amount) {
                    Self::apply_event(
                        state,
                        &AccountAction::ValidationFailed {
                            error: error.clone(),
                        },
                    );
                    return SmallVec::new();
                }

                // Create event
                let event = AccountAction::MoneyDeposited {
                    account_id,
                    amount,
                };

                // Apply event to state
                Self::apply_event(state, &event);

                SmallVec::new()
            }

            AccountAction::Withdraw {
                account_id,
                amount,
            } => {
                // Validate command
                if let Err(error) = Self::validate_withdraw(state, &account_id, amount) {
                    Self::apply_event(
                        state,
                        &AccountAction::ValidationFailed {
                            error: error.clone(),
                        },
                    );
                    return SmallVec::new();
                }

                // Create event
                let event = AccountAction::MoneyWithdrawn {
                    account_id,
                    amount,
                };

                // Apply event to state
                Self::apply_event(state, &event);

                SmallVec::new()
            }

            // ========== Events ==========
            AccountAction::AccountOpened { .. }
            | AccountAction::MoneyDeposited { .. }
            | AccountAction::MoneyWithdrawn { .. }
            | AccountAction::ValidationFailed { .. } => {
                // Events are applied (for replay or external events)
                Self::apply_event(state, &action);
                SmallVec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use composable_rust_core::environment::SystemClock;
    use composable_rust_testing::{assertions, ReducerTest};
    use std::sync::Arc;

    fn create_test_env() -> AccountEnvironment {
        AccountEnvironment::new(Arc::new(SystemClock))
    }

    #[test]
    fn test_open_account_success() {
        let id = AccountId::new();

        ReducerTest::new(AccountReducer::new())
            .with_env(create_test_env())
            .given_state(AccountState::new())
            .when_action(AccountAction::OpenAccount {
                id: id.clone(),
                holder_name: "Alice".to_string(),
                initial_balance: Money::from_dollars(100),
            })
            .then_state(move |state| {
                assert_eq!(state.count(), 1);
                assert!(state.exists(&id));
                let account = state.get(&id).unwrap();
                assert_eq!(account.holder_name, "Alice");
                assert_eq!(account.balance, Money::from_dollars(100));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_open_account_duplicate() {
        let id = AccountId::new();

        ReducerTest::new(AccountReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = AccountState::new();
                let account = Account::new(
                    id.clone(),
                    "Existing".to_string(),
                    Money::from_dollars(50),
                    Utc::now(),
                );
                state.accounts.insert(id.clone(), account);
                state
            })
            .when_action(AccountAction::OpenAccount {
                id,
                holder_name: "Duplicate".to_string(),
                initial_balance: Money::from_dollars(100),
            })
            .then_state(|state| {
                assert_eq!(state.count(), 1); // Still only one account
                assert!(state.last_error.is_some());
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("already exists"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_deposit_success() {
        let id = AccountId::new();

        ReducerTest::new(AccountReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = AccountState::new();
                let account = Account::new(
                    id.clone(),
                    "Alice".to_string(),
                    Money::from_dollars(100),
                    Utc::now(),
                );
                state.accounts.insert(id.clone(), account);
                state
            })
            .when_action(AccountAction::Deposit {
                account_id: id.clone(),
                amount: Money::from_dollars(50),
            })
            .then_state(move |state| {
                assert_eq!(state.balance(&id), Some(Money::from_dollars(150)));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_withdraw_success() {
        let id = AccountId::new();

        ReducerTest::new(AccountReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = AccountState::new();
                let account = Account::new(
                    id.clone(),
                    "Alice".to_string(),
                    Money::from_dollars(100),
                    Utc::now(),
                );
                state.accounts.insert(id.clone(), account);
                state
            })
            .when_action(AccountAction::Withdraw {
                account_id: id.clone(),
                amount: Money::from_dollars(30),
            })
            .then_state(move |state| {
                assert_eq!(state.balance(&id), Some(Money::from_dollars(70)));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_withdraw_insufficient_funds() {
        let id = AccountId::new();

        ReducerTest::new(AccountReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = AccountState::new();
                let account = Account::new(
                    id.clone(),
                    "Alice".to_string(),
                    Money::from_dollars(50),
                    Utc::now(),
                );
                state.accounts.insert(id.clone(), account);
                state
            })
            .when_action(AccountAction::Withdraw {
                account_id: id.clone(),
                amount: Money::from_dollars(100),
            })
            .then_state(move |state| {
                // Balance unchanged
                assert_eq!(state.balance(&id), Some(Money::from_dollars(50)));
                assert!(state.last_error.is_some());
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("Insufficient funds"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }
}
