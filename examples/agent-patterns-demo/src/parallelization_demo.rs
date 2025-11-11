//! Parallelization Pattern Demo
//!
//! Demonstrates concurrent task execution with aggregation.
//! Use case: Multi-language document translation

use composable_rust_agent_patterns::parallelization::{
    ParallelAction, ParallelReducer, ParallelState, Task,
};
use composable_rust_anthropic::Tool;
use composable_rust_core::agent::{AgentAction, AgentConfig, AgentEnvironment};
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;

/// Mock environment
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

    fn call_claude(&self, _request: composable_rust_anthropic::MessagesRequest) -> Effect<AgentAction> {
        Effect::None
    }

    fn call_claude_streaming(&self, _request: composable_rust_anthropic::MessagesRequest) -> Effect<AgentAction> {
        Effect::None
    }

    fn execute_tool(&self, _: String, _: String, _: String) -> Effect<AgentAction> {
        Effect::None
    }

    fn execute_tool_streaming(&self, _: String, _: String, _: String) -> Effect<AgentAction> {
        Effect::None
    }
}

fn main() {
    println!("=== Parallelization Pattern Demo ===\n");
    println!("Use Case: Multi-Language Document Translation");
    println!("Tasks: Spanish, French, German, Japanese, Chinese\n");

    // Define translation tasks
    let tasks = vec![
        Task {
            id: "es".to_string(),
            input: "Translate to Spanish: 'Welcome to our platform'".to_string(),
        },
        Task {
            id: "fr".to_string(),
            input: "Translate to French: 'Welcome to our platform'".to_string(),
        },
        Task {
            id: "de".to_string(),
            input: "Translate to German: 'Welcome to our platform'".to_string(),
        },
        Task {
            id: "ja".to_string(),
            input: "Translate to Japanese: 'Welcome to our platform'".to_string(),
        },
        Task {
            id: "zh".to_string(),
            input: "Translate to Chinese: 'Welcome to our platform'".to_string(),
        },
    ];

    let reducer = ParallelReducer::new(3); // Max 3 concurrent tasks
    let mut state = ParallelState::new();
    let env = DemoEnvironment {
        config: AgentConfig::default(),
    };

    // Execute tasks in parallel
    println!("🚀 Executing {} translation tasks in parallel (max 3 concurrent)...\n", tasks.len());
    let effects = reducer.reduce(
        &mut state,
        ParallelAction::Execute { tasks: tasks.clone() },
        &env,
    );

    println!("   Tasks dispatched: {}", effects.len());
    println!("   Pending tasks: {}", state.pending_count());
    println!("   Completed tasks: {}", state.completed_count());

    // Simulate task completions
    println!("\n📥 Task completions:");

    // Spanish completes
    let effects = reducer.reduce(
        &mut state,
        ParallelAction::TaskComplete {
            task_id: "es".to_string(),
            result: Ok("Bienvenido a nuestra plataforma".to_string()),
        },
        &env,
    );
    println!("   ✅ Spanish: Completed");
    println!("      Pending: {} | Completed: {} | Effects: {}",
             state.pending_count(), state.completed_count(), effects.len());

    // French completes
    let _effects = reducer.reduce(
        &mut state,
        ParallelAction::TaskComplete {
            task_id: "fr".to_string(),
            result: Ok("Bienvenue sur notre plateforme".to_string()),
        },
        &env,
    );
    println!("   ✅ French: Completed");
    println!("      Pending: {} | Completed: {}", state.pending_count(), state.completed_count());

    // German completes
    let _effects = reducer.reduce(
        &mut state,
        ParallelAction::TaskComplete {
            task_id: "de".to_string(),
            result: Ok("Willkommen auf unserer Plattform".to_string()),
        },
        &env,
    );
    println!("   ✅ German: Completed");
    println!("      Pending: {} | Completed: {}", state.pending_count(), state.completed_count());

    // Japanese completes
    let _effects = reducer.reduce(
        &mut state,
        ParallelAction::TaskComplete {
            task_id: "ja".to_string(),
            result: Ok("私たちのプラットフォームへようこそ".to_string()),
        },
        &env,
    );
    println!("   ✅ Japanese: Completed");
    println!("      Pending: {} | Completed: {}", state.pending_count(), state.completed_count());

    // Chinese completes
    let _effects = reducer.reduce(
        &mut state,
        ParallelAction::TaskComplete {
            task_id: "zh".to_string(),
            result: Ok("欢迎来到我们的平台".to_string()),
        },
        &env,
    );
    println!("   ✅ Chinese: Completed");
    println!("      Pending: {} | Completed: {} | All done: {}",
             state.pending_count(), state.completed_count(), state.all_tasks_complete());

    // Aggregate results
    if state.all_tasks_complete() {
        println!("\n📊 Aggregating translations...");
        let effects = reducer.reduce(&mut state, ParallelAction::Aggregate, &env);
        println!("   Aggregation effect generated: {}", effects.len());
    }

    println!("\n💡 Key Benefits:");
    println!("   • Concurrent execution for speed");
    println!("   • Configurable concurrency limit");
    println!("   • Independent task failures");
    println!("   • Automatic aggregation when complete");
}
