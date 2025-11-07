//! Transfer saga reducer.
//!
//! Orchestrates money transfers between accounts using the saga pattern.
//! Demonstrates compensation (rollback) when transfers fail.

use crate::types::{
    AccountId, Money, Transfer, TransferAction, TransferId, TransferState, TransferStatus,
};
use composable_rust_core::{
    effect::Effect, environment::Clock, reducer::Reducer, SmallVec,
};

/// Environment dependencies for the Transfer reducer
#[derive(Clone)]
pub struct TransferEnvironment {
    /// Clock for generating timestamps
    pub clock: std::sync::Arc<dyn Clock>,
}

impl TransferEnvironment {
    /// Creates a new `TransferEnvironment`
    #[must_use]
    pub fn new(clock: std::sync::Arc<dyn Clock>) -> Self {
        Self { clock }
    }
}

/// Reducer for money transfers (saga coordinator)
#[derive(Clone, Debug)]
pub struct TransferReducer;

impl TransferReducer {
    /// Creates a new `TransferReducer`
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Validates an `InitiateTransfer` command
    fn validate_initiate_transfer(
        state: &TransferState,
        id: &TransferId,
        from_account: &AccountId,
        to_account: &AccountId,
        amount: Money,
    ) -> Result<(), String> {
        if state.exists(id) {
            return Err(format!("Transfer with ID {id} already exists"));
        }

        if from_account == to_account {
            return Err("Cannot transfer to the same account".to_string());
        }

        if amount.is_zero() {
            return Err("Transfer amount must be greater than zero".to_string());
        }

        Ok(())
    }

    /// Applies an event to state
    fn apply_event(state: &mut TransferState, action: &TransferAction) {
        match action {
            TransferAction::TransferInitiated {
                id,
                from_account,
                to_account,
                amount,
                initiated_at,
            } => {
                let transfer = Transfer::new(
                    id.clone(),
                    from_account.clone(),
                    to_account.clone(),
                    *amount,
                    *initiated_at,
                );
                state.transfers.insert(id.clone(), transfer);
                state.last_error = None;
            }
            TransferAction::DebitApplied { transfer_id, .. } => {
                if let Some(transfer) = state.transfers.get_mut(transfer_id) {
                    transfer.status = TransferStatus::Debited;
                }
                state.last_error = None;
            }
            TransferAction::CreditApplied { transfer_id, .. } => {
                if let Some(transfer) = state.transfers.get_mut(transfer_id) {
                    // Credit applied, but wait for TransferCompleted event
                    transfer.status = TransferStatus::Debited; // Stay in Debited until completed
                }
                state.last_error = None;
            }
            TransferAction::TransferCompleted { transfer_id } => {
                if let Some(transfer) = state.transfers.get_mut(transfer_id) {
                    transfer.status = TransferStatus::Completed;
                }
                state.last_error = None;
            }
            TransferAction::TransferFailed {
                transfer_id,
                reason,
            } => {
                if let Some(transfer) = state.transfers.get_mut(transfer_id) {
                    transfer.status = TransferStatus::Failed {
                        reason: reason.clone(),
                    };
                }
                state.last_error = Some(reason.clone());
            }
            TransferAction::TransferCompensated { transfer_id } => {
                if let Some(transfer) = state.transfers.get_mut(transfer_id) {
                    transfer.status = TransferStatus::Compensated;
                }
                state.last_error = None;
            }
            TransferAction::ValidationFailed { error } => {
                state.last_error = Some(error.clone());
            }
            // Commands are not applied to state
            TransferAction::InitiateTransfer { .. } => {}
        }
    }

    /// Creates effects for account operations
    fn withdraw_from_source(
        _transfer_id: &TransferId,
        _from_account: &AccountId,
        _amount: Money,
    ) -> Effect<TransferAction> {
        // In a real system, this would publish an event to the event bus
        // For now, we'll simulate it by describing the effect
        Effect::None // Placeholder - will be implemented with event bus
    }

    /// Creates effects for account operations
    fn deposit_to_destination(
        _transfer_id: &TransferId,
        _to_account: &AccountId,
        _amount: Money,
    ) -> Effect<TransferAction> {
        // In a real system, this would publish an event to the event bus
        Effect::None // Placeholder - will be implemented with event bus
    }

    /// Creates compensation effect (return money to source)
    fn compensate_transfer(
        _transfer_id: &TransferId,
        _to_account: &AccountId,
        _amount: Money,
    ) -> Effect<TransferAction> {
        // In a real system, this would publish a compensation event
        Effect::None // Placeholder - will be implemented with event bus
    }
}

impl Default for TransferReducer {
    fn default() -> Self {
        Self::new()
    }
}

impl Reducer for TransferReducer {
    type State = TransferState;
    type Action = TransferAction;
    type Environment = TransferEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ========== Commands ==========
            TransferAction::InitiateTransfer {
                id,
                from_account,
                to_account,
                amount,
            } => {
                // Validate command
                if let Err(error) = Self::validate_initiate_transfer(
                    state,
                    &id,
                    &from_account,
                    &to_account,
                    amount,
                ) {
                    Self::apply_event(
                        state,
                        &TransferAction::ValidationFailed {
                            error: error.clone(),
                        },
                    );
                    return SmallVec::new();
                }

                // Create event
                let event = TransferAction::TransferInitiated {
                    id: id.clone(),
                    from_account: from_account.clone(),
                    to_account: to_account.clone(),
                    amount,
                    initiated_at: env.clock.now(),
                };

                // Apply event to state
                Self::apply_event(state, &event);

                // Emit effect to withdraw from source account
                // In a real implementation, this would trigger the account aggregate
                SmallVec::new() // Placeholder - will return withdrawal effect
            }

            // ========== Events ==========
            TransferAction::TransferInitiated { .. } => {
                // Event replayed from event store
                Self::apply_event(state, &action);
                SmallVec::new()
            }

            TransferAction::DebitApplied {
                ref transfer_id,
                account_id: _,
                amount: _,
            } => {
                Self::apply_event(state, &action);

                // Debit successful, now credit the destination
                if let Some(transfer) = state.get(transfer_id) {
                    if transfer.status == TransferStatus::Debited {
                        // Emit effect to deposit to destination
                        // Placeholder - will return deposit effect
                    }
                }

                SmallVec::new()
            }

            TransferAction::CreditApplied {
                ref transfer_id,
                account_id: _,
                amount: _,
            } => {
                Self::apply_event(state, &action);

                // Credit successful, mark transfer as completed
                let completion_event = TransferAction::TransferCompleted {
                    transfer_id: transfer_id.clone(),
                };
                Self::apply_event(state, &completion_event);

                SmallVec::new()
            }

            TransferAction::TransferCompleted { .. }
            | TransferAction::TransferFailed { .. }
            | TransferAction::TransferCompensated { .. }
            | TransferAction::ValidationFailed { .. } => {
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

    fn create_test_env() -> TransferEnvironment {
        TransferEnvironment::new(Arc::new(SystemClock))
    }

    #[test]
    fn test_initiate_transfer_success() {
        let id = TransferId::new();
        let from = AccountId::new();
        let to = AccountId::new();

        ReducerTest::new(TransferReducer::new())
            .with_env(create_test_env())
            .given_state(TransferState::new())
            .when_action(TransferAction::InitiateTransfer {
                id: id.clone(),
                from_account: from,
                to_account: to,
                amount: Money::from_dollars(100),
            })
            .then_state(move |state| {
                assert_eq!(state.count(), 1);
                assert!(state.exists(&id));
                let transfer = state.get(&id).unwrap();
                assert_eq!(transfer.status, TransferStatus::Initiated);
                assert_eq!(transfer.amount, Money::from_dollars(100));
            })
            .then_effects(assertions::assert_no_effects) // Will change when effects are added
            .run();
    }

    #[test]
    fn test_initiate_transfer_same_account() {
        let id = TransferId::new();
        let account = AccountId::new();

        ReducerTest::new(TransferReducer::new())
            .with_env(create_test_env())
            .given_state(TransferState::new())
            .when_action(TransferAction::InitiateTransfer {
                id,
                from_account: account.clone(),
                to_account: account,
                amount: Money::from_dollars(100),
            })
            .then_state(|state| {
                assert_eq!(state.count(), 0); // No transfer created
                assert!(state.last_error.is_some());
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("Cannot transfer to the same account"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_initiate_transfer_zero_amount() {
        let id = TransferId::new();

        ReducerTest::new(TransferReducer::new())
            .with_env(create_test_env())
            .given_state(TransferState::new())
            .when_action(TransferAction::InitiateTransfer {
                id,
                from_account: AccountId::new(),
                to_account: AccountId::new(),
                amount: Money::from_cents(0),
            })
            .then_state(|state| {
                assert_eq!(state.count(), 0);
                assert!(state.last_error.is_some());
                assert!(state
                    .last_error
                    .as_ref()
                    .unwrap()
                    .contains("must be greater than zero"));
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }

    #[test]
    fn test_debit_applied_event() {
        let transfer_id = TransferId::new();
        let from = AccountId::new();
        let to = AccountId::new();

        ReducerTest::new(TransferReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = TransferState::new();
                let transfer = Transfer::new(
                    transfer_id.clone(),
                    from.clone(),
                    to,
                    Money::from_dollars(100),
                    Utc::now(),
                );
                state.transfers.insert(transfer_id.clone(), transfer);
                state
            })
            .when_action(TransferAction::DebitApplied {
                transfer_id: transfer_id.clone(),
                account_id: from,
                amount: Money::from_dollars(100),
            })
            .then_state(move |state| {
                let transfer = state.get(&transfer_id).unwrap();
                assert_eq!(transfer.status, TransferStatus::Debited);
            })
            .then_effects(assertions::assert_no_effects) // Will change when effects are added
            .run();
    }

    #[test]
    fn test_transfer_completion_flow() {
        let transfer_id = TransferId::new();
        let from = AccountId::new();
        let to = AccountId::new();

        ReducerTest::new(TransferReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = TransferState::new();
                let mut transfer = Transfer::new(
                    transfer_id.clone(),
                    from.clone(),
                    to.clone(),
                    Money::from_dollars(100),
                    Utc::now(),
                );
                transfer.status = TransferStatus::Debited;
                state.transfers.insert(transfer_id.clone(), transfer);
                state
            })
            .when_action(TransferAction::CreditApplied {
                transfer_id: transfer_id.clone(),
                account_id: to,
                amount: Money::from_dollars(100),
            })
            .then_state(move |state| {
                let transfer = state.get(&transfer_id).unwrap();
                assert_eq!(transfer.status, TransferStatus::Completed);
            })
            .then_effects(assertions::assert_no_effects)
            .run();
    }
}
