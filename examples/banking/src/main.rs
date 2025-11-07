//! Simple CLI demo for the banking example.
//!
//! This demonstrates bank accounts and money transfers with the saga pattern.

use banking::{
    AccountAction, AccountEnvironment, AccountId, AccountReducer, AccountState, Money,
    TransferAction, TransferEnvironment, TransferId, TransferReducer, TransferState,
};
use composable_rust_core::environment::SystemClock;
use composable_rust_runtime::Store;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Banking Example: Money Transfers ===\n");

    // Create account environment and store
    let account_env = AccountEnvironment::new(Arc::new(SystemClock));
    let account_store = Store::new(AccountState::new(), AccountReducer::new(), account_env);

    // Create accounts
    println!("Opening accounts...");
    let alice_id = AccountId::new();
    let bob_id = AccountId::new();

    account_store
        .send(AccountAction::OpenAccount {
            id: alice_id.clone(),
            holder_name: "Alice".to_string(),
            initial_balance: Money::from_dollars(1000),
        })
        .await?;

    account_store
        .send(AccountAction::OpenAccount {
            id: bob_id.clone(),
            holder_name: "Bob".to_string(),
            initial_balance: Money::from_dollars(500),
        })
        .await?;

    // Show initial balances
    let state = account_store.state(|s| s.clone()).await;
    println!("\nInitial balances:");
    for account in state.accounts.values() {
        println!("  {}: {}", account.holder_name, account.balance);
    }

    // Deposit to Alice's account
    println!("\nAlice deposits $200...");
    account_store
        .send(AccountAction::Deposit {
            account_id: alice_id.clone(),
            amount: Money::from_dollars(200),
        })
        .await?;

    // Show updated balances
    let state = account_store.state(|s| s.clone()).await;
    println!("\nAfter deposit:");
    for account in state.accounts.values() {
        println!("  {}: {}", account.holder_name, account.balance);
    }

    // Bob withdraws money
    println!("\nBob withdraws $50...");
    account_store
        .send(AccountAction::Withdraw {
            account_id: bob_id.clone(),
            amount: Money::from_dollars(50),
        })
        .await?;

    // Show updated balances
    let state = account_store.state(|s| s.clone()).await;
    println!("\nAfter withdrawal:");
    for account in state.accounts.values() {
        println!("  {}: {}", account.holder_name, account.balance);
    }

    // Demonstrate transfer saga
    println!("\n=== Money Transfer (Saga Pattern) ===");
    println!("\nInitiating transfer: Alice -> Bob ($300)...");

    let transfer_env = TransferEnvironment::new(Arc::new(SystemClock));
    let transfer_store = Store::new(TransferState::new(), TransferReducer::new(), transfer_env);

    let transfer_id = TransferId::new();
    transfer_store
        .send(TransferAction::InitiateTransfer {
            id: transfer_id.clone(),
            from_account: alice_id.clone(),
            to_account: bob_id.clone(),
            amount: Money::from_dollars(300),
        })
        .await?;

    // Check transfer state
    let transfer_state = transfer_store.state(|s| s.clone()).await;
    if let Some(transfer) = transfer_state.get(&transfer_id) {
        println!("Transfer status: {:?}", transfer.status);
        println!("Transfer amount: {}", transfer.amount);
    }

    println!("\nNote: Full end-to-end transfer requires event bus integration");
    println!("(Account withdrawals and deposits would be triggered via events)");

    // For now, manually demonstrate what would happen:
    println!("\n=== Simulating Transfer Steps ===");
    println!("Step 1: Withdraw from Alice's account...");
    account_store
        .send(AccountAction::Withdraw {
            account_id: alice_id.clone(),
            amount: Money::from_dollars(300),
        })
        .await?;

    println!("Step 2: Deposit to Bob's account...");
    account_store
        .send(AccountAction::Deposit {
            account_id: bob_id.clone(),
            amount: Money::from_dollars(300),
        })
        .await?;

    // Show final balances
    let state = account_store.state(|s| s.clone()).await;
    println!("\nFinal balances:");
    for account in state.accounts.values() {
        println!("  {}: {}", account.holder_name, account.balance);
    }

    println!("\nExpected:");
    println!("  Alice: $900 (1000 + 200 - 300)");
    println!("  Bob:   $750 (500 - 50 + 300)");

    // Test insufficient funds
    println!("\n=== Testing Insufficient Funds ===");
    println!("Attempting to withdraw $10,000 from Bob's account...");
    account_store
        .send(AccountAction::Withdraw {
            account_id: bob_id.clone(),
            amount: Money::from_dollars(10000),
        })
        .await?;

    let state = account_store.state(|s| s.clone()).await;
    if let Some(error) = &state.last_error {
        println!("Error (as expected): {error}");
    }

    println!("\nBob's balance unchanged: {}", state.balance(&bob_id).unwrap());

    println!("\n=== Demo Complete ===");
    println!("\nKey Concepts Demonstrated:");
    println!("- Bank account aggregate (open, deposit, withdraw)");
    println!("- Money transfer saga (multi-step coordination)");
    println!("- Command validation (insufficient funds, etc.)");
    println!("- Event-driven state updates");
    println!("\nNext: Integrate with event bus for full saga coordination");

    Ok(())
}
