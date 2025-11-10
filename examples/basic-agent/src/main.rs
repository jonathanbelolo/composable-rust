//! Simple Q&A Agent Example
//!
//! Demonstrates a basic conversational agent without tools.
//! This example shows the simplest possible agent: user asks questions,
//! Claude responds.
//!
//! ## Usage
//!
//! Set your API key:
//! ```bash
//! export ANTHROPIC_API_KEY="your-key-here"
//! ```
//!
//! Run the example:
//! ```bash
//! cargo run --example basic-agent
//! ```

use basic_agent::{environment::ProductionAgentEnvironment, BasicAgentReducer};
use composable_rust_core::agent::{AgentAction, AgentConfig, BasicAgentState};
use composable_rust_runtime::Store;
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    println!("=== Simple Q&A Agent ===");
    println!("Ask me anything! Type 'quit' to exit.\n");

    // Create agent configuration
    let config = AgentConfig::default()
        .with_system_prompt("You are a helpful assistant. Provide concise, accurate answers.".to_string());

    // Create production environment (connects to real Claude API)
    let environment = ProductionAgentEnvironment::new(config.clone())?;

    // Create reducer and store
    let reducer = BasicAgentReducer::new();
    let initial_state = BasicAgentState::new(config);
    let store = Store::new(initial_state, reducer, environment);

    // Subscribe to actions for displaying responses
    let mut action_rx = store.subscribe_actions();

    // Spawn task to handle Claude's responses
    tokio::spawn(async move {
        while let Ok(action) = action_rx.recv().await {
            match action {
                AgentAction::ClaudeResponse { content, .. } => {
                    for block in content {
                        if let composable_rust_core::agent::ContentBlock::Text { text } = block {
                            println!("\nAssistant: {text}\n");
                        }
                    }
                }
                AgentAction::Error { error } => {
                    eprintln!("\nError: {error}\n");
                }
                _ => {}
            }
        }
    });

    // Main conversation loop
    loop {
        // Prompt user
        print!("You: ");
        io::stdout().flush()?;

        // Read user input
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        // Check for quit command
        if input.eq_ignore_ascii_case("quit") || input.eq_ignore_ascii_case("exit") {
            println!("\nGoodbye!");
            break;
        }

        // Skip empty input
        if input.is_empty() {
            continue;
        }

        // Send user message to agent
        store
            .send(AgentAction::UserMessage {
                content: input.to_string(),
            })
            .await?;

        // Give the response handler time to print
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    Ok(())
}
