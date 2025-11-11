//! Orchestrator-Workers Pattern Demo
//!
//! Use case: Complex software build pipeline with dependency management

use composable_rust_agent_patterns::orchestrator::{
    OrchestratorAction, OrchestratorReducer, OrchestratorState, Subtask, WorkerRegistry,
};
use composable_rust_anthropic::Tool;
use composable_rust_core::agent::{AgentAction, AgentConfig, AgentEnvironment};
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use std::sync::Arc;

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
    println!("=== Orchestrator-Workers Pattern Demo ===\n");
    println!("Use Case: Software Build Pipeline with Dependencies\n");

    // Define workers
    let lint_worker = Arc::new(|_input: String| {
        Box::pin(async { Ok("Linting passed: 0 errors".to_string()) })
            as std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
    });

    let test_worker = Arc::new(|_input: String| {
        Box::pin(async { Ok("Tests passed: 47/47".to_string()) })
            as std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
    });

    let build_worker = Arc::new(|_input: String| {
        Box::pin(async { Ok("Build successful: binary created".to_string()) })
            as std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
    });

    let deploy_worker = Arc::new(|_input: String| {
        Box::pin(async { Ok("Deployed to staging environment".to_string()) })
            as std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
    });

    let registry = WorkerRegistry::new()
        .register("lint", lint_worker)
        .register("test", test_worker)
        .register("build", build_worker)
        .register("deploy", deploy_worker);

    // Define subtasks with dependencies
    let subtasks = vec![
        Subtask { id: "lint".to_string(), worker_type: "lint".to_string(), input: "src/".to_string(), dependencies: vec![] },
        Subtask { id: "test".to_string(), worker_type: "test".to_string(), input: "tests/".to_string(), dependencies: vec![] },
        Subtask { id: "build".to_string(), worker_type: "build".to_string(), input: "release".to_string(), dependencies: vec!["lint".to_string(), "test".to_string()] },
        Subtask { id: "deploy".to_string(), worker_type: "deploy".to_string(), input: "staging".to_string(), dependencies: vec!["build".to_string()] },
    ];

    let reducer = OrchestratorReducer::new(registry);
    let mut state = OrchestratorState::new();
    let env = DemoEnvironment { config: AgentConfig::default() };

    println!("📋 Pipeline: lint + test → build → deploy\n");

    // Plan execution (first decompose task)
    let _effects = reducer.reduce(&mut state, OrchestratorAction::Plan {
        task: "Deploy application to staging".to_string()
    }, &env);

    // Then provide the planned subtasks
    let effects = reducer.reduce(&mut state, OrchestratorAction::Planned { subtasks }, &env);
    println!("✅ Plan created: {} subtasks", state.subtask_count());
    println!("   Ready to execute: {} effects", effects.len());

    // Simulate completions
    println!("\n🔄 Execution:");
    reducer.reduce(&mut state, OrchestratorAction::SubtaskComplete {
        subtask_id: "lint".to_string(), result: Ok("Linting OK".to_string())
    }, &env);
    println!("   ✅ lint complete");

    reducer.reduce(&mut state, OrchestratorAction::SubtaskComplete {
        subtask_id: "test".to_string(), result: Ok("Tests OK".to_string())
    }, &env);
    println!("   ✅ test complete → build ready");

    reducer.reduce(&mut state, OrchestratorAction::SubtaskComplete {
        subtask_id: "build".to_string(), result: Ok("Build OK".to_string())
    }, &env);
    println!("   ✅ build complete → deploy ready");

    reducer.reduce(&mut state, OrchestratorAction::SubtaskComplete {
        subtask_id: "deploy".to_string(), result: Ok("Deploy OK".to_string())
    }, &env);
    println!("   ✅ deploy complete");

    if state.all_subtasks_complete() {
        println!("\n🎉 All subtasks complete!");
    }

    println!("\n💡 Key Benefits:");
    println!("   • Dependency-aware execution");
    println!("   • Parallel where possible");
    println!("   • Specialized workers per task type");
}
