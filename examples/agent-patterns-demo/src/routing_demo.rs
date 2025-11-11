//! Routing Pattern Demo
//!
//! Demonstrates input classification and routing to specialist handlers.
//! Use case: Customer support ticket routing system

use composable_rust_agent_patterns::routing::{Route, RouterAction, RouterState, RoutingReducer};
use composable_rust_anthropic::Tool;
use composable_rust_core::agent::{AgentAction, AgentConfig, AgentEnvironment};
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use std::sync::Arc;

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
    println!("=== Routing Pattern Demo ===\n");
    println!("Use Case: Customer Support Ticket Routing");
    println!("Routes: Technical → Billing → Sales → General\n");

    // Define specialist handlers
    let technical_specialist = Arc::new(|input: String| {
        Box::pin(async move {
            println!("  🔧 Technical specialist processing: {}", input);
            Ok("Technical issue resolved: Reset router and update firmware.".to_string())
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
    });

    let billing_specialist = Arc::new(|input: String| {
        Box::pin(async move {
            println!("  💰 Billing specialist processing: {}", input);
            Ok("Billing inquiry handled: Invoice sent to registered email.".to_string())
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
    });

    let sales_specialist = Arc::new(|input: String| {
        Box::pin(async move {
            println!("  📈 Sales specialist processing: {}", input);
            Ok("Sales inquiry handled: Product demo scheduled for next week.".to_string())
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
    });

    // Define routes
    let routes = vec![
        Route {
            category: "technical".to_string(),
            description: "Technical support issues".to_string(),
            specialist: technical_specialist,
        },
        Route {
            category: "billing".to_string(),
            description: "Billing and payment inquiries".to_string(),
            specialist: billing_specialist,
        },
        Route {
            category: "sales".to_string(),
            description: "Sales and product inquiries".to_string(),
            specialist: sales_specialist,
        },
    ];

    let reducer = RoutingReducer::new(routes);
    let mut state = RouterState::new();
    let env = DemoEnvironment {
        config: AgentConfig::default(),
    };

    // Example 1: Technical issue
    println!("🎫 Ticket 1: \"My internet connection keeps dropping\"");
    let effects = reducer.reduce(
        &mut state,
        RouterAction::Classify {
            input: "My internet connection keeps dropping".to_string(),
        },
        &env,
    );
    println!("   Classification step initiated ({} effects)", effects.len());

    // Simulate classification result
    state = RouterState::new();
    let effects = reducer.reduce(
        &mut state,
        RouterAction::Classified {
            category: "technical".to_string(),
            input: "My internet connection keeps dropping".to_string(),
        },
        &env,
    );
    println!("   ✅ Routed to: technical");
    println!("   Specialist execution: {} effects", effects.len());

    // Example 2: Billing issue
    println!("\n🎫 Ticket 2: \"I was charged twice for my subscription\"");
    state = RouterState::new();
    let effects = reducer.reduce(
        &mut state,
        RouterAction::Classified {
            category: "billing".to_string(),
            input: "I was charged twice for my subscription".to_string(),
        },
        &env,
    );
    println!("   ✅ Routed to: billing");
    println!("   Specialist execution: {} effects", effects.len());

    // Example 3: Sales inquiry
    println!("\n🎫 Ticket 3: \"What are your enterprise pricing options?\"");
    state = RouterState::new();
    let effects = reducer.reduce(
        &mut state,
        RouterAction::Classified {
            category: "sales".to_string(),
            input: "What are your enterprise pricing options?".to_string(),
        },
        &env,
    );
    println!("   ✅ Routed to: sales");
    println!("   Specialist execution: {} effects", effects.len());

    println!("\n💡 Key Benefits:");
    println!("   • Automatic categorization");
    println!("   • Specialized handling per category");
    println!("   • Scalable specialist system");
    println!("   • Improved response quality");
}
