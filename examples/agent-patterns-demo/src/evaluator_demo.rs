//! Evaluator-Optimizer Pattern Demo
//!
//! Use case: Iterative code generation with quality improvement

use composable_rust_agent_patterns::evaluator::{
    Evaluation, EvaluatorAction, EvaluatorConfig, EvaluatorReducer, EvaluatorState,
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
    println!("=== Evaluator-Optimizer Pattern Demo ===\n");
    println!("Use Case: Iterative Code Generation with Quality Improvement\n");

    let config = EvaluatorConfig {
        max_iterations: 4,
        quality_threshold: 0.85,
    };

    let reducer = EvaluatorReducer::new(config);
    let mut state = EvaluatorState::new();
    let env = DemoEnvironment { config: AgentConfig::default() };

    println!("🎯 Target: Quality score >= 0.85 (max 4 iterations)\n");

    // Start optimization
    reducer.reduce(&mut state, EvaluatorAction::Start {
        task: "Generate efficient sorting algorithm".to_string()
    }, &env);
    println!("✅ Optimization started");

    // Iteration 1 - poor quality
    println!("\n📊 Iteration 1:");
    reducer.reduce(&mut state, EvaluatorAction::CandidateGenerated {
        candidate: "def sort(arr): return sorted(arr)".to_string()
    }, &env);
    println!("   Candidate: Simple built-in sort");

    reducer.reduce(&mut state, EvaluatorAction::Evaluated {
        evaluation: Evaluation {
            score: 0.50,
            feedback: "Too simple, lacks efficiency analysis".to_string(),
        }
    }, &env);
    println!("   Score: 0.50 ❌ (below threshold)");
    println!("   Best so far: 0.50");

    // Iteration 2 - better
    println!("\n📊 Iteration 2:");
    reducer.reduce(&mut state, EvaluatorAction::CandidateGenerated {
        candidate: "Quicksort with median-of-three pivot".to_string()
    }, &env);
    println!("   Candidate: Improved algorithm");

    reducer.reduce(&mut state, EvaluatorAction::Evaluated {
        evaluation: Evaluation {
            score: 0.75,
            feedback: "Good approach, needs edge case handling".to_string(),
        }
    }, &env);
    println!("   Score: 0.75 ❌ (below threshold)");
    println!("   Best so far: 0.75");

    // Iteration 3 - meets threshold
    println!("\n📊 Iteration 3:");
    reducer.reduce(&mut state, EvaluatorAction::CandidateGenerated {
        candidate: "Optimized quicksort with edge cases and benchmarks".to_string()
    }, &env);
    println!("   Candidate: Production-ready implementation");

    let effects = reducer.reduce(&mut state, EvaluatorAction::Evaluated {
        evaluation: Evaluation {
            score: 0.90,
            feedback: "Excellent: efficient, handles edge cases, well-tested".to_string(),
        }
    }, &env);
    println!("   Score: 0.90 ✅ (meets threshold!)");
    println!("   Best so far: 0.90");
    println!("   Optimization complete: {} effects", effects.len());

    println!("\n🎉 Final Result:");
    println!("   Iterations: {}", state.iteration());
    println!("   Best score: {}", state.best_score());
    println!("   Status: {}", if state.is_completed() { "Complete" } else { "In progress" });

    println!("\n💡 Key Benefits:");
    println!("   • Iterative quality improvement");
    println!("   • Automatic termination on threshold");
    println!("   • Best candidate tracking");
    println!("   • Feedback-driven refinement");
}
