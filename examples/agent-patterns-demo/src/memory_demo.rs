//! Memory/RAG Pattern Demo
//!
//! Use case: Context-aware customer support with conversation history

use composable_rust_agent_patterns::memory::{
    Memory, MemoryAction, MemoryConfig, MemoryReducer, MemoryState,
};
use composable_rust_anthropic::Tool;
use composable_rust_core::agent::{AgentAction, AgentConfig, AgentEnvironment};
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;

struct DemoEnvironment { config: AgentConfig }

impl AgentEnvironment for DemoEnvironment {
    fn tools(&self) -> &[Tool] { &[] }
    fn config(&self) -> &AgentConfig { &self.config }
    fn call_claude(&self, _: composable_rust_anthropic::MessagesRequest) -> Effect<AgentAction> { Effect::None }
    fn call_claude_streaming(&self, _: composable_rust_anthropic::MessagesRequest) -> Effect<AgentAction> { Effect::None }
    fn execute_tool(&self, _: String, _: String, _: String) -> Effect<AgentAction> { Effect::None }
    fn execute_tool_streaming(&self, _: String, _: String, _: String) -> Effect<AgentAction> { Effect::None }
}

fn main() {
    println!("=== Memory/RAG Pattern Demo ===\n");
    println!("Use Case: Context-Aware Customer Support\n");

    let config = MemoryConfig {
        top_k: 3,
        similarity_threshold: 0.7,
    };

    let reducer = MemoryReducer::new(config);
    let mut state = MemoryState::new();
    let env = DemoEnvironment { config: AgentConfig::default() };

    println!("🔍 Query: 'How do I reset my password?'");
    println!("   Settings: top_k=3, threshold=0.7\n");

    // Query with retrieval
    let effects = reducer.reduce(&mut state, MemoryAction::Query {
        query: "How do I reset my password?".to_string()
    }, &env);
    println!("✅ Query initiated: {} effects", effects.len());

    // Simulate memory retrieval from vector store
    println!("\n📚 Retrieving relevant memories from vector store...");

    let memories = vec![
        Memory {
            id: "mem_1".to_string(),
            content: "Password reset: Click 'Forgot Password' on login page, enter email, check inbox for reset link".to_string(),
            score: 0.95,
        },
        Memory {
            id: "mem_2".to_string(),
            content: "Common issue: Password reset emails may take 5-10 minutes to arrive. Check spam folder".to_string(),
            score: 0.88,
        },
        Memory {
            id: "mem_3".to_string(),
            content: "Security: Password must be 12+ characters with uppercase, lowercase, number, and symbol".to_string(),
            score: 0.75,
        },
        Memory {
            id: "mem_4".to_string(),
            content: "Account creation requires email verification within 24 hours".to_string(),
            score: 0.45, // Below threshold
        },
    ];

    let effects = reducer.reduce(&mut state, MemoryAction::MemoriesRetrieved {
        memories
    }, &env);

    println!("\n📋 Retrieved memories:");
    for memory in state.memories() {
        println!("   [Score: {:.2}] {}", memory.score, memory.content);
    }
    println!("\n   Filtered count: {} (threshold: 0.7)", state.memories().len());
    println!("   Generate response effect: {}", effects.len());

    // Generate response with context
    println!("\n🤖 Generating response with context...");
    let effects = reducer.reduce(&mut state, MemoryAction::ResponseGenerated {
        response: "To reset your password: 1) Click 'Forgot Password' 2) Enter your email 3) Check inbox (may take 5-10 min, check spam) 4) Follow link to set new password (12+ chars, mixed case, number, symbol)".to_string()
    }, &env);

    if let Some(response) = state.response() {
        println!("\n💬 Response:");
        println!("   {}", response);
    }

    println!("\n   Complete: {}", state.is_completed());
    println!("   Effects: {}", effects.len());

    // Optional: Store interaction for future retrieval
    println!("\n💾 Storing interaction for future queries...");
    let response_text = state.response().unwrap_or("").to_string();
    reducer.reduce(&mut state, MemoryAction::StoreInteraction {
        query: "How do I reset my password?".to_string(),
        response: response_text,
    }, &env);
    println!("   ✅ Interaction stored in vector database");

    println!("\n💡 Key Benefits:");
    println!("   • Context-aware responses");
    println!("   • Relevant knowledge retrieval");
    println!("   • Similarity-based filtering");
    println!("   • Conversation history learning");
}
