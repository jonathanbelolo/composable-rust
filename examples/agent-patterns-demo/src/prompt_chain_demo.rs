//! Prompt Chaining Pattern Demo
//!
//! Demonstrates sequential execution where each step uses the result from the previous step.
//! Use case: Research paper analysis pipeline

use composable_rust_agent_patterns::prompt_chain::{
    ChainAction, ChainState, ChainStep, PromptChainReducer,
};
use composable_rust_anthropic::{MessagesRequest, Tool};
use composable_rust_core::agent::{AgentAction, AgentConfig, AgentEnvironment};
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;

/// Mock environment for demonstration
struct DemoEnvironment {
    config: AgentConfig,
}

impl AgentEnvironment for DemoEnvironment {
    fn tools(&self) -> &[Tool] {
        &[]
    }

    fn config(&self) -> &AgentConfig {
        &self.config
    }

    fn call_claude(&self, request: MessagesRequest) -> Effect<AgentAction> {
        // In real implementation, would call Claude API
        println!("\n[Calling Claude API]");
        println!("Request: {:?}", request.messages.last().unwrap());

        Effect::None
    }

    fn call_claude_streaming(&self, _request: MessagesRequest) -> Effect<AgentAction> {
        Effect::None
    }

    fn execute_tool(
        &self,
        _tool_use_id: String,
        _tool_name: String,
        _tool_input: String,
    ) -> Effect<AgentAction> {
        Effect::None
    }

    fn execute_tool_streaming(
        &self,
        _tool_use_id: String,
        _tool_name: String,
        _tool_input: String,
    ) -> Effect<AgentAction> {
        Effect::None
    }
}

fn main() {
    println!("=== Prompt Chaining Pattern Demo ===\n");
    println!("Use Case: Research Paper Analysis Pipeline");
    println!("Steps: Extract → Summarize → Critique → Synthesize\n");

    // Define the chain of prompts
    let steps = vec![
        ChainStep {
            name: "extract".to_string(),
            prompt_template: "Extract key findings from this research: {}".to_string(),
        },
        ChainStep {
            name: "summarize".to_string(),
            prompt_template: "Summarize these findings concisely: {}".to_string(),
        },
        ChainStep {
            name: "critique".to_string(),
            prompt_template: "Provide critical analysis of: {}".to_string(),
        },
        ChainStep {
            name: "synthesize".to_string(),
            prompt_template: "Synthesize final insights from: {}".to_string(),
        },
    ];

    let reducer = PromptChainReducer::new(steps);
    let mut state = ChainState::new();
    let env = DemoEnvironment {
        config: AgentConfig::default(),
    };

    // Start the chain
    println!("📝 Starting chain with research paper...\n");
    let effects = reducer.reduce(
        &mut state,
        ChainAction::Start {
            input: "Research on quantum computing breakthroughs...".to_string(),
        },
        &env,
    );

    println!("✅ Chain initiated");
    println!("   Current step: {}", state.current_step());
    println!("   Effects generated: {}", effects.len());

    // Simulate step completion
    println!("\n📊 Simulating step 1 completion...");
    let _effects = reducer.reduce(
        &mut state,
        ChainAction::StepComplete {
            step: 0,
            result: "Key findings: quantum supremacy achieved, 100+ qubit systems...".to_string(),
        },
        &env,
    );

    println!("✅ Step 1 complete, moving to step 2");
    println!("   Current step: {}", state.current_step());
    println!("   Accumulated context preserved: {}", !state.accumulated_result().is_empty());

    // Simulate another step
    println!("\n📊 Simulating step 2 completion...");
    let _effects = reducer.reduce(
        &mut state,
        ChainAction::StepComplete {
            step: 1,
            result: "Summary: Major breakthrough in quantum computing scalability...".to_string(),
        },
        &env,
    );

    println!("✅ Step 2 complete");
    println!("   Current step: {}", state.current_step());
    println!("   Total steps: {}", reducer.step_count());

    println!("\n💡 Key Benefits:");
    println!("   • Sequential refinement of output");
    println!("   • Each step builds on previous results");
    println!("   • Context preservation across steps");
    println!("   • Ideal for complex analysis workflows");
}
