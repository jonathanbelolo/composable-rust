//! Weather Agent Example
//!
//! Demonstrates an agent with tool use - specifically a weather lookup tool.
//! This example shows:
//! - Defining a tool with input schema
//! - Implementing `ToolExecutor` trait
//! - Registering tools with the environment
//! - Claude automatically calling tools when needed
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
//! cargo run -p weather-agent
//! ```
//!
//! Try asking: "What's the weather in San Francisco?"

use composable_rust_anthropic::Tool;
use composable_rust_core::agent::{AgentAction, AgentConfig, BasicAgentState, ToolResult};
use composable_rust_runtime::Store;
use serde_json::json;
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::Arc;

// Import from basic-agent crate
use composable_rust_core::reducer::Reducer;

/// Weather agent reducer (reuse `BasicAgentReducer` logic)
#[derive(Clone)]
struct WeatherAgentReducer;

impl Reducer for WeatherAgentReducer {
    type State = BasicAgentState;
    type Action = AgentAction;
    type Environment = WeatherEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> smallvec::SmallVec<[composable_rust_core::effect::Effect<Self::Action>; 4]> {
        // Delegate to basic-agent's reducer logic
        let basic_reducer = basic_agent::BasicAgentReducer::<WeatherEnvironment>::new();
        basic_reducer.reduce(state, action, env)
    }
}

/// Weather environment with tool support
#[derive(Clone)]
struct WeatherEnvironment {
    inner: basic_agent::environment::ProductionAgentEnvironment,
}

impl WeatherEnvironment {
    fn new(
        config: AgentConfig,
    ) -> Result<Self, composable_rust_anthropic::error::ClaudeError> {
        // Define the weather tool
        let weather_tool = Tool {
            name: "get_weather".to_string(),
            description: "Get the current weather for a location".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "The city and state, e.g. San Francisco, CA"
                    }
                },
                "required": ["location"]
            }),
        };

        // Create tool executor function
        let executor = Arc::new(|input: String| {
            Box::pin(async move {
                // Parse the input JSON
                let parsed: serde_json::Value = match serde_json::from_str(&input) {
                    Ok(v) => v,
                    Err(e) => {
                        return Err(composable_rust_core::agent::ToolError {
                            message: format!("Invalid input JSON: {e}"),
                        });
                    }
                };

                let location = parsed["location"].as_str().unwrap_or("unknown");

                // Mock weather data (in production, call a real weather API)
                let weather_data = json!({
                    "location": location,
                    "temperature": "72°F",
                    "conditions": "Partly cloudy",
                    "humidity": "65%",
                    "wind": "10 mph"
                });

                Ok(weather_data.to_string())
            })
                as Pin<Box<dyn std::future::Future<Output = ToolResult> + Send>>
        }) as basic_agent::environment::ToolExecutorFn;

        // Create environment with the tool
        let inner = basic_agent::environment::ProductionAgentEnvironment::new(config)?
            .with_tool(&weather_tool, executor);

        Ok(Self { inner })
    }
}

impl composable_rust_core::agent::AgentEnvironment for WeatherEnvironment {
    fn tools(&self) -> &[Tool] {
        self.inner.tools()
    }

    fn config(&self) -> &AgentConfig {
        self.inner.config()
    }

    fn call_claude(
        &self,
        request: composable_rust_anthropic::MessagesRequest,
    ) -> composable_rust_core::effect::Effect<AgentAction> {
        self.inner.call_claude(request)
    }

    fn call_claude_streaming(
        &self,
        request: composable_rust_anthropic::MessagesRequest,
    ) -> composable_rust_core::effect::Effect<AgentAction> {
        self.inner.call_claude_streaming(request)
    }

    fn execute_tool(
        &self,
        tool_use_id: String,
        tool_name: String,
        tool_input: String,
    ) -> composable_rust_core::effect::Effect<AgentAction> {
        self.inner.execute_tool(tool_use_id, tool_name, tool_input)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    println!("=== Weather Agent ===");
    println!("Ask me about the weather! Type 'quit' to exit.");
    println!("Try: 'What's the weather in San Francisco?'\n");

    // Create agent configuration
    let config = AgentConfig::default().with_system_prompt(
        "You are a helpful weather assistant. When users ask about weather, \
         use the get_weather tool to look up current conditions."
            .to_string(),
    );

    // Create environment with weather tool
    let environment = WeatherEnvironment::new(config.clone())?;

    // Create reducer and store
    let reducer = WeatherAgentReducer;
    let initial_state = BasicAgentState::new(config);
    let store = Store::new(initial_state, reducer, environment);

    // Subscribe to actions for displaying responses
    let mut action_rx = store.subscribe_actions();

    // Spawn task to handle responses
    tokio::spawn(async move {
        while let Ok(action) = action_rx.recv().await {
            match action {
                AgentAction::ClaudeResponse { content, .. } => {
                    for block in content {
                        match block {
                            composable_rust_core::agent::ContentBlock::Text { text } => {
                                println!("\nAssistant: {text}\n");
                            }
                            composable_rust_core::agent::ContentBlock::ToolUse {
                                name, input, ..
                            } => {
                                println!("\n[Using tool: {name} with input: {input}]");
                            }
                            composable_rust_core::agent::ContentBlock::ToolResult { .. } => {}
                        }
                    }
                }
                AgentAction::ToolResult {
                    result: Ok(output), ..
                } => {
                    println!("[Tool result: {output}]");
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
        print!("You: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if input.eq_ignore_ascii_case("quit") || input.eq_ignore_ascii_case("exit") {
            println!("\nGoodbye!");
            break;
        }

        if input.is_empty() {
            continue;
        }

        store
            .send(AgentAction::UserMessage {
                content: input.to_string(),
            })
            .await?;

        // Give time for tool execution and response
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    Ok(())
}
