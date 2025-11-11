//! Aggregation Pattern Demo
//!
//! Use case: Multi-source news aggregation and synthesis

use composable_rust_agent_patterns::aggregation::{
    AggregationAction, AggregationReducer, AggregationState, Source,
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
    println!("=== Aggregation Pattern Demo ===\n");
    println!("Use Case: Multi-Source News Aggregation\n");

    // Define news sources
    let sources = vec![
        Source {
            id: "tech_news".to_string(),
            query: "Latest AI developments".to_string(),
        },
        Source {
            id: "research_papers".to_string(),
            query: "Recent ML research papers".to_string(),
        },
        Source {
            id: "industry_reports".to_string(),
            query: "AI industry trends".to_string(),
        },
        Source {
            id: "social_media".to_string(),
            query: "AI discussions on Twitter/Reddit".to_string(),
        },
    ];

    let reducer = AggregationReducer::new();
    let mut state = AggregationState::new();
    let env = DemoEnvironment { config: AgentConfig::default() };

    println!("📰 Querying {} sources in parallel...\n", sources.len());

    // Start aggregation
    let effects = reducer.reduce(&mut state, AggregationAction::Start {
        sources: sources.clone()
    }, &env);

    println!("✅ Queries dispatched: {} effects", effects.len());
    println!("   Sources: {}", state.source_count());
    println!("   Responses collected: {}", state.response_count());

    // Simulate source responses
    println!("\n📥 Receiving responses:");

    reducer.reduce(&mut state, AggregationAction::SourceResponse {
        source_id: "tech_news".to_string(),
        response: Ok("OpenAI releases GPT-5, Google announces Gemini 2.0...".to_string()),
    }, &env);
    println!("   ✅ tech_news: Success");
    println!("      Collected: {}/{}", state.response_count(), state.source_count());

    reducer.reduce(&mut state, AggregationAction::SourceResponse {
        source_id: "research_papers".to_string(),
        response: Ok("New transformer architecture achieves SOTA on benchmarks...".to_string()),
    }, &env);
    println!("   ✅ research_papers: Success");
    println!("      Collected: {}/{}", state.response_count(), state.source_count());

    reducer.reduce(&mut state, AggregationAction::SourceResponse {
        source_id: "industry_reports".to_string(),
        response: Err("API rate limit exceeded".to_string()),
    }, &env);
    println!("   ❌ industry_reports: Failed (rate limit)");
    println!("      Collected: {}/{}", state.response_count(), state.source_count());

    let _effects = reducer.reduce(&mut state, AggregationAction::SourceResponse {
        source_id: "social_media".to_string(),
        response: Ok("Community excited about multimodal capabilities...".to_string()),
    }, &env);
    println!("   ✅ social_media: Success");
    println!("      Collected: {}/{}",state.response_count(), state.source_count());
    println!("      All responses collected: {}", state.all_responses_collected());

    // Synthesize
    if state.all_responses_collected() {
        println!("\n🔄 Synthesizing unified perspective...");
        let effects = reducer.reduce(&mut state, AggregationAction::Synthesize, &env);
        println!("   Synthesis effect generated: {}", effects.len());

        if let Some(result) = state.result() {
            println!("\n📊 Aggregated Result:");
            println!("{}", result);
        }
    }

    println!("\n💡 Key Benefits:");
    println!("   • Multi-source data gathering");
    println!("   • Parallel queries for speed");
    println!("   • Graceful failure handling");
    println!("   • Unified synthesis of perspectives");
}
